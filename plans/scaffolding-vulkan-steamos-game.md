# Scaffolding: 2D Vulkan Game for SteamOS / Steam Machine (Rust)

## Context

`galactic_repoman` is currently a default `cargo init` (empty deps, `Hello, world!` `main.rs`,
edition 2024). We are building the **scaffolding** for a 2D game that will ship on Steam,
primarily targeting **SteamOS / the 2026 Steam Machine** (Linux/Vulkan), with other platforms
left open since it sells on Steam.

The scaffold must prove the whole stack works end-to-end before any game logic exists:

1. Open a window and initialize **Vulkan**, rendering a **colored triangle**.
2. Play a **one-shot sound effect** and **looping music** to confirm audio.
3. Read input from the **Steam Controller** via the **Steam Input action-based API**.
4. Be **publishable to Steam via SteamPipe** (steamcmd + VDF depot scripts).

### Locked decisions

| Area | Choice |
|---|---|
| Graphics | **ash** (raw Vulkan) + `ash-window` + `winit` |
| Audio | **rodio** |
| Steam | **steamworks** crate (0.13.x), **Steam Input action-based API** |
| Steam account | Real **AppID + DepotID** in hand; steamcmd not yet set up; **no IGA manifest yet** (build from scratch) |
| Structure | **Single binary crate** with modules |
| Target | SteamOS/Linux primary & test platform; keep cross-platform doors open |

## Crate stack (`Cargo.toml`)

```toml
[dependencies]
ash               = "0.38"    # raw Vulkan bindings
ash-window        = "0.13"    # surface creation from a window handle
raw-window-handle = "0.6"     # handle interop between winit and ash-window
winit             = "0.30"    # windowing + event loop (ApplicationHandler API)
rodio             = "0.19"    # audio playback (sfx + music)
steamworks        = "0.13"    # Steamworks SDK bindings incl. Input
anyhow            = "1"       # error handling for scaffold code
log               = "0.4"
env_logger        = "0.11"
```

Shaders compiled offline to SPIR-V with `glslc` (Vulkan SDK) and committed; no build-time shader
toolchain dependency. (Optional later: `shaderc` crate for build-time compilation.)

> Note: `winit 0.30` uses the `ApplicationHandler` trait (`resumed`/`window_event`/`about_to_wait`),
> not the old closure-based `run`. Plan targets this API. The `ash` repo's `examples/` directory is
> the canonical reference for the triangle (instance → device → swapchain → render pass → pipeline →
> command buffers → per-frame sync).

## Project structure

Single binary crate, modules under `src/`:

```
src/
  main.rs            # entry: init logging + Steam, run App via winit event loop
  app.rs             # App struct: implements winit ApplicationHandler, owns Renderer/Audio/Input, frame loop
  renderer/
    mod.rs           # Renderer: orchestrates draw(); resize handling
    context.rs       # VkInstance, debug messenger, physical/logical device, queues
    swapchain.rs     # surface, swapchain, image views, framebuffers, recreate-on-resize
    pipeline.rs      # render pass + graphics pipeline + shader module loading
    frame.rs         # command pool/buffers, per-frame sync (image_available / render_finished / in_flight)
  audio.rs           # Audio: rodio OutputStream, play_sfx(), start_music() (looping)
  steam.rs           # Steam: Client init, callback pump; Input: action set/handles, poll each frame
  shaders/
    triangle.vert    # gl_Position from hardcoded positions; pass vertex color
    triangle.frag    # output interpolated color
    triangle.vert.spv  triangle.frag.spv   # committed SPIR-V
assets/
  sfx.ogg            # short sound effect (placeholder)
  music.ogg          # looping track (placeholder)
  game_actions_<APPID>.vdf   # Steam Input In-Game Actions manifest
  controller_config/ ...     # action manifest + per-controller bindings (if using Action Manifest form)
```

## Implementation steps

### Step 1 — Window + Vulkan colored triangle (the bulk of the work)

Reference the **`ash` repo `examples/`** for the proven sequence; reuse `ash-window` for surface
creation rather than hand-rolling platform surface extensions.

1. `App` implements `winit::application::ApplicationHandler`:
   - `resumed`: create the `Window`, then build `Renderer` (Vulkan init happens here so we have a
     window handle for the surface).
   - `window_event`: handle `CloseRequested`, `Resized` (flag swapchain recreate), `RedrawRequested`
     (call `renderer.draw()`).
   - `about_to_wait`: `window.request_redraw()` for a continuous loop.
2. `renderer/context.rs`: create instance (enable `VK_KHR_surface` + platform surface ext via
   `ash_window::enumerate_required_extensions`); validation layers + debug messenger under
   `cfg!(debug_assertions)`; pick a physical device with graphics + present support; create logical
   device + graphics/present queues.
3. `renderer/swapchain.rs`: `ash_window::create_surface`; choose format (prefer `B8G8R8A8_SRGB`) /
   present mode (`FIFO` for vsync, good default on SteamOS/gamescope) / extent; create swapchain,
   image views, framebuffers; a `recreate()` path keyed off window resize.
4. `renderer/pipeline.rs`: render pass (single color attachment, clear→store); load `*.spv` shader
   modules; fixed-function state for a hardcoded 3-vertex triangle (no vertex buffer — positions and
   colors baked into the vertex shader, matching the classic ash example).
5. `renderer/frame.rs`: command pool + buffers; record clear + draw(3,1,0,0); per-frame semaphores +
   fence with `MAX_FRAMES_IN_FLIGHT = 2`; standard acquire→submit→present, handling
   `ERROR_OUT_OF_DATE_KHR`/`SUBOPTIMAL` by recreating the swapchain.
6. Proper `Drop` ordering: `device_wait_idle` then destroy in reverse creation order (this is the
   most bug-prone part of raw Vulkan — call it out in review).

**Exit criteria:** a window showing a colored (per-vertex interpolated) triangle, resizable without
validation errors, clean exit.

### Step 2 — Audio (rodio)

`audio.rs`: hold the `OutputStream` + `OutputStreamHandle` for the process lifetime (dropping the
stream stops audio). `play_sfx()` decodes `assets/sfx.ogg` into a `Sink` (fire-and-forget).
`start_music()` plays `assets/music.ogg` wrapped in `.repeat_infinite()` on a dedicated `Sink` kept
alive in the `Audio` struct. Trigger `start_music()` at startup and `play_sfx()` on a Steam Controller
button press (ties Step 3 together). Placeholder royalty-free `.ogg` assets committed under `assets/`.

### Step 3 — Steam Controller via Steam Input (action-based API)

1. **Steam client init** (`steam.rs`): `steamworks::Client::init_app(<APPID>)` (or `init()` reading
   `steam_appid.txt`). Place `steam_appid.txt` containing the AppID next to the binary for local runs,
   and the redistributable **`libsteam_api.so`** (from the Steamworks SDK
   `redistributable_bin/linux64/`) on the library path — the `steamworks` crate loads the SDK
   **dynamically**, it is not vendored. Pump callbacks once per frame (`client.run_callbacks()`).
2. **Build the IGA / action manifest from scratch** (none exists yet):
   - Author `game_actions_<APPID>.vdf` defining one action set (e.g. `InGameControls`) with a digital
     action (e.g. `fire` → plays the sfx) and an analog action (e.g. `Move`). Add a localization block.
   - For local dev, call `Input::set_input_action_manifest_file_path()` pointing at the manifest so we
     can test from `cargo run` without the game installed through Steam. For shipping, bundle the
     manifest in the depot and register its path in Steamworks partner settings (Custom Configuration —
     Bundled with game).
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

### Step 4 — SteamPipe publishing (steamcmd) — set up from scratch

1. Download **steamcmd** + the **Steamworks SDK ContentBuilder** tooling (not yet installed).
2. Create depot/build VDF scripts (keep under `ci/steampipe/`, do **not** commit credentials):
   - `app_build_<APPID>.vdf`: references the AppID, `Desc`, `ContentRoot` (points at a staged build
     dir), `BuildOutput`, and a `depots { <DEPOTID> "depot_build_<DEPOTID>.vdf" }` mapping.
   - `depot_build_<DEPOTID>.vdf`: `DepotID`, `FileMapping` (`LocalPath "*"`, `DepotPath "."`,
     `recursive 1`), `FileExclusion` for `*.pdb`/debug junk.
3. Stage a release build: `cargo build --release`, copy the binary + `assets/` + shaders' `.spv` +
   `libsteam_api.so` + `steam_appid.txt` into the content root.
4. Upload: `steamcmd +login <builder_account> +run_app_build ../scripts/app_build_<APPID>.vdf +quit`,
   then set the build live on a beta branch from the Steamworks partner site.
5. A short `scripts/package.sh` (or `xtask`) to automate staging is in scope; CI wiring (GameCI /
   GitHub Actions) is noted as a follow-up, not built now.

> **SteamOS runtime note:** for reliable execution under the Steam Linux Runtime (Sniper) container,
> plan to build against / test inside the Sniper SDK image so glibc/Vulkan loader deps match. Flagged
> as a hardening follow-up; the initial scaffold targets the local SteamOS/Linux dev machine.

## Key files to create / modify

- `Cargo.toml` — add the dependency stack above.
- `src/main.rs` — replace `Hello, world!`; init logging, Steam client, run winit app.
- `src/app.rs`, `src/renderer/*`, `src/audio.rs`, `src/steam.rs` — new modules per structure above.
- `src/shaders/triangle.{vert,frag}` + committed `.spv`.
- `assets/sfx.ogg`, `assets/music.ogg`, `assets/game_actions_<APPID>.vdf`.
- `ci/steampipe/app_build_<APPID>.vdf`, `ci/steampipe/depot_build_<DEPOTID>.vdf`, `scripts/package.sh`.
- `.gitignore` — add `steam_appid.txt`, SteamPipe `output/` logs, vendored SDK binaries, credentials.

## Verification

1. **Graphics:** `cargo run` → window with a colored triangle; resize the window (no validation-layer
   errors in debug); close cleanly. Validation layers active in debug builds catch most Vulkan misuse.
2. **Audio:** music loops from launch; the sfx fires on the mapped controller button.
3. **Steam Input:** with Steam running (or the manifest path overridden for dev), connect a Steam
   Controller / Steam Deck pad → button triggers sfx, analog stick logs non-zero values. Confirms the
   action-based path and the IGA manifest are wired.
4. **SteamPipe:** run `steamcmd ... +run_app_build` against the real AppID/DepotID → build appears in
   the Steamworks partner "Builds" page; set it live on a test branch and install via Steam on the
   SteamOS box to confirm it launches and the triangle/audio/input all work in the shipped form.

## Risks / call-outs

- **Raw Vulkan teardown** (`Drop` order + `device_wait_idle`) and **swapchain recreation** are the
  highest-bug-density areas — lean on validation layers and the `ash` examples.
- **`steamworks` Input coverage:** verify 0.13.x method names against docs.rs before implementing;
  fall back to the `ISteamInput` C++ semantics where docs are thin.
- **SDK not vendored:** `libsteam_api.so` + `steam_appid.txt` must ship beside the binary or Steam
  init fails — bake this into the packaging step.
- **Sniper runtime parity** for SteamOS is a follow-up hardening item, noted above.
