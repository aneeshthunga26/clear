use super::{
    scene::logical,
    state::{ClientState, Compositor, ManagedWindow},
};
use crate::core::{Command, WindowId};
use smithay::{
    backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state},
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, Window, find_popup_root_surface,
        get_popup_toplevel_coords,
    },
    input::{
        Seat, SeatHandler, SeatState,
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::{CursorImageStatus, Focus},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface, with_states,
        },
        output::OutputHandler,
        selection::{
            SelectionHandler,
            data_device::{
                DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
            },
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
    },
};

impl CompositorHandler for Compositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("all accepted clients have compositor state")
            .compositor_state
    }
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(id) = self.window_id(&root) {
                let entry = self.windows.get_mut(&id).expect("known window");
                entry.window.on_commit();
                let top = entry.window.toplevel().expect("XDG window");
                let has_buffer =
                    with_renderer_surface_state(&root, |state| state.buffer().is_some())
                        .unwrap_or(false);
                if has_buffer && !entry.mapped {
                    entry.mapped = true;
                    let (title, app_id) = metadata(&root);
                    self.runtime.desktop.add_window(id, title, app_id.clone());
                    self.runtime.classify_window(id, &app_id);
                    let is_dialog = with_states(&root, |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .is_some_and(|data| data.lock().unwrap().parent.is_some())
                    });
                    if is_dialog {
                        self.runtime.desktop.command(Command::ToggleFloating);
                    }
                    eprintln!("clear: mapped window {}", id.0);
                } else if !has_buffer && entry.mapped {
                    entry.mapped = false;
                    self.space.unmap_elem(&entry.window);
                    self.runtime.desktop.remove_window(id);
                    eprintln!("clear: unmapped window {}", id.0);
                }
                if entry.mapped {
                    let size = entry.window.geometry().size;
                    self.runtime
                        .desktop
                        .set_window_committed_size(id, size.w, size.h);
                }
                if !top.is_initial_configure_sent() {
                    let is_launcher = self.runtime.config.shell.is_launcher(&metadata(&root).1);
                    top.with_pending_state(|pending| {
                        pending.size = (!is_launcher).then_some((640, 480).into());
                    });
                    top.send_configure();
                }
            }
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                if let Err(error) = popup.send_configure() {
                    eprintln!("clear: popup configure failed: {error}");
                }
            }
        }
        self.layer_commit(surface);
        self.dirty = true;
    }
}
impl BufferHandler for Compositor {
    fn buffer_destroyed(&mut self, _: &wl_buffer::WlBuffer) {}
}
impl ShmHandler for Compositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
impl OutputHandler for Compositor {}

impl SeatHandler for Compositor {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
    fn cursor_image(&mut self, _: &Seat<Self>, _: CursorImageStatus) {}
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
    }
}
impl SelectionHandler for Compositor {
    type SelectionUserData = ();
}
impl DataDeviceHandler for Compositor {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}
impl DndGrabHandler for Compositor {}
impl WaylandDndGrabHandler for Compositor {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        kind: GrabType,
    ) {
        if let GrabType::Pointer = kind {
            if let Some(pointer) = seat.get_pointer() {
                if let Some(start) = pointer.grab_start_data() {
                    let grab = DnDGrab::new_pointer(&self.display_handle, start, source, seat);
                    pointer.set_grab(self, grab, serial, Focus::Keep);
                    return;
                }
            }
        }
        source.cancel();
    }
}

impl XdgShellHandler for Compositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let id = WindowId(self.next_window);
        self.next_window += 1;
        self.windows.insert(
            id,
            ManagedWindow {
                window: Window::new_wayland_window(surface),
                mapped: false,
            },
        );
    }
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.window_id(surface.wl_surface()) {
            if self.drag.as_ref().is_some_and(|drag| drag.window == id) {
                self.end_drag();
            }
            if let Some(entry) = self.windows.remove(&id) {
                self.space.unmap_elem(&entry.window);
            }
            self.runtime.desktop.remove_window(id);
            self.dirty = true;
            eprintln!("clear: destroyed window {}", id.0);
        }
    }
    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.metadata_changed(surface.wl_surface());
    }
    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.metadata_changed(surface.wl_surface());
    }
    fn new_popup(&mut self, surface: PopupSurface, _: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            eprintln!("clear: popup tracking failed: {error}");
        }
    }
    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }
    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            return;
        };
        let popup = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&popup) else {
            return;
        };
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        if keyboard.current_focus().as_ref() != Some(&root) && !keyboard.is_grabbed() {
            return;
        }
        if !pointer.has_grab(serial) && !keyboard.has_grab(serial) {
            return;
        }
        if let Ok(grab) = self.popups.grab_popup(root, popup, &seat, serial) {
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }
    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.client_drag(surface, seat, serial, 0);
    }
    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        self.client_drag(surface, seat, serial, edges as u32);
    }
}

impl Compositor {
    fn metadata_changed(&mut self, surface: &WlSurface) {
        if let Some(id) = self.window_id(surface) {
            let (title, app_id) = metadata(surface);
            self.runtime
                .desktop
                .update_window_metadata(id, title, app_id.clone());
            self.runtime.classify_window(id, &app_id);
            self.dirty = true;
        }
    }
    fn client_drag(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: u32,
    ) {
        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start) = pointer.grab_start_data() else {
            return;
        };
        if start
            .focus
            .as_ref()
            .is_none_or(|(focus, _)| !focus.id().same_client_as(&surface.wl_surface().id()))
        {
            return;
        }
        if let Some(id) = self.window_id(surface.wl_surface()) {
            self.begin_drag(id, pointer.current_location(), edges, start.button);
        }
    }
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let Some(id) = self.window_id(&root) else {
            return;
        };
        let Some(placement) = self.placements.iter().find(|p| p.window == id) else {
            return;
        };
        let area = placement.clip.or_else(|| {
            self.runtime
                .desktop
                .window(id)
                .and_then(|w| w.output)
                .and_then(|o| self.runtime.desktop.output(o))
                .map(|o| o.area)
        });
        let Some(area) = area else { return };
        let mut target = logical(area);
        target.loc -= get_popup_toplevel_coords(&kind);
        target.loc -= smithay::utils::Point::from((placement.rect.x, placement.rect.y));
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target)
        });
    }
}

fn metadata(surface: &WlSurface) -> (String, String) {
    with_states(surface, |states| {
        let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() else {
            return Default::default();
        };
        let data = data.lock().unwrap();
        (
            data.title.clone().unwrap_or_default(),
            data.app_id.clone().unwrap_or_default(),
        )
    })
}

smithay::delegate_dispatch2!(Compositor);
