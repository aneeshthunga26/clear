use clear::{
    config::Config,
    core::{
        Command, Desktop, Mode, OutputId, Rect, ResizeEdges, WindowId, WindowRole, WorkspaceId,
    },
    input::{Action, Bindings, Modifiers},
    runtime::Runtime,
    scripting::ScriptHost,
    shell::{Request, RequestKind, execute, parse_request, snapshot},
};

fn desktop(mode: Mode) -> Desktop {
    let mut desktop = Desktop::new();
    desktop.add_output(OutputId(1), "left".into(), Rect::new(0, 36, 1000, 764));
    desktop.command(Command::SetWorkspaceMode(mode));
    for id in 1..=3 {
        desktop.add_window(WindowId(id), format!("Window {id}"), "test".into());
    }
    desktop
}

fn rect(desktop: &Desktop, id: u64) -> Rect {
    desktop
        .placements()
        .iter()
        .find(|p| p.window == WindowId(id))
        .unwrap()
        .rect
}

#[test]
fn maximize_restore_preserves_every_modes_saved_state_and_order() {
    for mode in [
        Mode::Floating,
        Mode::Scrolling,
        Mode::MasterStack,
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Monocle,
        Mode::Script("test".into()),
    ] {
        let mut d = desktop(mode);
        let saved = Rect::new(-200, -100, 900, 700);
        d.command(Command::SetFloatingRect(WindowId(3), saved));
        let before = d.placements();
        let order = d.workspace(WorkspaceId(1)).unwrap().windows().to_vec();
        d.command(Command::ToggleMaximized);
        let p = d.placements().pop().unwrap();
        assert_eq!(p.window, WindowId(3));
        assert_eq!(p.rect, Rect::new(0, 36, 1000, 764));
        assert_eq!(p.clip, Some(p.rect));
        assert!(!p.tiled);
        assert!(!d.window(WindowId(3)).unwrap().floating);
        assert_eq!(d.window(WindowId(3)).unwrap().floating_rect, saved);
        d.set_window_committed_size(WindowId(3), 1000, 764);
        d.command(Command::ToggleMaximized);
        assert_eq!(d.placements(), before);
        assert_eq!(d.workspace(WorkspaceId(1)).unwrap().windows(), order);
    }
}

#[test]
fn resized_tile_proportions_return_after_maximize_and_minimize() {
    let mut d = desktop(Mode::Columns);
    let edges = ResizeEdges {
        right: true,
        ..Default::default()
    };
    let session = d.begin_resize(WindowId(1), edges).unwrap();
    assert!(d.update_resize(&session, 120, 0));
    let before = d.placements();
    for command in [
        Command::SetMaximized(WindowId(1), true),
        Command::SetMinimized(WindowId(1), true),
    ] {
        d.command(command);
        assert!(!d.update_resize(&session, 130, 0));
        assert!(d.begin_resize(WindowId(1), edges).is_none());
        d.command(Command::SetMaximized(WindowId(1), false));
        d.command(Command::SetMinimized(WindowId(1), false));
        assert_eq!(d.placements(), before);
    }
}

#[test]
fn minimize_keeps_ownership_and_maximize_but_excludes_layout_and_focus_cycle() {
    let mut d = desktop(Mode::Columns);
    d.command(Command::ToggleMaximized);
    d.command(Command::MinimizeFocused);
    assert_eq!(d.windows().count(), 3);
    assert_eq!(d.placements().len(), 2);
    assert!(d.window(WindowId(3)).unwrap().maximized);
    for _ in 0..7 {
        d.command(Command::FocusNext);
        assert_ne!(d.focused_window(), Some(WindowId(3)));
    }
    d.command(Command::SwitchWorkspace(WorkspaceId(2)));
    d.command(Command::SwitchWorkspace(WorkspaceId(1)));
    assert!(d.window(WindowId(3)).unwrap().minimized);
    d.command(Command::Focus(WindowId(3)));
    assert!(!d.window(WindowId(3)).unwrap().minimized);
    assert!(d.window(WindowId(3)).unwrap().maximized);
    assert_eq!(d.focused_window(), Some(WindowId(3)));
    assert_eq!(rect(&d, 3), d.output(OutputId(1)).unwrap().area);
    for id in 1..=3 {
        d.command(Command::SetMinimized(WindowId(id), true));
    }
    assert!(d.placements().is_empty());
    assert_eq!(d.focused_window(), None);
    d.command(Command::FocusNext);
    assert_eq!(d.focused_window(), None);
    d.command(Command::SetMinimized(WindowId(2), false));
    assert_eq!(d.focused_window(), Some(WindowId(2)));
}

#[test]
fn maximize_tracks_reservations_home_output_and_topology_without_changing_float_rect() {
    let mut d = desktop(Mode::Floating);
    let saved = Rect::new(-1100, 200, 900, 500);
    d.command(Command::SetFloatingRect(WindowId(3), saved));
    d.command(Command::ToggleFloating);
    d.command(Command::ToggleMaximized);
    d.set_output_area(OutputId(1), Rect::new(0, 72, 700, 428));
    assert_eq!(rect(&d, 3), Rect::new(0, 72, 700, 428));
    d.add_output(OutputId(2), "right".into(), Rect::new(700, 20, 500, 600));
    d.command(Command::StretchAll);
    d.command(Command::MoveToOutput(OutputId(2)));
    assert_eq!(rect(&d, 3), Rect::new(700, 20, 500, 600));
    d.remove_output(OutputId(2));
    assert_eq!(rect(&d, 3), Rect::new(0, 72, 700, 428));
    d.command(Command::ToggleMaximized);
    assert_eq!(rect(&d, 3), saved);
    assert!(d.window(WindowId(3)).unwrap().floating);
}

#[test]
fn focus_can_raise_a_normal_window_above_a_maximized_neighbor_but_not_launchers() {
    let mut d = desktop(Mode::Columns);
    d.command(Command::ToggleMaximized);
    d.command(Command::Focus(WindowId(1)));
    assert_eq!(d.placements().last().unwrap().window, WindowId(1));
    d.set_window_role(WindowId(2), WindowRole::Launcher);
    assert_eq!(d.placements().last().unwrap().window, WindowId(2));
}

#[test]
fn unsupported_roles_invalid_ids_and_late_role_changes_are_safe() {
    let mut d = desktop(Mode::Columns);
    d.command(Command::ToggleMaximized);
    d.command(Command::MinimizeFocused);
    d.set_window_role(WindowId(3), WindowRole::Launcher);
    let w = d.window(WindowId(3)).unwrap();
    assert!(!w.maximized && !w.minimized);
    for id in [WindowId(3), WindowId(999)] {
        d.command(Command::SetMaximized(id, true));
        d.command(Command::SetMinimized(id, true));
    }
    assert!(!d.window(WindowId(3)).unwrap().maximized);
    assert!(!d.window(WindowId(3)).unwrap().minimized);
    d.remove_window(WindowId(3));
    d.add_window(WindowId(3), "remapped".into(), "test".into());
    assert!(!d.window(WindowId(3)).unwrap().maximized);
}

#[test]
fn shortcuts_rhai_actions_and_alt_tab_restore_minimized_windows() {
    for config in [
        Config::from_source("[keys]\nleader='Ctrl+Alt'").unwrap(),
        Config::from_source(include_str!("../examples/dual-virtual-monitors.toml")).unwrap(),
        Config::from_source(include_str!("../examples/single-monitor.toml")).unwrap(),
    ] {
        let bindings = Bindings::new(&config.bindings).unwrap();
        let mods = Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(bindings.action(mods, "Up"), Some(Action::ToggleMaximized));
        assert_eq!(bindings.action(mods, "Down"), Some(Action::Minimize));
    }
    let mut host = ScriptHost::from_source(
        "fn states() { [#{ action: \"toggle_maximized\" }, #{ action: \"minimize\" }] }",
    )
    .unwrap();
    assert_eq!(
        host.action("states").unwrap(),
        vec![Action::ToggleMaximized, Action::Minimize]
    );
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    runtime.desktop = desktop(Mode::Columns);
    runtime.action(Action::ToggleMaximized);
    runtime.action(Action::Minimize);
    runtime.desktop.command(Command::Focus(WindowId(2)));
    runtime.advance_switcher();
    assert_eq!(runtime.switcher.as_ref().unwrap().selected, WindowId(3));
    assert!(runtime.desktop.window(WindowId(3)).unwrap().minimized);
    runtime.cancel_switcher();
    assert!(runtime.desktop.window(WindowId(3)).unwrap().minimized);
    runtime.advance_switcher();
    runtime.finish_switcher();
    assert!(!runtime.desktop.window(WindowId(3)).unwrap().minimized);
    assert!(runtime.desktop.window(WindowId(3)).unwrap().maximized);
}

#[test]
fn shell_window_state_requests_are_strict_atomic_and_snapshotted() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    runtime.desktop = desktop(Mode::Columns);
    for (kind, field) in [
        ("set_maximized", "maximized"),
        ("set_minimized", "minimized"),
    ] {
        let source = format!(
            r#"{{"version":1,"id":9,"request":{{"type":"{kind}","window":"3","{field}":true}}}}"#
        );
        let request = parse_request(source.as_bytes()).unwrap();
        execute(&mut runtime, &request).unwrap();
        let state = serde_json::to_value(snapshot(&runtime)).unwrap();
        assert_eq!(state["windows"][2][field], true);
        assert!(parse_request(source.replace("true", "1").as_bytes()).is_err());
        assert!(parse_request(source.replace("true", "true,\"extra\":0").as_bytes()).is_err());
        let before = format!("{:?}", runtime.desktop);
        let invalid = parse_request(source.replace("\"3\"", "\"999\"").as_bytes()).unwrap();
        assert!(execute(&mut runtime, &invalid).is_err());
        assert_eq!(format!("{:?}", runtime.desktop), before);
    }
    execute(
        &mut runtime,
        &Request {
            version: 1,
            id: 10,
            request: RequestKind::FocusWindow { window: "3".into() },
        },
    )
    .unwrap();
    assert!(!runtime.desktop.window(WindowId(3)).unwrap().minimized);
    runtime
        .desktop
        .set_window_role(WindowId(3), WindowRole::Launcher);
    let before = format!("{:?}", runtime.desktop);
    for request in [
        RequestKind::SetMaximized {
            window: "3".into(),
            maximized: true,
        },
        RequestKind::SetMinimized {
            window: "3".into(),
            minimized: true,
        },
    ] {
        assert!(
            execute(
                &mut runtime,
                &Request {
                    version: 1,
                    id: 11,
                    request
                }
            )
            .is_err()
        );
    }
    assert_eq!(format!("{:?}", runtime.desktop), before);
}
