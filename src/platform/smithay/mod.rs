//! Smithay objects and Wayland protocol state never escape this adapter.

mod backend;
mod blur;
mod decorations;
mod frame_scheduler;
mod input;
mod layers;
mod overview;
mod protocols;
mod rounded;
mod scene;
mod shell;
mod state;
mod titlebar;
mod wallpaper;

use crate::runtime::{Options, Runtime};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use state::Compositor;
use std::time::Duration;

// Equivalent to Smithay's delegate_dispatch2!, with the missing decoration
// destruction notification forwarded before any subsequent surface commit.
impl<I, UserData> smithay::reexports::wayland_server::Dispatch<I, UserData> for Compositor
where
    I: smithay::reexports::wayland_server::Resource + 'static,
    UserData: smithay::wayland::Dispatch2<I, Self>,
{
    fn request(
        state: &mut Self,
        client: &smithay::reexports::wayland_server::Client,
        resource: &I,
        request: I::Request,
        data: &UserData,
        handle: &smithay::reexports::wayland_server::DisplayHandle,
        data_init: &mut smithay::reexports::wayland_server::DataInit<'_, Self>,
    ) {
        data.request(state, client, resource, request, handle, data_init);
    }

    fn destroyed(
        state: &mut Self,
        client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &I,
        data: &UserData,
    ) {
        data.destroyed(state, client, resource);
        if let Some(decoration) = (resource as &dyn std::any::Any).downcast_ref::<
            smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        >() {
            state.xdg_decoration_destroyed(decoration);
        }
    }
}

impl<I, UserData> smithay::reexports::wayland_server::GlobalDispatch<I, UserData> for Compositor
where
    I: smithay::reexports::wayland_server::Resource,
    UserData: smithay::wayland::GlobalDispatch2<I, Self>,
{
    fn bind(
        state: &mut Self,
        handle: &smithay::reexports::wayland_server::DisplayHandle,
        client: &smithay::reexports::wayland_server::Client,
        resource: smithay::reexports::wayland_server::New<I>,
        data: &UserData,
        data_init: &mut smithay::reexports::wayland_server::DataInit<'_, Self>,
    ) {
        data.bind(state, handle, client, resource, data_init);
    }

    fn can_view(client: smithay::reexports::wayland_server::Client, data: &UserData) -> bool {
        data.can_view(&client)
    }
}

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
    state.init_shell(!options.no_shell_ipc);
    if !options.command.is_empty() {
        state.spawn(options.command);
    }
    eprintln!(
        "clear: ready WAYLAND_DISPLAY={} outputs={}",
        state.socket_name.to_string_lossy(),
        state.outputs.len()
    );
    event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        state.service_overview();
        state.reconcile();
        state.dispatch_shell();
        state.popups.cleanup();
        state.space.refresh();
        state.refresh_overview_outputs();
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
