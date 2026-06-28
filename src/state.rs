use std::{
    ffi::OsString,
    process::{Child, Command},
    sync::Arc,
};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType, layer_map_for_output},
    input::{Seat, SeatState, keyboard::Keysym},
    output::Output,
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{
            wlr_layer::{LayerSurface as WlrLayerSurface, WlrLayerShellState},
            xdg::XdgShellState,
        },
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::config::Config;

pub struct Clear {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    /// Runtime config loaded from the user's XDG config file.
    pub config: Config,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    /// Tracks a compositor-handled shortcut key so its release is not forwarded.
    pub suppressed_launcher_key: Option<Keysym>,
    /// Set after spawning the launcher, cleared after its first window is centered.
    pub launcher_pending: bool,
    /// First toplevel seen after launching the launcher, even before app_id is set.
    pub pending_launcher_surface: Option<WlSurface>,
    /// Currently open launcher toplevel, used to toggle it closed.
    pub active_launcher_surface: Option<WlSurface>,
    /// Currently open status bar layer surface, used to toggle it closed.
    pub active_status_bar_surface: Option<WlrLayerSurface>,
    /// Child process for the configured status bar command.
    pub active_status_bar_process: Option<Child>,
    /// Set when the bar should be respawned after its current layer is destroyed.
    pub status_bar_restart_pending: bool,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Clear>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
}

impl Clear {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>, config: Config) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        // Here we initialize implementations of some wayland protocols
        // Some of them require us to implement traits on the Clear state,
        // you can find those implementations in the `crate::handlers` module

        // Initialize protocols needed for displaying windows
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        // Data device is responsible for clipboard and drag-and-drop
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        // Setup a wayland socket that will be used to accept clients
        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,
            config,

            space,
            loop_signal,
            suppressed_launcher_key: None,
            launcher_pending: false,
            pending_launcher_surface: None,
            active_launcher_surface: None,
            active_status_bar_surface: None,
            active_status_bar_process: None,
            status_bar_restart_pending: false,
            socket_name,

            compositor_state,
            xdg_shell_state,
            layer_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            popups,
            seat,
        }
    }

    fn init_wayland_listener(
        display: Display<Clear>,
        event_loop: &mut EventLoop<Self>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.space.outputs().find(|output| {
            self.space
                .output_geometry(output)
                .is_some_and(|geometry| geometry.contains(pos.to_i32_round()))
        })?;
        let output_geo = self.space.output_geometry(output)?;
        let layers = layer_map_for_output(output);

        for layer_kind in [
            smithay::wayland::shell::wlr_layer::Layer::Overlay,
            smithay::wayland::shell::wlr_layer::Layer::Top,
        ] {
            if let Some(layer) = layers.layer_under(layer_kind, pos - output_geo.loc.to_f64()) {
                let layer_geo = layers.layer_geometry(layer)?;
                if let Some((surface, surface_loc)) = layer.surface_under(
                    pos - output_geo.loc.to_f64() - layer_geo.loc.to_f64(),
                    WindowSurfaceType::ALL,
                ) {
                    return Some((
                        surface,
                        (surface_loc + layer_geo.loc + output_geo.loc).to_f64(),
                    ));
                }
            }
        }
        drop(layers);

        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
            .or_else(|| {
                let layers = layer_map_for_output(output);
                for layer_kind in [
                    smithay::wayland::shell::wlr_layer::Layer::Bottom,
                    smithay::wayland::shell::wlr_layer::Layer::Background,
                ] {
                    if let Some(layer) =
                        layers.layer_under(layer_kind, pos - output_geo.loc.to_f64())
                    {
                        let layer_geo = layers.layer_geometry(layer)?;
                        if let Some((surface, surface_loc)) = layer.surface_under(
                            pos - output_geo.loc.to_f64() - layer_geo.loc.to_f64(),
                            WindowSurfaceType::ALL,
                        ) {
                            return Some((
                                surface,
                                (surface_loc + layer_geo.loc + output_geo.loc).to_f64(),
                            ));
                        }
                    }
                }

                None
            })
    }

    /// Output area available for normal windows after exclusive layers reserve space.
    pub fn usable_output_geometry(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let output_geo = self.space.output_geometry(output)?;
        let usable = layer_map_for_output(output).non_exclusive_zone();
        Some(Rectangle::new(output_geo.loc + usable.loc, usable.size))
    }

    /// Recompute layer placement and restart the bar against the latest output size.
    pub fn handle_output_resize(&mut self, output: &Output) {
        let changed = layer_map_for_output(output).arrange();
        self.restart_status_bar();

        if changed || self.status_bar_restart_pending {
            let _ = self.display_handle.flush_clients();
        }
    }

    /// Toggle the configured launcher, closing an existing one before spawning.
    pub fn toggle_launcher(&mut self) {
        if self.close_active_launcher() {
            return;
        }

        let command = self.config.launcher_command().to_string();
        if command.is_empty() {
            return;
        }

        // Use the shell so users can configure commands with arguments.
        if Command::new("sh").arg("-c").arg(command).spawn().is_ok() {
            self.launcher_pending = true;
            self.pending_launcher_surface = None;
        }
    }

    fn close_active_launcher(&mut self) -> bool {
        let Some(surface) = self.active_launcher_surface.take() else {
            return false;
        };

        if let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == &surface)
        {
            window.toplevel().unwrap().send_close();
            self.launcher_pending = false;
            self.pending_launcher_surface = None;
            return true;
        }

        false
    }

    /// Toggle the configured status bar, closing an existing one before spawning.
    pub fn toggle_status_bar(&mut self) {
        if self.active_status_bar_surface.is_some() || self.active_status_bar_process.is_some() {
            self.stop_status_bar(false);
            return;
        }

        self.spawn_status_bar();
    }

    /// Spawn the configured status bar inside Clear's Wayland session.
    pub fn spawn_status_bar(&mut self) {
        if self.active_status_bar_surface.is_some() || self.active_status_bar_process.is_some() {
            return;
        }

        let command = self.config.apps.status_bar.command().to_string();
        if command.is_empty() {
            return;
        }

        // Use `exec` so the tracked child is the bar process for simple commands.
        if let Ok(child) = Command::new("sh")
            .arg("-c")
            .arg(format!("exec {command}"))
            .spawn()
        {
            self.active_status_bar_process = Some(child);
        }
    }

    /// Close the bar so it can be recreated against the latest output geometry.
    pub fn restart_status_bar(&mut self) {
        self.stop_status_bar(true);
    }

    fn stop_status_bar(&mut self, restart: bool) {
        self.status_bar_restart_pending = restart;
        let had_surface = self.active_status_bar_surface.is_some();

        if let Some(surface) = self.active_status_bar_surface.take() {
            surface.send_close();
        }

        if let Some(mut child) = self.active_status_bar_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        if restart && !had_surface {
            self.status_bar_restart_pending = false;
            self.spawn_status_bar();
        }
    }
}

/// Data associated with a wayland client that connects to Clear.
/// One instance of this type per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
