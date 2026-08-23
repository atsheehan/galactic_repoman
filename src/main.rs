//! Galactic Repoman — a window and a Vulkan colored triangle (vulkano, dynamic
//! rendering), currently rotated by a fixed angle pushed to the vertex shader.

mod app;
mod logging;
mod renderer;

use anyhow::Context;
use winit::event_loop::{ControlFlow, EventLoop};

use crate::app::App;

fn main() -> anyhow::Result<()> {
    logging::init();

    let event_loop = EventLoop::new().context("creating the event loop")?;
    // The scene is still static, so render on demand (initial show, resize, expose)
    // rather than spinning a continuous loop. Step 3b introduces the clock and with it
    // `ControlFlow::Poll` while animating, falling back to `Wait` when idle or occluded.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new(&event_loop)?;
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;

    Ok(())
}
