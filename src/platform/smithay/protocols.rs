use super::{
    scene::logical,
    state::{ClientState, Compositor, ManagedWindow},
    titlebar::content_rect,
};
use crate::core::{Command, OutputId, WindowId};
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
            protocol::{wl_buffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
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
                let unmapped = !has_buffer && entry.mapped;
                if surface == &root {
                    entry.commit_decoration(has_buffer);
                    entry.committed_fullscreen = has_buffer
                        && top.with_cached_state(|state| {
                            state.last_acked.as_ref().is_some_and(|configure| {
                                configure
                                    .state
                                    .states
                                    .contains(xdg_toplevel::State::Fullscreen)
                            })
                        });
                }
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
                    self.runtime
                        .desktop
                        .command(Command::SetMaximized(id, entry.initial_maximized));
                    self.runtime
                        .desktop
                        .command(Command::SetFullscreen(id, entry.initial_fullscreen));
                    if entry.initial_fullscreen
                        && let Some(output) = entry.initial_fullscreen_output
                        && self.runtime.desktop.output(output).is_some()
                        && self
                            .runtime
                            .desktop
                            .window(id)
                            .is_some_and(|w| w.role == crate::core::WindowRole::Normal)
                    {
                        self.runtime.desktop.command(Command::Focus(id));
                        self.runtime.desktop.command(Command::MoveToOutput(output));
                    }
                    entry.initial_maximized = false;
                    entry.initial_fullscreen = false;
                    entry.initial_fullscreen_output = None;
                    eprintln!("clear: mapped window {}", id.0);
                } else if !has_buffer && entry.mapped {
                    entry.mapped = false;
                    entry.initial_maximized = false;
                    entry.initial_fullscreen = false;
                    entry.initial_fullscreen_output = None;
                    entry.committed_fullscreen = false;
                    top.with_pending_state(|pending| {
                        pending.states.unset(xdg_toplevel::State::Maximized);
                        pending.states.unset(xdg_toplevel::State::Fullscreen);
                        pending.states.unset(xdg_toplevel::State::Suspended);
                        pending.states.unset(xdg_toplevel::State::Resizing);
                    });
                    self.space.unmap_elem(&entry.window);
                    self.runtime.desktop.remove_window(id);
                    eprintln!("clear: unmapped window {}", id.0);
                }
                if entry.mapped {
                    let size = entry.window.geometry().size;
                    self.runtime.desktop.set_window_committed_size(
                        id,
                        size.w,
                        size.h.saturating_add(if entry.uses_ssd() {
                            self.runtime.config.theme.titlebar.height
                        } else {
                            0
                        }),
                    );
                }
                if !top.is_initial_configure_sent() && !unmapped {
                    let top = top.clone();
                    self.configure_initial_window(id, &top);
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
    type PointerFocus = super::presentation_input::PointerFocus;
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
                initial_maximized: false,
                initial_fullscreen: false,
                initial_fullscreen_output: None,
                committed_fullscreen: false,
                last_frame: Default::default(),
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
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.set_client_maximized(surface, true);
    }
    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.set_client_maximized(surface, false);
    }
    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        let output = output.and_then(|output| {
            self.outputs
                .iter()
                .find(|region| region.output.owns(&output))
                .map(|region| region.id)
        });
        self.set_client_fullscreen(surface, true, output);
    }
    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.set_client_fullscreen(surface, false, None);
    }
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.window_id(surface.wl_surface()) {
            self.runtime
                .desktop
                .command(Command::SetMinimized(id, true));
            self.dirty = true;
            self.reconcile();
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
    fn configure_initial_window(&self, id: WindowId, surface: &ToplevelSurface) {
        let entry = &self.windows[&id];
        entry.prepare_decoration_configure();
        let launcher = self
            .runtime
            .config
            .shell
            .is_launcher(&metadata(surface.wl_surface()).1);
        let fullscreen = entry.initial_fullscreen && !launcher;
        let maximized = entry.initial_maximized && !launcher;
        let output = entry
            .initial_fullscreen_output
            .filter(|output| self.runtime.desktop.output(*output).is_some())
            .or(self.runtime.desktop.focused_output())
            .and_then(|output| self.runtime.desktop.output(output));
        let area = output.map(|output| {
            if fullscreen {
                output.bounds
            } else {
                output.area
            }
        });
        // Compute decoration insets before taking XDG pending state: pending_ssd
        // reads this surface's protocol data and must not recursively lock it.
        let size = if (fullscreen || maximized)
            && let Some(area) = area
        {
            entry.remember_frame(area);
            let content = content_rect(
                area,
                !fullscreen && entry.pending_ssd(),
                self.runtime.config.theme.titlebar.height,
            );
            Some((content.width, content.height).into())
        } else {
            (!launcher).then_some((640, 480).into())
        };
        surface.with_pending_state(|pending| {
            if fullscreen {
                pending.states.set(xdg_toplevel::State::Fullscreen);
            } else {
                pending.states.unset(xdg_toplevel::State::Fullscreen);
            }
            if maximized {
                pending.states.set(xdg_toplevel::State::Maximized);
            } else {
                pending.states.unset(xdg_toplevel::State::Maximized);
            }
            pending.size = size;
        });
        surface.send_configure();
    }

    fn set_client_fullscreen(
        &mut self,
        surface: ToplevelSurface,
        fullscreen: bool,
        output: Option<OutputId>,
    ) {
        let Some(id) = self.window_id(surface.wl_surface()) else {
            return;
        };
        if self.windows[&id].mapped {
            if self
                .runtime
                .desktop
                .window(id)
                .is_some_and(|w| w.role == crate::core::WindowRole::Normal)
            {
                if fullscreen && let Some(output) = output {
                    self.runtime.desktop.command(Command::Focus(id));
                    self.runtime.desktop.command(Command::MoveToOutput(output));
                }
                self.runtime
                    .desktop
                    .command(Command::SetFullscreen(id, fullscreen));
            }
            self.dirty = true;
            self.reconcile();
            surface.send_configure();
        } else {
            let entry = self.windows.get_mut(&id).unwrap();
            entry.initial_fullscreen = fullscreen;
            entry.initial_fullscreen_output = fullscreen.then_some(output).flatten();
            if surface.is_initial_configure_sent() {
                self.configure_initial_window(id, &surface);
            }
        }
    }

    fn set_client_maximized(&mut self, surface: ToplevelSurface, maximized: bool) {
        let Some(id) = self.window_id(surface.wl_surface()) else {
            return;
        };
        if self.windows[&id].mapped {
            self.runtime
                .desktop
                .command(Command::SetMaximized(id, maximized));
            self.dirty = true;
            self.reconcile();
            // XDG requires a configure response even when the requested state is unchanged.
            surface.send_configure();
        } else {
            self.windows.get_mut(&id).unwrap().initial_maximized = maximized;
            if surface.is_initial_configure_sent() {
                self.configure_initial_window(id, &surface);
            }
        }
    }

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
        let content = content_rect(
            placement.rect,
            self.windows[&id].uses_ssd(),
            self.runtime.config.theme.titlebar.height,
        );
        target.loc -= smithay::utils::Point::from((content.x, content.y));
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target)
        });
    }
}

pub(super) fn metadata(surface: &WlSurface) -> (String, String) {
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
