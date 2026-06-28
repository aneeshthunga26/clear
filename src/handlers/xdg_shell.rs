use smithay::{
    desktop::{
        PopupKind, PopupManager, Space, Window, find_popup_root_surface, get_popup_toplevel_coords,
    },
    input::{
        Seat,
        pointer::{Focus, GrabStartData as PointerGrabStartData},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    Clear,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
};

impl XdgShellHandler for Clear {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface.clone());
        let location = self
            .space
            .outputs()
            .next()
            .and_then(|output| self.usable_output_geometry(output))
            .map(|geometry| geometry.loc)
            .unwrap_or_default();
        self.space.map_element(window, location, false);
        if self.launcher_pending && self.pending_launcher_surface.is_none() {
            self.pending_launcher_surface = Some(surface.wl_surface().clone());
        }
        // Some clients set app_id before mapping, so try immediately.
        self.try_center_launcher(&surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            let window = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            let window = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });

            surface.send_pending_configure();

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO popup grabs
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        // Other clients set app_id shortly after mapping; catch that path too.
        self.try_center_launcher(&surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.clear_launcher_surface(surface.wl_surface());
    }
}

fn check_grab(
    seat: &Seat<Clear>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Clear>> {
    let pointer = seat.get_pointer()?;

    // Check that this surface has a click grab.
    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;

    let (focus, _) = start_data.focus.as_ref()?;
    // If the focus was for a different surface, ignore the request.
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    // Handle toplevel commits.
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
    }

    // Handle popup commits.
    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    // NOTE: This should never fail as the initial configure is always
                    // allowed.
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Clear {
    /// Try centering a committed surface if it belongs to the pending launcher.
    pub(crate) fn try_center_launcher_surface(&mut self, surface: &WlSurface) {
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == surface)
            .cloned()
        else {
            return;
        };
        let Some(toplevel) = window.toplevel().cloned() else {
            return;
        };

        self.try_center_launcher_window(window, &toplevel);
    }

    /// Try centering a toplevel handle if it matches the configured launcher app id.
    fn try_center_launcher(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.toplevel().unwrap() == surface)
            .cloned()
        else {
            return;
        };

        self.try_center_launcher_window(window, surface);
    }

    fn try_center_launcher_window(&mut self, window: Window, surface: &ToplevelSurface) {
        let is_pending_launcher = self
            .pending_launcher_surface
            .as_ref()
            .is_some_and(|pending| pending == surface.wl_surface());
        let is_active_launcher = self
            .active_launcher_surface
            .as_ref()
            .is_some_and(|active| active == surface.wl_surface());
        let has_launcher_app_id =
            toplevel_app_id(surface).as_deref() == Some(self.config.launcher_app_id());

        // The pending surface covers launchers that set app_id late or not at all.
        if !self.launcher_pending && !is_active_launcher {
            return;
        }
        if !is_pending_launcher && !is_active_launcher && !has_launcher_app_id {
            return;
        }

        let Some(output) = self.space.outputs().next() else {
            return;
        };
        let Some(output_geo) = self.usable_output_geometry(output) else {
            return;
        };

        let window_geo = window.geometry();
        let window_size = window_geo.size;
        // Before the first real buffer commit, Wayland windows can report no useful size.
        if window_size.w <= 0 || window_size.h <= 0 {
            return;
        }

        // Space stores the window geometry location. Keep recalculating this
        // while the launcher is active because clients like wofi can resize
        // after their first non-empty commit.
        let location = Point::<i32, Logical>::from((
            output_geo.loc.x + (output_geo.size.w - window_size.w) / 2,
            output_geo.loc.y + (output_geo.size.h - window_size.h) / 2,
        ));

        self.space.map_element(window, location, true);
        let serial = SERIAL_COUNTER.next_serial();
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Some(surface.wl_surface().clone()),
            serial,
        );
        self.launcher_pending = false;
        self.pending_launcher_surface = None;
        self.active_launcher_surface = Some(surface.wl_surface().clone());
    }

    fn clear_launcher_surface(&mut self, surface: &WlSurface) {
        if self
            .pending_launcher_surface
            .as_ref()
            .is_some_and(|pending| pending == surface)
        {
            self.pending_launcher_surface = None;
            self.launcher_pending = false;
        }

        if self
            .active_launcher_surface
            .as_ref()
            .is_some_and(|active| active == surface)
        {
            self.active_launcher_surface = None;
        }
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &root)
        else {
            return;
        };

        let output = self.space.outputs().next().unwrap();
        let output_geo = self.space.output_geometry(output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

fn toplevel_app_id(surface: &ToplevelSurface) -> Option<String> {
    // Smithay stores XDG toplevel metadata in the surface's role data.
    with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().unwrap().app_id.clone())
    })
}
