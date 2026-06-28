# Step 1 (detailed) — Window + Vulkan colored triangle (vulkano, dynamic rendering)

## Context

`galactic_repoman` is a fresh `cargo init` (empty deps, `Hello, world!`, edition 2024). The
parent plan (`plans/scaffolding-vulkan-steamos-game.md`) scaffolds a 2D Vulkan game for
SteamOS/Steam Machine. **Step 1 is the bulk of the work**: prove the graphics stack end-to-end
by opening a window, initializing Vulkan, and drawing a per-vertex-interpolated colored triangle
that resizes cleanly and exits without errors.

**Two decisions change from the parent plan** (both user-approved):

1. **Graphics library: `vulkano` instead of `ash`.** `vulkano` is a safe, high-level Rust wrapper
   that _owns_ the highest-bug-density parts of the parent plan — object-lifetime/`Drop` teardown,
   per-frame synchronization, and memory allocation — via `Arc` and its `GpuFuture` system. For a 2D
   game whose Step-1 cost is mostly scaffolding, this removes the code most likely to ship bugs. The
   trade-off: we bind to vulkano's abstractions (less 1:1 with C++ Vulkan tutorials) and its version
   churn. This supersedes the parent plan's `ash` + `ash-window` + `raw-window-handle` row.
2. **Dynamic rendering (Vulkan 1.3)** instead of render pass + framebuffers — no `RenderPass`/
   `Framebuffer`/subpass objects. Fully supported by vulkano and by RADV/Mesa on SteamOS.

**Canonical reference:** the upstream **`vulkano-rs/vulkano` `examples/triangle-v1_3/`** — a
dynamic-rendering colored triangle using the winit `ApplicationHandler` trait. Mirror its structure.

**Local toolchain verified ready:** Vulkan loader (`libvulkan.so.1`), `vulkaninfo`, AMD/`radeon`
GPU. (`glslc` is present but no longer needed for shaders — see below.) Audio (rodio) and Steam
(steamworks) deps are deferred to their own steps; this step adds only the graphics stack.

## Cargo.toml

```toml
[dependencies]
vulkano = "0.35" # safe Vulkan wrapper (instance/device/swapchain/sync/alloc)
vulkano-shaders = "0.35" # build-time GLSL→SPIR-V macro, generates typed shader bindings
winit = "0.30" # windowing + event loop (ApplicationHandler)
anyhow = "1"
log = "0.4"
env_logger = "0.11"
```

Dropped vs. parent plan: `ash`, `ash-window`, `raw-window-handle` (vulkano handles surface creation
and handle interop internally).

> **Build-time note:** `vulkano-shaders` compiles GLSL at build time via the `shaderc` crate, which
> builds/links `libshaderc` (needs a C++ toolchain; may fetch on first build). This _reverses_ the
> parent plan's "compile offline with glslc, commit `.spv`, no build-time shader toolchain" decision
> — committed `.spv` files are no longer used. Flag for the Sniper/CI hardening follow-up.

## Files to create

```
src/main.rs                 # replace Hello world: init env_logger, build EventLoop, run App
src/app.rs                  # App: winit ApplicationHandler; owns device-level Vulkan state + Option<Renderer>
src/renderer.rs             # Renderer: per-window render state (swapchain/images/pipeline/viewport) + render(); declares `mod context; mod pipeline;`
src/renderer/context.rs     # device-level init: library/instance/device/queue selection + allocators
src/renderer/pipeline.rs    # graphics pipeline (dynamic rendering) + vulkano_shaders shader modules
src/shaders/triangle.vert   # GLSL, referenced by vulkano_shaders::shader!{ path: ... } (no committed .spv)
src/shaders/triangle.frag
```

## Structure (mirrors `triangle-v1_3`: device-level vs per-window split)

**`App` owns the device-level state** (created once, survives resize): `Arc<Instance>`,
`Arc<Device>`, `Arc<Queue>`, and the three allocators. It also holds `Option<Renderer>`.

**`Renderer` owns the per-window / resize-coupled state**: `Arc<Window>`, `Arc<Swapchain>` + its
`Vec<Arc<Image>>`/image views, the `Arc<GraphicsPipeline>`, a cached `Viewport`, a
`recreate_swapchain: bool` flag, and `previous_frame_end: Option<Box<dyn GpuFuture>>`.

### `main.rs`

`env_logger::init()`; `EventLoop::new()?`; `event_loop.set_control_flow(ControlFlow::Poll)`;
`event_loop.run_app(&mut App::new(&event_loop)?)`. `App::new` does device-level init (it needs the
event loop to query `Surface::required_extensions`).

### `renderer/context.rs` — device-level init (called from `App::new`)

- `VulkanLibrary::new()?` → `Instance::new(library, InstanceCreateInfo { enabled_extensions:
  Surface::required_extensions(event_loop)?, flags: InstanceCreateFlags::ENUMERATE_PORTABILITY, .. })`.
- Choose physical device: require **Vulkan 1.3 OR the `khr_dynamic_rendering` extension**, plus
  `khr_swapchain`, and a queue family with graphics support (present support is checked later against
  the surface; on SteamOS the graphics family presents). Prefer discrete > integrated.
- `Device::new(physical_device, DeviceCreateInfo { enabled_extensions: DeviceExtensions {
  khr_swapchain: true, .. }, enabled_features: DeviceFeatures { dynamic_rendering: true, .. },
  queue_create_infos, .. })?` → take the single `Arc<Queue>`.
- Build allocators (all `Arc`): `StandardMemoryAllocator`, `StandardCommandBufferAllocator`,
  `StandardDescriptorSetAllocator` (the descriptor allocator is unused in Step 1 but standard to set
  up now).

### `renderer/pipeline.rs` — shaders + pipeline (dynamic rendering)

- Shaders via the macro, one module each, GLSL kept in `src/shaders/`:
  ```rust
  mod vs { vulkano_shaders::shader!{ ty: "vertex",   path: "src/shaders/triangle.vert" } }
  mod fs { vulkano_shaders::shader!{ ty: "fragment", path: "src/shaders/triangle.frag" } }
  ```
  Each generates `load(device) -> Result<Arc<ShaderModule>>`. Get entry points via
  `.entry_point("main")`.
- Build `GraphicsPipeline::new(device, None, GraphicsPipelineCreateInfo { .. })`:
  - `stages`: vertex + fragment `PipelineShaderStageCreateInfo`.
  - `vertex_input_state`: **empty** (`VertexInputState::default()`) — positions+colors are baked into
    the vertex shader, indexed by `gl_VertexIndex`. No vertex buffer.
  - `input_assembly_state`: triangle list (default).
  - `viewport_state`: default; declare `DynamicState::Viewport` in `dynamic_state` so resize never
    rebuilds the pipeline (set the viewport per-frame).
  - `rasterization_state`: default (cull none is fine for Step 1).
  - `multisample_state`, `color_blend_state`: one attachment, no blend.
  - **Dynamic-rendering hookup (the key bit):** `subpass:
    Some(PipelineRenderingCreateInfo { color_attachment_formats: vec![Some(swapchain_image_format)],
    .. }.into())` — this replaces a render pass/subpass. The pipeline must be (re)built knowing the
    swapchain format; rebuild it inside swapchain (re)creation, or pass the format in.

### `renderer.rs` — `Renderer` (swapchain + frame loop)

- `Renderer::new(app_state, window)`:
  - `Surface::from_window(instance.clone(), window.clone())?`.
  - Query `physical_device.surface_capabilities/surface_formats`; pick format (prefer
    `B8G8R8A8_SRGB` + `SrgbNonLinear`) and `min_image_count` (`caps.min_image_count + 1`, clamped to
    `max_image_count`).
  - `Swapchain::new(device, surface, SwapchainCreateInfo { min_image_count, image_format,
    image_extent: window.inner_size().into(), image_usage: ImageUsage::COLOR_ATTACHMENT,
    composite_alpha, present_mode: Fifo, .. })?` → `(swapchain, images)`.
  - `ImageView::new_default(image)` for each swapchain image (cache the views).
  - Build the pipeline (pipeline.rs) with the chosen image format; cache the `Viewport`.
  - `previous_frame_end = Some(sync::now(device).boxed())`.
- `render()` (called on `RedrawRequested`):
  1. Skip if window extent is zero (minimized).
  2. `previous_frame_end.as_mut().unwrap().cleanup_finished()`.
  3. If `recreate_swapchain`: `swapchain.recreate(SwapchainCreateInfo { image_extent, ..current })?`,
     rebuild image views + viewport (+ pipeline if format changed), clear the flag.
  4. `acquire_next_image(swapchain, None)?` → `(image_index, suboptimal, acquire_future)`; if
     `suboptimal`, set `recreate_swapchain = true`.
  5. Record with `AutoCommandBufferBuilder::primary(cb_allocator, queue.queue_family_index(),
     CommandBufferUsage::OneTimeSubmit)?`:
     - `.begin_rendering(RenderingInfo { color_attachments: vec![Some(RenderingAttachmentInfo {
       load_op: AttachmentLoadOp::Clear, store_op: AttachmentStoreOp::Store, clear_value:
       Some([r,g,b,a].into()), ..RenderingAttachmentInfo::image_view(views[image_index].clone())
       })], .. })?`
     - `.set_viewport(0, [viewport.clone()].into())?`
     - `.bind_pipeline_graphics(pipeline.clone())?`
     - `.draw(3, 1, 0, 0)?` (3 vertices, baked in the shader)
     - `.end_rendering()?`; `let cb = builder.build()?;`
  6. Chain the future:
     ```
     previous_frame_end.take().unwrap()
        .join(acquire_future)
        .then_execute(queue, cb)?
        .then_swapchain_present(queue,
            SwapchainPresentInfo::swapchain_image_index(swapchain, image_index))
        .then_signal_fence_and_flush()
     ```
     On `Ok(future)` store it as `previous_frame_end`. On `Err(VulkanError::OutOfDate)` set
     `recreate_swapchain = true` and reset `previous_frame_end = sync::now(...)`. On other errors,
     log and reset the future.

### `app.rs` — `App: ApplicationHandler`

- `resumed`: guard against refire; create the window (`Window::default_attributes().with_title(..)
  .with_inner_size(LogicalSize::new(1280, 720))`, wrapped in `Arc`); build `Renderer::new(.., window)`
  into `self.renderer`. (All swapchain/surface creation lives in `Renderer`.)
- `window_event`: `CloseRequested`/`Escape` → `event_loop.exit()`; `Resized` →
  `renderer.recreate_swapchain = true`; `RedrawRequested` → `renderer.render()` (log + exit on error).
- `about_to_wait`: `renderer.window.request_redraw()` — continuous loop.

> **No manual teardown / sync code.** vulkano objects are `Arc`-managed and drop in dependency order;
> `previous_frame_end` + `then_signal_fence_and_flush` handle per-frame sync. This is the whole reason
> for the vulkano switch — the parent plan's "Drop teardown order" and "per-frame sync object" sections
> do not apply.

## Shaders (compiled at build time by `vulkano-shaders`; no committed `.spv`)

`src/shaders/triangle.vert` — three baked positions + colors indexed by `gl_VertexIndex`, output
`fragColor` (`#version 450`). `src/shaders/triangle.frag` — `outColor = vec4(fragColor, 1.0)`. No
vertex buffer, no descriptors. The `shader!` macro rebuilds these automatically on change.

## Verification

1. `cargo run` → window shows a colored (red/green/blue-interpolated) triangle.
2. Resize repeatedly incl. maximize/restore and minimize/restore — triangle rescales, no errors, no
   crash on zero-extent (the minimized-skip guard).
3. Close via window button or Escape → clean exit (no panics; vulkano `Drop` handles teardown).
4. If init fails, sanity-check the GPU/loader with `vulkaninfo | head`.
5. First build will compile `shaderc` (C++ toolchain) — confirm the build completes; note the added
   build dependency for the CI/Sniper follow-up.

## Verify against docs.rs / the `triangle-v1_3` example during implementation (vulkano 0.35 churns)

- `Instance::new` / `InstanceCreateInfo` field names and `Surface::required_extensions(event_loop)`
  signature in 0.35.
- `DeviceFeatures { dynamic_rendering }` and the physical-device "1.3 or `khr_dynamic_rendering`"
  filter.
- `PipelineRenderingCreateInfo { color_attachment_formats }` → `.into()` for the pipeline `subpass`
  field, and `GraphicsPipelineCreateInfo` field names.
- `RenderingInfo` / `RenderingAttachmentInfo::image_view` / `AttachmentLoadOp`/`AttachmentStoreOp`.
- `acquire_next_image` return tuple `(u32, bool, SwapchainAcquireFuture)` and
  `SwapchainPresentInfo::swapchain_image_index`.
- `AutoCommandBufferBuilder` method names (`begin_rendering`/`set_viewport`/`bind_pipeline_graphics`/
  `draw`/`end_rendering`) and the `GpuFuture` chain (`then_execute`/`then_swapchain_present`/
  `then_signal_fence_and_flush`/`cleanup_finished`).
- `vulkano_shaders::shader!{ path: .. }` relative-path resolution (relative to crate root).
