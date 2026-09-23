use clear::config::{Config, PanelLayer, PanelRule, ShellConfig};

#[test]
fn omitted_shell_and_empty_table_use_wofi_and_top_layer_waybar_defaults() {
    let expected = ShellConfig {
        launcher_app_ids: vec!["wofi".into()],
        panels: vec![PanelRule {
            namespace: "waybar".into(),
            layer: PanelLayer::Top,
        }],
    };
    assert_eq!(ShellConfig::default(), expected);
    assert_eq!(Config::default().shell, expected);
    for source in ["", "[shell]\n"] {
        assert_eq!(Config::from_source(source).unwrap().shell, expected);
    }
    assert!(expected.is_launcher("wofi"));
    for app_id in [
        "",
        "Wofi",
        "wofi-extra",
        "org.example.wofi",
        " wofi",
        "wofi ",
    ] {
        assert!(!expected.is_launcher(app_id), "matched {app_id:?}");
    }
}

#[test]
fn shell_arrays_replace_defaults_independently_and_can_disable_rules() {
    let launchers = Config::from_source("[shell]\nlauncher_app_ids = ['custom', 'Wofi']")
        .unwrap()
        .shell;
    assert_eq!(launchers.launcher_app_ids, ["custom", "Wofi"]);
    assert_eq!(launchers.panels, ShellConfig::default().panels);
    assert!(launchers.is_launcher("custom"));
    assert!(launchers.is_launcher("Wofi"));
    assert!(!launchers.is_launcher("wofi"));

    for (name, layer) in [
        ("background", PanelLayer::Background),
        ("bottom", PanelLayer::Bottom),
        ("top", PanelLayer::Top),
        ("overlay", PanelLayer::Overlay),
    ] {
        let source = format!("[[shell.panels]]\nnamespace = 'custom-panel'\nlayer = '{name}'");
        let shell = Config::from_source(&source).unwrap().shell;
        assert_eq!(shell.launcher_app_ids, ["wofi"]);
        assert_eq!(
            shell.panels,
            [PanelRule {
                namespace: "custom-panel".into(),
                layer,
            }]
        );
    }

    let no_launchers = Config::from_source("[shell]\nlauncher_app_ids = []").unwrap();
    assert!(!no_launchers.shell.is_launcher("wofi"));
    assert_eq!(no_launchers.shell.panels, ShellConfig::default().panels);
    let no_panels = Config::from_source("[shell]\npanels = []").unwrap();
    assert!(no_panels.shell.panels.is_empty());
    assert!(no_panels.shell.is_launcher("wofi"));
    let disabled = Config::from_source("[shell]\nlauncher_app_ids = []\npanels = []").unwrap();
    assert!(disabled.shell.launcher_app_ids.is_empty());
    assert!(disabled.shell.panels.is_empty());
}

#[test]
fn shell_rejects_empty_duplicate_malformed_and_unsupported_rules() {
    for source in [
        "[shell]\nlauncher_app_ids = ['']",
        "[shell]\nlauncher_app_ids = ['   ']",
        "[shell]\nlauncher_app_ids = ['wofi', 'wofi']",
        "[shell]\nlauncher_app_ids = 'wofi'",
        "[shell]\nlauncher_app_ids = [1]",
        "[shell]\nautostart = ['waybar']",
        "[shell]\ntoggle = 'wofi'",
        "[[shell.panels]]\nnamespace = ''\nlayer = 'top'",
        "[[shell.panels]]\nnamespace = '   '\nlayer = 'top'",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'top'\n[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'bottom'",
        "[[shell.panels]]\nnamespace = 'waybar'",
        "[[shell.panels]]\nlayer = 'top'",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'Top'",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'foreground'",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 2",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'top'\nanchors = ['top']",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'top'\nexclusive_zone = 32",
        "[[shell.panels]]\nnamespace = 'waybar'\nlayer = 'top'\ncommand = ['waybar']",
    ] {
        assert!(
            Config::from_source(source).is_err(),
            "accepted invalid shell config: {source}"
        );
    }
}

#[test]
fn shipped_configs_document_the_default_shell_rules() {
    for source in [
        include_str!("../examples/config.toml"),
        include_str!("../examples/vm.toml"),
    ] {
        assert_eq!(
            Config::from_source(source).unwrap().shell,
            ShellConfig::default()
        );
    }
}
