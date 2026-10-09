//! Closed-form presentation motion, without desktop policy or GPU resources.

use std::{collections::BTreeMap, time::Duration};

use crate::config::{AnimationCurve, AnimationEffect, AnimationKind, AnimationsConfig};

/// Maximum simultaneously retained pose tracks (there are no retained images here).
pub const MAX_ANIMATION_TRACKS: usize = 256;
/// Hard bound for springs in animation seconds, independent of sampling cadence.
pub const MAX_SPRING_SECONDS: f64 = 2.0;
/// Input geometry covers core's i32 range while keeping spring algebra finite.
pub const MAX_POSE_COMPONENT: f64 = 2_147_483_648.0;

/// Pure epoch mapping; predicted samples never mutate shared time.
#[derive(Debug, Clone)]
pub struct AnimationClock {
    real_epoch: Duration,
    animation_epoch: f64,
    speed: f64,
}

impl AnimationClock {
    /// Create a zero-phase clock at a monotonic timestamp.
    pub fn new(now: Duration, speed: f64) -> Result<Self, String> {
        validate_speed(speed)?;
        Ok(Self {
            real_epoch: now,
            animation_epoch: 0.0,
            speed,
        })
    }

    /// Map a timestamp into animation seconds; older predictions clamp at the epoch.
    pub fn sample(&self, now: Duration) -> f64 {
        self.animation_epoch + now.saturating_sub(self.real_epoch).as_secs_f64() * self.speed
    }

    /// Change speed while preserving the animation phase at this timestamp.
    pub fn rebase(&mut self, now: Duration, speed: f64) -> Result<(), String> {
        validate_speed(speed)?;
        if now < self.real_epoch {
            return Err("animation clock rebase must be monotonic".into());
        }
        self.animation_epoch = self.sample(now);
        self.real_epoch = now;
        self.speed = speed;
        Ok(())
    }
}

fn validate_speed(speed: f64) -> Result<(), String> {
    if speed.is_finite() && (0.1..=10.0).contains(&speed) {
        Ok(())
    } else {
        Err("animation speed must be finite and in 0.1..=10".into())
    }
}

/// Unrounded logical presentation geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Presentation-only rectangle and opacity, independent of committed layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationPose {
    pub rect: AnimationRect,
    pub opacity: f64,
}

impl AnimationPose {
    fn values(self) -> [f64; 5] {
        [
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.opacity,
        ]
    }
    fn from_values(v: [f64; 5]) -> Self {
        Self {
            rect: AnimationRect {
                x: v[0],
                y: v[1],
                width: v[2],
                height: v[3],
            },
            opacity: v[4].clamp(0.0, 1.0),
        }
    }
    fn validate(self) -> Result<(), String> {
        if !self.values().iter().all(|v| v.is_finite())
            || self.values()[..4]
                .iter()
                .any(|v| v.abs() > MAX_POSE_COMPONENT)
            || self.rect.width <= 0.0
            || self.rect.height <= 0.0
            || !(0.0..=1.0).contains(&self.opacity)
        {
            return Err("animation pose requires finite geometry within the core i32 range, positive dimensions, and opacity in 0..=1".into());
        }
        Ok(())
    }
}

/// A deterministic sample, including whether the exact destination was reached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationSample {
    pub pose: AnimationPose,
    pub finished: bool,
}

/// One captured geometry/opacity motion law and transition group timestamp.
#[derive(Debug, Clone)]
struct Track {
    effect: AnimationEffect,
    group: u64,
    start_time: f64,
    start: [f64; 5],
    target: [f64; 5],
    velocity: [f64; 5],
    kind: AnimationKind,
    physical_scale: f64,
    instant: bool,
}

impl Track {
    fn sample(&self, now: f64) -> (AnimationSample, [f64; 5]) {
        let t = (now - self.start_time).max(0.0);
        let mut values = self.target;
        let mut velocity = [0.0; 5];
        let finished = if self.instant {
            true
        } else {
            match self.kind {
                AnimationKind::Easing { duration_ms, curve } => {
                    let duration = f64::from(duration_ms) / 1000.0;
                    if duration == 0.0 || t >= duration {
                        true
                    } else {
                        let p = t / duration;
                        let progress = match curve {
                            AnimationCurve::Linear => p,
                            AnimationCurve::EaseOutQuad => 1.0 - (1.0 - p).powi(2),
                            AnimationCurve::EaseOutCubic => 1.0 - (1.0 - p).powi(3),
                            AnimationCurve::EaseOutExpo => {
                                (1.0 - 2.0_f64.powf(-10.0 * p)) / (1.0 - 2.0_f64.powf(-10.0))
                            }
                        };
                        for (i, value) in values.iter_mut().enumerate() {
                            *value = self.start[i] + (self.target[i] - self.start[i]) * progress;
                        }
                        false
                    }
                }
                AnimationKind::Spring { stiffness } => {
                    if t >= MAX_SPRING_SECONDS {
                        true
                    } else {
                        let omega = stiffness.sqrt();
                        let decay = (-omega * t).exp();
                        for i in 0..5 {
                            let c1 = self.start[i] - self.target[i];
                            let c2 = self.velocity[i] + omega * c1;
                            values[i] = self.target[i] + (c1 + c2 * t) * decay;
                            velocity[i] = (c2 - omega * (c1 + c2 * t)) * decay;
                        }
                        // Test both corners: independent origin/size errors can add up.
                        let corner_errors = [
                            values[0] - self.target[0],
                            values[1] - self.target[1],
                            values[0] + values[2] - self.target[0] - self.target[2],
                            values[1] + values[3] - self.target[1] - self.target[3],
                        ];
                        let corner_velocity = [
                            velocity[0],
                            velocity[1],
                            velocity[0] + velocity[2],
                            velocity[1] + velocity[3],
                        ];
                        corner_errors
                            .iter()
                            .all(|v| v.abs() <= 0.25 / self.physical_scale)
                            && corner_velocity
                                .iter()
                                .all(|v| v.abs() <= 1.0 / self.physical_scale)
                            && (values[4] - self.target[4]).abs() <= 0.001
                            && velocity[4].abs() <= 0.001
                    }
                }
            }
        };
        if finished {
            values = self.target;
            velocity = [0.0; 5];
        } else if t == 0.0 {
            // Preserve the captured pose exactly, avoiding cancellation roundoff
            // at spring retarget boundaries.
            values = self.start;
            if matches!(self.kind, AnimationKind::Spring { .. }) {
                velocity = self.velocity;
            }
        }
        // Inherited spring velocity may overshoot; unsafe dimensions/opacity stop at bounds.
        for i in [2, 3] {
            if values[i] <= 0.0 {
                values[i] = f64::EPSILON;
                velocity[i] = 0.0;
            }
        }
        if !(0.0..=1.0).contains(&values[4]) {
            values[4] = values[4].clamp(0.0, 1.0);
            velocity[4] = 0.0;
        }
        (
            AnimationSample {
                pose: AnimationPose::from_values(values),
                finished,
            },
            velocity,
        )
    }
}

/// Bounded pose tracks, captured settings, and a common monotonic speed clock.
/// No operation changes desktop geometry, owns a surface, or allocates a GPU image.
#[derive(Debug, Clone)]
pub struct AnimationEngine {
    pub clock: AnimationClock,
    config: AnimationsConfig,
    tracks: BTreeMap<u64, Track>,
}

impl AnimationEngine {
    /// Construct a deterministic engine at an injected monotonic timestamp.
    pub fn new(config: AnimationsConfig, now: Duration) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            clock: AnimationClock::new(now, config.speed)?,
            config,
            tracks: BTreeMap::new(),
        })
    }

    /// Rebase speed atomically; disabling effects settles existing tracks exactly.
    /// Curve/duration changes are captured only by subsequently started tracks.
    pub fn apply_config(&mut self, config: AnimationsConfig, now: Duration) -> Result<(), String> {
        config.validate()?;
        self.clock.rebase(now, config.speed)?;
        for track in self.tracks.values_mut() {
            if !config.enabled || config.reduced_motion || !config.effect(track.effect).enabled {
                track.instant = true;
            }
        }
        self.config = config;
        Ok(())
    }

    /// Start/retarget a track. Related calls sharing group and now begin together.
    /// A full engine refuses a new ID; callers must present the destination instantly.
    pub fn retarget(
        &mut self,
        id: u64,
        group: u64,
        effect: AnimationEffect,
        from: AnimationPose,
        to: AnimationPose,
        now: Duration,
        physical_scale: f64,
    ) -> Result<(), String> {
        from.validate()?;
        to.validate()?;
        if !physical_scale.is_finite() || physical_scale <= 0.0 {
            return Err("animation physical scale must be positive and finite".into());
        }
        if !self.tracks.contains_key(&id) && self.tracks.len() >= MAX_ANIMATION_TRACKS {
            return Err("animation track limit reached; settle this effect instantly".into());
        }
        let time = self.clock.sample(now);
        let config = self.config.effect(effect);
        let (start, velocity) = self
            .tracks
            .get(&id)
            .map_or((from.values(), [0.0; 5]), |old| {
                let (sample, velocity) = old.sample(time);
                (
                    sample.pose.values(),
                    if matches!(old.kind, AnimationKind::Spring { .. })
                        && matches!(config.kind, AnimationKind::Spring { .. })
                    {
                        velocity
                    } else {
                        [0.0; 5]
                    },
                )
            });
        if let AnimationKind::Spring { stiffness } = config.kind {
            let omega = stiffness.sqrt();
            for i in 0..5 {
                let c1 = start[i] - to.values()[i];
                let c2 = velocity[i] + omega * c1;
                // Bound the un-decayed polynomial and derivative over the entire
                // settling interval, including inherited velocities after reversal.
                let envelope = c1.abs() + c2.abs() * MAX_SPRING_SECONDS;
                if !(to.values()[i].abs() + envelope).is_finite()
                    || !(c2.abs() + omega * envelope).is_finite()
                {
                    return Err("animation spring coefficients would overflow; settle this effect instantly".into());
                }
            }
        }
        self.tracks.insert(
            id,
            Track {
                effect,
                group,
                start_time: time,
                start,
                target: to.values(),
                velocity,
                kind: config.kind,
                physical_scale,
                instant: !self.config.enabled || self.config.reduced_motion || !config.enabled,
            },
        );
        Ok(())
    }

    /// Sample without advancing a shared clock or deleting completed tracks.
    pub fn sample(&self, id: u64, now: Duration) -> Option<AnimationSample> {
        self.tracks
            .get(&id)
            .map(|track| track.sample(self.clock.sample(now)).0)
    }

    /// Return the caller-supplied transition group ID for an existing track.
    pub fn group(&self, id: u64) -> Option<u64> {
        self.tracks.get(&id).map(|t| t.group)
    }

    /// Immediately expose the exact destination until the track is retired.
    pub fn settle(&mut self, id: u64) {
        if let Some(t) = self.tracks.get_mut(&id) {
            t.instant = true;
        }
    }

    /// Release a completed/cancelled track; presentation owners decide safe retirement.
    pub fn remove(&mut self, id: u64) {
        self.tracks.remove(&id);
    }

    /// Number of retained tracks, including completed tracks awaiting retirement.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether there are no retained tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}
