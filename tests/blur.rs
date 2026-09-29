use clear::{config::Config, decoration::BlurMethod, runtime::Runtime};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn global_blur_defaults_off_and_accepts_bounded_fractional_radii() {
    assert_eq!(Config::default().theme.blur_radius, 0.0);
    assert_eq!(Config::default().theme.blur_method, BlurMethod::Gaussian);
    assert_eq!(Config::default().theme.blur_passes, 3);
    for (source, expected) in [("0", 0.0), ("12", 12.0), ("0.5", 0.5), ("32.0", 32.0)] {
        let config = Config::from_source(&format!("[theme]\nblur_radius={source}")).unwrap();
        assert_eq!(config.theme.blur_radius, expected);
    }
    for example in [
        include_str!("../examples/config.toml"),
        include_str!("../examples/dual-virtual-monitors.toml"),
        include_str!("../examples/single-monitor.toml"),
    ] {
        Config::from_source(example).unwrap();
    }
}

#[test]
fn blur_rejects_negative_nonfinite_out_of_range_and_non_scalar_values() {
    for source in [
        "-1",
        "32.5",
        "nan",
        "inf",
        "-inf",
        "[12]",
        "'12'",
        "true",
        "{radius=12}",
    ] {
        assert!(
            Config::from_source(&format!("[theme]\nblur_radius={source}")).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn blur_methods_and_kawase_depth_are_strict_and_backward_compatible() {
    let old = Config::from_source("[theme]\nblur_radius=12").unwrap();
    assert_eq!(old.theme.blur_method, BlurMethod::Gaussian);
    for (name, method) in [
        ("gaussian", BlurMethod::Gaussian),
        ("kawase", BlurMethod::Kawase),
    ] {
        for passes in 1..=6 {
            let config = Config::from_source(&format!(
                "[theme]\nblur_method='{name}'\nblur_radius=1.5\nblur_passes={passes}"
            ))
            .unwrap();
            assert_eq!(config.theme.blur_method, method);
            assert_eq!(config.theme.blur_passes, passes);
        }
    }
    for value in ["'glass'", "'Kawase'", "''", "true", "1", "['kawase']"] {
        assert!(
            Config::from_source(&format!("[theme]\nblur_method={value}")).is_err(),
            "accepted method {value}"
        );
    }
    // Validate even when blur is off or Gaussian is selected, so typos never lurk.
    for value in ["0", "7", "256", "-1", "1.5", "3.0", "'3'", "true", "[3]"] {
        assert!(
            Config::from_source(&format!("[theme]\nblur_passes={value}")).is_err(),
            "accepted passes {value}"
        );
    }
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn blur_reload_is_atomic_and_zero_disables_without_affecting_opacity() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "clear-blur-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("config.toml");
    fs::write(&path, "[theme]\nblur_radius=12").unwrap();
    let mut runtime = Runtime::load(Some(path.clone())).unwrap();
    let background = runtime.config.theme.background;
    fs::write(&path, "[theme]\nblur_radius=4.5").unwrap();
    runtime.reload().unwrap();
    assert_eq!(runtime.config.theme.blur_radius, 4.5);
    fs::write(
        &path,
        "[theme]\nblur_method='kawase'\nblur_radius=2\nblur_passes=6",
    )
    .unwrap();
    runtime.reload().unwrap();
    assert_eq!(runtime.config.theme.blur_method, BlurMethod::Kawase);
    assert_eq!(runtime.config.theme.blur_passes, 6);
    let before = runtime.config.theme.clone();
    for invalid in ["blur_passes=0", "blur_method='glass'", "blur_passes=7"] {
        fs::write(&path, format!("[theme]\nblur_radius=8\n{invalid}")).unwrap();
        assert!(runtime.reload().is_err());
        assert_eq!(runtime.config.theme, before);
    }
    fs::write(&path, "[theme]\nblur_radius=33\nborder_width=8").unwrap();
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.config.theme, before);
    fs::write(&path, "[theme]\nblur_radius=0").unwrap();
    runtime.reload().unwrap();
    assert_eq!(runtime.config.theme.blur_radius, 0.0);
    assert_eq!(runtime.config.theme.blur_method, BlurMethod::Gaussian);
    assert_eq!(runtime.config.theme.blur_passes, 3);
    assert_eq!(runtime.config.theme.background, background);
}
