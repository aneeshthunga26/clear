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
    execute(&mut runtime, &request).unwrap();
    let state = snapshot(&runtime);
    let window = &state.windows[0];
    assert!(window.fullscreen && window.minimized);
    assert_eq!(state.outputs[0].workspace, "3");
    assert_eq!(runtime.desktop.focused_output(), original_focus);
    assert!(runtime.placements().is_empty());
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
                        fullscreen: true
                    },
                }
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(snapshot(&runtime)).unwrap(), before);
    }
}
