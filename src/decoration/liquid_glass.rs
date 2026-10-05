//! Reference magnifying-glass controls with independent magnification strength.

use serde::Deserialize;

/// Optional magnifying glass; foreground opacity and geometry remain independent.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LiquidGlass {
    /// Enable the reference filter, including when backdrop blur is disabled.
    pub enabled: bool,
    /// Alpha multiplier for the specular map, in `0..=1`.
    pub specular_opacity: f32,
    /// SVG saturation of the backdrop under the specular mask, in `0..=50`.
    pub specular_saturation: f32,
    /// Scale of the second displacement map, in `0..=10`.
    pub refraction_level: f32,
    /// Multiplier of the refraction rim width, in `0..=10`; zero disables refraction.
    pub refraction_width: f32,
    /// Multiplier of the reference magnification displacement, in `0..=2`.
    pub zoom_level: f32,
}

impl Default for LiquidGlass {
    fn default() -> Self {
        Self {
            enabled: false,
            specular_opacity: 0.5,
            specular_saturation: 9.0,
            refraction_level: 1.0,
            refraction_width: 1.0,
            zoom_level: 1.0,
        }
    }
}

impl LiquidGlass {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, value, max) in [
            ("specular_opacity", self.specular_opacity, 1.0),
            ("specular_saturation", self.specular_saturation, 50.0),
            ("refraction_level", self.refraction_level, 10.0),
            ("refraction_width", self.refraction_width, 10.0),
            ("zoom_level", self.zoom_level, 2.0),
        ] {
            if !value.is_finite() || !(0.0..=max).contains(&value) {
                return Err(format!(
                    "theme.liquid_glass.{name} must be finite and in 0..={max}"
                ));
            }
        }
        Ok(())
    }
}
