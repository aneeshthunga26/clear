//! Sampling on host redraw opportunities; this does not infer presentation feedback.

use crate::config::AnimationFrameRate;
use std::time::Duration;

/// Current host monitor metadata, or the explicitly nominal fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HostTiming {
    pub refresh_millihertz: u32,
    monitor: Option<u128>,
}

impl HostTiming {
    pub fn from_monitor(mode: Option<(u128, u32)>) -> Self {
        match mode.filter(|(_, rate)| *rate > 0 && *rate <= i32::MAX as u32) {
            Some((monitor, refresh_millihertz)) => Self {
                refresh_millihertz,
                monitor: Some(monitor),
            },
            None => Self {
                refresh_millihertz: 60_000,
                monitor: None,
            },
        }
    }

    pub fn source(self) -> &'static str {
        if self.monitor.is_some() {
            "host monitor metadata (no presentation feedback)"
        } else {
            "nominal 60 Hz estimate (host metadata unavailable)"
        }
    }
}

/// One captured redraw time and the most recently eligible animation sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameTiming {
    pub now: Duration,
    pub animation_sample: Duration,
}

/// Coalesces requests and selects fixed-rate samples without changing client pacing.
pub(super) struct FrameScheduler {
    host: HostTiming,
    rate: AnimationFrameRate,
    epoch: Duration,
    next_tick: u128,
    sample: Duration,
    redraw_requested: bool,
}

impl FrameScheduler {
    pub fn new(host: HostTiming, rate: AnimationFrameRate, now: Duration) -> Self {
        Self {
            host,
            rate,
            epoch: now,
            next_tick: 0,
            sample: now,
            redraw_requested: false,
        }
    }

    /// Reset the sampling phase on a monitor/mode/source or configured rate change.
    pub fn update(&mut self, host: HostTiming, rate: AnimationFrameRate, now: Duration) {
        if self.host != host || self.rate != rate {
            self.host = host;
            self.rate = rate;
            self.epoch = now;
            self.next_tick = 0;
        }
    }

    /// Returns true only when a new host redraw request is needed.
    pub fn request_redraw(&mut self) -> bool {
        if self.redraw_requested {
            return false;
        }
        self.redraw_requested = true;
        true
    }

    /// Capture once per host opportunity; late frames skip expired deadlines.
    pub fn redraw(&mut self, now: Duration) -> FrameTiming {
        self.redraw_requested = false;
        match self.rate {
            AnimationFrameRate::RefreshRate => self.sample = now,
            AnimationFrameRate::Fixed(fps) => {
                let rate = (u32::from(fps) * 1000).min(self.host.refresh_millihertz);
                // Select the host opportunity nearest the absolute cap deadline.
                // Half an estimated host interval preserves 2/3 or 1/2 patterns
                // on 144 Hz, instead of rounding all intervals to a divisor.
                let elapsed = now.saturating_sub(self.epoch).as_nanos();
                let midpoint =
                    elapsed + 1_000_000_000_000 / u128::from(self.host.refresh_millihertz) / 2;
                let reached = midpoint * u128::from(rate) / 1_000_000_000_000;
                if self.next_tick <= reached {
                    self.sample = now;
                    self.next_tick = reached + 1;
                }
            }
        }
        FrameTiming {
            now,
            animation_sample: self.sample,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_tick(tick: u64, hz: u32) -> Duration {
        Duration::from_nanos(tick * 1_000_000_000 / u64::from(hz))
    }

    #[test]
    fn fixed_caps_keep_absolute_phase_on_144_hz() {
        for (fps, expected_gaps) in [(30, [4, 5]), (60, [2, 3]), (120, [1, 2])] {
            let mut scheduler = FrameScheduler::new(
                HostTiming::from_monitor(Some((1, 144_000))),
                AnimationFrameRate::Fixed(fps),
                Duration::ZERO,
            );
            let mut selected = Vec::new();
            for tick in 0..=1440 {
                let now = at_tick(tick, 144);
                if scheduler.redraw(now).animation_sample == now {
                    selected.push(tick);
                }
            }
            assert_eq!(selected.len(), usize::from(fps) * 10 + 1);
            let gaps: std::collections::BTreeSet<_> =
                selected.windows(2).map(|pair| pair[1] - pair[0]).collect();
            assert_eq!(gaps, expected_gaps.into_iter().collect());
        }
    }

    #[test]
    fn missed_frames_do_not_replay_or_lower_the_target() {
        let mut scheduler = FrameScheduler::new(
            HostTiming::from_monitor(Some((1, 60_000))),
            AnimationFrameRate::Fixed(60),
            Duration::ZERO,
        );
        scheduler.redraw(Duration::ZERO);
        let late = Duration::from_millis(950);
        assert_eq!(scheduler.redraw(late).animation_sample, late);
        // Not one frame per 33 ms after a late render: the next 60 Hz opportunity advances.
        let next = at_tick(58, 60);
        assert_eq!(scheduler.redraw(next).animation_sample, next);
        assert_eq!(scheduler.rate, AnimationFrameRate::Fixed(60));
    }

    #[test]
    fn a_59_fps_workload_keeps_the_60_hz_sampling_target() {
        let mut scheduler = FrameScheduler::new(
            HostTiming::from_monitor(Some((1, 60_000))),
            AnimationFrameRate::Fixed(60),
            Duration::ZERO,
        );
        for tick in 0..=590 {
            let now = at_tick(tick, 59);
            assert_eq!(scheduler.redraw(now).animation_sample, now);
        }
        assert_eq!(scheduler.rate, AnimationFrameRate::Fixed(60));
    }

    #[test]
    fn callbacks_can_redraw_between_animation_samples() {
        let mut scheduler = FrameScheduler::new(
            HostTiming::from_monitor(Some((1, 120_000))),
            AnimationFrameRate::Fixed(30),
            Duration::ZERO,
        );
        scheduler.redraw(Duration::ZERO);
        let frame = scheduler.redraw(at_tick(1, 120));
        assert_eq!(frame.now, at_tick(1, 120));
        assert_eq!(frame.animation_sample, Duration::ZERO);
    }

    #[test]
    fn higher_cap_is_bounded_by_the_host_rate() {
        let mut scheduler = FrameScheduler::new(
            HostTiming::from_monitor(Some((1, 60_000))),
            AnimationFrameRate::Fixed(120),
            Duration::ZERO,
        );
        for tick in 0..=60 {
            let now = at_tick(tick, 60);
            assert_eq!(scheduler.redraw(now).animation_sample, now);
        }
    }

    #[test]
    fn source_and_config_changes_reset_phase_without_stale_samples() {
        let estimated = HostTiming::from_monitor(None);
        assert_eq!(estimated.refresh_millihertz, 60_000);
        assert!(estimated.source().contains("estimate"));
        let mut scheduler =
            FrameScheduler::new(estimated, AnimationFrameRate::Fixed(30), Duration::ZERO);
        scheduler.redraw(Duration::ZERO);
        let now = Duration::from_millis(7);
        scheduler.update(
            HostTiming::from_monitor(Some((1, 60_000))),
            AnimationFrameRate::Fixed(30),
            now,
        );
        assert_eq!(scheduler.redraw(now).animation_sample, now);
        let now = Duration::from_millis(10);
        scheduler.update(
            HostTiming::from_monitor(Some((2, 60_000))),
            AnimationFrameRate::RefreshRate,
            now,
        );
        assert_eq!(scheduler.redraw(now).animation_sample, now);
    }

    #[test]
    fn redraw_requests_coalesce_until_the_host_opportunity() {
        let mut scheduler = FrameScheduler::new(
            HostTiming::from_monitor(None),
            AnimationFrameRate::RefreshRate,
            Duration::ZERO,
        );
        assert!(scheduler.request_redraw());
        assert!(!scheduler.request_redraw());
        scheduler.redraw(Duration::ZERO);
        assert!(scheduler.request_redraw());
        assert!(!scheduler.request_redraw());
    }
}
