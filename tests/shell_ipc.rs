use clear::{
    config::Config,
    core::{Command, Mode, OutputId, Rect, WindowId, WorkspaceId},
    runtime::Runtime,
    shell::{
        PROTOCOL_VERSION, Request, RequestKind, encode_error, encode_ok, encode_snapshot,
        encode_update, execute, parse_request, snapshot,
    },
};
use serde_json::{Value, json};

const HIDDEN_WINDOW: u64 = u64::MAX;

fn runtime() -> Runtime {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    let desktop = &mut runtime.desktop;
    desktop.add_output(OutputId(11), "left".into(), Rect::new(-800, 30, 800, 570));
    desktop.add_output(OutputId(22), "right".into(), Rect::new(0, 0, 1000, 700));
    desktop.configure_workspace(
        WorkspaceId(1),
        "main".into(),
        Mode::Script("Arrange".into()),
    );
    desktop.configure_workspace(WorkspaceId(2), "other".into(), Mode::Columns);
    desktop.set_workspace_output_mode(WorkspaceId(1), OutputId(22), Some(Mode::Scrolling));
    desktop.add_window(WindowId(20), "normal".into(), "terminal".into());
    desktop.command(Command::ToggleFloating);
    desktop.command(Command::SetFloatingRect(
        WindowId(20),
        Rect::new(-950, -10, 1200, 800),
    ));
    desktop.set_window_committed_size(WindowId(20), 640, 480);
    desktop.add_window(WindowId(10), "launcher".into(), "wofi".into());
    desktop.command(Command::FocusOutput(OutputId(22)));
    desktop.add_window(WindowId(30), "other".into(), "editor".into());
    desktop.command(Command::SwitchWorkspace(WorkspaceId(3)));
    desktop.add_window(
        WindowId(HIDDEN_WINDOW),
        "hidden".into(),
        "hidden.app".into(),
    );
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    desktop.command(Command::FocusOutput(OutputId(11)));
    runtime.classify_window(WindowId(10), "wofi");
    runtime
}

fn request(kind: RequestKind) -> Request {
    Request {
        version: PROTOCOL_VERSION,
        id: 17,
        request: kind,
    }
}

fn run(runtime: &mut Runtime, kind: RequestKind) {
    execute(runtime, &request(kind)).unwrap();
}

fn decode(bytes: Vec<u8>) -> Value {
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn alt_tab_snapshot_tracks_selection_until_commit_or_cancel() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    runtime
        .desktop
        .add_output(OutputId(1), "screen".into(), Rect::new(0, 0, 800, 600));
    runtime
        .desktop
        .add_window(WindowId(1), "One".into(), "app".into());
    runtime
        .desktop
        .add_window(WindowId(2), "Two".into(), "app".into());
    assert_eq!(runtime.desktop.focused_window(), Some(WindowId(2)));

    runtime.advance_switcher();
    let state = snapshot(&runtime);
    let switcher = state.switcher.unwrap();
    assert_eq!(switcher.output, "1");
    assert_eq!(switcher.windows, ["1", "2"]);
    assert_eq!(switcher.selected, "1");
    assert_eq!(runtime.desktop.focused_window(), Some(WindowId(2)));
    runtime.advance_switcher();
    assert_eq!(snapshot(&runtime).switcher.unwrap().selected, "2");
    runtime.cancel_switcher();
    assert!(snapshot(&runtime).switcher.is_none());
    assert_eq!(runtime.desktop.focused_window(), Some(WindowId(2)));

    runtime.advance_switcher();
    runtime.finish_switcher();
    assert!(snapshot(&runtime).switcher.is_none());
    assert_eq!(runtime.desktop.focused_window(), Some(WindowId(1)));
}

#[test]
fn parses_every_allowlisted_request_and_u32_request_ids() {
    let cases = [
        (json!({"type":"snapshot"}), RequestKind::Snapshot),
        (json!({"type":"subscribe"}), RequestKind::Subscribe),
        (
            json!({"type":"focus_output","output":"11"}),
            RequestKind::FocusOutput {
                output: "11".into(),
            },
        ),
        (
            json!({"type":"switch_workspace","output":"22","workspace":"3"}),
            RequestKind::SwitchWorkspace {
                output: "22".into(),
                workspace: "3".into(),
            },
        ),
        (
            json!({"type":"focus_window","window":HIDDEN_WINDOW.to_string()}),
            RequestKind::FocusWindow {
                window: HIDDEN_WINDOW.to_string(),
            },
        ),
        (
            json!({"type":"set_mode","output":"11","mode":"script:Arrange"}),
            RequestKind::SetMode {
                output: "11".into(),
                mode: "script:Arrange".into(),
            },
        ),
        (
            json!({"type":"clear_mode","output":"11"}),
            RequestKind::ClearMode {
                output: "11".into(),
            },
        ),
        (
            json!({"type":"stretch","output":"11"}),
            RequestKind::Stretch {
                output: "11".into(),
            },
        ),
        (
            json!({"type":"split","output":"22"}),
            RequestKind::Split {
                output: "22".into(),
            },
        ),
    ];
    for (body, expected) in cases {
        for id in [0, u32::MAX] {
            let mut bytes =
                serde_json::to_vec(&json!({"version":1,"id":id,"request":body})).unwrap();
            bytes.push(b'\n');
            let parsed = parse_request(&bytes).unwrap();
            assert_eq!(
                parsed,
                Request {
                    version: 1,
                    id,
                    request: expected.clone()
                }
            );
        }
    }
}

#[test]
fn strict_shape_rejects_unknown_missing_duplicate_and_wrong_typed_fields() {
    for source in [
        r#"{}"#,
        r#"[]"#,
        r#"null"#,
        r#"{"version":1,"id":1,"request":{"type":"snapshot"},"extra":true}"#,
        r#"{"version":1,"id":1,"request":{"type":"snapshot","extra":true}}"#,
        r#"{"version":1,"id":1,"request":{"type":"subscribe","extra":true}}"#,
        r#"{"version":1,"id":1,"request":{"type":"focus_output","output":"11","extra":true}}"#,
        r#"{"version":1,"id":1,"request":{"type":"focus_output"}}"#,
        r#"{"version":1,"id":1,"request":{"type":"switch_workspace","output":"11"}}"#,
        r#"{"version":1,"id":1,"request":{"type":"set_mode","output":"11"}}"#,
        r#"{"version":1,"id":1,"request":{"type":"focus_window","window":1}}"#,
        r#"{"version":1,"id":1,"request":{"type":"focus_output","output":11}}"#,
        r#"{"version":1,"id":1,"request":{"type":"switch_workspace","output":"11","workspace":3}}"#,
        r#"{"version":1,"id":1,"request":{"type":"set_mode","output":"11","mode":null}}"#,
        r#"{"version":1,"id":1,"request":{"type":"Snapshot"}}"#,
        r#"{"version":1,"id":1,"request":{"Snapshot":{}}}"#,
        r#"{"version":1,"id":1,"request":null}"#,
        r#"{"version":1,"id":1,"id":2,"request":{"type":"snapshot"}}"#,
        r#"{"version":1,"version":1,"id":1,"request":{"type":"snapshot"}}"#,
        r#"{"version":1,"id":1,"request":{"type":"snapshot","type":"subscribe"}}"#,
        r#"{"version":1,"id":1,"request":{"type":"focus_output","output":"11","output":"22"}}"#,
        r#"{"version":1,"id":-1,"request":{"type":"snapshot"}}"#,
        r#"{"version":1,"id":4294967296,"request":{"type":"snapshot"}}"#,
        r#"{"version":1,"id":"1","request":{"type":"snapshot"}}"#,
        r#"{"version":1,"id":1.0,"request":{"type":"snapshot"}}"#,
        r#"{"version":0,"id":1,"request":{"type":"snapshot"}}"#,
        r#"{"version":2,"id":1,"request":{"type":"snapshot"}}"#,
        r#"{"version":"1","id":1,"request":{"type":"snapshot"}}"#,
        r#"{"version":1,"id":1,"request":{"type":"snapshot"}} {}"#,
        r#"{"version":1,"id":1,"request":{"type":"snapshot"}} garbage"#,
        r#"{"version":1,"id":1,"request":{"type":"snapshot"}"#,
    ] {
        assert!(
            parse_request(source.as_bytes()).is_err(),
            "accepted {source}"
        );
    }
    assert!(parse_request(&[0xff]).is_err());
    for kind in ["spawn", "script", "quit", "reload", "close", "action"] {
        let bytes =
            serde_json::to_vec(&json!({"version":1,"id":1,"request":{"type":kind}})).unwrap();
        assert!(parse_request(&bytes).is_err(), "accepted {kind}");
    }
    // Deserialize itself, not just the parser, enforces strict nested fields.
    assert!(
        serde_json::from_str::<Request>(
            r#"{"version":1,"id":1,"request":{"type":"snapshot","extra":true}}"#
        )
        .is_err()
    );
}

#[test]
fn snapshot_reports_usable_geometry_hidden_windows_roles_and_stable_order() {
    let runtime = runtime();
    let before = format!("{:?}", runtime.desktop);
    let state = snapshot(&runtime);
    assert_eq!(snapshot(&runtime), state.clone());
    assert_eq!(format!("{:?}", runtime.desktop), before);
    assert_eq!(
        state
            .outputs
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        ["11", "22"]
    );
    let output = &state.outputs[0];
    assert_eq!(output.name, "left");
    assert_eq!(
        (
            output.area.x,
            output.area.y,
            output.area.width,
            output.area.height
        ),
        (-800, 30, 800, 570)
    );
    assert_eq!(output.workspace, "1");
    assert_eq!(output.effective_mode, "script:Arrange");
    assert_eq!(output.mode_override, None);
    assert_eq!(state.outputs[1].workspace, "2");
    assert_eq!(state.outputs[1].effective_mode, "columns");
    assert_eq!(state.outputs[1].mode_override, None);
    assert_eq!(state.workspaces[0].name, "main");
    assert_eq!(state.workspaces[0].mode, "script:Arrange");
    assert_eq!(state.workspaces[0].windows, ["20", "10"]);
    assert_eq!(state.workspaces[2].windows, [HIDDEN_WINDOW.to_string()]);
    assert_eq!(
        state
            .windows
            .iter()
            .map(|w| w.id.clone())
            .collect::<Vec<_>>(),
        [
            "10".into(),
            "20".into(),
            "30".into(),
            HIDDEN_WINDOW.to_string()
        ]
    );
    assert_eq!(state.windows[0].role, "launcher");
    assert!(!state.windows[0].floating);
    assert_eq!(state.windows[1].role, "normal");
    assert!(state.windows[1].floating);
    assert_eq!(state.windows[3].workspace, "3");
    assert_eq!(state.windows[3].output.as_deref(), Some("22"));
    assert!(!state.windows[3].focused);
    assert!(!state.groups.iter().any(|g| g.workspace == "3"));
    assert_eq!(state.focused_output.as_deref(), Some("11"));
    assert_eq!(state.focused_window.as_deref(), Some("10"));
    assert_eq!(state.windows.iter().filter(|w| w.focused).count(), 1);
}

#[test]
fn snapshots_without_outputs_and_with_large_ids_are_lossless() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    let state = snapshot(&runtime);
    assert!(state.outputs.is_empty());
    assert!(state.groups.is_empty());
    assert!(state.windows.is_empty());
    assert_eq!(state.focused_output, None);
    assert_eq!(state.focused_window, None);
    runtime
        .desktop
        .add_window(WindowId(u64::MAX), "offline".into(), "app".into());
    let state = snapshot(&runtime);
    assert_eq!(state.windows[0].output, None);
    assert!(!state.windows[0].focused);
    let workspace = WorkspaceId(u64::MAX - 1);
    runtime
        .desktop
        .configure_workspace(workspace, "large".into(), Mode::Monocle);
    runtime.desktop.add_output(
        OutputId(u64::MAX),
        "large".into(),
        Rect::new(0, 0, 100, 100),
    );
    run(
        &mut runtime,
        RequestKind::SwitchWorkspace {
            output: u64::MAX.to_string(),
            workspace: workspace.0.to_string(),
        },
    );
    let wire = decode(encode_snapshot(u32::MAX, &snapshot(&runtime)));
    assert_eq!(wire["id"], u32::MAX);
    assert_eq!(wire["state"]["outputs"][0]["id"], u64::MAX.to_string());
    assert_eq!(
        wire["state"]["outputs"][0]["workspace"],
        workspace.0.to_string()
    );
    assert_eq!(wire["state"]["windows"][0]["id"], u64::MAX.to_string());
}

#[test]
fn modes_roundtrip_canonically_and_overrides_are_workspace_specific() {
    let mut runtime = runtime();
    let focus = (
        runtime.desktop.focused_output(),
        runtime.desktop.focused_window(),
    );
    let windows: Vec<_> = runtime.desktop.windows().cloned().collect();
    for (input, canonical) in [
        ("float", "floating"),
        ("scroll", "scrolling"),
        ("tiling", "master_stack"),
        ("columns", "columns"),
        ("rows", "rows"),
        ("grid", "grid"),
        ("fibonacci", "spiral"),
        ("dwindle", "spiral"),
        ("monocle", "monocle"),
        (" SCRIPT:Arrange ", "script:Arrange"),
    ] {
        run(
            &mut runtime,
            RequestKind::SetMode {
                output: "22".into(),
                mode: input.into(),
            },
        );
        let state = snapshot(&runtime);
        assert_eq!(state.outputs[1].effective_mode, canonical);
        assert_eq!(state.outputs[1].mode_override.as_deref(), Some(canonical));
        assert_eq!(state.workspaces[1].mode, "columns");
        run(
            &mut runtime,
            RequestKind::SetMode {
                output: "22".into(),
                mode: state.outputs[1].effective_mode.clone(),
            },
        );
        assert_eq!(snapshot(&runtime), state);
        assert_eq!(
            (
                runtime.desktop.focused_output(),
                runtime.desktop.focused_window()
            ),
            focus
        );
        assert_eq!(
            runtime.desktop.windows().cloned().collect::<Vec<_>>(),
            windows
        );
    }
    run(
        &mut runtime,
        RequestKind::SwitchWorkspace {
            output: "22".into(),
            workspace: "3".into(),
        },
    );
    assert_eq!(snapshot(&runtime).outputs[1].mode_override, None);
    run(
        &mut runtime,
        RequestKind::ClearMode {
            output: "22".into(),
        },
    );
    run(
        &mut runtime,
        RequestKind::SwitchWorkspace {
            output: "22".into(),
            workspace: "2".into(),
        },
    );
    assert_eq!(
        snapshot(&runtime).outputs[1].mode_override.as_deref(),
        Some("script:Arrange")
    );
    run(
        &mut runtime,
        RequestKind::ClearMode {
            output: "22".into(),
        },
    );
    assert_eq!(snapshot(&runtime).outputs[1].mode_override, None);
    assert_eq!(snapshot(&runtime).outputs[1].effective_mode, "columns");
    // A hidden workspace's override is not cleared by the active workspace operation.
    assert_eq!(
        runtime
            .desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .output_mode(OutputId(22)),
        Some(&Mode::Scrolling)
    );
}

#[test]
fn stretch_and_split_report_groups_and_preserve_window_policy() {
    let mut runtime = runtime();
    let windows: Vec<_> = runtime.desktop.windows().cloned().collect();
    run(
        &mut runtime,
        RequestKind::Stretch {
            output: "11".into(),
        },
    );
    let state = snapshot(&runtime);
    assert_eq!(state.groups.len(), 1);
    assert_eq!(state.groups[0].outputs, ["11", "22"]);
    assert_eq!(state.groups[0].workspace, "1");
    assert_eq!(state.outputs[0].workspace, "1");
    assert_eq!(state.outputs[1].workspace, "1");
    assert_eq!(state.outputs[0].effective_mode, "script:Arrange");
    assert_eq!(state.outputs[1].effective_mode, "scrolling");
    assert_eq!(state.outputs[1].mode_override.as_deref(), Some("scrolling"));
    assert_eq!(state.windows.len(), windows.len());
    run(
        &mut runtime,
        RequestKind::Split {
            output: "22".into(),
        },
    );
    let state = snapshot(&runtime);
    assert_eq!(state.groups.len(), 2);
    assert!(state.groups.iter().all(|g| g.outputs.len() == 1));
    assert_eq!(state.outputs[1].workspace, "1");
    assert_eq!(state.outputs[1].effective_mode, "scrolling");
    assert_ne!(state.outputs[0].workspace, "1");
    for original in windows {
        let current = runtime.desktop.window(original.id).unwrap();
        assert_eq!(current.workspace, original.workspace);
        assert_eq!(current.floating_rect, original.floating_rect);
        assert_eq!(current.floating, original.floating);
        assert_eq!(current.role, original.role);
        assert_eq!(current.committed_size, original.committed_size);
    }
}

#[test]
fn targeted_commands_match_core_behavior_including_hidden_window_reveal() {
    let mut runtime = runtime();
    let sequence = [
        (
            RequestKind::FocusOutput {
                output: "22".into(),
            },
            vec![Command::FocusOutput(OutputId(22))],
        ),
        (
            RequestKind::SwitchWorkspace {
                output: "11".into(),
                workspace: "2".into(),
            },
            vec![
                Command::FocusOutput(OutputId(11)),
                Command::SwitchWorkspace(WorkspaceId(2)),
            ],
        ),
        (
            RequestKind::FocusWindow {
                window: HIDDEN_WINDOW.to_string(),
            },
            vec![Command::Focus(WindowId(HIDDEN_WINDOW))],
        ),
        (
            RequestKind::Stretch {
                output: "22".into(),
            },
            vec![Command::FocusOutput(OutputId(22)), Command::StretchAll],
        ),
        (
            RequestKind::Split {
                output: "11".into(),
            },
            vec![Command::FocusOutput(OutputId(11)), Command::Unstretch],
        ),
    ];
    for (kind, commands) in sequence {
        let reveal = matches!(kind, RequestKind::FocusWindow { .. });
        let mut expected = runtime.desktop.clone();
        for command in commands {
            assert!(expected.command(command).is_empty());
        }
        run(&mut runtime, kind);
        assert_eq!(format!("{:?}", runtime.desktop), format!("{expected:?}"));
        if reveal {
            let state = snapshot(&runtime);
            assert_eq!(state.focused_window, Some(HIDDEN_WINDOW.to_string()));
            assert!(state.groups.iter().any(|g| g.workspace == "3"));
        }
    }
}

#[test]
fn errors_are_atomic_even_when_the_first_target_is_valid_and_unfocused() {
    let mut runtime = runtime();
    let before = format!("{:?}", runtime.desktop);
    let state = snapshot(&runtime);
    let invalid = [
        RequestKind::FocusOutput {
            output: "missing".into(),
        },
        RequestKind::FocusOutput {
            output: "left".into(),
        },
        RequestKind::FocusOutput {
            output: "011".into(),
        },
        RequestKind::SwitchWorkspace {
            output: "22".into(),
            workspace: "missing".into(),
        },
        RequestKind::SwitchWorkspace {
            output: "missing".into(),
            workspace: "1".into(),
        },
        RequestKind::FocusWindow { window: "0".into() },
        RequestKind::FocusWindow {
            window: "18446744073709551616".into(),
        },
        RequestKind::FocusWindow { window: "".into() },
        RequestKind::SetMode {
            output: "22".into(),
            mode: "".into(),
        },
        RequestKind::SetMode {
            output: "22".into(),
            mode: "   ".into(),
        },
        RequestKind::SetMode {
            output: "22".into(),
            mode: "script: ".into(),
        },
        RequestKind::SetMode {
            output: "22".into(),
            mode: "unknown".into(),
        },
        RequestKind::SetMode {
            output: "22".into(),
            mode: format!("script:{}", "a".repeat(250)),
        },
        RequestKind::SetMode {
            output: "22".into(),
            mode: format!("script:{}", "é".repeat(125)),
        },
        RequestKind::SetMode {
            output: "missing".into(),
            mode: "columns".into(),
        },
        RequestKind::ClearMode {
            output: "missing".into(),
        },
        RequestKind::Stretch {
            output: "missing".into(),
        },
        RequestKind::Split {
            output: "missing".into(),
        },
    ];
    for kind in invalid {
        assert!(
            execute(&mut runtime, &request(kind.clone())).is_err(),
            "accepted {kind:?}"
        );
        assert_eq!(
            format!("{:?}", runtime.desktop),
            before,
            "mutated on {kind:?}"
        );
        assert_eq!(snapshot(&runtime), state);
    }
    for kind in [
        RequestKind::Snapshot,
        RequestKind::Subscribe,
        RequestKind::FocusOutput {
            output: "22".into(),
        },
    ] {
        let mut request = request(kind);
        request.version = 2;
        assert!(execute(&mut runtime, &request).is_err());
        assert_eq!(format!("{:?}", runtime.desktop), before);
    }
}

#[test]
fn mode_byte_limit_is_inclusive_and_direct_execution_validates_it() {
    let mut runtime = runtime();
    let mode = format!("script:{}", "a".repeat(249));
    assert_eq!(mode.len(), 256);
    run(
        &mut runtime,
        RequestKind::SetMode {
            output: "11".into(),
            mode: mode.clone(),
        },
    );
    assert_eq!(snapshot(&runtime).outputs[0].effective_mode, mode);
}

#[test]
fn snapshot_and_subscribe_execution_are_exact_noops() {
    let mut runtime = runtime();
    let before = format!("{:?}", runtime.desktop);
    for kind in [RequestKind::Snapshot, RequestKind::Subscribe] {
        run(&mut runtime, kind);
        assert_eq!(format!("{:?}", runtime.desktop), before);
    }
}

#[test]
fn response_envelopes_escape_metadata_and_preserve_exact_shapes() {
    let mut runtime = runtime();
    let title = "title \"quoted\" \\ slash\nline\r\t\0 雪";
    let app_id = "app\n{\"type\":\"ok\"}\n💻";
    runtime
        .desktop
        .update_window_metadata(WindowId(20), title.into(), app_id.into());
    let state = snapshot(&runtime);
    let value = serde_json::to_value(&state).unwrap();
    let response = decode(encode_snapshot(42, &state));
    assert_eq!(
        response,
        json!({"version":1,"type":"snapshot","id":42,"state":value})
    );
    assert_eq!(response["state"]["windows"][1]["title"], title);
    assert_eq!(response["state"]["windows"][1]["app_id"], app_id);
    assert_eq!(
        decode(encode_update(&state)),
        json!({"version":1,"type":"state","state":value})
    );
    assert_eq!(
        decode(encode_ok(u32::MAX)),
        json!({"version":1,"type":"ok","id":u32::MAX})
    );
    for id in [None, Some(0), Some(u32::MAX)] {
        assert_eq!(
            decode(encode_error(id, title)),
            json!({"version":1,"type":"error","id":id,"message":title})
        );
    }
}
