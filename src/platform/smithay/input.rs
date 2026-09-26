use super::{
    scene::{HitOwner, logical},
    state::{Compositor, Drag},
};
use crate::{
    core::{Command, WindowId, WindowRole},
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
                    self.dirty = true;
                    return;
                }
                if pressed && self.drag.is_none() && !pointer.is_grabbed() {
                    let logo = self
                        .seat
                        .get_keyboard()
                        .expect("keyboard initialized")
                        .modifier_state()
                        .logo;
                    match self.hit_test(position).map(|hit| hit.owner) {
                        Some(HitOwner::Layer(surface)) => self.focus_layer(&surface),
                        Some(HitOwner::Window(id)) => {
                            self.layer_focus = None;
                            self.runtime.desktop.command(Command::Focus(id));
                            self.dirty = true;
                            if logo
                                && (button == 0x110 || button == 0x111)
                                && self
                                    .runtime
                                    .desktop
                                    .window(id)
                                    .is_some_and(|w| w.role != WindowRole::Launcher)
                            {
                                self.suppressed_buttons.insert(button);
                                self.begin_drag(
                                    id,
                                    position,
                                    if button == 0x111 { 10 } else { 0 },
                                    button,
                                );
                                self.reconcile();
                                return;
                            }
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

    fn motion(&mut self, position: Point<f64, Logical>, time: u32) {
        let position = Point::from((
            position
                .x
                .clamp(0.0, f64::from((self.host_size.w - 1).max(0))),
            position
                .y
                .clamp(0.0, f64::from((self.host_size.h - 1).max(0))),
        ));
        if let Some(drag) = &self.drag {
            let id = drag.window;
            let delta = position - drag.origin;
            let dx = delta.x.round() as i32;
            let dy = delta.y.round() as i32;
            let mut rect = drag.rect;
            if drag.edges == 0 {
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
            } else {
                if drag.edges & 8 != 0 {
                    rect.width = rect.width.saturating_add(dx).max(64);
                }
                if drag.edges & 2 != 0 {
                    rect.height = rect.height.saturating_add(dy).max(48);
                }
                if drag.edges & 4 != 0 {
                    let width = rect.width.saturating_sub(dx).max(64);
                    rect.x = rect.x.saturating_add(rect.width - width);
                    rect.width = width;
                }
                if drag.edges & 1 != 0 {
                    let height = rect.height.saturating_sub(dy).max(48);
                    rect.y = rect.y.saturating_add(rect.height - height);
                    rect.height = height;
                }
            }
            self.runtime
                .desktop
                .command(Command::SetFloatingRect(id, rect));
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
        if self
            .runtime
            .desktop
            .window(id)
            .is_none_or(|w| w.role == WindowRole::Launcher)
        {
            return;
        }
        let Some(placement) = self.placements.iter().find(|p| p.window == id).cloned() else {
            return;
        };
        self.runtime.desktop.command(Command::Focus(id));
        if placement.tiled {
            self.runtime.desktop.command(Command::ToggleFloating);
        }
        self.runtime
            .desktop
            .command(Command::SetFloatingRect(id, placement.rect));
        if edges != 0 {
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
            edges,
            button,
        });
        self.dirty = true;
    }

    pub fn end_drag(&mut self) {
        if let Some(drag) = self.drag.take() {
            if let Some(top) = self
                .windows
                .get(&drag.window)
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
