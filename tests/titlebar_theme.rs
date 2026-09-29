use clear::{
    config::Config,
    core::{Command, OutputId, Rect, WindowId, WorkspaceId},
    decoration::{ControlsSide, TitlebarControls, TitlebarTheme},
    runtime::{
        Runtime,
        titlebar::{MAX_APP_ICONS, MAX_RASTER_BYTES, MAX_SVG_BYTES, TitlebarAssets, TitlebarImage},
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "clear-titlebar-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn config(&self, source: &str) -> PathBuf {
        write(&self.0.join("config.toml"), source);
        self.0.join("config.toml")
    }
    fn theme(&self, source: &str) -> TitlebarTheme {
        let mut config = Config::from_source(source).unwrap();
        config.resolve_paths(&self.0);
        config.theme.titlebar
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(path: &Path, source: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}
fn svg(path: &Path, color: &str) {
    write(
        path,
        &format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="{color}" fill-opacity="0.5"/></svg>"#
        ),
    );
}
fn pixel(image: &TitlebarImage, x: u32, y: u32) -> &[u8] {
    let offset = ((y * image.width + x) * 4) as usize;
    &image.pixels[offset..offset + 4]
}

#[test]
fn defaults_and_partial_overrides_preserve_previous_appearance() {
    let theme = Config::from_source("").unwrap().theme.titlebar;
    assert_eq!(theme, TitlebarTheme::default());
    for (color, bytes) in [
        (theme.active_background, [35, 40, 52, 255]),
        (theme.inactive_background, [27, 30, 38, 255]),
        (theme.active_foreground, [239, 243, 250, 255]),
        (theme.inactive_foreground, [174, 183, 199, 255]),
    ] {
        assert_eq!(color.map(|channel| (channel * 255.0).round() as u8), bytes);
    }
    assert_eq!(theme.height, 32);
    assert_eq!(theme.icon_size(), 20);
    assert_eq!(theme.controls_side, ControlsSide::Right);
    assert!(!theme.show_icon);
    assert!(theme.show_title);
    assert_eq!(theme.controls, TitlebarControls::default());
    let config = Config::from_source("[theme.titlebar]\nheight=16\ncontrols_side='left'\nshow_icon=true\nshow_title=false\nactive_background=[0.1,0.2,0.3,0.4]").unwrap();
    assert_eq!(config.theme.titlebar.icon_size(), 4);
    assert_eq!(config.theme.titlebar.controls_side, ControlsSide::Left);
    assert!(config.theme.titlebar.show_icon);
    assert!(!config.theme.titlebar.show_title);
    assert_eq!(
        config.theme.titlebar.active_background,
        [0.1, 0.2, 0.3, 0.4]
    );
    assert_eq!(
        config.theme.titlebar.inactive_background,
        theme.inactive_background
    );
    for (height, size) in [
        (i32::MIN, 1),
        (12, 1),
        (16, 4),
        (24, 12),
        (32, 20),
        (128, 20),
        (i32::MAX, 20),
    ] {
        assert_eq!(
            TitlebarTheme {
                height,
                ..Default::default()
            }
            .icon_size(),
            size
        );
    }
    assert!(Config::from_source("[theme.titlebar]\nheight=128").is_ok());
}

#[test]
fn rejects_unknown_fields_bad_types_ranges_and_paths() {
    for field in [
        "active_background",
        "inactive_background",
        "active_foreground",
        "inactive_foreground",
    ] {
        for value in [
            "[0,0,0]",
            "[0,0,0,1,1]",
            "[0,0,-0.1,1]",
            "[0,0,0,1.01]",
            "[nan,0,0,1]",
            "[0,inf,0,1]",
            "[0,0,-inf,1]",
            "'red'",
        ] {
            let source = format!("[theme.titlebar]\n{field}={value}");
            assert!(Config::from_source(&source).is_err(), "{source}");
        }
    }
    for fields in [
        "height=15",
        "height=129",
        "height=32.0",
        "height=-1",
        "controls_side='Left'",
        "controls_side='middle'",
        "show_icon='true'",
        "show_title=1",
        "unknown=true",
        "controls={unknown='a.svg'}",
    ] {
        let source = format!("[theme.titlebar]\n{fields}");
        assert!(Config::from_source(&source).is_err(), "{source}");
    }
    for control in ["minimize", "maximize", "restore", "close"] {
        for value in ["''", "'  '", "42", "\"a\\u0000.svg\""] {
            let source = format!("[theme.titlebar.controls]\n{control}={value}");
            assert!(Config::from_source(&source).is_err(), "{source}");
        }
    }
}

#[test]
fn paths_resolve_at_file_load_without_shell_expansion() {
    let mut config = Config::from_source("[theme.titlebar.controls]\nminimize='min.svg'\nmaximize='/absolute/max.svg'\nrestore='~/restore.svg'\nclose='$ICONS/close.svg'").unwrap();
    assert_eq!(
        config.theme.titlebar.controls.minimize.as_deref(),
        Some(Path::new("min.svg"))
    );
    config.resolve_paths(Path::new("/config"));
    let controls = &config.theme.titlebar.controls;
    assert_eq!(
        controls.minimize.as_deref(),
        Some(Path::new("/config/min.svg"))
    );
    assert_eq!(
        controls.maximize.as_deref(),
        Some(Path::new("/absolute/max.svg"))
    );
    assert_eq!(
        controls.restore.as_deref(),
        Some(Path::new("/config/~/restore.svg"))
    );
    assert_eq!(
        controls.close.as_deref(),
        Some(Path::new("/config/$ICONS/close.svg"))
    );
    let fixture = Fixture::new();
    svg(&fixture.0.join("close.svg"), "red");
    let runtime = Runtime::load(Some(
        fixture.config("[theme.titlebar.controls]\nclose='close.svg'"),
    ))
    .unwrap();
    assert_eq!(
        runtime.config.theme.titlebar.controls.close,
        Some(fixture.0.join("close.svg"))
    );
    assert!(runtime.titlebar_assets.control("close").is_some());
}

#[test]
fn custom_svg_colors_are_premultiplied_fitted_shared_and_cached() {
    let fixture = Fixture::new();
    let path = fixture.0.join("control.svg");
    svg(&path, "rgb(200,100,50)");
    let theme = fixture.theme("[theme.titlebar.controls]\nminimize='control.svg'\nmaximize='control.svg'\nrestore='control.svg'\nclose='control.svg'");
    let assets = TitlebarAssets::prepare(&theme).unwrap();
    let image = assets.control("close").unwrap();
    assert_eq!(
        (image.width, image.height, image.pixels.len()),
        (20, 20, 1600)
    );
    assert_eq!(pixel(image, 10, 10), [100, 50, 25, 128]);
    assert_eq!(pixel(image, 10, 1), [0, 0, 0, 0]);
    assert_eq!(pixel(image, 10, 18), [0, 0, 0, 0]);
    for name in ["minimize", "maximize", "restore"] {
        assert!(Arc::ptr_eq(image, assets.control(name).unwrap()));
    }
    assert!(assets.control("unknown").is_none());
    fs::remove_file(path).unwrap();
    assert_eq!(
        pixel(assets.control("close").unwrap(), 10, 10),
        [100, 50, 25, 128]
    );
}

#[test]
fn invalid_startup_falls_back_and_reload_is_atomic_and_rereads_same_paths() {
    let fixture = Fixture::new();
    let source =
        "gaps=17\n[theme.titlebar]\nheight=24\n[theme.titlebar.controls]\nclose='control.svg'";
    let mut runtime = Runtime::load(Some(fixture.config(source))).unwrap();
    assert_eq!(runtime.config.gaps, 17);
    assert_eq!(runtime.config.theme.titlebar.height, 24);
    assert!(runtime.titlebar_assets.control("close").is_none());
    svg(&fixture.0.join("control.svg"), "red");
    runtime.reload().unwrap();
    let before = runtime.titlebar_assets.control("close").unwrap().clone();
    assert_eq!(before.width, 12);
    svg(&fixture.0.join("control.svg"), "blue");
    runtime.reload().unwrap();
    let good = runtime.titlebar_assets.control("close").unwrap().clone();
    assert!(!Arc::ptr_eq(&before, &good));
    assert_eq!(pixel(&before, 6, 6), [128, 0, 0, 128]);
    assert_eq!(pixel(&good, 6, 6), [0, 0, 128, 128]);
    let theme = runtime.config.theme.clone();
    for bad in [
        "missing.svg",
        "malformed.svg",
        "too-large.svg",
        "directory.svg",
    ] {
        match bad {
            "malformed.svg" => write(&fixture.0.join(bad), "not SVG"),
            "too-large.svg" => fs::File::create(fixture.0.join(bad))
                .unwrap()
                .set_len(MAX_SVG_BYTES + 1)
                .unwrap(),
            "directory.svg" => fs::create_dir(fixture.0.join(bad)).unwrap(),
            _ => {}
        }
        fixture.config(&format!("gaps=99\n[theme.titlebar]\nheight=48\n[theme.titlebar.controls]\nminimize='control.svg'\nclose='{bad}'"));
        assert!(runtime.reload().is_err(), "{bad}");
        assert_eq!(runtime.config.gaps, 17);
        assert_eq!(runtime.config.theme, theme);
        assert!(runtime.titlebar_assets.control("minimize").is_none());
        assert!(Arc::ptr_eq(
            &good,
            runtime.titlebar_assets.control("close").unwrap()
        ));
    }
    fixture.config("gaps=99\n[theme.titlebar]\nheight=15");
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.config.theme, theme);
    assert!(Arc::ptr_eq(
        &good,
        runtime.titlebar_assets.control("close").unwrap()
    ));
    fixture.config("");
    runtime.reload().unwrap();
    assert!(runtime.titlebar_assets.control("close").is_none());
}

#[test]
fn svg_resource_limits_reject_expansion_and_disable_external_images_and_fonts() {
    let fixture = Fixture::new();
    let path = fixture.0.join("control.svg");
    let theme = fixture.theme("[theme.titlebar.controls]\nclose='control.svg'");
    for body in [
        "<svg xmlns='http://www.w3.org/2000/svg' width='1025' height='1'/>",
        "<!DOCTYPE svg [<!ENTITY x 'expanded'>]><svg xmlns='http://www.w3.org/2000/svg'>&x;</svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><use href='#x'/></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><pattern id='x'/></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'><marker id='x'/></svg>",
    ] {
        write(&path, body);
        assert!(TitlebarAssets::prepare(&theme).is_err(), "{body}");
    }
    write(
        &path,
        &format!(
            "<svg xmlns='http://www.w3.org/2000/svg'>{}</svg>",
            "<path/>".repeat(4096)
        ),
    );
    assert!(TitlebarAssets::prepare(&theme).is_err());
    write(
        &path,
        &format!(
            "<svg xmlns='http://www.w3.org/2000/svg'>{}{}</svg>",
            "<g>".repeat(40),
            "</g>".repeat(40)
        ),
    );
    assert!(TitlebarAssets::prepare(&theme).is_err());
    svg(&fixture.0.join("external.svg"), "red");
    for href in [
        fixture
            .0
            .join("external.svg")
            .to_string_lossy()
            .into_owned(),
        "https://example.invalid/icon.svg".into(),
        "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E".into(),
    ] {
        write(
            &path,
            &format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><image href="{href}" width="20" height="20"/><text x="0" y="15">Text</text></svg>"#
            ),
        );
        let assets = TitlebarAssets::prepare(&theme).unwrap();
        assert!(
            assets
                .control("close")
                .unwrap()
                .pixels
                .iter()
                .all(|byte| *byte == 0)
        );
    }
}

#[test]
fn svg_preflight_rejects_expansion_definitions_by_local_name() {
    let fixture = Fixture::new();
    let path = fixture.0.join("control.svg");
    let theme = fixture.theme("[theme.titlebar.controls]\nclose='control.svg'");
    for name in ["use", "pattern", "marker", "mask", "clipPath", "filter"] {
        for prefix in ["", "svg:", "other:"] {
            for reference in ["", "href='#earlier'", "xlink:href='#earlier'"] {
                write(
                    &path,
                    &format!(
                        "<svg xmlns='http://www.w3.org/2000/svg' xmlns:svg='http://www.w3.org/2000/svg' xmlns:other='urn:other' xmlns:xlink='http://www.w3.org/1999/xlink' width='20' height='20'><defs><{prefix}{name} id='unused' {reference}/></defs></svg>"
                    ),
                );
                let error = TitlebarAssets::prepare(&theme).unwrap_err();
                assert!(
                    error.contains(&format!("SVG {name} is unsupported for titlebar icons")),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn svg_preflight_rejects_shallow_chained_masks_and_clips_before_conversion() {
    let fixture = Fixture::new();
    let path = fixture.0.join("control.svg");
    let theme = fixture.theme("[theme.titlebar.controls]\nclose='control.svg'");
    for (name, units, property) in [
        ("mask", "maskUnits", "mask"),
        ("clipPath", "clipPathUnits", "clip-path"),
    ] {
        for syntax in ["attribute", "inline-style", "stylesheet"] {
            let mut definitions = format!(
                "<{name} id='n0' {units}='objectBoundingBox'><rect width='1' height='1' fill='white'/></{name}>"
            );
            let mut stylesheet = String::new();
            // Keep the fixture safe even if the preflight regresses: sibling depth
            // stays constant while each definition references its predecessor twice.
            for index in 1..=6 {
                let previous = index - 1;
                let reference = match syntax {
                    "attribute" => format!("{property}='url(#n{previous})'"),
                    "inline-style" => format!("style='{property}: url(#n{previous})'"),
                    _ => {
                        stylesheet
                            .push_str(&format!(".r{index} {{ {property}: url(#n{previous}); }}"));
                        format!("class='r{index}'")
                    }
                };
                let child = format!("<rect width='1' height='1' fill='white' {reference}/>");
                definitions.push_str(&format!(
                    "<{name} id='n{index}' {units}='objectBoundingBox'>{child}{child}</{name}>"
                ));
            }
            write(
                &path,
                &format!(
                    "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'><style>{stylesheet}</style><defs>{definitions}</defs><rect width='20' height='20' {property}='url(#n6)'/></svg>"
                ),
            );
            let error = TitlebarAssets::prepare(&theme).unwrap_err();
            assert!(
                error.contains(&format!("SVG {name} is unsupported for titlebar icons")),
                "{syntax}: {error}"
            );
        }
    }
}

#[test]
fn svg_paths_and_gradients_remain_supported() {
    let fixture = Fixture::new();
    let path = fixture.0.join("control.svg");
    let theme = fixture.theme("[theme.titlebar.controls]\nclose='control.svg'");
    write(
        &path,
        "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'><defs><linearGradient id='color'><stop offset='0' stop-color='red'/><stop offset='1' stop-color='blue'/></linearGradient></defs><path d='M0 0H20V20H0Z' fill='url(#color)'/></svg>",
    );
    let assets = TitlebarAssets::prepare(&theme).unwrap();
    let image = assets.control("close").unwrap();
    let left = pixel(image, 1, 10);
    let right = pixel(image, 18, 10);
    assert!(left[0] > left[2]);
    assert!(right[2] > right[0]);
    assert_eq!(left[3], 255);
    assert_eq!(right[3], 255);
}

#[test]
fn app_icon_lookup_classification_reload_and_cache_limits_in_isolated_environment() {
    const CHILD: &str = "CLEAR_TITLEBAR_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let fixture = Fixture::new();
        let output = Process::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app_icon_lookup_classification_reload_and_cache_limits_in_isolated_environment",
                "--nocapture",
            ])
            .env(CHILD, &fixture.0)
            .env("HOME", fixture.0.join("home"))
            .env("XDG_DATA_HOME", fixture.0.join("data"))
            .env("XDG_DATA_DIRS", fixture.0.join("system"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // Only the child environment changes: no unsafe process-global set_var in tests.
    let root = PathBuf::from(std::env::var_os(CHILD).unwrap());
    let data = root.join("data");
    let desktop = |id: &str, icon: &str| {
        write(
            &data.join("applications").join(format!("{id}.desktop")),
            &format!(
                "[Desktop Entry]\nName=Fixture\nIcon={icon}\nExec=never-execute-me\n[Desktop Action Other]\nIcon=wrong-group\n"
            ),
        )
    };
    let theme = TitlebarTheme {
        show_icon: true,
        ..Default::default()
    };
    let path = data.join("icons/hicolor/scalable/apps/test-icon.svg");
    svg(&path, "red");
    desktop("org.example.App", "test-icon");
    let mut assets = TitlebarAssets::prepare(&theme).unwrap();
    assert!(assets.app_icon("org.example.App").is_none());
    assets.prepare_app_icon("org.example.App");
    let first = assets.app_icon("org.example.App").unwrap().clone();
    assert_eq!(pixel(&first, 10, 10), [128, 0, 0, 128]);
    assets.prepare_app_icon("org.example.App.desktop");
    assert!(assets.app_icon("org.example.App.desktop").is_some());
    fs::remove_file(&path).unwrap();
    assets.prepare_app_icon("org.example.App");
    assert!(Arc::ptr_eq(
        &first,
        assets.app_icon("org.example.App").unwrap()
    ));
    assets.prepare_app_icon("org.example.Late");
    svg(&path, "blue");
    desktop("org.example.Late", "test-icon");
    assets.prepare_app_icon("org.example.Late");
    assert!(assets.app_icon("org.example.Late").is_none());
    let mut fresh = TitlebarAssets::prepare(&theme).unwrap();
    fresh.prepare_app_icon("org.example.Late");
    assert_eq!(
        pixel(fresh.app_icon("org.example.Late").unwrap(), 10, 10),
        [0, 0, 128, 128]
    );

    let png = data.join("pixmaps/png-icon.png");
    fs::create_dir_all(png.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 100, 50, 128]))
        .save(&png)
        .unwrap();
    desktop("org.example.Png", "png-icon");
    fresh.prepare_app_icon("org.example.Png");
    let png_image = fresh.app_icon("org.example.Png").unwrap();
    assert_eq!(pixel(png_image, 10, 10), [100, 50, 25, 128]);
    assert_eq!(pixel(png_image, 10, 0), [0, 0, 0, 0]);
    desktop("org.example.Absolute", png.to_str().unwrap());
    fresh.prepare_app_icon("org.example.Absolute");
    assert!(fresh.app_icon("org.example.Absolute").is_some());
    let system = root.join("system/icons/hicolor/32x32/apps/system.png");
    fs::create_dir_all(system.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]))
        .save(&system)
        .unwrap();
    write(
        &root.join("system/applications/org.example.System.desktop"),
        "[Desktop Entry]\nIcon=system\n",
    );
    fresh.prepare_app_icon("org.example.System");
    assert!(fresh.app_icon("org.example.System").is_some());
    desktop("org.example.Hidden", "test-icon");
    write(
        &data.join("applications/org.example.Hidden.desktop"),
        "[Desktop Entry]\nHidden=true\nIcon=test-icon\n",
    );
    fresh.prepare_app_icon("org.example.Hidden");
    assert!(fresh.app_icon("org.example.Hidden").is_none());
    desktop("org.example.Traversal", "../test-icon");
    fresh.prepare_app_icon("org.example.Traversal");
    assert!(fresh.app_icon("org.example.Traversal").is_none());
    let huge = data.join("pixmaps/huge.png");
    fs::File::create(&huge)
        .unwrap()
        .set_len(MAX_RASTER_BYTES + 1)
        .unwrap();
    desktop("org.example.Huge", "huge");
    fresh.prepare_app_icon("org.example.Huge");
    assert!(fresh.app_icon("org.example.Huge").is_none());
    image::RgbaImage::new(1025, 1)
        .save(data.join("pixmaps/wide.png"))
        .unwrap();
    desktop("org.example.Wide", "wide");
    fresh.prepare_app_icon("org.example.Wide");
    assert!(fresh.app_icon("org.example.Wide").is_none());
    let mut disabled = TitlebarAssets::prepare(&TitlebarTheme::default()).unwrap();
    disabled.prepare_app_icon("org.example.App");
    assert!(disabled.app_icon("org.example.App").is_none());
    let mut capped = TitlebarAssets::prepare(&theme).unwrap();
    for index in 0..MAX_APP_ICONS {
        capped.prepare_app_icon(&format!("missing-{index}"));
    }
    capped.prepare_app_icon("org.example.App");
    assert!(capped.app_icon("org.example.App").is_none());
    let mut invalid_ids = TitlebarAssets::prepare(&theme).unwrap();
    for id in [
        "../org.example.App",
        "/org.example.App",
        "",
        ".",
        "..",
        &"a".repeat(256),
    ] {
        invalid_ids.prepare_app_icon(id);
        assert!(invalid_ids.app_icon(id).is_none());
    }
    invalid_ids.prepare_app_icon("org.example.App");
    assert!(invalid_ids.app_icon("org.example.App").is_some());

    let config_path = root.join("config.toml");
    write(&config_path, "[theme.titlebar]\nshow_icon=true");
    let mut runtime = Runtime::load(Some(config_path.clone())).unwrap();
    runtime
        .desktop
        .add_output(OutputId(1), "virtual-1".into(), Rect::new(0, 0, 800, 600));
    runtime
        .desktop
        .add_window(WindowId(1), "App".into(), "org.example.App".into());
    runtime.classify_window(WindowId(1), "org.example.App");
    let before = runtime
        .titlebar_assets
        .app_icon("org.example.App")
        .unwrap()
        .clone();
    runtime
        .desktop
        .command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert!(runtime.placements().is_empty());
    svg(&path, "green");
    runtime.reload().unwrap();
    let after = runtime
        .titlebar_assets
        .app_icon("org.example.App")
        .unwrap()
        .clone();
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(pixel(&after, 10, 10), [0, 64, 0, 128]);
    write(
        &config_path,
        "gaps=99\n[theme.titlebar]\nshow_icon=true\n[theme.titlebar.controls]\nclose='missing.svg'",
    );
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.config.gaps, 8);
    assert!(Arc::ptr_eq(
        &after,
        runtime.titlebar_assets.app_icon("org.example.App").unwrap()
    ));
}
