//! Galactic Repoman — Step 1: open a window and draw a Vulkan colored triangle
//! (vulkano, dynamic rendering).

mod app;
mod logging;
mod renderer;

use anyhow::Context;
use winit::event_loop::{ControlFlow, EventLoop};

use crate::app::App;

fn main() -> anyhow::Result<()> {
    logging::init();

    let event_loop = EventLoop::new().context("creating the event loop")?;
    // The scene is static, so render on demand (initial show, resize, expose) rather
    // than spinning a continuous loop. Switch to `ControlFlow::Poll` + a per-frame
    // redraw request once we add animation or input-driven updates.
    //
    // `App::about_to_wait` relaxes this to `WaitUntil` so the idle heartbeat can fire;
    // it still parks between wake-ups and never draws unless something asks it to.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new(&event_loop)?;
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;

    Ok(())
}
