use crate::{
    core::{Effect, OutputId, Placement, Rect, WindowId},
    input::Action,
    runtime::Runtime,
};
use smithay::{
    desktop::{PopupManager, Space, Window},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{wlr_layer::WlrLayerShellState, xdg::XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    process::{Child, Command},
    sync::Arc,
    time::Instant,
};

pub(super) struct ManagedWindow {
    pub window: Window,
    pub mapped: bool,
}

pub(super) struct OutputRegion {
    pub id: OutputId,
    pub output: Output,
    pub rect: Rect,
}

pub(super) struct Drag {
    pub window: WindowId,
    pub origin: Point<f64, Logical>,
    pub rect: Rect,
    pub edges: u32,
    pub button: u32,
}

pub(super) struct Compositor {
    pub runtime: Runtime,
    pub display_handle: DisplayHandle,
    pub socket_name: OsString,
    pub loop_signal: LoopSignal,
    pub start: Instant,
    pub error: Option<String>,
    pub space: Space<Window>,
    pub windows: BTreeMap<WindowId, ManagedWindow>,
    pub next_window: u64,
    pub outputs: Vec<OutputRegion>,
    pub placements: Vec<Placement>,
    pub layers: Vec<super::layers::LayerEntry>,
    pub layer_focus: Option<WlSurface>,
    pub shell_server: Option<crate::shell::server::Server>,
    pub shell_snapshot: Option<crate::shell::Snapshot>,
    pub shell_dirty: bool,
    pub host_size: Size<i32, Logical>,
    pub dirty: bool,
    pub host_focused: bool,
    pub children: Vec<Child>,
    pub suppressed_keys: BTreeSet<u32>,
    pub suppressed_buttons: BTreeSet<u32>,
    pub drag: Option<Drag>,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub _output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,
    pub seat: Seat<Self>,
}

impl Compositor {
    pub fn new(
        event_loop: &EventLoop<Self>,
        display: Display<Self>,
        runtime: Runtime,
        name: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let dh = display.handle();
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "clear");
        seat.add_keyboard(Default::default(), 200, 25)?;
        seat.add_pointer();
        let listener = match name {
            Some(name) => ListeningSocketSource::with_name(name)?,
            None => ListeningSocketSource::new_auto()?,
        };
        let socket_name = listener.socket_name().to_os_string();
        event_loop
            .handle()
            .insert_source(listener, |stream, _, state| {
                if let Err(error) = state
                    .display_handle
                    .insert_client(stream, Arc::new(ClientState::default()))
                {
                    eprintln!("clear: accepting client failed: {error}");
                }
            })?;
        event_loop.handle().insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| {
                // The display remains owned by the event source for the entire dispatch.
                if let Err(error) = unsafe { display.get_mut().dispatch_clients(state) } {
                    state.fail(format!("Wayland dispatch failed: {error}"));
                }
                Ok(PostAction::Continue)
            },
        )?;
        Ok(Self {
            runtime,
            socket_name,
            compositor_state: CompositorState::new::<Self>(&dh),
            xdg_shell_state: XdgShellState::new::<Self>(&dh),
            layer_shell_state: WlrLayerShellState::new::<Self>(&dh),
            shm_state: ShmState::new::<Self>(&dh, vec![]),
            _output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&dh),
            data_device_state: DataDeviceState::new::<Self>(&dh),
            display_handle: dh,
            seat_state,
            seat,
            loop_signal: event_loop.get_signal(),
            start: Instant::now(),
            error: None,
            space: Space::default(),
            windows: BTreeMap::new(),
            next_window: 1,
            outputs: Vec::new(),
            placements: Vec::new(),
            layers: Vec::new(),
            layer_focus: None,
            shell_server: None,
            shell_snapshot: None,
            shell_dirty: true,
            host_size: (1, 1).into(),
            dirty: true,
            host_focused: true,
            children: Vec::new(),
            suppressed_keys: BTreeSet::new(),
            suppressed_buttons: BTreeSet::new(),
            drag: None,
            popups: PopupManager::default(),
        })
    }

    pub fn fail(&mut self, message: String) {
        eprintln!("clear: {message}");
        self.error = Some(message);
        self.loop_signal.stop();
    }

    pub fn window_id(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows.iter().find_map(|(id, entry)| {
            (entry
                .window
                .toplevel()
                .is_some_and(|top| top.wl_surface() == surface))
            .then_some(*id)
        })
    }

    pub fn action(&mut self, action: Action) {
        let effects = self.runtime.action(action);
        for effect in effects {
            match effect {
                Effect::Quit => self.loop_signal.stop(),
                Effect::Spawn(command) => self.spawn(command),
                Effect::Close(id) => {
                    if let Some(top) = self
                        .windows
                        .get(&id)
                        .and_then(|entry| entry.window.toplevel())
                    {
                        top.send_close();
                    }
                }
            }
        }
        self.dirty = true;
        self.reconcile();
        eprintln!(
            "clear: desktop groups={:?} focus={:?}",
            self.runtime.desktop.groups(),
            self.runtime.desktop.focused_window()
        );
    }

    pub fn spawn(&mut self, command: Vec<String>) {
        let Some(program) = command.first() else {
            return;
        };
        let mut child = Command::new(program);
        // Never leak a parent compositor's bridge into a nested instance.
        child.env_remove("CLEAR_SOCKET");
        if let Some(server) = &self.shell_server {
            child.env("CLEAR_SOCKET", server.path());
        }
        match child
            .args(&command[1..])
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DISPLAY")
            .env("XDG_SESSION_TYPE", "wayland")
            .spawn()
        {
            Ok(child) => self.children.push(child),
            Err(error) => eprintln!("clear: cannot spawn {program:?}: {error}"),
        }
    }
}

#[derive(Default)]
pub(super) struct ClientState {
    pub compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}
