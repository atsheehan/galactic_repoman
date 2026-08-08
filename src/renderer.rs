//! Per-window render state: swapchain, image views, pipeline, and the per-frame
//! synchronization future. Recreated on resize; the device-level [`VulkanContext`]
//! it borrows from survives across recreations.

mod context;
mod pipeline;

pub use context::VulkanContext;

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, anyhow};
use log::Level;
use vulkano::Validated;
use vulkano::VulkanError;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, RenderingAttachmentInfo, RenderingInfo,
};
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::ImageUsage;
use vulkano::image::view::ImageView;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::pipeline::graphics::viewport::Viewport;
use vulkano::render_pass::{AttachmentLoadOp, AttachmentStoreOp};
use vulkano::swapchain::{
    ColorSpace, PresentMode, Surface, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo,
    acquire_next_image,
};
use vulkano::sync::{self, GpuFuture};
use winit::window::Window;

use context::VulkanContext as Ctx;

/// The opening frames decide whether anything ever reaches the screen, so they are
/// logged at `info`; a long-running session drops to `debug` to stay readable.
const FRAMES_LOGGED_AT_INFO: u64 = 5;

pub struct Renderer {
    // Device-level handles needed every frame (clones of the shared `VulkanContext`).
    device: Arc<Device>,
    queue: Arc<Queue>,
    command_buffer_allocator: Arc<StandardCommandBufferAllocator>,

    pub window: Arc<Window>,
    swapchain: Arc<Swapchain>,
    image_views: Vec<Arc<ImageView>>,
    pipeline: Arc<GraphicsPipeline>,
    viewport: Viewport,

    pub recreate_swapchain: bool,
    previous_frame_end: Option<Box<dyn GpuFuture>>,

    // Diagnostics: draw attempts, successful presents, and when the last one landed.
    // A run whose attempts climb while presents stay flat is a very different bug from
    // one where neither moves.
    frames: u64,
    presents: u64,
    last_present: Option<Instant>,
}

impl Renderer {
    pub fn new(ctx: &Ctx, window: Arc<Window>) -> anyhow::Result<Self> {
        let surface = Surface::from_window(ctx.instance.clone(), window.clone())
            .context("creating the window surface")?;
        let window_size = window.inner_size();

        let surface_capabilities = ctx
            .physical_device
            .surface_capabilities(&surface, Default::default())
            .context("querying surface capabilities")?;

        log::info!(
            "surface capabilities: image_count {}..{:?}, current_extent {:?}, \
             extent {:?}..{:?}, current_transform {:?}, composite_alpha {:?}",
            surface_capabilities.min_image_count,
            surface_capabilities.max_image_count,
            surface_capabilities.current_extent,
            surface_capabilities.min_image_extent,
            surface_capabilities.max_image_extent,
            surface_capabilities.current_transform,
            surface_capabilities.supported_composite_alpha,
        );

        // The compositor's idea of the surface size and winit's idea of the window size
        // are supposed to agree. When they don't, we are about to create a swapchain the
        // compositor will not show.
        if let Some(current_extent) = surface_capabilities.current_extent
            && current_extent != [window_size.width, window_size.height]
        {
            log::warn!(
                "surface current_extent {current_extent:?} disagrees with window {}x{}",
                window_size.width,
                window_size.height,
            );
        }

        // Prefer the conventional sRGB swapchain format; fall back to whatever the
        // surface offers first.
        let surface_formats = ctx
            .physical_device
            .surface_formats(&surface, Default::default())
            .context("querying surface formats")?;
        log::debug!("surface formats: {surface_formats:?}");
        let (image_format, image_color_space) = surface_formats
            .iter()
            .copied()
            .find(|(format, color_space)| {
                *format == Format::B8G8R8A8_SRGB && *color_space == ColorSpace::SrgbNonLinear
            })
            .unwrap_or_else(|| surface_formats[0]);

        // One more image than the minimum reduces the chance of stalling on present,
        // clamped to the maximum the surface allows.
        let min_image_count = (surface_capabilities.min_image_count + 1)
            .min(surface_capabilities.max_image_count.unwrap_or(u32::MAX));

        let composite_alpha = surface_capabilities
            .supported_composite_alpha
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("surface reports no supported composite alpha"))?;

        log::info!(
            "creating swapchain: {}x{} (window scale {:.2}), format {image_format:?} / \
             {image_color_space:?}, {min_image_count} images, composite alpha {composite_alpha:?}",
            window_size.width,
            window_size.height,
            window.scale_factor(),
        );

        let (swapchain, images) = Swapchain::new(
            ctx.device.clone(),
            surface,
            SwapchainCreateInfo {
                min_image_count,
                image_format,
                image_color_space,
                image_extent: [window_size.width, window_size.height],
                image_usage: ImageUsage::COLOR_ATTACHMENT,
                composite_alpha,
                present_mode: PresentMode::Fifo,
                ..Default::default()
            },
        )
        .context("creating the swapchain")?;

        log_swapchain("created", &swapchain);

        let image_views = create_image_views(&images)?;
        let viewport = viewport_for(window_size.width, window_size.height);
        let pipeline = pipeline::create_pipeline(ctx.device.clone(), image_format)?;

        let previous_frame_end = Some(sync::now(ctx.device.clone()).boxed());

        Ok(Self {
            device: ctx.device.clone(),
            queue: ctx.queue.clone(),
            command_buffer_allocator: ctx.command_buffer_allocator.clone(),
            window,
            swapchain,
            image_views,
            pipeline,
            viewport,
            recreate_swapchain: false,
            previous_frame_end,
            frames: 0,
            presents: 0,
            last_present: None,
        })
    }

    /// One-line summary for the idle heartbeat and for shutdown: how much work we have
    /// actually pushed to the screen, and what the swapchain looks like right now.
    pub fn status(&self) -> String {
        let last_present = self.last_present.map_or_else(
            || "never".to_string(),
            |at| format!("{:.1}s ago", at.elapsed().as_secs_f32()),
        );

        format!(
            "{} draw attempts, {} presents (last {last_present}), swapchain {:?} \
             ({} images, {:?}), recreate_pending={}",
            self.frames,
            self.presents,
            self.swapchain.image_extent(),
            self.swapchain.image_count(),
            self.swapchain.present_mode(),
            self.recreate_swapchain,
        )
    }

    /// Draw one frame. Called on `RedrawRequested`.
    pub fn render(&mut self) -> anyhow::Result<()> {
        let window_size = self.window.inner_size();
        // Skip rendering while minimized (a zero-extent swapchain is invalid).
        if window_size.width == 0 || window_size.height == 0 {
            log::debug!("redraw skipped: window reports a zero extent");
            return Ok(());
        }

        self.frames += 1;
        let frame = self.frames;
        let level = if frame <= FRAMES_LOGGED_AT_INFO {
            Level::Info
        } else {
            Level::Debug
        };

        log::log!(
            level,
            "frame {frame}: begin, window {}x{}, swapchain {:?}, recreate_pending={}",
            window_size.width,
            window_size.height,
            self.swapchain.image_extent(),
            self.recreate_swapchain,
        );

        // Release resources held by the previous frame's GPU work that has completed.
        self.previous_frame_end.as_mut().unwrap().cleanup_finished();

        if self.recreate_swapchain {
            log::info!(
                "frame {frame}: recreating swapchain {:?} -> {}x{}",
                self.swapchain.image_extent(),
                window_size.width,
                window_size.height,
            );
            let (new_swapchain, new_images) = self
                .swapchain
                .recreate(SwapchainCreateInfo {
                    image_extent: [window_size.width, window_size.height],
                    ..self.swapchain.create_info()
                })
                .context("recreating the swapchain")?;
            self.swapchain = new_swapchain;
            self.image_views = create_image_views(&new_images)?;
            self.viewport = viewport_for(window_size.width, window_size.height);
            self.recreate_swapchain = false;
            log_swapchain("recreated", &self.swapchain);
        }

        // Drawing at an extent the compositor is not expecting is one of the ways a
        // frame gets silently dropped instead of shown.
        if self.swapchain.image_extent() != [window_size.width, window_size.height] {
            log::warn!(
                "frame {frame}: swapchain extent {:?} disagrees with window {}x{}",
                self.swapchain.image_extent(),
                window_size.width,
                window_size.height,
            );
        }

        // Logged before the call because the acquire blocks with no timeout: if this is
        // the last line a run ever produces, we were starved of swapchain images.
        log::log!(level, "frame {frame}: acquiring image");
        let (image_index, suboptimal, acquire_future) =
            match acquire_next_image(self.swapchain.clone(), None).map_err(Validated::unwrap) {
                Ok(r) => r,
                Err(VulkanError::OutOfDate) => {
                    log::info!("frame {frame}: acquire out-of-date; recreating and redrawing");
                    self.recreate_swapchain = true;
                    self.window.request_redraw();
                    return Ok(());
                }
                Err(e) => {
                    log::error!("frame {frame}: acquire failed: {e}");
                    return Err(anyhow!("acquiring next swapchain image: {e}"));
                }
            };
        log::log!(level, "frame {frame}: acquired image {image_index}");
        if suboptimal {
            log::info!("frame {frame}: image {image_index} is suboptimal; will recreate");
            self.recreate_swapchain = true;
        }

        let mut builder = AutoCommandBufferBuilder::primary(
            self.command_buffer_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
        .context("allocating command buffer")?;

        builder
            .begin_rendering(RenderingInfo {
                color_attachments: vec![Some(RenderingAttachmentInfo {
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_value: Some([0.0, 0.0, 0.0, 1.0].into()),
                    ..RenderingAttachmentInfo::image_view(
                        self.image_views[image_index as usize].clone(),
                    )
                })],
                ..Default::default()
            })
            .context("begin_rendering")?
            .set_viewport(0, [self.viewport.clone()].into_iter().collect())
            .context("set_viewport")?
            .bind_pipeline_graphics(self.pipeline.clone())
            .context("bind_pipeline_graphics")?;
        // SAFETY: 3 baked vertices, no out-of-bounds vertex/index access.
        unsafe { builder.draw(3, 1, 0, 0) }.context("draw")?;
        builder.end_rendering().context("end_rendering")?;

        let command_buffer = builder.build().context("building command buffer")?;

        // The window manager may want to know we're about to present (e.g. for frame
        // pacing); harmless if it doesn't.
        self.window.pre_present_notify();

        let future = self
            .previous_frame_end
            .take()
            .unwrap()
            .join(acquire_future)
            .then_execute(self.queue.clone(), command_buffer)
            .context("submitting command buffer")?
            .then_swapchain_present(
                self.queue.clone(),
                SwapchainPresentInfo::swapchain_image_index(self.swapchain.clone(), image_index),
            )
            .then_signal_fence_and_flush();

        match future.map_err(Validated::unwrap) {
            Ok(future) => {
                self.presents += 1;
                self.last_present = Some(Instant::now());
                log::log!(
                    level,
                    "frame {frame}: presented image {image_index} ({} presents total)",
                    self.presents,
                );
                self.previous_frame_end = Some(future.boxed());
            }
            Err(VulkanError::OutOfDate) => {
                log::info!("frame {frame}: present out-of-date; recreating");
                self.recreate_swapchain = true;
                self.previous_frame_end = Some(sync::now(self.device.clone()).boxed());
            }
            Err(e) => {
                log::error!("frame {frame}: failed to flush: {e}");
                self.previous_frame_end = Some(sync::now(self.device.clone()).boxed());
            }
        }

        // A suboptimal present or a flush error above flags the swapchain for
        // recreation; in `Wait` mode we must schedule the frame that does it.
        if self.recreate_swapchain {
            log::log!(
                level,
                "frame {frame}: redraw requested to apply the recreate"
            );
            self.window.request_redraw();
        } else {
            // Nothing else will call `render` until the compositor sends an event. If
            // this is where a black-screen run stops, the frame we just presented is
            // the only one there will ever be.
            log::log!(level, "frame {frame}: done, no further redraw scheduled");
        }

        Ok(())
    }
}

/// Report what a swapchain actually came back as, which is not always what was asked
/// for — the compositor gets the final say on extent and image count.
fn log_swapchain(what: &str, swapchain: &Arc<Swapchain>) {
    log::info!(
        "swapchain {what}: extent {:?}, {} images, format {:?} / {:?}, present mode {:?}",
        swapchain.image_extent(),
        swapchain.image_count(),
        swapchain.image_format(),
        swapchain.image_color_space(),
        swapchain.present_mode(),
    );
}

fn create_image_views(
    images: &[Arc<vulkano::image::Image>],
) -> anyhow::Result<Vec<Arc<ImageView>>> {
    images
        .iter()
        .map(|image| ImageView::new_default(image.clone()).context("creating swapchain image view"))
        .collect()
}

fn viewport_for(width: u32, height: u32) -> Viewport {
    Viewport {
        offset: [0.0, 0.0],
        extent: [width as f32, height as f32],
        depth_range: 0.0..=1.0,
    }
}
