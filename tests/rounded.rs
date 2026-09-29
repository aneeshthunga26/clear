use clear::{config::Config, runtime::Runtime};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn corner_radius_defaults_to_square_and_expands_supported_forms_clockwise() {
    assert_eq!(Config::default().theme.corner_radius.values(), [0.0; 4]);
    assert!(!Config::default().theme.corner_radius.is_rounded());
    for (value, expected) in [
        ("12", [12.0; 4]),
        ("[12]", [12.0; 4]),
        ("[16, 4]", [16.0, 16.0, 4.0, 4.0]),
        ("[1, 2, 3, 4]", [1.0, 2.0, 3.0, 4.0]),
        ("[0, 256, 0.5, 12.25]", [0.0, 256.0, 0.5, 12.25]),
    ] {
        let config = Config::from_source(&format!("[theme]\ncorner_radius={value}")).unwrap();
        assert_eq!(config.theme.corner_radius.values(), expected);
        assert!(config.theme.corner_radius.is_rounded());
    }
}

#[test]
fn malformed_negative_nonfinite_and_out_of_range_corner_radii_are_rejected() {
    for value in [
        "[]",
        "[1, 2, 3]",
        "[1, 2, 3, 4, 5]",
        "-1",
        "[1, -1]",
        "257",
        "[0, 0, 0, 256.5]",
        "nan",
        "inf",
        "-inf",
        "[1, nan]",
        "'12'",
        "true",
        "[[12]]",
        "{top=12}",
    ] {
        assert!(
            Config::from_source(&format!("[theme]\ncorner_radius={value}")).is_err(),
            "accepted {value}"
        );
    }
    assert!(Config::from_source("[theme]\ncorner_radii=12").is_err());
}

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn radii_reload_atomically_and_disabling_restores_the_square_default() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "clear-rounded-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("config.toml");
    fs::write(&path, "[theme]\ncorner_radius=12").unwrap();
    let mut runtime = Runtime::load(Some(path.clone())).unwrap();
    fs::write(&path, "[theme]\ncorner_radius=[20, 4]").unwrap();
    runtime.reload().unwrap();
    assert_eq!(
        runtime.config.theme.corner_radius.values(),
        [20.0, 20.0, 4.0, 4.0]
    );
    let before = runtime.config.theme.clone();
    fs::write(&path, "[theme]\nborder_width=8\ncorner_radius=[1, 2, 3]").unwrap();
    assert!(runtime.reload().is_err());
    assert_eq!(runtime.config.theme, before);
    fs::write(&path, "[theme]\ncorner_radius=0").unwrap();
    runtime.reload().unwrap();
    assert!(!runtime.config.theme.corner_radius.is_rounded());
}
