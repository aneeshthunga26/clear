//! Strict animation preferences; visual effects are connected in later slices.

use serde::{Deserialize, Deserializer, de};

/// Requested animation sampling cadence, independent of the display mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnimationFrameRate {
    /// Follow the timing source's current refresh rate.
    #[default]
    RefreshRate,
    /// Request one of the supported fixed rates: 30, 60, or 120 frames/second.
    Fixed(u16),
}

impl<'de> Deserialize<'de> for AnimationFrameRate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Name(String),
            Rate(u16),
        }
        match Value::deserialize(deserializer)? {
            Value::Name(name) if name == "refresh-rate" => Ok(Self::RefreshRate),
            Value::Rate(rate @ (30 | 60 | 120)) => Ok(Self::Fixed(rate)),
            _ => Err(de::Error::custom(
                "animation frame_rate must be 'refresh-rate', 30, 60, or 120",
            )),
        }
    }
}

/// Built-in timed progress curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationCurve {
    Linear,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutExpo,
}

/// Captured motion law; spring damping is always critical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationKind {
    Easing {
        duration_ms: u16,
        curve: AnimationCurve,
    },
    Spring {
        stiffness: f64,
    },
}

/// One effect's motion preferences.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationConfig {
    pub enabled: bool,
    pub kind: AnimationKind,
    /// Only opening/closing accept a configured scale; other effects use 1.
    pub scale: f64,
}

/// Stable effect names for track settings and reload settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationEffect {
    WorkspaceSwitch,
    WindowOpen,
    WindowClose,
    WindowMovement,
    Minimize,
    Maximize,
    Overview,
    Fullscreen,
}

/// Global preferences and all eight independently configurable effects.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationsConfig {
    pub enabled: bool,
    pub reduced_motion: bool,
    pub speed: f64,
    pub frame_rate: AnimationFrameRate,
    pub workspace_switch: AnimationConfig,
    pub window_open: AnimationConfig,
    pub window_close: AnimationConfig,
    pub window_movement: AnimationConfig,
    pub minimize: AnimationConfig,
    pub maximize: AnimationConfig,
    pub overview: AnimationConfig,
    pub fullscreen: AnimationConfig,
}

impl Default for AnimationsConfig {
    fn default() -> Self {
        let spring = |stiffness| AnimationConfig {
            enabled: true,
            kind: AnimationKind::Spring { stiffness },
            scale: 1.0,
        };
        let easing = |duration_ms, curve, scale| AnimationConfig {
            enabled: true,
            kind: AnimationKind::Easing { duration_ms, curve },
            scale,
        };
        Self {
            enabled: false,
            reduced_motion: false,
            speed: 1.0,
            frame_rate: AnimationFrameRate::RefreshRate,
            workspace_switch: spring(1000.0),
            window_open: easing(150, AnimationCurve::EaseOutExpo, 1.04),
            window_close: easing(150, AnimationCurve::EaseOutQuad, 1.04),
            window_movement: spring(800.0),
            minimize: easing(250, AnimationCurve::EaseOutCubic, 1.0),
            maximize: spring(800.0),
            overview: spring(800.0),
            fullscreen: spring(800.0),
        }
    }
}

impl AnimationsConfig {
    /// Return the independently captured preferences for this effect.
    pub fn effect(&self, effect: AnimationEffect) -> AnimationConfig {
        match effect {
            AnimationEffect::WorkspaceSwitch => self.workspace_switch,
            AnimationEffect::WindowOpen => self.window_open,
            AnimationEffect::WindowClose => self.window_close,
            AnimationEffect::WindowMovement => self.window_movement,
            AnimationEffect::Minimize => self.minimize,
            AnimationEffect::Maximize => self.maximize,
            AnimationEffect::Overview => self.overview,
            AnimationEffect::Fullscreen => self.fullscreen,
        }
    }

    /// Check public programmatic values as well as TOML values, including disabled effects.
    pub fn validate(&self) -> Result<(), String> {
        finite_range(self.speed, 0.1, 10.0, "animation speed")?;
        if matches!(self.frame_rate, AnimationFrameRate::Fixed(rate) if !matches!(rate, 30 | 60 | 120))
        {
            return Err("animation frame_rate must be 30, 60, 120, or refresh-rate".into());
        }
        for effect in [
            AnimationEffect::WorkspaceSwitch,
            AnimationEffect::WindowOpen,
            AnimationEffect::WindowClose,
            AnimationEffect::WindowMovement,
            AnimationEffect::Minimize,
            AnimationEffect::Maximize,
            AnimationEffect::Overview,
            AnimationEffect::Fullscreen,
        ] {
            let config = self.effect(effect);
            match config.kind {
                AnimationKind::Easing { duration_ms, .. } if duration_ms > 2000 => {
                    return Err("animation duration_ms must be in 0..=2000".into());
                }
                AnimationKind::Spring { stiffness } => {
                    finite_range(stiffness, 1.0, 10000.0, "animation stiffness")?
                }
                _ => {}
            }
            if matches!(
                effect,
                AnimationEffect::WindowOpen | AnimationEffect::WindowClose
            ) {
                finite_range(config.scale, 1.0, 1.25, "animation scale")?;
            } else if config.scale != 1.0 {
                return Err(
                    "animation scale is only supported for window_open/window_close".into(),
                );
            }
        }
        Ok(())
    }
}

fn finite_range(value: f64, min: f64, max: f64, name: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be finite and in {min}..={max}"))
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EffectSource {
    enabled: Option<bool>,
    kind: Option<String>,
    duration_ms: Option<u16>,
    curve: Option<AnimationCurve>,
    stiffness: Option<f64>,
    scale: Option<f64>,
}

impl EffectSource {
    fn apply(self, default: AnimationConfig, allow_scale: bool) -> Result<AnimationConfig, String> {
        let kind = match self.kind.as_deref() {
            None => default.kind,
            Some("easing") => AnimationKind::Easing {
                duration_ms: 250,
                curve: AnimationCurve::EaseOutCubic,
            },
            Some("spring") => AnimationKind::Spring { stiffness: 800.0 },
            Some(_) => return Err("animation kind must be easing or spring".into()),
        };
        // Explicitly naming the default kind still inherits that effect's parameters.
        let kind = if std::mem::discriminant(&kind) == std::mem::discriminant(&default.kind) {
            default.kind
        } else {
            kind
        };
        let kind = match kind {
            AnimationKind::Easing { duration_ms, curve } => {
                if self.stiffness.is_some() {
                    return Err("easing animation cannot have stiffness".into());
                }
                AnimationKind::Easing {
                    duration_ms: self.duration_ms.unwrap_or(duration_ms),
                    curve: self.curve.unwrap_or(curve),
                }
            }
            AnimationKind::Spring { stiffness } => {
                if self.duration_ms.is_some() || self.curve.is_some() {
                    return Err("spring animation cannot have duration_ms or curve".into());
                }
                AnimationKind::Spring {
                    stiffness: self.stiffness.unwrap_or(stiffness),
                }
            }
        };
        if !allow_scale && self.scale.is_some() {
            return Err("scale is only accepted for window_open/window_close".into());
        }
        Ok(AnimationConfig {
            enabled: self.enabled.unwrap_or(default.enabled),
            kind,
            scale: self.scale.unwrap_or(default.scale),
        })
    }
}

impl<'de> Deserialize<'de> for AnimationsConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Source {
            enabled: Option<bool>,
            reduced_motion: Option<bool>,
            speed: Option<f64>,
            frame_rate: Option<AnimationFrameRate>,
            workspace_switch: EffectSource,
            window_open: EffectSource,
            window_close: EffectSource,
            window_movement: EffectSource,
            minimize: EffectSource,
            maximize: EffectSource,
            overview: EffectSource,
            fullscreen: EffectSource,
        }
        let s = Source::deserialize(deserializer)?;
        let d = Self::default();
        let config = Self {
            enabled: s.enabled.unwrap_or(d.enabled),
            reduced_motion: s.reduced_motion.unwrap_or(d.reduced_motion),
            speed: s.speed.unwrap_or(d.speed),
            frame_rate: s.frame_rate.unwrap_or(d.frame_rate),
            workspace_switch: s
                .workspace_switch
                .apply(d.workspace_switch, false)
                .map_err(de::Error::custom)?,
            window_open: s
                .window_open
                .apply(d.window_open, true)
                .map_err(de::Error::custom)?,
            window_close: s
                .window_close
                .apply(d.window_close, true)
                .map_err(de::Error::custom)?,
            window_movement: s
                .window_movement
                .apply(d.window_movement, false)
                .map_err(de::Error::custom)?,
            minimize: s
                .minimize
                .apply(d.minimize, false)
                .map_err(de::Error::custom)?,
            maximize: s
                .maximize
                .apply(d.maximize, false)
                .map_err(de::Error::custom)?,
            overview: s
                .overview
                .apply(d.overview, false)
                .map_err(de::Error::custom)?,
            fullscreen: s
                .fullscreen
                .apply(d.fullscreen, false)
                .map_err(de::Error::custom)?,
        };
        config.validate().map_err(de::Error::custom)?;
        Ok(config)
    }
}
