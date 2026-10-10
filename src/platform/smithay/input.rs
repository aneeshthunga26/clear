use super::{
    overview::OverviewDrag,
    scene::{HitOwner, logical},
    state::{Compositor, Drag},
    titlebar::TitlebarPart,
};
use crate::{
    core::{Command, Rect, ResizeEdges, WindowId, WindowRole},
    input::{Action, Bindings, Modifiers},
    runtime::{OverviewNavigation, OverviewTarget},
};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::{FilterResult, Keycode, Keysym, xkb},
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

fn is_modifier(symbol: Keysym) -> bool {
    matches!(
        symbol,
        Keysym::Shift_L
            | Keysym::Shift_R
            | Keysym::Control_L
            | Keysym::Control_R
            | Keysym::Alt_L
            | Keysym::Alt_R
            | Keysym::Super_L
            | Keysym::Super_R
            | Keysym::Meta_L
            | Keysym::Meta_R
            | Keysym::ISO_Level3_Shift
            | Keysym::ISO_Level5_Shift
    )
}

fn overview_navigation(symbols: &[Keysym], shift: bool, ctrl: bool) -> Option<OverviewNavigation> {
    symbols.iter().find_map(|symbol| {
        Some(match *symbol {
            Keysym::Tab | Keysym::ISO_Left_Tab => {
                if shift {
                    OverviewNavigation::Previous
                } else {
                    OverviewNavigation::Next
                }
            }
            Keysym::Left if ctrl => OverviewNavigation::WorkspacePrevious,
            Keysym::Right if ctrl => OverviewNavigation::WorkspaceNext,
            Keysym::Left => OverviewNavigation::Left,
            Keysym::Right => OverviewNavigation::Right,
            Keysym::Up => OverviewNavigation::Up,
            Keysym::Down => OverviewNavigation::Down,
            _ => return None,
        })
    })
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
        let config =
            Config::from_source(include_str!("../../../examples/dual-virtual-monitors.toml"))
                .unwrap();
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

    fn overview_input_fixture(
        event_loop: &smithay::reexports::calloop::EventLoop<Compositor>,
    ) -> Compositor {
        use crate::{core::OutputId, runtime::Runtime};
        use smithay::{
            output::{Output, PhysicalProperties, Subpixel},
            reexports::wayland_server::Display,
        };
        let runtime = Runtime::new(Config::default(), None).unwrap();
        let mut state = Compositor::new(
            event_loop,
            Display::new().unwrap(),
            runtime,
            Some("overview-input"),
        )
        .unwrap();
        let output = Output::new(
            "test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: "test".into(),
            },
        );
        state.outputs.push(super::super::state::OutputRegion {
            id: OutputId(1),
            output,
            rect: Rect::new(0, 0, 800, 600),
        });
        state
            .runtime
            .desktop
            .add_output(OutputId(1), "test".into(), Rect::new(0, 0, 800, 600));
        state
            .runtime
            .desktop
            .add_window(WindowId(1), "one".into(), "test".into());
        state
            .runtime
            .desktop
            .add_window(WindowId(2), "two".into(), "test".into());
        state.host_size = (800, 600).into();
        assert!(state.shell_server.is_none());
        state
    }

    #[test]
    fn fullscreen_refuses_drag_without_mutation_and_restore_allows_drag() {
        const CHILD: &str = "CLEAR_FULLSCREEN_DRAG_TEST";
        if std::env::var_os(CHILD).is_none() {
            let directory =
                std::env::temp_dir().join(format!("clear-fullscreen-drag-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "platform::smithay::input::tests::fullscreen_refuses_drag_without_mutation_and_restore_allows_drag",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("XDG_RUNTIME_DIR", &directory)
                .status()
                .unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(status.success());
            return;
        }

        let event_loop = smithay::reexports::calloop::EventLoop::try_new().unwrap();
        let mut state = overview_input_fixture(&event_loop);
        let id = WindowId(1);
        let saved = Rect::new(31, 47, 280, 190);
        state
            .runtime
            .desktop
            .command(Command::SetFloatingRect(id, saved));
        state
            .runtime
            .desktop
            .command(Command::SetFullscreen(id, true));
        state.placements = state.runtime.placements();
        state.dirty = false;
        let original = state.runtime.desktop.window(id).unwrap().clone();
        let focused = state.runtime.desktop.focused_window();
        let output = state.runtime.desktop.focused_output();
        assert_eq!(focused, Some(WindowId(2)));
        assert_eq!(original.floating_rect, saved);
        let point = Point::from((120.0, 90.0));

        // Exercise the shared adapter entry for compositor and validated native
        // move/resize gestures; no physical device or client serial is injected.
        for edges in [0, 10] {
            state.begin_drag(id, point, edges, 0x110);
            assert!(state.drag.is_none());
            assert!(!state.dirty);
            assert_eq!(state.runtime.desktop.window(id), Some(&original));
            assert_eq!(state.runtime.desktop.focused_window(), focused);
            assert_eq!(state.runtime.desktop.focused_output(), output);
        }

        state
            .runtime
            .desktop
            .command(Command::SetFullscreen(id, false));
        state.placements = state.runtime.placements();
        state.begin_drag(id, point, 10, 0x111);
        let drag = state.drag.as_ref().expect("restored columns allow resize");
        assert_eq!(drag.window, id);
        assert!(drag.resize.is_some());
        assert_eq!(
            state.runtime.desktop.window(id).unwrap().floating_rect,
            saved
        );
        assert!(!state.runtime.desktop.window(id).unwrap().floating);
        state.end_drag();

        let restored = state
            .placements
            .iter()
            .find(|p| p.window == id)
            .unwrap()
            .rect;
        state.begin_drag(id, point, 0, 0x110);
        let drag = state.drag.as_ref().expect("restored tile allows move");
        assert_eq!(drag.window, id);
        assert!(drag.resize.is_none());
        assert!(state.runtime.desktop.window(id).unwrap().floating);
        assert_eq!(
            state.runtime.desktop.window(id).unwrap().floating_rect,
            restored
        );
        state.end_drag();
    }

    #[test]
    fn workspace_motion_suppresses_press_and_release_after_motion_finishes() {
        const CHILD: &str = "CLEAR_WORKSPACE_INPUT_TEST";
        if std::env::var_os(CHILD).is_none() {
            let directory =
                std::env::temp_dir().join(format!("clear-workspace-input-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "platform::smithay::input::tests::workspace_motion_suppresses_press_and_release_after_motion_finishes", "--nocapture"])
                .env(CHILD, "1").env("XDG_RUNTIME_DIR", &directory).status().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(status.success());
            return;
        }
        let event_loop = smithay::reexports::calloop::EventLoop::try_new().unwrap();
        let mut state = overview_input_fixture(&event_loop);
        let output = crate::core::OutputId(1);
        let pointer = state.seat.get_pointer().unwrap();
        state.motion((200.0, 200.0).into(), 0);
        let focus = state.runtime.desktop.focused_window();
        state.window_animations.test_workspace_motion(output, true);
        state.pointer_button(0x110, ButtonState::Pressed, 1);
        assert!(state.suppressed_buttons.contains(&0x110));
        assert!(!pointer.is_grabbed());
        assert!(state.drag.is_none());
        assert_eq!(state.runtime.desktop.focused_window(), focus);
        // Completing motion cannot leak the intercepted press's release to clients.
        state.window_animations.test_workspace_motion(output, false);
        state.pointer_button(0x110, ButtonState::Released, 2);
        assert!(state.suppressed_buttons.is_empty());
        assert!(!pointer.is_grabbed());
        // A new ordinary press after completion restores the seat's normal path.
        state.pointer_button(0x110, ButtonState::Pressed, 3);
        assert!(pointer.is_grabbed());
        state.pointer_button(0x110, ButtonState::Released, 4);
        assert!(!pointer.is_grabbed());
    }

    #[test]
    fn overview_hover_click_and_drag_without_shell_ipc() {
        const CHILD: &str = "CLEAR_OVERVIEW_DRAG_TEST";
        if std::env::var_os(CHILD).is_none() {
            let directory =
                std::env::temp_dir().join(format!("clear-overview-drag-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "platform::smithay::input::tests::overview_hover_click_and_drag_without_shell_ipc", "--nocapture"])
                .env(CHILD, "1").env("XDG_RUNTIME_DIR", &directory).status().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(status.success());
            return;
        }
        use crate::core::WorkspaceId;
        let event_loop = smithay::reexports::calloop::EventLoop::try_new().unwrap();
        let mut state = overview_input_fixture(&event_loop);
        let location = |state: &Compositor, target| {
            let layout = state.overview_layout().unwrap();
            let item = layout.items.iter().find(|i| i.target == target).unwrap();
            let rect = item.preview.unwrap_or(item.rect);
            Point::from((
                (rect.x + rect.width / 2) as f64,
                (rect.y + rect.height / 2) as f64,
            ))
        };
        let ws1 = OverviewTarget::Workspace(WorkspaceId(1));
        let ws3 = OverviewTarget::Workspace(WorkspaceId(3));
        state.runtime.toggle_overview();
        let desktop3 = location(&state, ws3);
        let selected = state.runtime.overview.as_ref().unwrap().selected;
        state.motion(desktop3, 0);
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().workspace,
            WorkspaceId(1)
        );
        assert_eq!(state.runtime.overview.as_ref().unwrap().selected, selected);
        state.runtime.config.overview.preview_workspace_on_hover = true;
        state.motion(desktop3, 1);
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().workspace,
            WorkspaceId(3)
        );
        state.runtime.config.overview.preview_workspace_on_hover = false;
        state
            .runtime
            .overview
            .as_mut()
            .unwrap()
            .select(&state.runtime.desktop, ws1);
        state.pointer_button(0x110, ButtonState::Pressed, 2);
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().workspace,
            WorkspaceId(1)
        );
        state.pointer_button(0x110, ButtonState::Released, 3);
        assert!(state.runtime.overview.is_none());
        assert_eq!(
            state
                .runtime
                .desktop
                .workspace_for_output(crate::core::OutputId(1)),
            Some(WorkspaceId(3))
        );
        // Click back without relying on hover changing the selection.
        state.runtime.toggle_overview();
        state.motion(location(&state, ws1), 4);
        state.pointer_button(0x110, ButtonState::Pressed, 5);
        state.pointer_button(0x110, ButtonState::Released, 6);
        assert!(state.runtime.overview.is_none());
        assert_eq!(
            state
                .runtime
                .desktop
                .workspace_for_output(crate::core::OutputId(1)),
            Some(WorkspaceId(1))
        );
        state.runtime.toggle_overview();
        let clicked = location(&state, OverviewTarget::Window(WindowId(2)));
        state.motion(clicked, 6);
        state.pointer_button(0x110, ButtonState::Pressed, 6);
        state.motion(clicked + Point::from((2.0, 2.0)), 6);
        state.pointer_button(0x110, ButtonState::Released, 6);
        assert!(state.runtime.overview.is_none());
        assert!(state.suppressed_buttons.is_empty());
        state.runtime.toggle_overview();
        let origin = location(&state, OverviewTarget::Window(WindowId(1)));
        let saved = state
            .runtime
            .desktop
            .window(WindowId(1))
            .unwrap()
            .floating_rect;
        let focused = state.runtime.desktop.focused_window();
        state.motion(origin, 7);
        state.pointer_button(0x110, ButtonState::Pressed, 8);
        state.motion(origin + Point::from((2.0, 2.0)), 9);
        assert!(!state.overview_drag.as_ref().unwrap().active);
        state.runtime.config.overview.preview_workspace_on_hover = true;
        state.motion(location(&state, ws3), 10);
        let drag = state.overview_drag.as_ref().unwrap();
        assert!(drag.active);
        assert_eq!(drag.destination, Some(WorkspaceId(3)));
        assert!(
            drag.ghost(state.overview_layout().unwrap().output)
                .is_some()
        );
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().workspace,
            WorkspaceId(1)
        );
        assert_eq!(
            state.runtime.desktop.window(WindowId(1)).unwrap().workspace,
            WorkspaceId(1)
        );
        state.keyboard_key(Keycode::new(106 + 8), KeyState::Pressed, 11);
        state.keyboard_key(Keycode::new(106 + 8), KeyState::Released, 12);
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().selected,
            OverviewTarget::Window(WindowId(1))
        );
        state.pointer_button(0x110, ButtonState::Released, 13);
        assert!(state.overview_drag.is_none());
        assert!(state.suppressed_buttons.is_empty());
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().workspace,
            WorkspaceId(1)
        );
        assert_eq!(
            state.runtime.desktop.window(WindowId(1)).unwrap().workspace,
            WorkspaceId(3)
        );
        assert_eq!(
            state
                .runtime
                .desktop
                .window(WindowId(1))
                .unwrap()
                .floating_rect,
            saved
        );
        assert_eq!(state.runtime.desktop.focused_window(), focused);
        let origin = location(&state, OverviewTarget::Window(WindowId(2)));
        // Same-desktop and outside-target drops stay open and change nothing.
        for destination in [location(&state, ws1), Point::from((0.0, 599.0))] {
            state.motion(origin, 14);
            state.pointer_button(0x110, ButtonState::Pressed, 15);
            state.motion(destination, 16);
            assert!(state.overview_drag.as_ref().unwrap().active);
            assert!(state.overview_drag.as_ref().unwrap().destination.is_none());
            state.pointer_button(0x110, ButtonState::Released, 17);
            assert_eq!(
                state.runtime.desktop.window(WindowId(2)).unwrap().workspace,
                WorkspaceId(1)
            );
            assert!(state.runtime.overview.is_some());
        }
        // Escape clears the ghost and suppresses the later physical release.
        state.motion(origin, 18);
        state.pointer_button(0x110, ButtonState::Pressed, 19);
        state.motion(desktop3, 20);
        state.keyboard_key(Keycode::new(1 + 8), KeyState::Pressed, 21);
        assert!(state.runtime.overview.is_none());
        assert!(state.overview_drag.is_none());
        state.pointer_button(0x110, ButtonState::Released, 22);
        state.keyboard_key(Keycode::new(1 + 8), KeyState::Released, 23);
        assert!(state.suppressed_buttons.is_empty());
        assert!(state.suppressed_keys.is_empty());
        assert!(!state.seat.get_pointer().unwrap().is_grabbed());
        // Independent transfers invalidate an armed source; release cannot activate.
        state.runtime.toggle_overview();
        state.motion(location(&state, OverviewTarget::Window(WindowId(2))), 24);
        state.pointer_button(0x110, ButtonState::Pressed, 25);
        state.motion(desktop3, 26);
        state
            .runtime
            .desktop
            .command(Command::MoveWindowToWorkspace(WindowId(2), WorkspaceId(3)));
        state.dirty = true;
        state.reconcile();
        assert!(state.overview_drag.is_none());
        assert!(state.overview_press.is_none());
        state.pointer_button(0x110, ButtonState::Released, 27);
        assert!(state.runtime.overview.is_some());
        assert_eq!(
            state
                .runtime
                .desktop
                .workspace_for_output(crate::core::OutputId(1)),
            Some(WorkspaceId(1))
        );
        for invalidation in 0..3 {
            state
                .runtime
                .desktop
                .add_window(WindowId(3), "three".into(), "test".into());
            state.dirty = true;
            state.reconcile();
            state.motion(location(&state, OverviewTarget::Window(WindowId(3))), 28);
            state.pointer_button(0x110, ButtonState::Pressed, 29);
            state.motion(desktop3, 30);
            match invalidation {
                0 => state.runtime.desktop.remove_window(WindowId(3)),
                1 => state
                    .runtime
                    .desktop
                    .set_window_role(WindowId(3), WindowRole::Launcher),
                _ => state.outputs[0].rect.width = 799,
            }
            state.dirty = true;
            state.reconcile();
            assert!(state.overview_drag.is_none());
            assert!(state.overview_press.is_none());
            state.pointer_button(0x110, ButtonState::Released, 31);
            assert!(state.runtime.overview.is_some());
            assert!(state.suppressed_buttons.is_empty());
            state.runtime.desktop.remove_window(WindowId(3));
            state.outputs[0].rect.width = 800;
            state.dirty = true;
            state.reconcile();
        }
    }

    #[test]
    fn overview_keyboard_pairs_and_deferred_entry_without_shell_ipc() {
        // A subprocess supplies a private runtime directory without mutating
        // global environment shared by parallel unit tests.
        const CHILD: &str = "CLEAR_OVERVIEW_INPUT_TEST";
        if std::env::var_os(CHILD).is_none() {
            let directory =
                std::env::temp_dir().join(format!("clear-overview-input-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "platform::smithay::input::tests::overview_keyboard_pairs_and_deferred_entry_without_shell_ipc", "--nocapture"])
                .env(CHILD, "1").env("XDG_RUNTIME_DIR", &directory).status().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(status.success());
            return;
        }
        let event_loop = smithay::reexports::calloop::EventLoop::try_new().unwrap();
        let mut state = overview_input_fixture(&event_loop);
        let press = KeyState::Pressed;
        let release = KeyState::Released;
        // evdev codes plus XKB's offset of 8: Super, W, Tab, Escape, Enter, A.
        let key = |code: u32| Keycode::new(code + 8);
        state.keyboard_key(key(125), press, 0);
        state.keyboard_key(key(17), press, 1);
        assert!(state.runtime.overview.is_some());
        assert_eq!(state.runtime.desktop.focused_window(), Some(WindowId(2)));
        state.keyboard_key(key(125), release, 2);
        state.keyboard_key(key(17), release, 3);
        assert!(!state.suppressed_keys.contains(&key(17).raw()));
        assert!(state.forwarded_keys.is_empty());
        state.keyboard_key(key(15), press, 4);
        assert_eq!(
            state.runtime.overview.as_ref().unwrap().selected,
            crate::runtime::OverviewTarget::Workspace(crate::core::WorkspaceId(1))
        );
        state.keyboard_key(key(1), press, 5);
        assert!(state.runtime.overview.is_none());
        assert_eq!(state.runtime.desktop.focused_window(), Some(WindowId(2)));
        // Both releases remain intercepted after closing; neither becomes a
        // forwarded press/release or a desktop shortcut on the restored scene.
        state.keyboard_key(key(15), release, 6);
        state.keyboard_key(key(1), release, 7);
        assert!(state.suppressed_keys.is_empty());
        assert!(state.forwarded_keys.is_empty());
        state.keyboard_key(key(30), press, 8);
        state.keyboard_key(key(125), press, 9);
        state.keyboard_key(key(17), press, 10);
        assert!(state.runtime.overview_requested);
        assert!(state.runtime.overview.is_none());
        state.keyboard_key(key(30), release, 11);
        assert!(state.runtime.overview.is_some());
        state.keyboard_key(key(17), release, 12);
        state.keyboard_key(key(125), release, 13);
        state.keyboard_key(key(103), press, 14); // Up selects workspace strip.
        state.keyboard_key(key(103), release, 15);
        state.keyboard_key(key(108), press, 16); // Down enters first card.
        state.keyboard_key(key(108), release, 17);
        state.keyboard_key(key(28), press, 18);
        assert!(state.runtime.overview.is_none());
        assert_eq!(state.runtime.desktop.focused_window(), Some(WindowId(1)));
        state.keyboard_key(key(28), release, 19);
        assert!(state.suppressed_keys.is_empty());
        assert!(state.forwarded_keys.is_empty());
        // A modifier first pressed in overview may remain held across activation;
        // its release is suppressed as a key but clears the restored modifier state.
        state.runtime.toggle_overview();
        state.keyboard_key(key(42), press, 20);
        let keyboard = state.seat.get_keyboard().unwrap();
        assert!(keyboard.modifier_state().shift);
        state.keyboard_key(key(28), press, 21);
        assert!(state.runtime.overview.is_none());
        state.keyboard_key(key(28), release, 22);
        state.keyboard_key(key(42), release, 23);
        assert!(!keyboard.modifier_state().shift);
        assert!(state.suppressed_keys.is_empty());
        // Suppressed compositor-button pairs and host loss defer a queued entry.
        state.runtime.overview_requested = true;
        state.suppressed_buttons.insert(0x110);
        state.service_overview();
        assert!(state.runtime.overview.is_none());
        state.suppressed_buttons.clear();
        state.host_focused = false;
        state.service_overview();
        assert!(state.runtime.overview.is_none());
        state.host_focused = true;
        state.service_overview();
        assert!(state.runtime.overview.is_some());
        state.host_size = (800, 600).into();
        // A left click activates only on matching release. A press cancelled by
        // Escape still suppresses its release after keyboard focus is restored.
        let layout = state.overview_layout().unwrap();
        let card = layout
            .items
            .iter()
            .find(|i| i.target == crate::runtime::OverviewTarget::Window(WindowId(2)))
            .unwrap();
        let position = ((card.rect.x + 10) as f64, (card.rect.y + 10) as f64).into();
        state.motion(position, 20);
        state.pointer_button(0x110, ButtonState::Pressed, 21);
        assert!(state.runtime.overview.is_some());
        state.pointer_button(0x110, ButtonState::Released, 22);
        assert!(state.runtime.overview.is_none());
        assert_eq!(state.runtime.desktop.focused_window(), Some(WindowId(2)));
        assert!(state.suppressed_buttons.is_empty());
        let pointer = state.seat.get_pointer().unwrap();
        assert!(!pointer.is_grabbed());
        state.runtime.overview_requested = true;
        state.service_overview();
        state.pointer_button(0x110, ButtonState::Pressed, 23);
        state.runtime.cancel_overview();
        state.pointer_button(0x110, ButtonState::Released, 24);
        assert!(state.suppressed_buttons.is_empty());
        assert!(!pointer.is_grabbed());
        // Real seat implicit pointer grabs defer entry until the held button ends.
        state.pointer_button(0x111, ButtonState::Pressed, 25);
        assert!(pointer.is_grabbed());
        state.runtime.overview_requested = true;
        state.service_overview();
        assert!(state.runtime.overview.is_none());
        state.pointer_button(0x111, ButtonState::Released, 26);
        assert!(state.runtime.overview.is_some());
    }
}

impl Compositor {
    fn keyboard_key(&mut self, code: Keycode, key_state: KeyState, time: u32) {
        let pressed = key_state == KeyState::Pressed;
        let keyboard = self.seat.get_keyboard().expect("keyboard initialized");
        let modifiers_before = keyboard.modifier_state();
        let mut cancelled_switcher = false;
        let mut overview_changed = false;
        let action = keyboard.input::<Option<Action>, _>(
            self,
            code,
            key_state,
            SERIAL_COUNTER.next_serial(),
            time,
            |state, mods, key| {
                if !pressed && state.suppressed_keys.remove(&code.raw()) {
                    return FilterResult::Intercept(None);
                }
                if pressed && state.suppressed_keys.contains(&code.raw()) {
                    return FilterResult::Intercept(None);
                }
                if state.overview_present() {
                    if pressed {
                        state.suppressed_keys.insert(code.raw());
                        let symbols = key.raw_syms();
                        let modifiers = Modifiers {
                            ctrl: mods.ctrl,
                            alt: mods.alt,
                            shift: mods.shift,
                            logo: mods.logo,
                        };
                        let toggle = matches!(
                            binding_action(&state.runtime.bindings, modifiers, symbols.clone()),
                            Some(Action::ToggleOverview)
                        );
                        if toggle && state.runtime.overview.is_none() {
                            state.runtime.toggle_overview();
                        } else if toggle || symbols.contains(&Keysym::Escape) {
                            state.runtime.cancel_overview();
                        } else if state.overview_drag.as_ref().is_some_and(|d| d.active) {
                            return FilterResult::Intercept(None);
                        } else if symbols.contains(&Keysym::Return)
                            || symbols.contains(&Keysym::KP_Enter)
                        {
                            state.runtime.finish_overview();
                        } else if let Some(direction) =
                            overview_navigation(&symbols, mods.shift, mods.ctrl)
                        {
                            let columns = state.overview_layout().map_or(1, |l| l.columns);
                            if let Some(session) = &mut state.runtime.overview {
                                session.navigate(&state.runtime.desktop, direction, columns);
                            }
                        }
                        overview_changed = true;
                        state.overview_press = None;
                        state.overview_drag = None;
                        return FilterResult::Intercept(None);
                    }
                    // Keys pressed before entry (leader modifiers) still update
                    // Smithay's forwarded-key bookkeeping while focus is absent.
                    state.forwarded_keys.remove(&code.raw());
                    return FilterResult::Forward;
                }
                if pressed
                    && state.runtime.overview_requested
                    && key.raw_syms().contains(&Keysym::Escape)
                {
                    state.runtime.cancel_overview();
                    state.suppressed_keys.insert(code.raw());
                    overview_changed = true;
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
                    let action = binding_action(&state.runtime.bindings, modifiers, key.raw_syms());
                    if let Some(action) = action {
                        state.suppressed_keys.insert(code.raw());
                        return FilterResult::Intercept(Some(action));
                    }
                }
                if pressed {
                    state.forwarded_keys.insert(
                        code.raw(),
                        key.raw_syms().iter().all(|sym| is_modifier(*sym)),
                    );
                } else {
                    state.forwarded_keys.remove(&code.raw());
                }
                FilterResult::Forward
            },
        );
        let intercepted = action.is_some();
        if let Some(Some(action)) = action {
            if matches!(action, Action::AltTab) {
                self.runtime.advance_switcher();
                self.dirty = true;
                self.reconcile();
            } else {
                self.action(action);
            }
        }
        self.service_overview();
        if overview_changed {
            self.dirty = true;
            self.reconcile();
        }
        if self.runtime.switcher.is_some() && !keyboard.modifier_state().alt {
            self.runtime.finish_switcher();
            self.dirty = true;
            self.reconcile();
        } else if cancelled_switcher {
            self.dirty = true;
            self.reconcile();
        }
        self.reconcile();
        if intercepted && keyboard.modifier_state() != modifiers_before {
            // Interception suppresses raw key delivery, including input_forward's
            // modifier notification. A modifier held across overview exit must
            // still update the restored client when its suppressed release arrives.
            keyboard.advertise_modifier_state(self);
        }
    }

    fn pointer_button(&mut self, button: u32, button_state: ButtonState, time: u32) {
        let pressed = button_state == ButtonState::Pressed;
        let pointer = self.seat.get_pointer().expect("pointer initialized");
        let position = pointer.current_location();
        if self.overview_present() && self.runtime.overview.is_none() {
            if pressed {
                self.suppressed_buttons.insert(button);
            } else {
                self.suppressed_buttons.remove(&button);
            }
            self.dirty = true;
            self.reconcile();
            return;
        }
        if self.overview_present() {
            if pressed {
                self.suppressed_buttons.insert(button);
                if button == 0x110 {
                    let target = self
                        .overview_layout()
                        .and_then(|l| l.hit(position.x, position.y));
                    self.overview_press = target;
                    self.overview_drag = None;
                    if let Some(OverviewTarget::Window(window)) = target
                        && let Some(session) = &self.runtime.overview
                        && let Some(preview) = self.overview_layout().and_then(|l| {
                            l.items
                                .iter()
                                .find(|i| i.target == OverviewTarget::Window(window))
                                .and_then(|i| i.preview)
                        })
                    {
                        self.overview_drag = Some(OverviewDrag {
                            window,
                            workspace: session.workspace,
                            output: session.output,
                            output_rect: self.overview_layout().expect("overview output").output,
                            origin: position,
                            position,
                            preview,
                            active: false,
                            destination: None,
                        });
                    }
                    if let Some(session) = &mut self.runtime.overview {
                        if let Some(target @ OverviewTarget::Window(_)) = target {
                            session.select(&self.runtime.desktop, target);
                        } else if target.is_none()
                            && let Some(region) = self
                                .outputs
                                .iter()
                                .find(|o| logical(o.rect).contains(position.to_i32_floor()))
                        {
                            if region.id != session.output {
                                session.select_output(&self.runtime.desktop, region.id);
                            }
                        }
                    }
                }
            } else {
                self.suppressed_buttons.remove(&button);
                if button == 0x110 {
                    let target = self
                        .overview_layout()
                        .and_then(|l| l.hit(position.x, position.y));
                    if let Some(drag) = self.overview_drag.take().filter(|d| d.active) {
                        self.overview_press = None;
                        if let Some(OverviewTarget::Workspace(destination)) = target
                            && let Some(session) = &mut self.runtime.overview
                        {
                            session.move_window(
                                &mut self.runtime.desktop,
                                drag.window,
                                destination,
                            );
                        }
                    } else if self
                        .overview_press
                        .take()
                        .is_some_and(|armed| Some(armed) == target)
                    {
                        if let Some(target) = target
                            && let Some(session) = &mut self.runtime.overview
                        {
                            session.select(&self.runtime.desktop, target);
                        }
                        self.runtime.finish_overview();
                    }
                }
            }
            self.dirty = true;
            self.reconcile();
            return;
        }
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
        // A moving application canvas never forwards a press without its matching release.
        // Static shell layers retain their normal ownership during workspace motion.
        if pressed
            && !pointer.is_grabbed()
            && self.outputs.iter().any(|output| {
                logical(output.rect).contains(position.to_i32_floor())
                    && self.window_animations.workspace_input_blocked(output.id)
            })
            && !matches!(
                self.hit_test(position).map(|hit| hit.owner),
                Some(HitOwner::Layer(_))
            )
        {
            self.suppressed_buttons.insert(button);
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
                state: button_state,
                serial: SERIAL_COUNTER.next_serial(),
                time: time,
            },
        );
        pointer.frame(self);
        if !pressed && self.drag.as_ref().is_some_and(|drag| drag.button == button) {
            self.end_drag();
        }
        self.service_overview();
        self.reconcile();
    }

    pub fn process_input<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                self.keyboard_key(event.key_code(), event.state(), event.time_msec());
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
                self.pointer_button(event.button_code(), event.state(), event.time_msec());
            }
            InputEvent::PointerAxis { event, .. } => {
                if self.overview_present() {
                    if self.overview_press.is_some() || self.overview_drag.is_some() {
                        return;
                    }
                    let amount = event
                        .amount(Axis::Vertical)
                        .or_else(|| event.amount_v120(Axis::Vertical))
                        .or_else(|| event.amount(Axis::Horizontal))
                        .unwrap_or(0.0);
                    if amount != 0.0 {
                        if let Some(session) = &mut self.runtime.overview {
                            session.navigate(
                                &self.runtime.desktop,
                                if amount > 0.0 {
                                    OverviewNavigation::WorkspaceNext
                                } else {
                                    OverviewNavigation::WorkspacePrevious
                                },
                                1,
                            );
                        }
                        self.dirty = true;
                        self.reconcile();
                    }
                    return;
                }
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
        self.service_overview();
        self.reconcile();
    }

    /// Defer entry while any client/compositor operation still owns input.
    pub fn service_overview(&mut self) {
        if !self.runtime.overview_requested {
            return;
        }
        let keyboard = self.seat.get_keyboard().expect("keyboard initialized");
        let pointer = self.seat.get_pointer().expect("pointer initialized");
        let blocked = keyboard.is_grabbed()
            || pointer.is_grabbed()
            || self.drag.is_some()
            || self.titlebar_press.is_some()
            || self.titlebar_drag.is_some()
            || self.has_exclusive_layer()
            || self.forwarded_keys.values().any(|modifier| !modifier)
            || !self.suppressed_buttons.is_empty()
            || !self.host_focused;
        if self.overview_present() || !blocked {
            self.runtime.toggle_overview();
            self.overview_press = None;
            self.overview_drag = None;
            self.dirty = true;
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
        if self.overview_present() {
            let target = self
                .overview_layout()
                .and_then(|l| l.hit(position.x, position.y));
            if let Some(drag) = &mut self.overview_drag {
                drag.position = position;
                drag.active |=
                    (position.x - drag.origin.x).hypot(position.y - drag.origin.y) >= 8.0;
                drag.destination = if drag.active {
                    match target {
                        Some(OverviewTarget::Workspace(id)) if id != drag.workspace => Some(id),
                        _ => None,
                    }
                } else {
                    None
                };
                self.dirty = true;
            } else if self.overview_press.is_none()
                && let Some(target) = target
                && (matches!(target, OverviewTarget::Window(_))
                    || self.runtime.config.overview.preview_workspace_on_hover)
            {
                if let Some(session) = &mut self.runtime.overview {
                    session.select(&self.runtime.desktop, target);
                }
                self.dirty = true;
            }
        }
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
        if self.overview_present()
            || self.runtime.overview_requested
            || self.drag.is_some()
            || self.outputs.iter().any(|output| {
                logical(output.rect).contains(position.to_i32_floor())
                    && self.window_animations.workspace_input_blocked(output.id)
            })
            || edges & !15 != 0
            || self.runtime.desktop.window(id).is_none_or(|w| {
                w.role == WindowRole::Launcher || w.maximized || w.fullscreen || w.minimized
            })
            || self
                .window_animations
                .presented_visual(id)
                .is_some_and(|visual| !visual.sampled.input_eligible)
        {
            return;
        }
        let Some(placement) = self.placements.iter().find(|p| p.window == id).cloned() else {
            return;
        };
        self.runtime.desktop.command(Command::Focus(id));
        let resize = if edges == 0 {
            self.settle_window_animation(id);
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
            self.settle_window_animation(id);
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
