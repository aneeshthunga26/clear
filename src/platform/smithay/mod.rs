//! Smithay objects and Wayland protocol state never escape this adapter.

mod animations;
mod backend;
mod blur;
mod decorations;
mod fps;
mod frame_scheduler;
mod input;
mod layers;
mod overview;
mod overview_animation;
mod overview_drag;
mod presentation_input;
mod protocols;
mod render_demand;
mod rounded;
mod scene;
mod shell;
mod shell_previews;
mod state;
mod titlebar;
mod wallpaper;
mod window_image;

use crate::runtime::{Options, Runtime};
use smithay::reexports::{
    calloop::{
        EventLoop, LoopHandle, RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    wayland_server::Display,
};
use state::Compositor;
use std::{cell::RefCell, rc::Rc, time::Duration};

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
    let backend = backend::init(&event_loop, &mut state, options.capture, capture_after)?;
    state.init_shell(!options.no_shell_ipc);
    let mut shell_readiness = match shell::ShellReadiness::new(&event_loop.handle(), &mut state) {
        Ok(readiness) => Some(readiness),
        Err(error) => {
            eprintln!("clear: shell readiness unavailable: {error}; continuing without it");
            state.shell_server = None;
            None
        }
    };
    if !options.command.is_empty() {
        state.spawn(options.command);
    }
    eprintln!(
        "clear: ready WAYLAND_DISPLAY={} outputs={}",
        state.socket_name.to_string_lossy(),
        state.outputs.len()
    );
    let mut timer = DeadlineTimer::new(event_loop.handle());
    let mut child_deadline = None;
    let initial = minimum(backend.service(&mut state), options.exit_after);
    timer.arm(initial, state.start.elapsed())?;
    event_loop.run(None, &mut state, |state| {
        state.service_overview();
        state.reconcile();
        state.dispatch_shell();
        if let Some(readiness) = shell_readiness.as_mut()
            && let Err(error) = readiness.sync(state)
        {
            eprintln!("clear: shell readiness failed: {error}; continuing without it");
            state.shell_server = None;
            state.shell_snapshot = None;
            shell_readiness = None;
        }
        state.popups.cleanup();
        state.space.refresh();
        state.refresh_overview_outputs();
        let now = state.start.elapsed();
        if state.children.is_empty() {
            child_deadline = None;
        } else if child_deadline.is_none_or(|deadline| now >= deadline) {
            state
                .children
                .retain_mut(|child| child.try_wait().map_or(false, |status| status.is_none()));
            child_deadline = (!state.children.is_empty()).then_some(now + Duration::from_secs(1));
        }
        let mut next = backend.service(state);
        let flush_deadline = match flush_wayland_clients(state) {
            Ok(()) => None,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // Only blocked output needs this bounded fallback. Quiet peers
                // do not create a periodic compositor maintenance wake.
                Some(now + Duration::from_millis(16))
            }
            Err(error) => {
                eprintln!("clear: flushing clients failed: {error}");
                None
            }
        };
        if options.exit_after.is_some_and(|deadline| now >= deadline) {
            state.loop_signal.stop();
        }
        next = minimum(next, options.exit_after);
        next = minimum(next, child_deadline);
        next = minimum(next, flush_deadline);
        next = minimum(next, state.shell_maintenance_deadline());
        if let Err(error) = timer.arm(next, state.start.elapsed()) {
            state.fail(format!("schedule deadline: {error}"));
        }
    })?;
    backend.report();
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

fn minimum(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// One cancellable deadline source for callback, diagnostic and maintenance work.
struct DeadlineTimer {
    handle: LoopHandle<'static, Compositor>,
    armed: Rc<RefCell<Option<(Duration, RegistrationToken)>>>,
}

impl DeadlineTimer {
    fn new(handle: LoopHandle<'static, Compositor>) -> Self {
        Self {
            handle,
            armed: Rc::new(RefCell::new(None)),
        }
    }

    fn arm(
        &mut self,
        deadline: Option<Duration>,
        now: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.armed.borrow().as_ref().map(|(deadline, _)| *deadline) == deadline {
            return Ok(());
        }
        if let Some((_, token)) = self.armed.borrow_mut().take() {
            self.handle.remove(token);
        }
        if let Some(deadline) = deadline {
            let armed = self.armed.clone();
            let token = self.handle.insert_source(
                Timer::from_duration(deadline.saturating_sub(now)),
                move |_, _, _| {
                    armed.borrow_mut().take();
                    TimeoutAction::Drop
                },
            )?;
            *self.armed.borrow_mut() = Some((deadline, token));
        }
        Ok(())
    }
}

/// The pin's bulk flush discards per-client EAGAIN. Query each live client so
/// blocked outgoing bytes retain an explicit retry rather than stalling at idle.
fn flush_wayland_clients(state: &Compositor) -> std::io::Result<()> {
    let mut handle = state.display_handle.backend_handle();
    let mut clients = Vec::new();
    handle.with_all_clients(|id| clients.push(id));
    let mut blocked = false;
    for id in clients {
        if let Err(error) = handle.flush(Some(id)) {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                blocked = true;
            }
            // Closed peers are reclaimed by normal Wayland dispatch/cleanup.
        }
    }
    if blocked {
        Err(std::io::ErrorKind::WouldBlock.into())
    } else {
        Ok(())
    }
}
