//! Static compositor colors; no client-side widgets or rendering dependencies.

use serde::Deserialize;

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
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: [0.07, 0.08, 0.10, 1.0],
            active_border: [0.40, 0.65, 0.95, 1.0],
            inactive_border: [0.22, 0.24, 0.28, 1.0],
            border_width: 2,
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
        Ok(())
    }
}
