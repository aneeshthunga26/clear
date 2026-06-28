#![allow(irrefutable_let_patterns)]

mod config;
mod handlers;

mod grabs;
mod input;
mod state;
mod winit;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::Clear;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let config = config::Config::load();

    let mut event_loop: EventLoop<Clear> = EventLoop::try_new()?;

    let display: Display<Clear> = Display::new()?;

    let mut state = Clear::new(&mut event_loop, display, config);

    // Open a Wayland/X11 window for our nested compositor
    crate::winit::init_winit(&mut event_loop, &mut state)?;

    // Set WAYLAND_DISPLAY to our socket name, so child processes connect to Clear rather
    // than the host compositor
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    // Start the configured layer-shell bar by default.
    state.spawn_status_bar();

    // Spawn a test client, that will run under Clear
    spawn_client();

    event_loop.run(None, &mut state, move |_| {
        // Clear is running
    })?;

    Ok(())
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}

fn spawn_client() {
    let mut args = std::env::args().skip(1);
    let flag = args.next();
    let arg = args.next();

    match (flag.as_deref(), arg) {
        (Some("-c") | Some("--command"), Some(command)) => {
            std::process::Command::new(command).spawn().ok();
        }
        _ => {
            std::process::Command::new("weston-terminal").spawn().ok();
        }
    }
}
