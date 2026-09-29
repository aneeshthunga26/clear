use super::{
    scene::{HitOwner, logical},
    state::{Compositor, Drag},
    titlebar::TitlebarPart,
};
use crate::{
    core::{Command, Rect, ResizeEdges, WindowId, WindowRole},
    input::{Action, Bindings, Modifiers},
};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::{FilterResult, Keysym, xkb},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Point, SERIAL_COUNTER},
};

// A focus click must not detach a tile or replace its remembered floating geometry.
fn titlebar_drag_ready(origin: Point<f64, Logical>, position: Point<f64, Logical>) -> bool {
    (position.x - origin.x).hypot(position.y - origin.y) >= 4.0
}

fn modifier_drag_target(owner: &Option<HitOwner>, logo: bool, button: u32) -> Option<WindowId> {
    if !logo || !matches!(button, 0x110 | 0x111) {
        return None;
    }
    match owner {
        Some(HitOwner::Window(id) | HitOwner::Decoration(id, _)) => Some(*id),
        _ => None,
    }
}

fn binding_action(
    bindings: &Bindings,
    modifiers: Modifiers,
    symbols: impl IntoIterator<Item = Keysym>,
) -> Option<Action> {
    // Keysym::name() is a debug label (e.g. "XK_Return"), not an XKB key name.
    symbols
        .into_iter()
        .find_map(|symbol| bindings.action(modifiers, &xkb::keysym_get_name(symbol)))
}

fn resize_corner(rect: Rect, position: Point<f64, Logical>) -> u32 {
    let horizontal = if position.x < f64::from(rect.x) + f64::from(rect.width) / 2.0 {
        4
    } else {
        8
    };
    let vertical = if position.y < f64::from(rect.y) + f64::from(rect.height) / 2.0 {
        1
    } else {
        2
    };
    horizontal | vertical
}

fn resize_edges(edges: u32) -> ResizeEdges {
    ResizeEdges {
        top: edges & 1 != 0,
        bottom: edges & 2 != 0,
        left: edges & 4 != 0,
        right: edges & 8 != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn stationary_titlebar_click_does_not_start_a_drag() {
        let origin = (100.0, 200.0).into();
        assert!(!titlebar_drag_ready(origin, origin));
        assert!(!titlebar_drag_ready(origin, (102.0, 202.0).into()));
        assert!(titlebar_drag_ready(origin, (104.0, 200.0).into()));
        assert!(titlebar_drag_ready(origin, (100.0, 196.0).into()));
    }

    #[test]
    fn modifier_gestures_preempt_every_titlebar_control() {
        let id = WindowId(7);
        for part in [
            TitlebarPart::Drag,
            TitlebarPart::Minimize,
            TitlebarPart::Maximize,
            TitlebarPart::Close,
        ] {
            let owner = Some(HitOwner::Decoration(id, part));
            for button in [0x110, 0x111] {
                assert_eq!(modifier_drag_target(&owner, true, button), Some(id));
                assert_eq!(modifier_drag_target(&owner, false, button), None);
            }
            assert_eq!(modifier_drag_target(&owner, true, 0x112), None);
        }
        assert_eq!(
            modifier_drag_target(&Some(HitOwner::Window(id)), true, 0x110),
            Some(id)
        );
        assert_eq!(modifier_drag_target(&None, true, 0x110), None);
    }

    #[test]
    fn resize_corner_selects_pointer_quadrant() {
        let rect = Rect::new(-100, 20, 200, 100);
        for (point, edges) in [
            ((-99.0, 21.0), 5),
            ((99.0, 21.0), 9),
            ((-99.0, 119.0), 6),
            ((99.0, 119.0), 10),
            ((0.0, 70.0), 10),
        ] {
            assert_eq!(resize_corner(rect, point.into()), edges);
        }
        assert_eq!(
            resize_corner(
                Rect::new(i32::MAX - 100, i32::MAX - 100, 200, 200),
                (f64::from(i32::MAX), f64::from(i32::MAX)).into()
            ),
            10
        );
    }

    #[test]
    fn native_resize_edges_preserve_requested_axes() {
        for edges in [1, 2, 4, 5, 6, 8, 9, 10] {
            let selected = resize_edges(edges);
            assert_eq!(selected.top, edges & 1 != 0);
            assert_eq!(selected.bottom, edges & 2 != 0);
            assert_eq!(selected.left, edges & 4 != 0);
            assert_eq!(selected.right, edges & 8 != 0);
        }
    }

    fn vm_bindings() -> Bindings {
        let config = Config::from_source(include_str!("../../../examples/vm.toml")).unwrap();
        Bindings::new(&config.bindings).unwrap()
    }

    #[test]
    fn vm_terminal_shortcut_matches_actual_return_keysym() {
        let modifiers = Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            binding_action(&vm_bindings(), modifiers, [Keysym::Return]),
            Some(Action::Spawn {
                command: vec!["foot".into()]
            })
        );
    }

    #[test]
    fn vm_letters_digits_and_special_keys_match_actual_keysyms() {
        let bindings = vm_bindings();
        let modifiers = Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        for (symbol, action) in [
            (Keysym::new(u32::from(b'm')), Action::CycleMode),
            (Keysym::new(u32::from(b's')), Action::StretchAll),
            (
                Keysym::new(u32::from(b'1')),
                Action::SwitchWorkspace { workspace: 1 },
            ),
            (Keysym::BackSpace, Action::ClearOutputMode),
            (Keysym::Escape, Action::Quit),
        ] {
            assert_eq!(binding_action(&bindings, modifiers, [symbol]), Some(action));
        }
        let shifted = Modifiers {
            shift: true,
            ..modifiers
        };
        assert_eq!(
            binding_action(&bindings, shifted, [Keysym::new(u32::from(b'1'))]),
            Some(Action::MoveToWorkspace { workspace: 1 })
        );
        assert_eq!(
            binding_action(&bindings, shifted, [Keysym::Right]),
            Some(Action::MoveToOutput { output: 2 })
        );
        assert_eq!(
            binding_action(&bindings, Modifiers::default(), [Keysym::Return]),
            None
        );
        assert_eq!(
            binding_action(
                &bindings,
                Modifiers {
                    alt: true,
                    ..Default::default()
                },
                [Keysym::Tab]
            ),
            Some(Action::AltTab)
        );
    }
}

impl Compositor {
    pub fn process_input<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let code = event.key_code();
                let pressed = event.state() == KeyState::Pressed;
                let keyboard = self.seat.get_keyboard().expect("keyboard initialized");
                let mut cancelled_switcher = false;
                let action = keyboard.input::<Option<Action>, _>(
                    self,
                    code,
                    event.state(),
                    SERIAL_COUNTER.next_serial(),
                    event.time_msec(),
                    |state, mods, key| {
                        if !pressed && state.suppressed_keys.remove(&code.raw()) {
                            return FilterResult::Intercept(None);
                        }
                        if pressed && state.suppressed_keys.contains(&code.raw()) {
                            return FilterResult::Intercept(None);
                        }
                        if pressed
                            && state.runtime.switcher.is_some()
                            && key.raw_syms().contains(&Keysym::Escape)
                        {
                            state.runtime.cancel_switcher();
                            state.suppressed_keys.insert(code.raw());
                            cancelled_switcher = true;
                            return FilterResult::Intercept(None);
                        }
                        if pressed {
                            let modifiers = Modifiers {
                                ctrl: mods.ctrl,
                                alt: mods.alt,
                                shift: mods.shift,
                                logo: mods.logo,
                            };
                            let action =
                                binding_action(&state.runtime.bindings, modifiers, key.raw_syms());
                            if let Some(action) = action {
                                state.suppressed_keys.insert(code.raw());
                                return FilterResult::Intercept(Some(action));
                            }
                        }
                        FilterResult::Forward
                    },
                );
                if let Some(Some(action)) = action {
                    if matches!(action, Action::AltTab) {
                        self.runtime.advance_switcher();
                        self.dirty = true;
                        self.reconcile();
                    } else {
                        self.action(action);
                    }
                }
                if self.runtime.switcher.is_some() && !keyboard.modifier_state().alt {
                    self.runtime.finish_switcher();
                    self.dirty = true;
                    self.reconcile();
                } else if cancelled_switcher {
                    self.dirty = true;
                    self.reconcile();
                }
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let position = event.position_transformed(self.host_size);
                self.motion(position, event.time_msec());
            }
            InputEvent::PointerMotion { event, .. } => {
                let pointer = self.seat.get_pointer().expect("pointer initialized");
                let position = pointer.current_location() + event.delta();
                self.motion(position, event.time_msec());
            }
            InputEvent::PointerButton { event, .. } => {
                let button = event.button_code();
                let pressed = event.state() == ButtonState::Pressed;
                let pointer = self.seat.get_pointer().expect("pointer initialized");
                let position = pointer.current_location();
                if !pressed && self.suppressed_buttons.remove(&button) {
                    if self.drag.as_ref().is_some_and(|drag| drag.button == button) {
                        self.end_drag();
                    }
                    if button == 0x110 {
                        self.titlebar_drag = None;
                        if let Some((id, part)) = self.titlebar_press.take()
                            && matches!(self.hit_test(position).map(|hit| hit.owner),
                                Some(HitOwner::Decoration(target, hit)) if target == id && hit == part)
                        {
                            self.activate_titlebar(id, part);
                        }
                    }
                    self.dirty = true;
                    self.reconcile();
                    return;
                }
                if pressed && self.drag.is_none() && !pointer.is_grabbed() {
                    let logo = self
                        .seat
                        .get_keyboard()
                        .expect("keyboard initialized")
                        .modifier_state()
                        .logo;
                    let owner = self.hit_test(position).map(|hit| hit.owner);
                    // Compositor gestures take precedence over SSD controls as well as clients.
                    if let Some(id) = modifier_drag_target(&owner, logo, button)
                        && self
                            .runtime
                            .desktop
                            .window(id)
                            .is_some_and(|w| w.role != WindowRole::Launcher)
                    {
                        self.layer_focus = None;
                        self.runtime.desktop.command(Command::Focus(id));
                        self.suppressed_buttons.insert(button);
                        self.titlebar_press = None;
                        self.titlebar_drag = None;
                        let edges = if button == 0x111 {
                            self.placements
                                .iter()
                                .find(|p| p.window == id)
                                .map(|p| resize_corner(p.rect, position))
                                .unwrap_or(10)
                        } else {
                            0
                        };
                        self.begin_drag(id, position, edges, button);
                        self.dirty = true;
                        self.reconcile();
                        return;
                    }
                    match owner {
                        Some(HitOwner::Layer(surface)) => self.focus_layer(&surface),
                        Some(HitOwner::Decoration(id, part)) => {
                            self.layer_focus = None;
                            self.runtime.desktop.command(Command::Focus(id));
                            self.suppressed_buttons.insert(button);
                            if button == 0x110 {
                                if part == TitlebarPart::Drag {
                                    self.titlebar_drag = Some((id, position));
                                } else {
                                    self.titlebar_press = Some((id, part));
                                }
                            }
                            self.dirty = true;
                            self.reconcile();
                            return;
                        }
                        Some(HitOwner::Window(id)) => {
                            self.layer_focus = None;
                            self.runtime.desktop.command(Command::Focus(id));
                            self.dirty = true;
                        }
                        None => {
                            self.layer_focus = None;
                            self.dirty = true;
                            if let Some(region) = self
                                .outputs
                                .iter()
                                .find(|o| logical(o.rect).contains(position.to_i32_floor()))
                            {
                                self.runtime
                                    .desktop
                                    .command(Command::FocusOutput(region.id));
                                self.dirty = true;
                            }
                        }
                    }
                    self.reconcile();
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: event.state(),
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
                if !pressed && self.drag.as_ref().is_some_and(|drag| drag.button == button) {
                    self.end_drag();
                }
            }
            InputEvent::PointerAxis { event, .. } => {
                let horizontal = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0
                });
                let vertical = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0
                });
                if self
                    .seat
                    .get_keyboard()
                    .expect("keyboard initialized")
                    .modifier_state()
                    .logo
                {
                    self.action(Action::Scroll {
                        amount: (if horizontal != 0.0 {
                            horizontal
                        } else {
                            vertical
                        } * 3.0) as i32,
                    });
                    return;
                }
                let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                for (axis, amount) in [(Axis::Horizontal, horizontal), (Axis::Vertical, vertical)] {
                    if amount != 0.0 {
                        frame = frame.value(axis, amount);
                        if let Some(discrete) = event.amount_v120(axis) {
                            frame = frame.v120(axis, discrete as i32);
                        }
                    } else if event.source() == AxisSource::Finger
                        && event.amount(axis) == Some(0.0)
                    {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().expect("pointer initialized");
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    fn activate_titlebar(&mut self, id: WindowId, part: TitlebarPart) {
        match part {
            TitlebarPart::Close => {
                if let Some(top) = self
                    .windows
                    .get(&id)
                    .and_then(|entry| entry.window.toplevel())
                {
                    top.send_close();
                }
            }
            TitlebarPart::Minimize => {
                self.runtime
                    .desktop
                    .command(Command::SetMinimized(id, true));
            }
            TitlebarPart::Maximize => {
                if let Some(window) = self.runtime.desktop.window(id) {
                    let maximized = !window.maximized;
                    self.runtime
                        .desktop
                        .command(Command::SetMaximized(id, maximized));
                }
            }
            TitlebarPart::Drag => {}
        }
    }

    fn motion(&mut self, position: Point<f64, Logical>, time: u32) {
        let position = Point::from((
            position
                .x
                .clamp(0.0, f64::from((self.host_size.w - 1).max(0))),
            position
                .y
                .clamp(0.0, f64::from((self.host_size.h - 1).max(0))),
        ));
        if let Some((id, origin)) = self.titlebar_drag
            && titlebar_drag_ready(origin, position)
        {
            self.titlebar_drag = None;
            self.begin_drag(id, origin, 0, 0x110);
        }
        if let Some(drag) = &self.drag {
            let id = drag.window;
            let delta = position - drag.origin;
            let dx = delta.x.round() as i32;
            let dy = delta.y.round() as i32;
            if let Some(session) = &drag.resize {
                if !self.runtime.desktop.update_resize(session, dx, dy) {
                    self.end_drag();
                }
            } else {
                let mut rect = drag.rect;
                rect.x = rect.x.saturating_add(dx);
                rect.y = rect.y.saturating_add(dy);
                if let Some(region) = self
                    .outputs
                    .iter()
                    .find(|o| logical(o.rect).contains(position.to_i32_floor()))
                {
                    if self
                        .runtime
                        .desktop
                        .window(id)
                        .is_some_and(|w| w.output != Some(region.id))
                    {
                        self.runtime
                            .desktop
                            .command(Command::MoveToOutput(region.id));
                    }
                }
                self.runtime
                    .desktop
                    .command(Command::SetFloatingRect(id, rect));
            }
            self.dirty = true;
            self.reconcile();
        }
        let pointer = self.seat.get_pointer().expect("pointer initialized");
        let target = if self.drag.is_some() {
            None
        } else {
            self.surface_under(position)
        };
        pointer.motion(
            self,
            target,
            &MotionEvent {
                location: position,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    pub fn begin_drag(
        &mut self,
        id: WindowId,
        position: Point<f64, Logical>,
        edges: u32,
        button: u32,
    ) {
        if self.drag.is_some()
            || edges & !15 != 0
            || self
                .runtime
                .desktop
                .window(id)
                .is_none_or(|w| w.role == WindowRole::Launcher || w.maximized || w.minimized)
        {
            return;
        }
        let Some(placement) = self.placements.iter().find(|p| p.window == id).cloned() else {
            return;
        };
        self.runtime.desktop.command(Command::Focus(id));
        let resize = if edges == 0 {
            if placement.tiled {
                self.runtime.desktop.command(Command::ToggleFloating);
            }
            self.runtime
                .desktop
                .command(Command::SetFloatingRect(id, placement.rect));
            None
        } else {
            let Some(session) = self.runtime.desktop.begin_resize(id, resize_edges(edges)) else {
                return;
            };
            Some(session)
        };
        if resize.is_some() {
            if let Some(top) = self.windows.get(&id).and_then(|w| w.window.toplevel()) {
                top.with_pending_state(|pending| {
                    pending.states.set(xdg_toplevel::State::Resizing);
                });
            }
        }
        self.drag = Some(Drag {
            window: id,
            origin: position,
            rect: placement.rect,
            resize,
            button,
        });
        self.dirty = true;
    }

    pub fn end_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            if let Some(top) = self
                .windows
                .get(&drag.window)
                .filter(|_| drag.resize.is_some())
                .and_then(|w| w.window.toplevel())
            {
                top.with_pending_state(|pending| {
                    pending.states.unset(xdg_toplevel::State::Resizing);
                });
                top.send_pending_configure();
            }
            self.dirty = true;
        }
    }
}
