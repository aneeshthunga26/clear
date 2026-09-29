//! Backend-neutral server-side titlebar styling.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, de::Error};

/// Edge on which the minimize, maximize/restore, and close controls appear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlsSide {
    Left,
    #[default]
    Right,
}

/// Optional SVG replacements; absent entries use the adapter's built-in glyphs.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TitlebarControls {
    pub minimize: Option<PathBuf>,
    pub maximize: Option<PathBuf>,
    pub restore: Option<PathBuf>,
    pub close: Option<PathBuf>,
}

impl TitlebarControls {
    pub(crate) fn paths(&self) -> [(&'static str, Option<&Path>); 4] {
        [
            ("minimize", self.minimize.as_deref()),
            ("maximize", self.maximize.as_deref()),
            ("restore", self.restore.as_deref()),
            ("close", self.close.as_deref()),
        ]
    }

    pub(crate) fn resolve_paths(&mut self, directory: &Path) {
        for path in [
            &mut self.minimize,
            &mut self.maximize,
            &mut self.restore,
            &mut self.close,
        ]
        .into_iter()
        .flatten()
        {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
    }
}

/// Straight RGBA colors and logical dimensions for negotiated server decorations.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TitlebarTheme {
    #[serde(deserialize_with = "deserialize_rgba")]
    pub active_background: [f32; 4],
    #[serde(deserialize_with = "deserialize_rgba")]
    pub inactive_background: [f32; 4],
    #[serde(deserialize_with = "deserialize_rgba")]
    pub active_foreground: [f32; 4],
    #[serde(deserialize_with = "deserialize_rgba")]
    pub inactive_foreground: [f32; 4],
    /// Total titlebar height in logical pixels, in `16..=128`.
    pub height: i32,
    pub controls_side: ControlsSide,
    pub show_icon: bool,
    pub show_title: bool,
    pub controls: TitlebarControls,
}

fn deserialize_rgba<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[f32; 4], D::Error> {
    // TOML's tuple deserializer can ignore extra elements; consume the full list.
    let values = Vec::<f32>::deserialize(deserializer)?;
    values.try_into().map_err(|_: Vec<f32>| {
        D::Error::custom("titlebar colors require exactly four RGBA channels")
    })
}

impl Default for TitlebarTheme {
    fn default() -> Self {
        let rgba = |r, g, b| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
        Self {
            active_background: rgba(35, 40, 52),
            inactive_background: rgba(27, 30, 38),
            active_foreground: rgba(239, 243, 250),
            inactive_foreground: rgba(174, 183, 199),
            height: 32,
            controls_side: ControlsSide::Right,
            show_icon: false,
            show_title: true,
            controls: TitlebarControls::default(),
        }
    }
}

impl TitlebarTheme {
    /// CPU icon raster box; independent of renderer scale and window width.
    pub fn icon_size(&self) -> u32 {
        self.height.saturating_sub(12).clamp(1, 20) as u32
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, color) in [
            ("active_background", self.active_background),
            ("inactive_background", self.inactive_background),
            ("active_foreground", self.active_foreground),
            ("inactive_foreground", self.inactive_foreground),
        ] {
            if color
                .iter()
                .any(|c| !c.is_finite() || !(0.0..=1.0).contains(c))
            {
                return Err(format!(
                    "theme.titlebar.{name}: RGBA channels must be finite and in 0..=1"
                ));
            }
        }
        if !(16..=128).contains(&self.height) {
            return Err("theme.titlebar.height must be in 16..=128".into());
        }
        for (name, path) in self.controls.paths() {
            if let Some(path) = path {
                let text = path.to_string_lossy();
                if text.trim().is_empty() || text.contains('\0') {
                    return Err(format!(
                        "theme.titlebar.controls.{name}: path must be nonempty and contain no NUL"
                    ));
                }
            }
        }
        Ok(())
    }
}
