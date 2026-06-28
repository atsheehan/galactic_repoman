//! winit `ApplicationHandler`: owns the device-level [`VulkanContext`] and the
//! per-window [`Renderer`]. The window (and thus the renderer) is created on
//! `resumed`, as winit requires.

use std::sync::Arc;

use anyhow::Context;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::renderer::{Renderer, VulkanContext};

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
            Renderer::new(&self.context, window)
        })();

        match result {
            Ok(renderer) => {
                // Kick off the first frame; in `Wait` mode nothing else would.
                renderer.window.request_redraw();
                self.renderer = Some(renderer);
            }
            Err(e) => {
                log::error!("failed to initialize renderer: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit();
            }
            WindowEvent::Resized(_) => {
                renderer.recreate_swapchain = true;
                // In `Wait` mode the resize itself won't schedule a draw.
                renderer.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = renderer.render() {
                    log::error!("render error: {e:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}
