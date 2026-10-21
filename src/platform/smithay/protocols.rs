use super::{
    scene::logical,
    state::{ClientState, Compositor, ManagedWindow},
    titlebar::content_rect,
};
use crate::core::{Command, OutputId, WindowId};
use smithay::{
    backend::renderer::utils::{
        CommitCounter, RendererSurfaceStateUserData, SurfaceView, on_commit_buffer_handler,
        with_renderer_surface_state,
    },
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
            backend::ObjectId,
            protocol::{wl_buffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            RectangleKind, SubsurfaceCachedState, SurfaceAttributes, TraversalAction, get_parent,
            is_sync_subsurface, with_states, with_surface_tree_upward,
        },
        output::OutputHandler,
        selection::{
            SelectionHandler,
            data_device::{
                DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
            },
        },
        shell::wlr_layer::{Anchor, ExclusiveZone, KeyboardInteractivity, Layer},
        shell::xdg::{
            PopupState, PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface,
            XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
    },
};

/// Only effective pixel state is remembered; ordinary callback/input changes are excluded.
#[derive(Default)]
struct CommittedVisual(std::sync::Mutex<CommittedVisualState>);

#[derive(Default)]
struct CommittedVisualState {
    fingerprint: Option<TreeVisual>,
    revision: u64,
}

/// A committed tree's appearance revision also changes for attachments with no reported damage.
pub(super) fn surface_visual_revision(surface: &WlSurface) -> u64 {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CommittedVisual>()
            .map_or(0, |visual| visual.0.lock().unwrap().revision)
    })
}

#[derive(PartialEq)]
struct TreeVisual {
    surfaces: Vec<SurfaceVisual>,
    geometry: Option<Rectangle<i32, Logical>>,
    decoration: Option<(bool, bool)>,
    layer: Option<LayerVisual>,
    popup: Option<PopupState>,
    popup_glass: Option<bool>,
}

#[derive(PartialEq)]
struct SurfaceVisual {
    id: ObjectId,
    buffer: Option<ObjectId>,
    damage: Option<CommitCounter>,
    view: Option<SurfaceView>,
    location: Point<i32, Logical>,
    scale: i32,
    transform: smithay::reexports::wayland_server::protocol::wl_output::Transform,
    opaque: Vec<(bool, Rectangle<i32, Logical>)>,
}

#[derive(PartialEq)]
struct LayerVisual {
    size: Size<i32, Logical>,
    anchor: Anchor,
    zone: ExclusiveZone,
    edge: Option<Anchor>,
    margin: (i32, i32, i32, i32),
    keyboard: KeyboardInteractivity,
    layer: Layer,
}

struct AssignedBuffer {
    surface: WlSurface,
    damage: CommitCounter,
}

fn tree_buffer_assignments(surface: &WlSurface) -> (bool, Vec<AssignedBuffer>) {
    let mut assigned = false;
    let mut buffers = Vec::new();
    with_surface_tree_upward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |surface, states, _| {
            let (has_assignment, new_buffer) = {
                let mut attrs = states.cached_state.get::<SurfaceAttributes>();
                match &attrs.current().buffer {
                    Some(BufferAssignment::NewBuffer(_)) => (true, true),
                    Some(BufferAssignment::Removed) => (true, false),
                    None => (false, false),
                }
            };
            assigned |= has_assignment;
            if new_buffer {
                let damage = states
                    .data_map
                    .get::<RendererSurfaceStateUserData>()
                    .map_or_else(CommitCounter::default, |data| {
                        data.lock().unwrap().current_commit()
                    });
                buffers.push(AssignedBuffer {
                    surface: surface.clone(),
                    damage,
                });
            }
        },
        |_, _, _| true,
    );
    (assigned, buffers)
}

fn assigned_buffer_damage_uncertain(buffers: &[AssignedBuffer]) -> bool {
    buffers.iter().any(|assignment| {
        // For an accepted buffer, Smithay advances this counter only for nonempty
        // damage after viewport conversion and clipping. Failed-buffer resets also
        // advance it, so those remain uncertain. An undamaged attachment clears the
        // import texture without advancing the damage counter.
        with_renderer_surface_state(&assignment.surface, |state| {
            state.buffer().is_none() || state.current_commit() == assignment.damage
        })
        .unwrap_or(true)
    })
}

impl Compositor {
    pub(super) fn sync_transient_parents(&mut self) {
        let parents: Vec<_> = self
            .windows
            .iter()
            .filter(|(_, w)| w.mapped)
            .filter_map(|(id, entry)| {
                let parent = with_states(entry.window.toplevel()?.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|data| data.lock().unwrap().parent.clone())
                });
                Some((
                    *id,
                    parent
                        .as_ref()
                        .and_then(|surface| self.window_id(surface))
                        .filter(|id| self.runtime.desktop.window(*id).is_some()),
                ))
            })
            .collect();
        for (id, parent) in parents {
            let changed = self
                .runtime
                .desktop
                .window(id)
                .is_some_and(|w| w.transient_for != parent);
            self.runtime.desktop.set_window_parent(id, parent);
            if changed && parent.is_some() && self.runtime.desktop.is_fullscreen_transient(id) {
                if let Some(entry) = self.windows.get(&id) {
                    let size = entry.window.geometry().size;
                    let height = size.h.saturating_add(if entry.uses_ssd() {
                        self.runtime.config.theme.titlebar.height
                    } else {
                        0
                    });
                    if let Some(bounds) = self
                        .runtime
                        .desktop
                        .window(id)
                        .and_then(|w| w.output)
                        .and_then(|o| self.runtime.desktop.output(o))
                        .map(|o| o.bounds)
                    {
                        self.runtime.desktop.command(Command::SetFloatingRect(
                            id,
                            bounds.centered_unbounded(size.w, height),
                        ));
                    }
                }
                if self
                    .runtime
                    .desktop
                    .fullscreen_slot()
                    .is_some_and(|s| s.active)
                {
                    self.runtime.desktop.command(Command::Focus(id));
                }
            }
        }
    }
    pub(super) fn revoke_stale_fullscreen_intents(&mut self) {
        let revision = self.runtime.desktop.fullscreen_revision();
        let stale: Vec<_> = self
            .windows
            .iter()
            .filter(|(_, w)| {
                !w.mapped && w.initial_fullscreen && w.initial_fullscreen_revision != revision
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            let entry = self.windows.get_mut(&id).unwrap();
            entry.initial_fullscreen = false;
            entry.initial_fullscreen_output = None;
            if let Some(top) = entry
                .window
                .toplevel()
                .filter(|top| top.is_initial_configure_sent())
                .cloned()
            {
                self.configure_initial_window(id, &top);
            }
        }
    }
    fn surface_tree_visual(&self, root: &WlSurface) -> TreeVisual {
        let mut surfaces = Vec::new();
        with_surface_tree_upward(
            root,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, states, _| {
                // Tree traversal already owns the compositor surface lock.
                // Calling with_renderer_surface_state here would lock it recursively.
                let renderer = states
                    .data_map
                    .get::<RendererSurfaceStateUserData>()
                    .map(|data| {
                        let state = data.lock().unwrap();
                        (
                            state.buffer().map(|buffer| buffer.id()),
                            state.current_commit(),
                            state.view(),
                        )
                    });
                let mut attrs = states.cached_state.get::<SurfaceAttributes>();
                let attrs = attrs.current();
                surfaces.push(SurfaceVisual {
                    id: surface.id(),
                    buffer: renderer.as_ref().and_then(|r| r.0.clone()),
                    damage: renderer.as_ref().map(|r| r.1),
                    view: renderer.and_then(|r| r.2),
                    location: states
                        .cached_state
                        .get::<SubsurfaceCachedState>()
                        .current()
                        .location,
                    scale: attrs.buffer_scale,
                    transform: attrs.buffer_transform,
                    opaque: attrs
                        .opaque_region
                        .as_ref()
                        .map_or_else(Vec::new, |region| {
                            region
                                .rects
                                .iter()
                                .map(|(kind, rect)| (matches!(kind, RectangleKind::Add), *rect))
                                .collect()
                        }),
                });
            },
            |_, _, _| true,
        );
        let geometry = with_states(root, |states| {
            states
                .cached_state
                .get::<SurfaceCachedState>()
                .current()
                .geometry
        });
        let decoration = self
            .window_id(root)
            .and_then(|id| self.windows.get(&id))
            .map(|entry| (entry.uses_ssd(), entry.committed_fullscreen));
        let layer = self
            .layers
            .iter()
            .find(|entry| entry.surface.wl_surface() == root)
            .map(|entry| {
                let state = entry.surface.cached_state();
                LayerVisual {
                    size: state.size,
                    anchor: state.anchor,
                    zone: state.exclusive_zone,
                    edge: state.exclusive_edge,
                    margin: (
                        state.margin.top,
                        state.margin.right,
                        state.margin.bottom,
                        state.margin.left,
                    ),
                    keyboard: state.keyboard_interactivity,
                    layer: state.layer,
                }
            });
        let popup = self.popups.find_popup(root).and_then(|popup| match popup {
            PopupKind::Xdg(popup) => popup.with_committed_state(|state| state.copied()),
            _ => None,
        });
        // An opt-in popup's empty-input state selects tooltip versus card radii.
        // Ordinary input-region changes remain nonpainting commits.
        let popup_glass = (self.runtime.config.theme.blur_radius > 0.0
            || self.runtime.config.theme.liquid_glass.enabled)
            .then(|| self.popups.find_popup(root))
            .flatten()
            .and_then(|popup| find_popup_root_surface(&popup).ok())
            .filter(|parent| {
                self.layers.iter().any(|layer| {
                    layer.surface.wl_surface() == parent
                        && super::scene::glass_popup_namespace(layer.surface.namespace())
                })
            })
            .map(|_| super::scene::glass_popup_interactive(root));
        TreeVisual {
            surfaces,
            geometry,
            decoration,
            layer,
            popup,
            popup_glass,
        }
    }

    fn surface_contributes_pixels(&self, root: &WlSurface) -> bool {
        if let Some(id) = self.window_id(root) {
            return self
                .placements
                .iter()
                .any(|placement| placement.window == id)
                || self.overview_live_windows().contains(&id);
        }
        if self
            .layers
            .iter()
            .any(|entry| entry.surface.wl_surface() == root)
        {
            return true; // An effective null-buffer commit erases its old mapped tree.
        }
        if let Some(popup) = self.popups.find_popup(root)
            && let Ok(owner) = find_popup_root_surface(&popup)
        {
            return self.surface_contributes_pixels(&owner);
        }
        // Unsupported trees have no scene ownership; later mapping invalidates policy.
        false
    }
}

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
        let was_dirty = self.dirty;
        let effective = !is_sync_subsurface(surface);
        let mut tree_root = surface.clone();
        while let Some(parent) = get_parent(&tree_root) {
            tree_root = parent;
        }
        // Applied synchronized child updates are inspected with their parent's tree;
        // cached child commits must neither paint nor service unapplied callbacks.
        let (assigned_buffer, new_buffers) = if effective {
            tree_buffer_assignments(&tree_root)
        } else {
            (false, Vec::new())
        };
        on_commit_buffer_handler::<Self>(surface);
        let uncertain_damage = assigned_buffer_damage_uncertain(&new_buffers);
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
                    entry.mapping_generation = entry
                        .mapping_generation
                        .checked_add(1)
                        .expect("window mapping generation exhausted");
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
                    if entry.initial_fullscreen
                        && entry.initial_fullscreen_revision
                            == self.runtime.desktop.fullscreen_revision()
                    {
                        self.runtime.desktop.request_fullscreen(
                            id,
                            entry.initial_fullscreen_output,
                            true,
                        );
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
        self.dirty = was_dirty;
        if effective {
            let visual = self.surface_tree_visual(&tree_root);
            let changed = with_states(&tree_root, |states| {
                states
                    .data_map
                    .insert_if_missing_threadsafe(CommittedVisual::default);
                let mut previous = states
                    .data_map
                    .get::<CommittedVisual>()
                    .unwrap()
                    .0
                    .lock()
                    .unwrap();
                let changed = assigned_buffer || previous.fingerprint.as_ref() != Some(&visual);
                if changed {
                    previous.revision = previous
                        .revision
                        .checked_add(1)
                        .expect("committed visual revision exhausted");
                }
                previous.fingerprint = Some(visual);
                changed
            });
            self.dirty |= changed;
            if changed && self.surface_contributes_pixels(&tree_root) {
                self.render_demand.invalidate(if uncertain_damage {
                    super::render_demand::Reason::UncertainCommit
                } else {
                    super::render_demand::Reason::Commit
                });
            } else {
                if !changed {
                    self.refresh_pointer_committed_focus();
                }
                self.render_demand.wake();
            }
        }
    }

    fn destroyed(&mut self, _: &WlSurface) {
        // Child and popup deletion can erase pixels without another parent commit.
        // The protocol teardown order on disconnect is unspecified.
        self.dirty = true;
        self.render_demand
            .invalidate(super::render_demand::Reason::Lifecycle);
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
                mapping_generation: 0,
                initial_maximized: false,
                initial_fullscreen: false,
                initial_fullscreen_output: None,
                initial_fullscreen_revision: 0,
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
            self.render_demand
                .invalidate(super::render_demand::Reason::Lifecycle);
            eprintln!("clear: destroyed window {}", id.0);
        }
    }
    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.metadata_changed(surface.wl_surface());
    }
    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.metadata_changed(surface.wl_surface());
    }
    fn popup_destroyed(&mut self, _: PopupSurface) {
        self.dirty = true;
        self.render_demand
            .invalidate(super::render_demand::Reason::Lifecycle);
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

    fn parent_changed(&mut self, _surface: ToplevelSurface) {
        self.dirty = true;
        self.render_demand
            .invalidate(super::render_demand::Reason::Policy);
        self.reconcile();
    }
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.window_id(surface.wl_surface()) {
            self.runtime
                .desktop
                .command(Command::SetMinimized(id, true));
            self.dirty = true;
            self.render_demand
                .invalidate(super::render_demand::Reason::Policy);
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
        if !self.windows[&id].mapped
            && self
                .runtime
                .config
                .shell
                .is_launcher(&metadata(surface.wl_surface()).1)
        {
            let entry = self.windows.get_mut(&id).unwrap();
            entry.initial_fullscreen = false;
            entry.initial_fullscreen_output = None;
            if surface.is_initial_configure_sent() {
                self.configure_initial_window(id, &surface);
            }
            return;
        }
        if self.windows[&id].mapped {
            if self
                .runtime
                .desktop
                .window(id)
                .is_some_and(|w| w.role == crate::core::WindowRole::Normal)
            {
                if fullscreen {
                    self.runtime.desktop.request_fullscreen(
                        id,
                        output,
                        self.runtime.desktop.focused_window() == Some(id),
                    );
                } else {
                    self.runtime.desktop.leave_fullscreen(id);
                }
            }
            self.dirty = true;
            self.render_demand
                .invalidate(super::render_demand::Reason::Policy);
            self.reconcile();
            surface.send_configure();
        } else {
            if fullscreen {
                // Only the latest pending enter survives. No unbuffered client
                // can evict the mapped occupant; core epochs also revoke intents
                // superseded by a mapped request or shortcut before first buffer.
                let revoked: Vec<_> = self
                    .windows
                    .iter()
                    .filter(|(other, w)| **other != id && !w.mapped && w.initial_fullscreen)
                    .map(|(other, _)| *other)
                    .collect();
                for other in revoked {
                    let entry = self.windows.get_mut(&other).unwrap();
                    entry.initial_fullscreen = false;
                    entry.initial_fullscreen_output = None;
                    if let Some(top) = entry
                        .window
                        .toplevel()
                        .filter(|top| top.is_initial_configure_sent())
                        .cloned()
                    {
                        self.configure_initial_window(other, &top);
                    }
                }
            }
            let entry = self.windows.get_mut(&id).unwrap();
            entry.initial_fullscreen = fullscreen;
            entry.initial_fullscreen_output = fullscreen.then_some(output).flatten();
            entry.initial_fullscreen_revision = self.runtime.desktop.fullscreen_revision();
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
            self.render_demand
                .invalidate(super::render_demand::Reason::Policy);
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
            self.render_demand
                .invalidate(super::render_demand::Reason::Metadata);
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
