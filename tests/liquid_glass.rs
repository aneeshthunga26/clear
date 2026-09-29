use clear::{config::Config, decoration::LiquidGlass, runtime::Runtime};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn glass_is_optional_and_independent_of_blur_method() {
    assert!(!Config::default().theme.liquid_glass.enabled);
    assert!(
        Config::from_source(include_str!("../examples/liquid-glass.toml"))
            .unwrap()
            .theme
            .liquid_glass
            .enabled
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
    for (field, low, high) in [
        ("refraction_strength", 0.0, 64.0),
        ("edge_width", 1.0, 128.0),
        ("liquidity", 0.0, 1.0),
        ("dispersion", 0.0, 1.0),
        ("highlight", 0.0, 1.0),
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
    for method in ["kawase", "gaussian"] {
        fs::write(&path, format!("[theme]\nblur_method='{method}'\nblur_radius=2\n[theme.liquid_glass]\nenabled=true\nrefraction_strength=18.5")).unwrap();
        runtime.reload().unwrap();
        assert!(runtime.config.theme.liquid_glass.enabled);
        assert_eq!(runtime.config.theme.liquid_glass.refraction_strength, 18.5);
        let before = runtime.config.theme.clone();
        fs::write(
            &path,
            "[theme]\nborder_width=8\n[theme.liquid_glass]\nenabled=false\ndispersion=2",
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
