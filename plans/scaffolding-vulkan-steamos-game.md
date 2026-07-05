# Scaffolding: 2D Vulkan Game for SteamOS / Steam Machine (Rust)

## Context

`galactic_repoman` is currently a default `cargo init` (empty deps, `Hello, world!` `main.rs`,
edition 2024). We are building the **scaffolding** for a 2D game that will ship on Steam,
primarily targeting **SteamOS / the 2026 Steam Machine** (Linux/Vulkan), with other platforms
left open since it sells on Steam.

The scaffold must prove the whole stack works end-to-end before any game logic exists:

1. Open a window and initialize **Vulkan**, rendering a **colored triangle**.
2. Be **publishable to Steam via SteamPipe** (steamcmd + VDF depot scripts).
3. Play a **one-shot sound effect** and **looping music** to confirm audio.
4. Read input from the **Steam Controller** via the **Steam Input action-based API**.

> **Ordering note:** SteamPipe publishing (Step 2) is deliberately sequenced *before* audio and
> input. Publishing is a pure content-delivery step with no dependency on those runtime features —
> proving it (and the SteamOS/Sniper runtime path) early de-risks the highest-unknown infrastructure
> before the two most code-heavy steps. The only coupling runs the other way: Steam Input's IGA
> manifest eventually ships *inside* the depot, but you don't need input to publish a depot.

### Locked decisions

| Area          | Choice                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| Graphics      | **vulkano** (safe Vulkan wrapper, dynamic rendering) + `winit`                                          |
| Audio         | **rodio**                                                                                               |
| Steam         | **steamworks** crate (0.13.x), **Steam Input action-based API**                                         |
| Steam account | Real **AppID + DepotID** in hand; steamcmd not yet set up; **no IGA manifest yet** (build from scratch) |
| Structure     | **Single binary crate** with modules                                                                    |
| Target        | SteamOS/Linux primary & test platform; keep cross-platform doors open                                   |

## Crate stack (`Cargo.toml`)

```toml
[dependencies]
vulkano = "0.35" # safe Vulkan wrapper (instance/device/swapchain/sync/alloc)
vulkano-shaders = "0.35" # build-time GLSL→SPIR-V macro, generates typed shader bindings
winit = "0.30" # windowing + event loop (ApplicationHandler API)
rodio = "0.19" # audio playback (sfx + music)
steamworks = "0.13" # Steamworks SDK bindings incl. Input
anyhow = "1" # error handling for scaffold code
log = "0.4"
env_logger = "0.11"
```

Shaders are compiled **at build time** by `vulkano-shaders` (which pulls the `shaderc` crate, needing
a C++ toolchain that may fetch on first build); no `.spv` files are committed. This reverses the
earlier "compile offline with `glslc`, commit `.spv`, no build-time toolchain" decision — flagged for
the Sniper/CI hardening follow-up.

> Note: `winit 0.30` uses the `ApplicationHandler` trait (`resumed`/`window_event`/`about_to_wait`),
> not the old closure-based `run`. Plan targets this API. The canonical reference for the triangle is
> the upstream **`vulkano-rs/vulkano` `examples/triangle-v1_3/`** — a dynamic-rendering colored
> triangle on the winit `ApplicationHandler` trait (library → instance → device → swapchain →
> pipeline → `GpuFuture` frame chain).

## Project structure

Single binary crate, modules under `src/` (per CLAUDE.md, modules use `src/renderer.rs`, not
`renderer/mod.rs`):

```
src/
  main.rs            # entry: init logging, run App via winit event loop
  app.rs             # App: winit ApplicationHandler; owns device-level Vulkan state + Option<Renderer>
  renderer.rs        # Renderer: per-window state (swapchain/images/pipeline/viewport) + render(); declares `mod context; mod pipeline;`
    renderer/
      context.rs     # device-level init: library/instance/device/queue selection + allocators
      pipeline.rs    # graphics pipeline (dynamic rendering) + vulkano_shaders shader modules
  audio.rs           # Audio: rodio OutputStream, play_sfx(), start_music() (looping)
  steam.rs           # Steam: Client init, callback pump; Input: action set/handles, poll each frame
  shaders/
    triangle.vert    # GLSL, referenced by vulkano_shaders::shader!{ path: ... } (no committed .spv)
    triangle.frag    # output interpolated color
assets/
  sfx.ogg            # short sound effect (placeholder)
  music.ogg          # looping track (placeholder)
  game_actions_<APPID>.vdf   # Steam Input In-Game Actions manifest
  controller_config/ ...     # action manifest + per-controller bindings (if using Action Manifest form)
```

## Implementation steps

### Step 1 — Window + Vulkan colored triangle (the bulk of the work)

Drawn with **vulkano** (safe wrapper) using **dynamic rendering (Vulkan 1.3)** — no
`RenderPass`/`Framebuffer`/subpass objects. Shape:

1. `App` implements `winit::application::ApplicationHandler` and owns the **device-level state**
   created once and surviving resize (`Arc<Instance>`, `Arc<Device>`, `Arc<Queue>`, allocators) plus
   `Option<Renderer>`:
   - `resumed`: create the `Window`, then build `Renderer` (surface/swapchain creation needs the
     window handle).
   - `window_event`: handle `CloseRequested`/`Escape` (exit), `Resized` (set `recreate_swapchain`),
     `RedrawRequested` (call `renderer.render()`).
   - `about_to_wait`: `window.request_redraw()` for a continuous loop.
2. `renderer/context.rs`: `VulkanLibrary::new()` → `Instance::new` (extensions from
   `Surface::required_extensions`); pick a physical device requiring **Vulkan 1.3 or
   `khr_dynamic_rendering`** + `khr_swapchain` + a graphics queue family; `Device::new` with
   `dynamic_rendering` feature; build the standard memory/command-buffer/descriptor allocators.
3. `renderer.rs`: `Surface::from_window`; choose format (prefer `B8G8R8A8_SRGB`) / present mode
   (`Fifo` for vsync, good on SteamOS/gamescope) / extent; create swapchain + image views; a
   `recreate_swapchain` path keyed off window resize; hold `previous_frame_end: Option<Box<dyn
   GpuFuture>>`.
4. `renderer/pipeline.rs`: `vulkano_shaders::shader!` modules for the `.vert`/`.frag`; build a
   `GraphicsPipeline` with empty vertex input (positions+colors baked in the vertex shader, indexed by
   `gl_VertexIndex`), dynamic viewport, and **dynamic-rendering hookup** via
   `PipelineRenderingCreateInfo { color_attachment_formats }` (replaces a render pass).
5. `render()`: record an `AutoCommandBufferBuilder` with `begin_rendering` (clear→store) →
   `set_viewport` → `bind_pipeline_graphics` → `draw(3,1,0,0)` → `end_rendering`; chain
   `previous_frame_end.join(acquire).then_execute(...).then_swapchain_present(...).then_signal_fence_and_flush()`;
   handle `OutOfDate`/suboptimal by setting `recreate_swapchain`.
6. **No manual teardown or per-frame sync objects** — vulkano `Arc`-managed objects drop in
   dependency order and the `GpuFuture` chain owns synchronization. (This is the whole reason for the
   vulkano switch; the old raw-Vulkan "Drop order" / "per-frame sync" concerns no longer apply.)

> See `scaffolding-vulkan-steamos-game/step-1-window-vulkan-triangle.md` for the detailed Step 1
> design (module-by-module, vulkano 0.35 API specifics).

**Exit criteria:** a window showing a colored (per-vertex interpolated) triangle, resizable without
validation errors, clean exit.

### Step 2 — SteamPipe publishing (steamcmd) — set up from scratch

Sequenced here (before audio/input) to prove the content-delivery + SteamOS runtime path early.

> **First-publish note:** the initial publish stages a **triangle-only** release build. At this point
> the binary has **no Steam integration**, so `libsteam_api.so` / `steam_appid.txt` / the IGA manifest
> are **not yet** in the content root. They get layered into these same depot scripts as Audio
> (Step 3) and Steam Input (Step 4) land — each later step re-publishes a richer build.

1. Download **steamcmd** + the **Steamworks SDK ContentBuilder** tooling (not yet installed).
2. Create depot/build VDF scripts (keep under `ci/steampipe/`, do **not** commit credentials):
   - `app_build_<APPID>.vdf`: references the AppID, `Desc`, `ContentRoot` (points at a staged build
     dir), `BuildOutput`, and a `depots { <DEPOTID> "depot_build_<DEPOTID>.vdf" }` mapping.
   - `depot_build_<DEPOTID>.vdf`: `DepotID`, `FileMapping` (`LocalPath "*"`, `DepotPath "."`,
     `recursive 1`), `FileExclusion` for `*.pdb`/debug junk.
3. Stage a release build: `cargo build --release`, copy the binary + shaders' assets into the content
   root. (Add `libsteam_api.so` + `steam_appid.txt` once Steam init lands in Step 4; add `assets/` +
   the IGA manifest as Steps 3–4 land.)
4. Upload: `steamcmd +login <builder_account> +run_app_build ../scripts/app_build_<APPID>.vdf +quit`,
   then set the build live on a beta branch from the Steamworks partner site.
5. A short `scripts/package.sh` (or `xtask`) to automate staging is in scope; CI wiring (GameCI /
   GitHub Actions) is noted as a follow-up, not built now.

> **SteamOS runtime note:** for reliable execution under the Steam Linux Runtime (Sniper) container,
> plan to build against / test inside the Sniper SDK image so glibc/Vulkan loader deps match. Flagged
> as a hardening follow-up; the initial scaffold targets the local SteamOS/Linux dev machine.

**Exit criteria:** the triangle-only build appears in the Steamworks "Builds" page, goes live on a
test branch, and installs + launches via Steam on the SteamOS box.

### Step 3 — Audio (rodio)

`audio.rs`: hold the `OutputStream` + `OutputStreamHandle` for the process lifetime (dropping the
stream stops audio). `play_sfx()` decodes `assets/sfx.ogg` into a `Sink` (fire-and-forget).
`start_music()` plays `assets/music.ogg` wrapped in `.repeat_infinite()` on a dedicated `Sink` kept
alive in the `Audio` struct. Trigger `start_music()` at startup and `play_sfx()` on a Steam Controller
button press (ties Step 4 together). Placeholder royalty-free `.ogg` assets committed under `assets/`.

### Step 4 — Steam Controller via Steam Input (action-based API)

1. **Steam client init** (`steam.rs`): `steamworks::Client::init_app(<APPID>)` (or `init()` reading
   `steam_appid.txt`). Place `steam_appid.txt` containing the AppID next to the binary for local runs,
   and the redistributable **`libsteam_api.so`** (from the Steamworks SDK
   `redistributable_bin/linux64/`) on the library path — the `steamworks` crate loads the SDK
   **dynamically**, it is not vendored. Pump callbacks once per frame (`client.run_callbacks()`).
2. **Build the IGA / action manifest from scratch** (none exists yet):
   - Author `game_actions_<APPID>.vdf` defining one action set (e.g. `InGameControls`) with a digital
     action (e.g. `fire` → plays the sfx) and an analog action (e.g. `Move`). Add a localization block.
   - For local dev, call `Input::set_input_action_manifest_file_path()` pointing at the manifest so we
     can test from `cargo run` without the game installed through Steam. For shipping, add the manifest
     to the **existing depot scripts from Step 2** and register its path in Steamworks partner settings
     (Custom Configuration — Bundled with game).
3. **Poll input each frame** via the `steamworks` `Input` interface:
   - `input.init()`, then per frame: `run_frame()`, enumerate connected controllers, `activate_action_set`
     for our set, fetch `get_digital_action_handle`/`get_analog_action_handle` (cache the handles), read
     `get_digital_action_data` / `get_analog_action_data`.
   - On the `fire` digital action edge → `audio.play_sfx()`. Log analog `Move` values. This proves the
     2026 Steam Controller path end-to-end (action-based, so it survives user remapping + works across
     controller types).
   - Verify exact method names against `docs.rs/steamworks` 0.13.x during implementation (the crate is
     ~50% doc-covered; cross-reference the `ISteamInput` C++ reference for semantics).

**Exit criteria:** pressing the mapped Steam Controller button plays the sfx; analog stick logs values.

## Key files to create / modify

- `Cargo.toml` — add the dependency stack above (vulkano + vulkano-shaders, winit, rodio, steamworks).
- `src/main.rs` — replace `Hello, world!`; init logging, run winit app.
- `src/app.rs`, `src/renderer.rs`, `src/renderer/*`, `src/audio.rs`, `src/steam.rs` — new modules per
  structure above.
- `src/shaders/triangle.{vert,frag}` (GLSL; compiled at build time by `vulkano-shaders`, no `.spv`).
- `assets/sfx.ogg`, `assets/music.ogg`, `assets/game_actions_<APPID>.vdf`.
- `ci/steampipe/app_build_<APPID>.vdf`, `ci/steampipe/depot_build_<DEPOTID>.vdf`, `scripts/package.sh`.
- `.gitignore` — add `steam_appid.txt`, SteamPipe `output/` logs, vendored SDK binaries, credentials.

## Verification

1. **Graphics:** `cargo run` → window with a colored triangle; resize the window (no validation-layer
   errors in debug); close cleanly. (First build compiles `shaderc` — confirm it completes.)
2. **SteamPipe:** run `steamcmd ... +run_app_build` against the real AppID/DepotID → the triangle-only
   build appears in the Steamworks partner "Builds" page; set it live on a test branch and install via
   Steam on the SteamOS box to confirm it launches and the triangle renders in the shipped form.
3. **Audio:** music loops from launch; the sfx fires on the mapped controller button.
4. **Steam Input:** with Steam running (or the manifest path overridden for dev), connect a Steam
   Controller / Steam Deck pad → button triggers sfx, analog stick logs non-zero values. Confirms the
   action-based path and the IGA manifest are wired.

## Risks / call-outs

- **vulkano version churn:** 0.35 APIs move between releases — verify field/method names against
  docs.rs and the `triangle-v1_3` example during implementation. (vulkano *owns* the old raw-Vulkan
  bug hotspots — `Drop` teardown order and per-frame sync — so those are no longer top concerns.)
- **`shaderc` build dependency:** build-time GLSL compilation needs a C++ toolchain and may fetch on
  first build; bake this into the Sniper/CI hardening follow-up.
- **`steamworks` Input coverage:** verify 0.13.x method names against docs.rs before implementing;
  fall back to the `ISteamInput` C++ semantics where docs are thin.
- **SDK not vendored:** `libsteam_api.so` + `steam_appid.txt` must ship beside the binary or Steam
  init fails — bake this into the packaging step (added in Step 4).
- **Sniper runtime parity** for SteamOS is a follow-up hardening item, noted above.
