use clear::{
    config::Config,
    input::{Action, Binding, Bindings, Modifiers},
    scripting::{ScriptContext, ScriptHost, ScriptRect, ScriptWindow},
};

fn context() -> ScriptContext {
    let area = ScriptRect {
        x: -100,
        y: 50,
        width: 800,
        height: 600,
    };
    ScriptContext {
        area,
        windows: vec![
            ScriptWindow { id: 1, rect: area },
            ScriptWindow { id: 2, rect: area },
        ],
        focused: Some(1),
        gaps: 8,
        scroll_offset: 0,
    }
}

fn binding(key: &str) -> Binding {
    Binding {
        key: key.into(),
        action: Action::Quit,
    }
}

#[test]
fn defaults_are_complete_and_collision_free() {
    let config = Config::from_source("").unwrap();
    assert_eq!(config.outputs.len(), 2);
    assert!(
        config
            .outputs
            .iter()
            .all(|o| o.width == 800 && o.height == 600)
    );
    assert_eq!(
        config.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(),
        (1..=9).collect::<Vec<_>>()
    );
    assert_eq!(config.bindings.len(), 32);
    assert!(config.script.is_none());
    let bindings = Bindings::new(&config.bindings).unwrap();
    assert_eq!(
        bindings.action(
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            "Tab"
        ),
        Some(Action::AltTab)
    );
    let logo = Modifiers {
        logo: true,
        ..Modifiers::default()
    };
    let shifted = Modifiers {
        shift: true,
        ..logo
    };
    assert_eq!(
        bindings.action(logo, "ENTER"),
        Some(Action::Spawn {
            command: vec!["foot".into()]
        })
    );
    for workspace in 1..=9 {
        assert_eq!(
            bindings.action(logo, &workspace.to_string()),
            Some(Action::SwitchWorkspace { workspace })
        );
        assert_eq!(
            bindings.action(shifted, &workspace.to_string()),
            Some(Action::MoveToWorkspace { workspace })
        );
    }
    for (key, action) in [
        ("q", Action::CloseFocused),
        ("Escape", Action::Quit),
        ("m", Action::CycleMode),
        ("o", Action::CycleOutput),
        ("s", Action::StretchAll),
        ("f", Action::ToggleFloating),
        ("Up", Action::ToggleMaximized),
        ("Down", Action::Minimize),
        ("j", Action::FocusNext),
        ("k", Action::FocusPrevious),
    ] {
        assert_eq!(bindings.action(logo, key), Some(action));
    }
    assert_eq!(bindings.action(shifted, "s"), Some(Action::Unstretch));
    assert_eq!(bindings.action(shifted, "r"), Some(Action::Reload));
}

#[test]
fn key_aliases_normalize_and_modifiers_are_exact() {
    let bindings = Bindings::new(&[binding(" Control + ALT + leader + Shift + Esc ")]).unwrap();
    let modifiers = Modifiers {
        ctrl: true,
        alt: true,
        logo: true,
        shift: true,
    };
    assert_eq!(bindings.action(modifiers, "Escape"), Some(Action::Quit));
    assert_eq!(
        bindings.action(
            Modifiers {
                shift: false,
                ..modifiers
            },
            "Escape"
        ),
        None
    );
    assert_eq!(bindings.action(modifiers, "not-a-key"), None);
    for (a, b) in [
        ("Super+Return", "logo+ENTER"),
        ("Ctrl+Prior", "CONTROL+PageUp"),
        ("Shift+A", "shift+a"),
    ] {
        assert!(
            Bindings::new(&[binding(a), binding(b)]).is_err(),
            "{a}, {b}"
        );
    }
}

#[test]
fn invalid_shortcuts_and_actions_are_rejected() {
    for key in [
        "",
        "Super",
        "Super+",
        "+a",
        "Hyper+a",
        "Super+Logo+a",
        "a+q",
        "Super+bogus",
        "Super+F36",
    ] {
        assert!(Bindings::new(&[binding(key)]).is_err(), "{key:?}");
    }
    for action in [
        Action::Spawn { command: vec![] },
        Action::Spawn {
            command: vec!["".into()],
        },
        Action::Spawn {
            command: vec!["foot\0".into()],
        },
        Action::SwitchWorkspace { workspace: 0 },
        Action::SetOutputMode { mode: "".into() },
        Action::Script {
            name: "not a function".into(),
        },
    ] {
        assert!(
            Bindings::new(&[Binding {
                key: "Super+a".into(),
                action
            }])
            .is_err()
        );
    }
}

#[test]
fn config_parses_flat_actions_and_leader() {
    let config = Config::from_source(
        r#"
        gaps = 12
        script = "columns.rhai"
        [keys]
        leader = "Ctrl+Alt"
        [[bindings]]
        key = "leader+Return"
        action = "spawn"
        command = ["foot", "--title", "a b"]
        [[bindings]]
        key = "leader+Shift+2"
        action = "move_to_workspace"
        workspace = 2
    "#,
    )
    .unwrap();
    assert_eq!(config.bindings.len(), 2);
    let bindings = Bindings::new(&config.bindings).unwrap();
    assert_eq!(
        bindings.action(
            Modifiers {
                ctrl: true,
                alt: true,
                ..Modifiers::default()
            },
            "return"
        ),
        Some(Action::Spawn {
            command: vec!["foot".into(), "--title".into(), "a b".into()]
        })
    );
    assert_eq!(config.gaps, 12);
    assert_eq!(config.script.unwrap().to_str(), Some("columns.rhai"));
    assert!(Config::from_source("script = ''").unwrap().script.is_none());
    assert!(
        Config::from_source("bindings = []")
            .unwrap()
            .bindings
            .is_empty()
    );
}

#[test]
fn config_rejects_malformed_or_ambiguous_values() {
    for source in [
        "not toml",
        "unknown = 1",
        "gaps = -1",
        "gaps = 1.5",
        "outputs = []",
        "workspaces = []",
        "[keys]\nleader = 'leader'",
        "[keys]\nleader = 'Hyper'",
        "[theme]\nborder_width = -1",
        "[theme]\nbackground = [nan, 0.0, 0.0, 1.0]",
        "[theme]\nactive_border = [2.0, 0.0, 0.0, 1.0]",
        "[[outputs]]\nname = 'a'\nwidth = 0\nheight = 600",
        "[[outputs]]\nname = 'a'\nwidth = 1\nheight = 1\n[[outputs]]\nname = 'a'\nwidth = 1\nheight = 1",
        "[[workspaces]]\nid = 0\nname = 'bad'",
        "[[workspaces]]\nid = 1\nname = 'a'\n[[workspaces]]\nid = 1\nname = 'b'",
        "[[workspaces]]\nid = 1\nname = 'a'\nmode = 'script:bad name'",
        "[[workspaces]]\nid = 1\nname = 'a'\n[workspaces.output_modes]\nmissing = 'columns'",
        "[[bindings]]\nkey = 'Super+a'\naction = 'unknown'",
        "[[bindings]]\nkey = 'Super+a'\naction = 'quit'\ntypo = true",
        "[[bindings]]\nkey = 'Super+a'\naction = 'spawn'\ncommand = 'foot'",
        "[[bindings]]\nkey = 'Super+a'\naction = 'switch_workspace'\nworkspace = -1",
        "[[bindings]]\nkey = 'Super+a'\naction = 'quit'\n[[bindings]]\nkey = 'Logo+A'\naction = 'reload'",
    ] {
        assert!(
            Config::from_source(source).is_err(),
            "accepted invalid config: {source}"
        );
    }
}

#[test]
fn shipped_examples_work() {
    Config::from_source(include_str!("../examples/config.toml")).unwrap();
    let mut host = ScriptHost::from_source(include_str!("../examples/columns.rhai")).unwrap();
    let ctx = context();
    let result = host.layout("columns", ctx.clone()).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].window, 1);
    assert_eq!(
        result[0].rect,
        ScriptRect {
            x: -92,
            y: 58,
            width: 388,
            height: 584
        }
    );
    assert_eq!(
        result[1].rect,
        ScriptRect {
            x: 304,
            y: 58,
            width: 388,
            height: 584
        }
    );
    assert_eq!(
        host.action("focus_and_stretch").unwrap(),
        vec![Action::FocusNext, Action::StretchAll]
    );
    let mut empty = ctx;
    empty.windows.clear();
    assert!(host.layout("missing", empty).unwrap().is_empty());
}

#[test]
fn monitor_examples_select_their_intended_topologies() {
    let single = Config::from_source(include_str!("../examples/single-monitor.toml")).unwrap();
    assert_eq!(single.outputs.len(), 1);
    assert_eq!(single.outputs[0].name, "virtual-1");
    assert_eq!(
        (single.outputs[0].width, single.outputs[0].height),
        (1280, 720)
    );
    assert_eq!(single.workspaces.len(), 3);
    assert!(single.workspaces.iter().all(|w| w.output_modes.is_empty()));
    Bindings::new(&single.bindings).unwrap();

    let dual = Config::from_source(include_str!("../examples/dual-virtual-monitors.toml")).unwrap();
    assert_eq!(dual.outputs.len(), 2);
    assert_eq!(dual.outputs[0].name, "virtual-1");
    assert_eq!(dual.outputs[1].name, "virtual-2");
    Bindings::new(&dual.bindings).unwrap();
}

#[test]
fn layouts_can_initialize_unconfigured_window_geometry() {
    let mut host = ScriptHost::from_source(include_str!("../examples/columns.rhai")).unwrap();
    let mut ctx = context();
    ctx.windows[0].rect.width = 0;
    ctx.windows[0].rect.height = 0;
    assert_eq!(host.layout("columns", ctx).unwrap().len(), 2);
}

#[test]
fn malformed_layout_results_are_rejected() {
    let valid = "#{window: 1, x: -100, y: 50, width: 400, height: 600}";
    let second = "#{window: 2, x: 300, y: 50, width: 400, height: 600}";
    let bad_results = vec![
        "42".into(),
        "#{}".into(),
        "[]".into(),
        format!("[{valid}]"),
        format!("[{valid}, {valid}]"),
        format!("[{valid}, {}]", second.replace("window: 2", "window: 3")),
        format!("[{valid}, {}]", second.replace("window: 2", "window: 2.0")),
        format!("[{valid}, {}]", second.replace("x: 300", "x: 300.5")),
        format!("[{valid}, {}]", second.replace("x: 300", "x: 2147483648")),
        format!("[{valid}, {}]", second.replace("width: 400", "width: 0")),
        format!("[{valid}, {}]", second.replace("height: 600", "height: -1")),
        format!("[{valid}, {}]", second.replace("x: 300", "x: 301")),
        format!("[{valid}, {}]", second.replace("y: 50", "y: 49")),
        format!("[{valid}, {}]", second.replace("window: 2", "window: -2")),
        format!("[{valid}, {}]", second.replace("width: 400, ", "")),
        format!(
            "[{valid}, {}]",
            second.replace("height: 600", "height: 600, extra: true")
        ),
        format!("[{valid}, ()]"),
    ];
    for result in bad_results {
        let mut host = ScriptHost::from_source(&format!("fn layout(ctx) {{ {result} }}")).unwrap();
        assert!(
            host.layout("layout", context()).is_err(),
            "accepted {result}"
        );
    }
}

#[test]
fn invalid_contexts_are_rejected() {
    let mut host = ScriptHost::from_source(include_str!("../examples/columns.rhai")).unwrap();
    let mut bad = context();
    bad.windows[1].id = 1;
    assert!(host.layout("columns", bad).is_err());
    let mut bad = context();
    bad.windows[1].id = u64::MAX;
    assert!(host.layout("columns", bad).is_err());
    let mut bad = context();
    bad.focused = Some(99);
    assert!(host.layout("columns", bad).is_err());
    let mut bad = context();
    bad.area.width = i32::MAX;
    assert!(host.layout("columns", bad).is_err());
}

#[test]
fn operation_and_recursion_limits_stop_scripts_and_host_recovers() {
    let mut host = ScriptHost::from_source(
        r#"
        fn spin() { loop {} }
        fn recurse() { recurse() }
        fn good() { [#{action: "quit"}] }
    "#,
    )
    .unwrap();
    let error = host.action("spin").unwrap_err();
    assert!(error.to_ascii_lowercase().contains("operations"), "{error}");
    assert!(host.action("recurse").is_err());
    assert_eq!(host.action("good").unwrap(), vec![Action::Quit]);
}

#[test]
fn imports_eval_and_io_are_unavailable_and_globals_do_not_run() {
    for source in [
        "import \"/etc/passwd\" as secret;",
        "fn denied() { import \"./other.rhai\" as other; [] }",
    ] {
        assert!(ScriptHost::from_source(source).is_err());
    }
    let eval = ScriptHost::from_source("fn denied() { eval(\"loop {}\"); [] }");
    if let Ok(mut host) = eval {
        assert!(host.action("denied").is_err());
    }
    let mut host = ScriptHost::from_source(
        r#"
        loop {}
        fn good() { [] }
        fn denied() { read_file("/etc/passwd"); [] }
    "#,
    )
    .unwrap();
    assert!(host.action("good").unwrap().is_empty());
    assert!(host.action("denied").is_err());
    assert!(host.action("unknown").is_err());
}

#[test]
fn resource_limits_reject_oversized_values() {
    for body in [
        "let s = \"a\"; for n in 0..20 { s += s; } []",
        "let a = []; for n in 0..5000 { a.push(n); } []",
        "let m = #{}; for n in 0..300 { m[n.to_string()] = n; } []",
        "let m = [#{}]; for n in 0..300 { m[0][n.to_string()] = n; } []",
    ] {
        let mut host = ScriptHost::from_source(&format!("fn oversized() {{ {body} }}")).unwrap();
        assert!(host.action("oversized").is_err());
    }
    assert!(ScriptHost::from_source(&" ".repeat(256 * 1024 + 1)).is_err());
}

#[test]
fn config_load_uses_xdg_and_falls_back_without_panicking() {
    const CHILD: &str = "CLEAR_EXTENSION_CONFIG_CHILD_GAPS";
    if let Ok(expected) = std::env::var(CHILD) {
        let config = Config::load();
        let expected: i32 = expected.parse().unwrap();
        assert_eq!(config.gaps, expected);
        if expected == 23 {
            let base = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
            assert_eq!(config.script, Some(base.join("clear/columns.rhai")));
        }
        return;
    }
    let temporary = TemporaryDirectory::new();
    let home = temporary.0.join("home");
    let xdg = temporary.0.join("xdg");
    std::fs::create_dir_all(home.join(".config/clear")).unwrap();
    std::fs::create_dir_all(xdg.join("clear")).unwrap();
    std::fs::write(home.join(".config/clear/config.toml"), "gaps = 19").unwrap();
    let xdg_config = xdg.join("clear/config.toml");
    std::fs::write(&xdg_config, "gaps = 23\nscript = 'columns.rhai'").unwrap();
    let run = |xdg_path: Option<&std::path::Path>, expected: i32| {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "config_load_uses_xdg_and_falls_back_without_panicking",
                "--nocapture",
            ])
            .env(CHILD, expected.to_string())
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME");
        if let Some(path) = xdg_path {
            child.env("XDG_CONFIG_HOME", path);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(Some(&xdg), 23);
    run(None, 19);
    run(Some(std::path::Path::new("relative-xdg-is-ignored")), 19);
    std::fs::write(&xdg_config, "gaps = -1").unwrap();
    let invalid = run(Some(&xdg), 8);
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid"));
    std::fs::remove_file(&xdg_config).unwrap();
    run(Some(&xdg), 8);
}

#[test]
fn script_files_are_loaded_once_and_read_errors_are_returned() {
    let temporary = TemporaryDirectory::new();
    let path = temporary.0.join("extension.rhai");
    assert!(ScriptHost::load(&path).is_err());
    std::fs::write(&path, "fn good() { [#{action: \"reload\"}] }").unwrap();
    let mut host = ScriptHost::load(&path).unwrap();
    std::fs::write(&path, "this is not valid Rhai").unwrap();
    assert!(ScriptHost::load(&path).is_err());
    assert_eq!(host.action("good").unwrap(), vec![Action::Reload]);
}

struct TemporaryDirectory(std::path::PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "clear-extension-tests-{}-{timestamp}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn all_script_actions_decode_to_typed_requests() {
    let mut host = ScriptHost::from_source(r#"
        fn actions() { [
            #{action: "focus_next"}, #{action: "focus_previous"}, #{action: "cycle_output"},
            #{action: "switch_workspace", workspace: 2}, #{action: "move_to_workspace", workspace: 3},
            #{action: "move_to_output", output: 0}, #{action: "set_workspace_mode", mode: "columns"},
            #{action: "set_output_mode", mode: "script:columns"}, #{action: "clear_output_mode"},
            #{action: "cycle_mode"}, #{action: "stretch_all"}, #{action: "unstretch"},
            #{action: "toggle_floating"}, #{action: "scroll", amount: -20}, #{action: "close_focused"},
            #{action: "spawn", command: ["foot", "--title", "a b"]}, #{action: "quit"},
            #{action: "reload"}, #{action: "script", name: "other"}
        ] }
    "#).unwrap();
    assert_eq!(
        host.action("actions").unwrap(),
        vec![
            Action::FocusNext,
            Action::FocusPrevious,
            Action::CycleOutput,
            Action::SwitchWorkspace { workspace: 2 },
            Action::MoveToWorkspace { workspace: 3 },
            Action::MoveToOutput { output: 0 },
            Action::SetWorkspaceMode {
                mode: "columns".into()
            },
            Action::SetOutputMode {
                mode: "script:columns".into()
            },
            Action::ClearOutputMode,
            Action::CycleMode,
            Action::StretchAll,
            Action::Unstretch,
            Action::ToggleFloating,
            Action::Scroll { amount: -20 },
            Action::CloseFocused,
            Action::Spawn {
                command: vec!["foot".into(), "--title".into(), "a b".into()]
            },
            Action::Quit,
            Action::Reload,
            Action::Script {
                name: "other".into()
            },
        ]
    );
}

#[test]
fn malformed_script_actions_fail_atomically() {
    for action in [
        "42",
        "#{action: \"unknown\"}",
        "#{action: \"quit\", extra: true}",
        "#{action: \"scroll\", amount: 1.5}",
        "#{action: \"scroll\", amount: 2147483648}",
        "#{action: \"switch_workspace\", workspace: 0}",
        "#{action: \"move_to_output\", output: -1}",
        "#{action: \"set_output_mode\", mode: \"\"}",
        "#{action: \"spawn\", command: []}",
        "#{action: \"spawn\", command: [42]}",
        "#{action: \"script\", name: \"bad name\"}",
    ] {
        let source = format!("fn bad() {{ [#{{action: \"focus_next\"}}, {action}] }}");
        let mut host = ScriptHost::from_source(&source).unwrap();
        assert!(host.action("bad").is_err(), "accepted {action}");
    }
}
