//! Per-window render state: swapchain, image views, pipeline, and the per-frame
//! synchronization future. Recreated on resize; the device-level [`VulkanContext`]
//! it borrows from survives across recreations.

mod context;
mod pipeline;

pub use context::VulkanContext;

use std::sync::Arc;

use anyhow::{Context, anyhow};
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

        // Prefer the conventional sRGB swapchain format; fall back to whatever the
        // surface offers first.
        let surface_formats = ctx
            .physical_device
            .surface_formats(&surface, Default::default())
            .context("querying surface formats")?;
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
        })
    }

    /// Draw one frame. Called on `RedrawRequested`.
    pub fn render(&mut self) -> anyhow::Result<()> {
        let window_size = self.window.inner_size();
        // Skip rendering while minimized (a zero-extent swapchain is invalid).
        if window_size.width == 0 || window_size.height == 0 {
            return Ok(());
        }

        // Release resources held by the previous frame's GPU work that has completed.
        self.previous_frame_end.as_mut().unwrap().cleanup_finished();

        if self.recreate_swapchain {
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
        }

        let (image_index, suboptimal, acquire_future) =
            match acquire_next_image(self.swapchain.clone(), None).map_err(Validated::unwrap) {
                Ok(r) => r,
                Err(VulkanError::OutOfDate) => {
                    self.recreate_swapchain = true;
                    self.window.request_redraw();
                    return Ok(());
                }
                Err(e) => return Err(anyhow!("acquiring next swapchain image: {e}")),
            };
        if suboptimal {
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
                self.previous_frame_end = Some(future.boxed());
            }
            Err(VulkanError::OutOfDate) => {
                self.recreate_swapchain = true;
                self.previous_frame_end = Some(sync::now(self.device.clone()).boxed());
            }
            Err(e) => {
                log::error!("failed to flush frame: {e}");
                self.previous_frame_end = Some(sync::now(self.device.clone()).boxed());
            }
        }

        // A suboptimal present or a flush error above flags the swapchain for
        // recreation; in `Wait` mode we must schedule the frame that does it.
        if self.recreate_swapchain {
            self.window.request_redraw();
        }

        Ok(())
    }
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
