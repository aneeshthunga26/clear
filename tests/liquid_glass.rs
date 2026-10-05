use clear::{config::Config, decoration::LiquidGlass, input::Action, runtime::Runtime};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn glass_is_optional_and_independent_of_blur_method() {
    assert!(!Config::default().theme.liquid_glass.enabled);
    let defaults = LiquidGlass::default();
    assert_eq!(defaults.specular_opacity, 0.5);
    assert_eq!(defaults.specular_saturation, 9.0);
    assert_eq!(defaults.refraction_level, 1.0);
    assert_eq!(defaults.refraction_width, 1.0);
    assert_eq!(defaults.zoom_level, 1.0);
    let example = Config::from_source(include_str!("../examples/liquid-glass.toml")).unwrap();
    assert!(example.theme.liquid_glass.enabled);
    assert!(
        example.bindings.iter().any(|binding| {
            binding.key == "Alt+Tab" && matches!(binding.action, Action::AltTab)
        })
    );
    assert_eq!(
        Config::from_source("[theme.liquid_glass]")
            .unwrap()
            .theme
            .liquid_glass,
        LiquidGlass::default()
    );
    for method in ["gaussian", "kawase"] {
        for radius in [0, 2, 32] {
            let config = Config::from_source(&format!(
                "[theme]\nblur_method='{method}'\nblur_radius={radius}\n[theme.liquid_glass]\nenabled=true"
            )).unwrap();
            assert!(config.theme.liquid_glass.enabled);
            assert_eq!(config.theme.blur_radius, radius as f32);
        }
    }
}

#[test]
fn glass_parameters_are_strict_and_validated_even_when_disabled() {
    for name in [
        "model",
        "lens_area",
        "refraction_strength",
        "edge_width",
        "liquidity",
        "dispersion",
        "highlight",
        "smooth_corners",
        "reflection_strength",
        "reflection_width",
        "mirror_strength",
        "mirror_width",
    ] {
        assert!(Config::from_source(&format!("[theme.liquid_glass]\n{name}=0")).is_err());
    }
    for (field, low, high) in [
        ("specular_opacity", 0.0, 1.0),
        ("specular_saturation", 0.0, 50.0),
        ("refraction_level", 0.0, 10.0),
        ("refraction_width", 0.0, 10.0),
        ("zoom_level", 0.0, 2.0),
    ] {
        for value in [low, (low + high) / 2.0, high] {
            Config::from_source(&format!("[theme.liquid_glass]\n{field}={value}")).unwrap();
        }
        for value in [
            "nan".into(),
            "inf".into(),
            "-inf".into(),
            "true".into(),
            "'1'".into(),
            "[1]".into(),
            (low - 0.01).to_string(),
            (high + 0.01).to_string(),
        ] {
            assert!(
                Config::from_source(&format!("[theme.liquid_glass]\n{field}={value}")).is_err(),
                "{field}={value}"
            );
        }
    }
    for value in ["1", "'true'", "[]", "{}"] {
        assert!(Config::from_source(&format!("[theme.liquid_glass]\nenabled={value}")).is_err());
    }
    assert!(Config::from_source("[theme.liquid_glass]\nunknown=1").is_err());
    assert!(Config::from_source("[theme]\nliquid_glass=true").is_err());
}

#[test]
fn glass_reload_preserves_last_good_theme_and_can_switch_filters_or_disable() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "clear-glass-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let path = dir.join("config.toml");
    fs::write(&path, "[theme]\nblur_radius=12").unwrap();
    let mut runtime = Runtime::load(Some(path.clone())).unwrap();
    for (method, zoom, width) in [("kawase", 0.0, 0.0), ("gaussian", 1.75, 10.0)] {
        fs::write(&path, format!("[theme]\nblur_method='{method}'\nblur_radius=0\n[theme.liquid_glass]\nenabled=true\nspecular_opacity=0.75\nspecular_saturation=12\nrefraction_level=10\nrefraction_width={width}\nzoom_level={zoom}")).unwrap();
        runtime.reload().unwrap();
        assert!(runtime.config.theme.liquid_glass.enabled);
        assert_eq!(runtime.config.theme.liquid_glass.specular_opacity, 0.75);
        assert_eq!(runtime.config.theme.liquid_glass.specular_saturation, 12.0);
        assert_eq!(runtime.config.theme.liquid_glass.refraction_level, 10.0);
        assert_eq!(runtime.config.theme.liquid_glass.refraction_width, width);
        assert_eq!(runtime.config.theme.liquid_glass.zoom_level, zoom);
        let before = runtime.config.theme.clone();
        fs::write(
            &path,
            "[theme]\nborder_width=8\n[theme.liquid_glass]\nenabled=false\nrefraction_width=10.01",
        )
        .unwrap();
        assert!(runtime.reload().is_err());
        assert_eq!(runtime.config.theme, before);
    }
    fs::write(
        &path,
        "[theme]\nblur_radius=2\n[theme.liquid_glass]\nenabled=false",
    )
    .unwrap();
    runtime.reload().unwrap();
    assert!(!runtime.config.theme.liquid_glass.enabled);
    assert_eq!(runtime.config.theme.blur_radius, 2.0);
}
