//! winit `ApplicationHandler`: owns the device-level [`VulkanContext`] and the
//! per-window [`Renderer`]. The window (and thus the renderer) is created on
//! `resumed`, as winit requires.

use std::sync::Arc;

use anyhow::Context;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::renderer::{Renderer, VulkanContext};

/// Step 3a: a fixed tilt, so the push constant path can be proved on its own before
/// a clock (3b) or input (3c) can be blamed for a triangle that will not move.
const ANGLE: f32 = 0.5;

pub struct App {
    context: VulkanContext,
    renderer: Option<Renderer>,
}

impl App {
    pub fn new(event_loop: &EventLoop<()>) -> anyhow::Result<Self> {
        Ok(Self {
            context: VulkanContext::new(event_loop)?,
            renderer: None,
        })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // `resumed` can fire more than once; only build the window/renderer once.
        if self.renderer.is_some() {
            log::info!("resumed with a live renderer; keeping the existing window");
            return;
        }

        let result = (|| -> anyhow::Result<Renderer> {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Galactic Repoman")
                            .with_inner_size(LogicalSize::new(1280, 720)),
                    )
                    .context("creating the window")?,
            );
            log::info!(
                "window created: {:?}, inner {:?}, outer {:?}, scale {:.2}, visible {:?}",
                window.id(),
                window.inner_size(),
                window.outer_size(),
                window.scale_factor(),
                window.is_visible(),
            );

            Renderer::new(&self.context, window)
        })();

        match result {
            Ok(renderer) => {
                // Kick off the first frame; in `Wait` mode nothing else would.
                redraw(&renderer, "renderer ready");
                self.renderer = Some(renderer);
            }
            Err(e) => {
                log::error!("failed to initialize renderer: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // We never tear the renderer down here, so a suspend/resume pair leaves us
        // presenting to a surface the compositor may already have dropped.
        log::warn!("suspended: keeping the existing surface and swapchain");
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        log::trace!("new events: {cause:?}");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            log::debug!("window event arrived before the renderer existed; ignored");
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                log::info!("resized to {size:?}; flagging swapchain recreation");
                renderer.recreate_swapchain = true;
                // In `Wait` mode the resize itself won't schedule a draw.
                redraw(renderer, "resize");
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                log::info!("scale factor changed to {scale_factor:.2}");
            }
            // The screen we last presented to is not necessarily the screen the user is
            // looking at now, and the frame we drew is not redrawn for us.
            WindowEvent::Occluded(occluded) => {
                if occluded {
                    log::info!("occluded; nothing will be drawn until we are shown again");
                } else {
                    redraw(renderer, "no longer occluded");
                }
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    redraw(renderer, "focus gained");
                } else {
                    log::info!("focus lost");
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = renderer.render(ANGLE) {
                    log::error!("render error: {e:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!(
            "exiting; {}",
            self.renderer.as_ref().map_or_else(
                || "no renderer was ever created".to_string(),
                Renderer::status
            ),
        );
    }
}

/// Ask for a frame and say who asked. On a static scene every frame is traceable to
/// one signal, so naming it is what turns "the screen is black" into "the signal we
/// were relying on never arrived".
fn redraw(renderer: &Renderer, reason: &str) {
    log::info!("redraw requested: {reason}");
    renderer.window.request_redraw();
}
