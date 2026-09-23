//! Smithay objects and Wayland protocol state never escape this adapter.

mod backend;
mod input;
mod layers;
mod protocols;
mod scene;
mod state;

use crate::runtime::{Options, Runtime};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use state::Compositor;
use std::time::Duration;

/// Run the nested compositor until closed, a quit action, or the optional test deadline.
pub fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = Runtime::load(options.config_path)?;
    let mut event_loop = EventLoop::<Compositor>::try_new()?;
    let display = Display::<Compositor>::new()?;
    let mut state = Compositor::new(
        &event_loop,
        display,
        runtime,
        options.socket_name.as_deref(),
    )?;
    let capture_after = options
        .exit_after
        .map(|duration| duration.saturating_sub(Duration::from_millis(250)))
        .unwrap_or(Duration::from_secs(3));
    backend::init(&event_loop, &mut state, options.capture, capture_after)?;
    if !options.command.is_empty() {
        state.spawn(options.command);
    }
    eprintln!(
        "clear: ready WAYLAND_DISPLAY={} outputs={}",
        state.socket_name.to_string_lossy(),
        state.outputs.len()
    );
    event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        state.reconcile();
        state.popups.cleanup();
        state.space.refresh();
        state
            .children
            .retain_mut(|child| child.try_wait().map_or(false, |status| status.is_none()));
        if let Err(error) = state.display_handle.flush_clients() {
            eprintln!("clear: flushing clients failed: {error}");
        }
        if options
            .exit_after
            .is_some_and(|duration| state.start.elapsed() >= duration)
        {
            state.loop_signal.stop();
        }
    })?;
    for child in &mut state.children {
        let _ = child.kill();
        let _ = child.wait();
    }
    eprintln!("clear: stopped");
    if let Some(error) = state.error {
        return Err(error.into());
    }
    Ok(())
}
