use clear::{
    config::Config,
    core::{Command, Effect, Mode, OutputId, Rect, WindowId, WindowRole, WorkspaceId},
    input::{Action, Bindings, Modifiers},
    runtime::Runtime,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "clear-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn config(&self, text: &str) -> PathBuf {
        let path = self.0.join("config.toml");
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn outputs(runtime: &mut Runtime) {
    runtime
        .desktop
        .add_output(OutputId(1), "virtual-1".into(), Rect::new(0, 0, 800, 600));
    runtime
        .desktop
        .add_output(OutputId(2), "virtual-2".into(), Rect::new(800, 0, 800, 600));
    runtime.configure_output_modes();
}

#[test]
fn configured_leader_applies_to_default_shortcuts() {
    let config = Config::from_source("[keys]\nleader = 'Ctrl+Alt'\n").unwrap();
    let bindings = Bindings::new(&config.bindings).unwrap();
    assert_eq!(
        bindings.action(
            Modifiers {
                ctrl: true,
                alt: true,
                ..Default::default()
            },
            "m"
        ),
        Some(Action::CycleMode)
    );
    assert_eq!(
        bindings.action(
            Modifiers {
                logo: true,
                ..Default::default()
            },
            "m"
        ),
        None
    );
}

#[test]
fn actions_wire_workspace_spanning_and_independent_region_modes() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    outputs(&mut runtime);
    runtime
        .desktop
        .add_window(WindowId(1), "one".into(), "test".into());
    runtime.action(Action::StretchAll);
    runtime.action(Action::SetWorkspaceMode {
        mode: "floating".into(),
    });
    runtime.action(Action::MoveToOutput { output: 2 });
    runtime.action(Action::SetOutputMode {
        mode: "scrolling".into(),
    });
    assert_eq!(runtime.desktop.groups().len(), 1);
    assert_eq!(
        runtime.desktop.effective_mode(WorkspaceId(1), OutputId(1)),
        Some(&Mode::Floating)
    );
    assert_eq!(
        runtime.desktop.effective_mode(WorkspaceId(1), OutputId(2)),
        Some(&Mode::Scrolling)
    );
    assert_eq!(runtime.placements().len(), 1);
    runtime.action(Action::SwitchWorkspace { workspace: 3 });
    assert!(runtime.placements().is_empty());
    runtime.action(Action::SwitchWorkspace { workspace: 1 });
    assert_eq!(runtime.placements()[0].window, WindowId(1));
    runtime.action(Action::Unstretch);
    assert_eq!(runtime.desktop.groups().len(), 2);
}

#[test]
fn reload_keeps_last_good_config_on_parse_script_or_topology_failure() {
    let fixture = Fixture::new();
    let path = fixture.config("gaps = 17\n");
    let mut runtime = Runtime::load(Some(path.clone())).unwrap();
    outputs(&mut runtime);
    runtime
        .desktop
        .add_window(WindowId(1), "one".into(), "test".into());
    let before = runtime.placements();
    fixture.config("not valid toml!");
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.placements(), before);
    fixture.config("gaps = 99\nscript = 'missing.rhai'\n");
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.config.gaps, 17);
    fixture.config("gaps = 99\n[[outputs]]\nname = 'changed'\nwidth = 800\nheight = 600\n");
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.config.gaps, 17);
    fixture.config("gaps = 3\n");
    runtime.reload().unwrap();
    assert_eq!(runtime.config.gaps, 3);
    assert_eq!(runtime.desktop.windows().count(), 1);
}

#[test]
fn overview_hover_reload_is_atomic_and_preserves_an_open_preview() {
    let fixture = Fixture::new();
    let mut runtime = Runtime::load(Some(
        fixture.config("[overview]\npreview_workspace_on_hover=false"),
    ))
    .unwrap();
    outputs(&mut runtime);
    runtime.toggle_overview();
    let session = runtime.overview.clone();
    fixture.config("[overview]\npreview_workspace_on_hover=true");
    runtime.reload().unwrap();
    assert!(runtime.config.overview.preview_workspace_on_hover);
    assert_eq!(runtime.overview, session);
    fixture.config("[overview]\npreview_workspace_on_hover='yes'");
    assert!(runtime.reload().is_err());
    assert!(runtime.config.overview.preview_workspace_on_hover);
    assert_eq!(runtime.overview, session);
}

#[test]
fn rhai_layout_and_actions_integrate_with_core_and_fail_closed() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("test.rhai"), r#"
        fn arrange(ctx) {
            let result = [];
            for w in ctx.windows { result.push(#{window:w.id, x:ctx.area.x+19, y:ctx.area.y+23, width:200, height:150}); }
            result
        }
        fn stretch() { [#{action:"stretch_all"}, #{action:"set_output_mode", mode:"floating"}] }
        fn bad(ctx) { loop {} }
        fn recurse() { [#{action:"script", name:"recurse"}] }
    "#).unwrap();
    let path = fixture.config(
        "script = 'test.rhai'\n[[workspaces]]\nid = 1\nname = 'script'\nmode = 'script:arrange'\n",
    );
    let mut runtime = Runtime::load(Some(path)).unwrap();
    outputs(&mut runtime);
    runtime
        .desktop
        .add_window(WindowId(1), "one".into(), "test".into());
    assert_eq!(runtime.placements()[0].rect, Rect::new(19, 23, 200, 150));
    runtime.action(Action::Script {
        name: "stretch".into(),
    });
    assert_eq!(runtime.desktop.groups().len(), 1);
    assert!(!runtime.placements()[0].tiled);
    runtime.action(Action::SetOutputMode {
        mode: "script:bad".into(),
    });
    let fallback = runtime.placements();
    assert_eq!(fallback.len(), 1);
    assert!(fallback[0].tiled);
    assert_eq!(runtime.placements(), fallback);
    assert!(
        runtime
            .action(Action::Script {
                name: "recurse".into()
            })
            .is_empty()
    );
}

#[test]
fn late_launcher_metadata_reclassifies_without_stealing_focus_or_saved_state() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    outputs(&mut runtime);
    runtime
        .desktop
        .add_window(WindowId(1), "starting".into(), "".into());
    runtime
        .desktop
        .add_window(WindowId(2), "normal".into(), "foot".into());
    let saved = Rect::new(-250, 700, 350, 220);
    runtime
        .desktop
        .command(Command::SetFloatingRect(WindowId(1), saved));
    runtime
        .desktop
        .set_window_committed_size(WindowId(1), 420, 180);
    let mut expected = runtime.desktop.window(WindowId(1)).unwrap().clone();
    let before = runtime.placements();

    // Mirror the adapter's metadata callback, including app IDs arriving after map.
    for (app_id, role) in [
        ("wofi", WindowRole::Launcher),
        ("wofi", WindowRole::Launcher),
        ("Wofi", WindowRole::Normal),
        ("wofi", WindowRole::Launcher),
        ("", WindowRole::Normal),
    ] {
        runtime
            .desktop
            .update_window_metadata(WindowId(1), "ready".into(), app_id.into());
        runtime.classify_window(WindowId(1), app_id);
        expected.title = "ready".into();
        expected.app_id = app_id.into();
        expected.role = role;
        assert_eq!(runtime.desktop.window(WindowId(1)), Some(&expected));
        assert_eq!(runtime.desktop.focused_window(), Some(WindowId(2)));
        assert_eq!(runtime.desktop.focused_output(), Some(OutputId(1)));
        assert_eq!(
            runtime.desktop.workspace(WorkspaceId(1)).unwrap().windows(),
            &[WindowId(1), WindowId(2)]
        );
        let placements = runtime.placements();
        if role == WindowRole::Launcher {
            assert_eq!(placements[0].window, WindowId(2));
            assert_eq!(placements[0].rect, Rect::new(8, 8, 784, 584));
            assert!(placements[0].tiled);
            assert_eq!(placements[1].window, WindowId(1));
            assert_eq!(placements[1].rect, Rect::new(190, 210, 420, 180));
            assert_eq!(placements[1].clip, None);
            assert!(!placements[1].tiled);
            assert!(!placements[1].focused);
        } else {
            assert_eq!(placements, before);
        }
    }
    runtime.classify_window(WindowId(999), "wofi");
    assert_eq!(runtime.desktop.windows().count(), 2);
}

#[test]
fn shell_reload_reclassifies_visible_and_hidden_windows_and_rejects_invalid_rules_atomically() {
    let fixture = Fixture::new();
    let mut runtime = Runtime::load(Some(fixture.config(""))).unwrap();
    outputs(&mut runtime);
    for (id, app_id) in [(1, "wofi"), (2, "custom"), (3, "wofi")] {
        runtime
            .desktop
            .add_window(WindowId(id), format!("window-{id}"), app_id.into());
        runtime.classify_window(WindowId(id), app_id);
        runtime
            .desktop
            .set_window_committed_size(WindowId(id), 320, 180);
        runtime.desktop.command(Command::SetFloatingRect(
            WindowId(id),
            Rect::new(-200, 900, 300, 200),
        ));
    }
    runtime.desktop.command(Command::ToggleFloating);
    runtime
        .desktop
        .command(Command::MoveToWorkspace(WorkspaceId(3)));
    runtime.desktop.command(Command::Focus(WindowId(1)));
    let saved: Vec<_> = runtime.desktop.windows().cloned().collect();
    let groups = runtime.desktop.groups().to_vec();

    for source in [
        "[shell]\nlauncher_app_ids = ['custom']\n[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'overlay'",
        "[shell]\nlauncher_app_ids = []\npanels = []",
        "",
    ] {
        fixture.config(source);
        runtime.reload().unwrap();
        let expected_shell = Config::from_source(source).unwrap().shell;
        assert_eq!(runtime.config.shell, expected_shell);
        for window in &saved {
            let mut expected = window.clone();
            expected.role = if expected_shell.is_launcher(&window.app_id) {
                WindowRole::Launcher
            } else {
                WindowRole::Normal
            };
            assert_eq!(runtime.desktop.window(window.id), Some(&expected));
        }
        assert_eq!(runtime.desktop.groups(), groups);
        assert_eq!(runtime.desktop.focused_window(), Some(WindowId(1)));
        assert_eq!(
            runtime.desktop.workspace(WorkspaceId(1)).unwrap().windows(),
            &[WindowId(1), WindowId(2)]
        );
        assert_eq!(
            runtime.desktop.workspace(WorkspaceId(3)).unwrap().windows(),
            &[WindowId(3)]
        );
        let before = runtime.placements();
        assert!(!before.iter().any(|p| p.window == WindowId(3)));
        let windows: Vec<_> = runtime.desktop.windows().cloned().collect();
        for invalid in [
            "gaps = 99\n[shell]\nlauncher_app_ids = ['custom', 'custom']",
            "gaps = 99\n[shell]\nlauncher_app_ids = ['custom']\n[[shell.panels]]\nnamespace = ''\nlayer = 'top'",
        ] {
            fixture.config(invalid);
            assert!(runtime.reload().is_err());
            assert_eq!(runtime.config.shell, expected_shell);
            assert_eq!(runtime.config.gaps, 8);
            assert_eq!(
                runtime.desktop.windows().cloned().collect::<Vec<_>>(),
                windows
            );
            assert_eq!(runtime.placements(), before);
        }
    }
}

#[test]
fn invalid_startup_shell_rules_fall_back_to_all_defaults() {
    let fixture = Fixture::new();
    let runtime = Runtime::load(Some(
        fixture.config("gaps = 99\n[shell]\nlauncher_app_ids = ['']"),
    ))
    .unwrap();
    assert_eq!(runtime.config.gaps, Config::default().gaps);
    assert_eq!(runtime.config.shell, Config::default().shell);
}

#[test]
fn platform_effects_are_explicit_and_do_not_execute_in_policy() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    assert_eq!(
        runtime.action(Action::Spawn {
            command: vec!["not-executed".into()]
        }),
        vec![Effect::Spawn(vec!["not-executed".into()])]
    );
    assert_eq!(runtime.action(Action::Quit), vec![Effect::Quit]);
    assert!(runtime.action(Action::CloseFocused).is_empty());
}
