use clear::{
    config::Config,
    core::{Command, OutputId, Rect, WindowId, WindowRole, WorkspaceId},
    input::{Action, Binding, Bindings, Modifiers},
    runtime::Runtime,
    scripting::ScriptHost,
    shell::{Request, RequestKind, execute, parse_request, snapshot},
};

fn runtime() -> Runtime {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    runtime.desktop.add_output_with_bounds(
        OutputId(1),
        "left".into(),
        Rect::new(0, 0, 800, 600),
        Rect::new(0, 32, 800, 568),
    );
    runtime.desktop.add_output_with_bounds(
        OutputId(2),
        "right".into(),
        Rect::new(800, 0, 800, 600),
        Rect::new(800, 32, 800, 568),
    );
    runtime.desktop.command(Command::FocusOutput(OutputId(1)));
    runtime
        .desktop
        .add_window(WindowId(1), "normal".into(), "app".into());
    runtime
}

#[test]
fn configurable_action_and_declarative_script_toggle_the_same_fullscreen_policy() {
    let binding: Binding =
        toml::from_str("key='leader+Shift+f'\naction='toggle_fullscreen'").unwrap();
    let bindings = Bindings::new(&[binding]).unwrap();
    let action = bindings
        .action(
            Modifiers {
                logo: true,
                shift: true,
                ..Default::default()
            },
            "f",
        )
        .unwrap();
    assert_eq!(action, Action::ToggleFullscreen);
    let mut runtime = runtime();
    assert!(runtime.action(action).is_empty());
    assert!(runtime.desktop.window(WindowId(1)).unwrap().fullscreen);
    assert_eq!(runtime.placements()[0].rect, Rect::new(0, 0, 800, 600));
    let mut host =
        ScriptHost::from_source("fn fullscreen() { [#{ action: \"toggle_fullscreen\" }] }")
            .unwrap();
    let actions = host.action("fullscreen").unwrap();
    assert_eq!(actions, vec![Action::ToggleFullscreen]);
    runtime.action(actions[0].clone());
    assert!(!runtime.desktop.window(WindowId(1)).unwrap().fullscreen);
    assert!(
        ScriptHost::from_source(
            "fn fullscreen() { [#{ action: \"toggle_fullscreen\", window: 1 }] }"
        )
        .unwrap()
        .action("fullscreen")
        .is_err()
    );
}

#[test]
fn fullscreen_ipc_is_strict_atomic_and_does_not_reveal_or_restore_hidden_windows() {
    let mut runtime = runtime();
    runtime
        .desktop
        .command(Command::SetMinimized(WindowId(1), true));
    runtime
        .desktop
        .command(Command::SwitchWorkspace(WorkspaceId(3)));
    let original_focus = runtime.desktop.focused_output();
    let request = parse_request(br#"{"version":1,"id":1,"request":{"type":"set_fullscreen","window":"1","fullscreen":true}}"#).unwrap();
    assert!(execute(&mut runtime, &request).is_err());
    let state = snapshot(&runtime);
    let window = &state.windows[0];
    assert!(!window.fullscreen && window.minimized);
    assert_eq!(state.outputs[0].workspace, "3");
    assert_eq!(runtime.desktop.focused_output(), original_focus);
    assert!(runtime.placements().is_empty());
    runtime
        .desktop
        .command(Command::SetMinimized(WindowId(1), false));
    execute(&mut runtime, &request).unwrap();
    assert!(runtime.desktop.window(WindowId(1)).unwrap().fullscreen);
    assert!(!runtime.desktop.fullscreen_slot().unwrap().active);
    for payload in [
        br#"{"version":1,"id":1,"request":{"type":"set_fullscreen","window":"1","fullscreen":true,"extra":1}}"#.as_slice(),
        br#"{"version":1,"id":1,"request":{"type":"set_fullscreen","window":"1","fullscreen":"true"}}"#.as_slice(),
        br#"{"version":1,"id":1,"request":{"type":"set_fullscreen","window":"1"}}"#.as_slice(),
    ] { assert!(parse_request(payload).is_err()); }
    runtime
        .desktop
        .add_window(WindowId(2), "launcher".into(), "wofi".into());
    runtime
        .desktop
        .set_window_role(WindowId(2), WindowRole::Launcher);
    let before = serde_json::to_value(snapshot(&runtime)).unwrap();
    for window in ["2", "999", "01"] {
        assert!(
            execute(
                &mut runtime,
                &Request {
                    version: 1,
                    id: 2,
                    request: RequestKind::SetFullscreen {
                        window: window.into(),
                        fullscreen: true,
                        mode: None
                    },
                }
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(snapshot(&runtime)).unwrap(), before);
    }
}

#[test]
fn low_latency_selection_and_in_place_switch_preserve_return_state() {
    use clear::core::FullscreenMode;
    let mut r = runtime();
    r.config = Config::from_source("[fullscreen]\ndefault_mode='low-latency'\n[[fullscreen.rules]]\napp_id='app'\nmode='desktop'").unwrap();
    r.action(Action::ToggleFullscreen);
    let before = r.desktop.fullscreen_slot().unwrap().clone();
    assert_eq!(before.mode, FullscreenMode::Desktop);
    r.action(Action::ToggleFullscreenLowLatency);
    let after = r.desktop.fullscreen_slot().unwrap();
    assert_eq!(after.mode, FullscreenMode::LowLatency);
    assert_eq!(after.return_workspace, before.return_workspace);
    assert_eq!(after.return_output, before.return_output);
    assert_eq!(after.return_focus, before.return_focus);
    assert_eq!(after.window, before.window);
    assert_eq!(after.output, before.output);
    assert_eq!(after.revision, before.revision);
    // Configuration changes affect future entries, not the occupied slot or repeated enters.
    r.config.fullscreen.default_mode = FullscreenMode::Desktop;
    r.request_fullscreen(WindowId(1), None, true, None);
    assert_eq!(
        r.desktop.fullscreen_slot().unwrap().mode,
        FullscreenMode::LowLatency
    );
    r.action(Action::ToggleFullscreenLowLatency);
    assert!(r.desktop.fullscreen_slot().is_none());
    r.config.fullscreen.rules.clear();
    r.config.fullscreen.default_mode = FullscreenMode::LowLatency;
    r.action(Action::ToggleFullscreen);
    assert_eq!(
        r.desktop.fullscreen_slot().unwrap().mode,
        FullscreenMode::LowLatency
    );
    r.action(Action::ToggleFullscreen);
    assert!(r.desktop.fullscreen_slot().is_none());
    let mut host =
        ScriptHost::from_source("fn game() { [#{ action: \"toggle_fullscreen_low_latency\" }] }")
            .unwrap();
    assert_eq!(
        host.action("game").unwrap(),
        vec![Action::ToggleFullscreenLowLatency]
    );
}

#[test]
fn low_latency_config_is_strict_bounded_and_exact() {
    use clear::core::FullscreenMode;
    assert_eq!(
        Config::default().fullscreen.default_mode,
        FullscreenMode::Desktop
    );
    let config =
        Config::from_source("[[fullscreen.rules]]\napp_id='Game'\nmode='low-latency'").unwrap();
    assert_eq!(
        config.fullscreen.mode_for("Game"),
        FullscreenMode::LowLatency
    );
    assert_eq!(config.fullscreen.mode_for("game"), FullscreenMode::Desktop);
    for source in [
        "[fullscreen]\ndefault_mode='game'".to_owned(),
        "[fullscreen]\ndirect_scanout=true".into(),
        "[[fullscreen.rules]]\napp_id=''\nmode='desktop'".into(),
        "[[fullscreen.rules]]\napp_id='app'\nmode='desktop'\n[[fullscreen.rules]]\napp_id='app'\nmode='low-latency'".into(),
        format!("[[fullscreen.rules]]\napp_id='{}'\nmode='desktop'", "a".repeat(256)),
        (0..257).map(|i| format!("[[fullscreen.rules]]\napp_id='app{i}'\nmode='desktop'\n")).collect(),
    ] { assert!(Config::from_source(&source).is_err(), "{source}"); }
}

#[test]
fn low_latency_ipc_modes_are_atomic_and_background_entry_stays_dormant() {
    let mut r = runtime();
    r.desktop.command(Command::SwitchWorkspace(WorkspaceId(3)));
    let enter = parse_request(br#"{"version":1,"id":1,"request":{"type":"set_fullscreen","window":"1","fullscreen":true,"mode":"low-latency"}}"#).unwrap();
    execute(&mut r, &enter).unwrap();
    let state = snapshot(&r);
    let slot = state.fullscreen_desktop.unwrap();
    assert_eq!(slot.mode, "low-latency");
    assert!(!slot.active);
    assert_eq!(
        r.desktop.window(WindowId(1)).unwrap().workspace,
        WorkspaceId(1)
    );
    let before = snapshot(&r);
    let invalid_exit = parse_request(br#"{"version":1,"id":2,"request":{"type":"set_fullscreen","window":"1","fullscreen":false,"mode":"desktop"}}"#).unwrap();
    assert!(execute(&mut r, &invalid_exit).is_err());
    assert_eq!(snapshot(&r), before);
    assert!(parse_request(br#"{"version":1,"id":2,"request":{"type":"set_fullscreen","window":"1","fullscreen":true,"mode":"game"}}"#).is_err());
    r.desktop.activate_fullscreen();
    r.desktop.command(Command::SetMinimized(WindowId(1), true));
    assert!(r.desktop.fullscreen_slot().is_none());
    assert!(execute(&mut r, &enter).is_err());
}
