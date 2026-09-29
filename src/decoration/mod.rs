//! Declarative compositor styling; no client-side widgets or rendering dependencies.

use serde::{Deserialize, Deserializer, de::Error};

mod titlebar;
pub use titlebar::{ControlsSide, TitlebarControls, TitlebarTheme};
mod liquid_glass;
pub use liquid_glass::LiquidGlass;

/// Logical-pixel outer corner radii, clockwise from top-left.
/// Deserialize a scalar or a list of one, two (top/bottom), or four values.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CornerRadii([f32; 4]);

impl CornerRadii {
    /// Expanded radii: top-left, top-right, bottom-right, bottom-left.
    pub fn values(self) -> [f32; 4] {
        self.0
    }

    /// Whether the window needs a rounded outline rather than a square one.
    pub fn is_rounded(self) -> bool {
        self.0.iter().any(|radius| *radius > 0.0)
    }
}

impl<'de> Deserialize<'de> for CornerRadii {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Source {
            One(f32),
            Many(Vec<f32>),
        }
        let values = match Source::deserialize(deserializer)? {
            Source::One(value) => [value; 4],
            Source::Many(values) => match values.as_slice() {
                [all] => [*all; 4],
                [top, bottom] => [*top, *top, *bottom, *bottom],
                [tl, tr, br, bl] => [*tl, *tr, *br, *bl],
                _ => {
                    return Err(D::Error::custom(
                        "theme.corner_radius expects one, two (top/bottom), or four (TL/TR/BR/BL) values",
                    ));
                }
            },
        };
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=256.0).contains(value))
        {
            return Err(D::Error::custom(
                "theme.corner_radius values must be finite and in 0..=256 logical pixels",
            ));
        }
        Ok(Self(values))
    }
}

/// Global backdrop filter. Gaussian is the default for existing configurations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlurMethod {
    /// Two-pass separable Gaussian, with radius measured in logical pixels.
    #[default]
    Gaussian,
    /// Dual Kawase downsample/upsample pyramid with configurable depth.
    Kawase,
}

/// Static colors and border dimensions used by the compositor renderer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    /// Output clear color in non-premultiplied RGBA, with channels in `0..=1`.
    pub background: [f32; 4],
    /// Border color for the focused window.
    pub active_border: [f32; 4],
    /// Border color for unfocused windows.
    pub inactive_border: [f32; 4],
    /// Border thickness in logical pixels.
    pub border_width: i32,
    /// Outer outline radii. Zero retains square corners; oversized radii fit the window.
    pub corner_radius: CornerRadii,
    /// Global backdrop filter; applies to windows, layers, and popups alike.
    pub blur_method: BlurMethod,
    /// Gaussian support radius or Kawase source-level sample offset; zero disables blur.
    pub blur_radius: f32,
    /// Kawase pyramid depth (1..=6); Gaussian always uses two filtering passes.
    pub blur_passes: u8,
    /// Optional refraction and edge lighting applied to either filtered backdrop.
    pub liquid_glass: LiquidGlass,
    /// Styling and optional icon resources for negotiated server-side titlebars.
    pub titlebar: TitlebarTheme,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: [0.07, 0.08, 0.10, 1.0],
            active_border: [0.40, 0.65, 0.95, 1.0],
            inactive_border: [0.22, 0.24, 0.28, 1.0],
            border_width: 2,
            corner_radius: CornerRadii::default(),
            blur_method: BlurMethod::default(),
            blur_radius: 0.0,
            blur_passes: 3,
            liquid_glass: LiquidGlass::default(),
            titlebar: TitlebarTheme::default(),
        }
    }
}

impl Theme {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, color) in [
            ("background", self.background),
            ("active_border", self.active_border),
            ("inactive_border", self.inactive_border),
        ] {
            if color
                .iter()
                .any(|c| !c.is_finite() || !(0.0..=1.0).contains(c))
            {
                return Err(format!(
                    "theme.{name}: RGBA channels must be finite and in 0..=1"
                ));
            }
        }
        if !(0..=64).contains(&self.border_width) {
            return Err("theme.border_width must be in 0..=64".into());
        }
        if !self.blur_radius.is_finite() || !(0.0..=32.0).contains(&self.blur_radius) {
            return Err("theme.blur_radius must be finite and in 0..=32".into());
        }
        if !(1..=6).contains(&self.blur_passes) {
            return Err("theme.blur_passes must be in 1..=6".into());
        }
        self.liquid_glass.validate()?;
        self.titlebar.validate()
    }
}
