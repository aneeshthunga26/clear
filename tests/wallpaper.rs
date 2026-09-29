use clear::{
    config::{Config, WallpaperMode},
    runtime::{
        Runtime,
        wallpaper::{MAX_ENCODED_BYTES, Wallpapers},
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
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
            "clear-wallpaper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn config(&self, source: &str) -> PathBuf {
        let path = self.0.join("config.toml");
        fs::write(&path, source).unwrap();
        path
    }
    fn png(&self, name: &str, color: [u8; 4]) -> PathBuf {
        let path = self.0.join(name);
        image::RgbaImage::from_pixel(3, 2, image::Rgba(color))
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn defaults_modes_inheritance_and_schema_validation() {
    let defaults = Config::from_source("").unwrap();
    assert_eq!(defaults.wallpaper.mode, WallpaperMode::Fill);
    assert!(defaults.wallpaper.path.is_none());
    assert!(
        Wallpapers::prepare(&defaults)
            .unwrap()
            .for_output("virtual-1")
            .is_none()
    );
    for (mode, expected) in [
        ("fill", WallpaperMode::Fill),
        ("fit", WallpaperMode::Fit),
        ("stretch", WallpaperMode::Stretch),
        ("center", WallpaperMode::Center),
    ] {
        let config = Config::from_source(&format!("[wallpaper]\npath='wall.png'\nmode='{mode}'\n[wallpaper.outputs.virtual-2]\nmode='center'")).unwrap();
        assert_eq!(
            config.wallpaper.for_output("virtual-1"),
            (Some(Path::new("wall.png")), expected)
        );
        assert_eq!(
            config.wallpaper.for_output("virtual-2"),
            (Some(Path::new("wall.png")), WallpaperMode::Center)
        );
    }
    for source in [
        "[wallpaper]\nmode='tile'",
        "[wallpaper]\npath=''",
        "[wallpaper]\npath='  '",
        "[wallpaper]\npath=42",
        "[wallpaper]\nunknown=true",
        "[wallpaper.outputs.missing]\npath='a.png'",
        "[wallpaper.outputs.virtual-2]\npath=''",
        "[wallpaper.outputs.virtual-2]\nmode='Fill'",
        "[wallpaper.outputs.virtual-2]\nunknown=true",
        "[wallpaper]\npath=\"a\\u0000b\"",
    ] {
        assert!(Config::from_source(source).is_err(), "{source}");
    }
    let config = Config::from_source("[wallpaper.outputs.virtual-2]\npath='second.png'").unwrap();
    assert_eq!(
        config.wallpaper.for_output("virtual-1"),
        (None, WallpaperMode::Fill)
    );
    assert_eq!(
        config.wallpaper.for_output("virtual-2"),
        (Some(Path::new("second.png")), WallpaperMode::Fill)
    );
}

#[test]
fn resolves_relative_and_absolute_paths_without_expansions() {
    let mut config = Config::from_source(
        "[wallpaper]\npath='~/wall.png'\n[wallpaper.outputs.virtual-2]\npath='/absolute/wall.jpg'",
    )
    .unwrap();
    config.resolve_paths(Path::new("/config"));
    assert_eq!(
        config.wallpaper.path.as_deref(),
        Some(Path::new("/config/~/wall.png"))
    );
    assert_eq!(
        config.wallpaper.for_output("virtual-2").0,
        Some(Path::new("/absolute/wall.jpg"))
    );
}

#[test]
fn png_is_premultiplied_shared_and_not_read_during_access() {
    let fixture = Fixture::new();
    let image = fixture.png("wall.data", [200, 100, 50, 128]);
    let runtime = Runtime::load(Some(
        fixture.config("[wallpaper]\npath='wall.data'\n[wallpaper.outputs.virtual-2]\nmode='fit'"),
    ))
    .unwrap();
    assert_eq!(
        runtime.config.wallpaper.path.as_deref(),
        Some(image.as_path())
    );
    let first = runtime.wallpapers.for_output("virtual-1").unwrap();
    let second = runtime.wallpapers.for_output("virtual-2").unwrap();
    assert!(Arc::ptr_eq(&first.image, &second.image));
    assert_eq!(first.image.size(), (3, 2));
    assert_eq!(&first.image.rgba()[..4], &[100, 50, 25, 128]);
    assert_eq!(second.mode, WallpaperMode::Fit);
    fs::remove_file(image).unwrap();
    assert_eq!(
        runtime
            .wallpapers
            .for_output("virtual-1")
            .unwrap()
            .image
            .rgba()
            .len(),
        24
    );
}

#[test]
fn jpeg_and_independent_output_override_are_prepared() {
    let fixture = Fixture::new();
    fixture.png("first.png", [255, 0, 0, 255]);
    image::RgbImage::from_pixel(2, 4, image::Rgb([0, 255, 0]))
        .save(fixture.0.join("second.jpg"))
        .unwrap();
    let runtime = Runtime::load(Some(fixture.config("[wallpaper]\npath='first.png'\nmode='fit'\n[wallpaper.outputs.virtual-2]\npath='second.jpg'"))).unwrap();
    let first = runtime.wallpapers.for_output("virtual-1").unwrap();
    let second = runtime.wallpapers.for_output("virtual-2").unwrap();
    assert!(!Arc::ptr_eq(&first.image, &second.image));
    assert_eq!(first.image.size(), (3, 2));
    assert_eq!(second.image.size(), (2, 4));
    assert_eq!(second.mode, WallpaperMode::Fit);
    assert!(
        second
            .image
            .rgba()
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 255)
    );
}

#[test]
fn explicit_reload_rereads_same_path_and_failures_keep_last_good_set_and_config() {
    let fixture = Fixture::new();
    fixture.png("wall.png", [255, 0, 0, 255]);
    let source = "gaps=17\n[wallpaper]\npath='wall.png'";
    let mut runtime = Runtime::load(Some(fixture.config(source))).unwrap();
    let before = runtime
        .wallpapers
        .for_output("virtual-1")
        .unwrap()
        .image
        .clone();
    fixture.png("wall.png", [0, 0, 255, 255]);
    assert_eq!(&before.rgba()[..4], &[255, 0, 0, 255]);
    runtime.reload().unwrap();
    let good = runtime
        .wallpapers
        .for_output("virtual-1")
        .unwrap()
        .image
        .clone();
    assert!(!Arc::ptr_eq(&before, &good));
    assert_eq!(&good.rgba()[..4], &[0, 0, 255, 255]);
    for bad in [
        "missing.png",
        "unsupported.gif",
        "broken.png",
        "too-large.png",
        "truncated.png",
    ] {
        match bad {
            "unsupported.gif" => fs::write(fixture.0.join(bad), b"GIF89a").unwrap(),
            "broken.png" => {
                fs::write(fixture.0.join(bad), b"not an image").unwrap();
            }
            "too-large.png" => fs::File::create(fixture.0.join(bad))
                .unwrap()
                .set_len(MAX_ENCODED_BYTES + 1)
                .unwrap(),
            "truncated.png" => {
                let bytes = fs::read(fixture.0.join("wall.png")).unwrap();
                fs::write(fixture.0.join(bad), &bytes[..40]).unwrap();
            }
            _ => {}
        }
        // A valid first resource and bad second resource must not partially apply.
        fixture.config(&format!(
            "gaps=99\n[wallpaper]\npath='wall.png'\n[wallpaper.outputs.virtual-2]\npath='{bad}'"
        ));
        assert!(runtime.reload().is_err(), "{bad}");
        assert_eq!(runtime.config.gaps, 17);
        assert!(runtime.config.wallpaper.outputs.is_empty());
        assert!(Arc::ptr_eq(
            &good,
            &runtime.wallpapers.for_output("virtual-1").unwrap().image
        ));
    }
    fixture.config("gaps=18\n[wallpaper]\npath='wall.png'\nmode='center'");
    runtime.reload().unwrap();
    assert_eq!(runtime.config.gaps, 18);
    assert_eq!(
        runtime.wallpapers.for_output("virtual-1").unwrap().mode,
        WallpaperMode::Center
    );
    fixture.config("");
    runtime.reload().unwrap();
    assert!(runtime.wallpapers.for_output("virtual-1").is_none());
}

#[test]
fn invalid_startup_resource_keeps_theme_and_other_settings_then_can_recover() {
    let fixture = Fixture::new();
    let mut runtime = Runtime::load(Some(fixture.config(
        "gaps=23\n[theme]\nbackground=[0.1,0.2,0.3,1.0]\n[wallpaper]\npath='later.png'",
    )))
    .unwrap();
    assert_eq!(runtime.config.gaps, 23);
    assert_eq!(runtime.config.theme.background, [0.1, 0.2, 0.3, 1.0]);
    assert!(runtime.wallpapers.for_output("virtual-1").is_none());
    fixture.png("later.png", [255, 255, 255, 255]);
    runtime.reload().unwrap();
    assert!(runtime.wallpapers.for_output("virtual-1").is_some());
}

#[test]
fn oversized_dimensions_are_rejected_from_header_without_decoding_pixels() {
    let fixture = Fixture::new();
    // A tiny valid PNG with a patched IHDR and checksum: no giant fixture allocation.
    let path = fixture.png("oversized.png", [0, 0, 0, 255]);
    let original = fs::read(&path).unwrap();
    for (width, height) in [(8193_u32, 1_u32), (8192, 8192)] {
        let mut bytes = original.clone();
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        let mut crc = !0_u32;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320_u32 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        fs::write(&path, bytes).unwrap();
        let mut config = Config::from_source("[wallpaper]\npath='oversized.png'").unwrap();
        config.resolve_paths(&fixture.0);
        let error = Wallpapers::prepare(&config).unwrap_err();
        assert!(
            error.contains("limit") || error.contains("dimensions"),
            "{error}"
        );
    }
}
