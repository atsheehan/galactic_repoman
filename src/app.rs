//! winit `ApplicationHandler`: owns the device-level [`VulkanContext`] and the
//! per-window [`Renderer`]. The window (and thus the renderer) is created on
//! `resumed`, as winit requires.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::renderer::{Renderer, VulkanContext};

/// How often the idle loop reports that it is still alive. The scene is static, so a
/// healthy run goes completely silent once its first frame is up; without a heartbeat
/// there is no way to tell "parked in `Wait`, nothing asked us to draw" apart from
/// "wedged inside `render`" or "process gone".
const HEARTBEAT: Duration = Duration::from_secs(5);

pub struct App {
    context: VulkanContext,
    renderer: Option<Renderer>,

    // Diagnostics only: what the heartbeat reports about the window's own state.
    started: Instant,
    last_heartbeat: Instant,
    resume_count: u32,
    occluded: bool,
}

impl App {
    pub fn new(event_loop: &EventLoop<()>) -> anyhow::Result<Self> {
        let started = Instant::now();

        Ok(Self {
            context: VulkanContext::new(event_loop)?,
            renderer: None,
            started,
            last_heartbeat: started,
            resume_count: 0,
            occluded: false,
        })
    }

    /// One line describing everything we can cheaply observe while idle. Read two
    /// consecutive heartbeats to tell a stalled run from a merely quiet one.
    fn log_heartbeat(&self) {
        let uptime = self.started.elapsed().as_secs_f32();

        match self.renderer.as_ref() {
            Some(renderer) => log::info!(
                "heartbeat: up {uptime:.0}s, resumed {}x, occluded={}, window {:?} scale {:.2} \
                 focused={} visible={:?}, {}",
                self.resume_count,
                self.occluded,
                renderer.window.inner_size(),
                renderer.window.scale_factor(),
                renderer.window.has_focus(),
                renderer.window.is_visible(),
                renderer.status(),
            ),
            // Alive with no renderer means `resumed` never landed — the window (and
            // therefore the surface) was never created, which looks identical to a
            // black screen from the outside.
            None => log::warn!(
                "heartbeat: up {uptime:.0}s, resumed {}x, still no window/renderer",
                self.resume_count
            ),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.resume_count += 1;
        log::info!(
            "resumed (#{}) at {:.2}s",
            self.resume_count,
            self.started.elapsed().as_secs_f32()
        );

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
        log_window_event(&event);

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
            // looking at now, and the frame we drew is not redrawn for us. Both edges are
            // logged, because whether they arrive at all is backend-dependent.
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
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
                if let Err(e) = renderer.render() {
                    log::error!("render error: {e:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.last_heartbeat.elapsed() >= HEARTBEAT {
            self.last_heartbeat = Instant::now();
            self.log_heartbeat();
        }

        // Re-arm the wake-up. Plain `Wait` would park until the next real event, and
        // on a static scene under a compositor that sends none, that is forever — the
        // heartbeat would never fire precisely in the runs we need it for.
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_heartbeat + HEARTBEAT));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!(
            "exiting after {:.1}s; {}",
            self.started.elapsed().as_secs_f32(),
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

/// Log every event winit hands us, so a run that draws nothing still shows what the
/// compositor did and did not send. Pointer-motion-style events are demoted to `trace`
/// so they cannot bury the window-lifecycle events that matter here.
fn log_window_event(event: &WindowEvent) {
    let noisy = matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::TouchpadPressure { .. }
    );

    if noisy {
        log::trace!("window event: {event:?}");
    } else {
        log::debug!("window event: {event:?}");
    }
}
