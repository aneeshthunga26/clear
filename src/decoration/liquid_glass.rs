//! Optional optical treatment of the filtered backdrop, independent of blur method.

use serde::Deserialize;

/// Bounded liquid-glass optics; never changes foreground opacity or geometry.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LiquidGlass {
    /// Apply optics after either blur filter; radius zero still bypasses all effects.
    pub enabled: bool,
    /// Maximum per-axis backdrop displacement in logical pixels, in `0..=64`.
    pub refraction_strength: f32,
    /// Width of the curved edge in logical pixels, in `1..=128`.
    pub edge_width: f32,
    /// Interior dome curvature, in `0..=1`.
    pub liquidity: f32,
    /// Chromatic dispersion, in `0..=1`; zero samples all channels together.
    pub dispersion: f32,
    /// Directional edge reflection strength, in `0..=1`.
    pub highlight: f32,
}

impl Default for LiquidGlass {
    fn default() -> Self {
        Self {
            enabled: false,
            refraction_strength: 12.0,
            edge_width: 24.0,
            liquidity: 0.5,
            dispersion: 0.15,
            highlight: 0.25,
        }
    }
}

impl LiquidGlass {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, value, min, max) in [
            ("refraction_strength", self.refraction_strength, 0.0, 64.0),
            ("edge_width", self.edge_width, 1.0, 128.0),
            ("liquidity", self.liquidity, 0.0, 1.0),
            ("dispersion", self.dispersion, 0.0, 1.0),
            ("highlight", self.highlight, 0.0, 1.0),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(format!(
                    "theme.liquid_glass.{name} must be finite and in {min}..={max}"
                ));
            }
        }
        Ok(())
    }
}
