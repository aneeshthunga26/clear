//! Pointer-anchored overview pickup and spatial miniature sizing.

use super::{
    overview::{OverviewLayout, fit, miniature_frame},
    state::Compositor,
};
use crate::{
    config::AnimationEffect,
    core::{OutputId, Rect, WindowId, WorkspaceId},
    runtime::{
        OverviewTarget,
        animation::{AnimationEngine, AnimationPose, AnimationRect},
    },
};
use smithay::utils::{Logical, Point};
use std::time::Duration;

/// A compositor-owned pointer gesture; policy changes only on a valid drop.
pub(super) struct OverviewDrag {
    pub window: WindowId,
    pub workspace: WorkspaceId,
    pub output: OutputId,
    pub output_rect: Rect,
    pub origin: Point<f64, Logical>,
    pub position: Point<f64, Logical>,
    pub preview: Rect,
    pub active: bool,
    pub destination: Option<WorkspaceId>,
    pub motion: DragMotion,
}

/// One gesture owns at most one shared engine track and its submitted geometry.
#[derive(Default)]
pub(super) struct DragMotion {
    track: Option<u64>,
    target: Option<Rect>,
    sampled: Option<Rect>,
    sampled_pose: Option<AnimationPose>,
    presented_pose: Option<AnimationPose>,
    pickup_offset: Option<(f64, f64)>,
    presented_time: Option<Duration>,
    presented_phase: f64,
}

impl OverviewDrag {
    /// Capture the last shown card when the threshold is crossed, preserving the grab fraction.
    pub fn begin_pickup(&mut self, preview: Rect) {
        let rx = (self.origin.x - f64::from(self.preview.x)) / f64::from(self.preview.width.max(1));
        let ry =
            (self.origin.y - f64::from(self.preview.y)) / f64::from(self.preview.height.max(1));
        self.origin = (
            f64::from(preview.x) + rx * f64::from(preview.width),
            f64::from(preview.y) + ry * f64::from(preview.height),
        )
            .into();
        self.preview = preview;
        self.active = true;
    }

    pub fn ghost(&self, output: Rect) -> Option<Rect> {
        if !self.active {
            return None;
        }
        if output.is_empty() {
            return None;
        }
        Some(
            self.motion
                .sampled
                .unwrap_or(self.preview)
                .clamped_to(output),
        )
    }

    fn target(&self, layout: &OverviewLayout, frame: Rect, home: Rect) -> Rect {
        let output = layout.output;
        let normal = fit(
            self.preview,
            Rect::new(0, 0, output.width.min(240), output.height.min(160)),
        );
        let nearest = layout
            .items
            .iter()
            .filter_map(|item| {
                let OverviewTarget::Workspace(_) = item.target else {
                    return None;
                };
                let preview = item.preview?;
                let dx = (f64::from(item.rect.x) - self.position.x)
                    .max(self.position.x - f64::from(item.rect.right()))
                    .max(0.0);
                let dy = (f64::from(item.rect.y) - self.position.y)
                    .max(self.position.y - f64::from(item.rect.bottom()))
                    .max(0.0);
                Some((dx.hypot(dy), preview))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let (width, height) = nearest.map_or((normal.width, normal.height), |(distance, tile)| {
            let miniature = fit(
                self.preview,
                miniature_frame(frame, home, tile).unwrap_or(tile),
            );
            let radius = (f64::from(tile.height) * 2.0).clamp(64.0, 160.0);
            let proximity = 1.0 - (distance / radius).clamp(0.0, 1.0);
            let weight = proximity * proximity * (3.0 - 2.0 * proximity);
            let blend = |large: i32, small: i32| {
                (f64::from(large) + f64::from(small.min(large) - large) * weight)
                    .round()
                    .max(1.0) as i32
            };
            (
                blend(normal.width, miniature.width),
                blend(normal.height, miniature.height),
            )
        });
        self.anchored_rect(drag_pose(Rect::new(0, 0, width, height)))
    }

    fn anchored_rect(&self, pose: AnimationPose) -> Rect {
        let rx = ((self.origin.x - f64::from(self.preview.x))
            / f64::from(self.preview.width.max(1)))
        .clamp(0.0, 1.0);
        let ry = ((self.origin.y - f64::from(self.preview.y))
            / f64::from(self.preview.height.max(1)))
        .clamp(0.0, 1.0);
        let width = pose.rect.width.round().max(1.0) as i32;
        let height = pose.rect.height.round().max(1.0) as i32;
        Rect::new(
            (self.position.x + pose.rect.x - rx * f64::from(width)).round() as i32,
            (self.position.y + pose.rect.y - ry * f64::from(height)).round() as i32,
            width,
            height,
        )
        .clamped_to(self.output_rect)
    }

    fn sample(&mut self, engine: &mut AnimationEngine, now: Duration, target: Rect) {
        // Position follows input on every host frame, including frames between
        // capped animation samples. Only scale and the one-time pickup offset ease.
        let target = Rect::new(0, 0, target.width, target.height);
        let config = engine.config();
        if !config.enabled || config.reduced_motion || !config.overview.enabled {
            self.clear_track(engine);
            self.motion.target = Some(target);
            self.motion.sampled_pose = Some(drag_pose(target));
            self.motion.sampled = Some(self.anchored_rect(drag_pose(target)));
            return;
        }
        if self.motion.target != Some(target) {
            let fresh = self.motion.track.is_none();
            let track = *self.motion.track.get_or_insert_with(|| {
                (1..=u64::MAX)
                    .rev()
                    .find(|id| !engine.contains(*id))
                    .expect("bounded engine has a free identity")
            });
            let (dx, dy) = *self.motion.pickup_offset.get_or_insert((
                self.origin.x - self.position.x,
                self.origin.y - self.position.y,
            ));
            let from = self.motion.presented_pose.unwrap_or_else(|| {
                let mut pose = drag_pose(self.preview);
                pose.rect.x = dx;
                pose.rect.y = dy;
                pose
            });
            // Start at the preceding submitted frame, so a target changing every
            // refresh still advances by this frame's elapsed time instead of freezing.
            let retarget_time = self.motion.presented_time.unwrap_or(now);
            let shown_phase = if fresh && self.motion.presented_pose.is_none() {
                engine.clock.sample(retarget_time)
            } else {
                self.motion.presented_phase
            };
            if engine
                .retarget_presented(
                    track,
                    track,
                    AnimationEffect::Overview,
                    from,
                    drag_pose(target),
                    retarget_time,
                    1.0,
                    shown_phase,
                )
                .is_err()
            {
                self.clear_track(engine);
            }
            self.motion.target = Some(target);
        }
        let pose = self
            .motion
            .track
            .and_then(|id| engine.sample(id, now))
            .map_or(drag_pose(target), |sample| sample.pose);
        self.motion.sampled_pose = Some(pose);
        self.motion.sampled = Some(self.anchored_rect(pose));
    }

    fn mark_presented(&mut self, engine: &AnimationEngine, now: Duration) {
        self.motion.presented_pose = self.motion.sampled_pose;
        self.motion.presented_time = Some(now);
        self.motion.presented_phase = engine.clock.sample(now);
    }

    fn clear_track(&mut self, engine: &mut AnimationEngine) {
        if let Some(track) = self.motion.track.take() {
            engine.remove(track);
        }
    }
}

fn drag_pose(rect: Rect) -> AnimationPose {
    AnimationPose {
        rect: AnimationRect {
            x: f64::from(rect.x),
            y: f64::from(rect.y),
            width: f64::from(rect.width),
            height: f64::from(rect.height),
        },
        opacity: 1.0,
    }
}

impl Compositor {
    /// Drop/cancellation removes the gesture's engine track as well as its input state.
    pub fn take_overview_drag(&mut self) -> Option<OverviewDrag> {
        let mut drag = self.overview_drag.take()?;
        drag.clear_track(&mut self.runtime.animations);
        Some(drag)
    }

    /// Sample pickup and proximity sizing at the same capped time as the overview scene.
    pub fn sample_overview_drag(&mut self) {
        let Some(layout) = self
            .overview_sampled_layout()
            .or_else(|| self.overview_layout())
        else {
            return;
        };
        let Some(drag) = self.overview_drag.as_ref().filter(|drag| drag.active) else {
            return;
        };
        let Some(window) = self.runtime.desktop.window(drag.window) else {
            return;
        };
        let frame = self
            .windows
            .get(&drag.window)
            .and_then(|entry| entry.last_frame.get())
            .unwrap_or(window.floating_rect);
        let home = self
            .outputs
            .iter()
            .find(|output| Some(output.id) == window.output)
            .map_or(layout.output, |output| output.rect);
        let target = drag.target(&layout, frame, home);
        let now = self
            .runtime
            .animation_time_at(self.start + self.animation_sample_time);
        self.overview_drag.as_mut().expect("active drag").sample(
            &mut self.runtime.animations,
            now,
            target,
        );
    }

    pub fn mark_overview_drag_presented(&mut self) {
        let now = self
            .runtime
            .animation_time_at(self.start + self.animation_sample_time);
        if let Some(drag) = self.overview_drag.as_mut().filter(|drag| drag.active) {
            drag.mark_presented(&self.runtime.animations, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::overview::OverviewItem;
    use super::*;
    use crate::config::{AnimationCurve, AnimationKind, AnimationsConfig};

    fn drag() -> OverviewDrag {
        OverviewDrag {
            window: WindowId(1),
            workspace: WorkspaceId(1),
            output: OutputId(1),
            output_rect: Rect::new(319, 11, 641, 481),
            origin: (510.0, 270.0).into(),
            position: (700.0, 280.0).into(),
            preview: Rect::new(350, 150, 320, 240),
            active: true,
            destination: None,
            motion: DragMotion::default(),
        }
    }
    fn layout(output: Rect) -> OverviewLayout {
        OverviewLayout {
            output,
            canvas: output,
            columns: 1,
            items: Vec::new(),
            page: 0,
            pages: 1,
        }
    }
    fn engine() -> AnimationEngine {
        let mut config = AnimationsConfig {
            enabled: true,
            ..Default::default()
        };
        config.overview.kind = AnimationKind::Easing {
            duration_ms: 200,
            curve: AnimationCurve::Linear,
        };
        AnimationEngine::new(config, Duration::ZERO).unwrap()
    }

    #[test]
    fn pickup_starts_at_the_card_and_retargets_from_the_submitted_ghost() {
        let mut drag = drag();
        let mut engine = engine();
        let target = drag.target(&layout(drag.output_rect), drag.preview, drag.output_rect);
        drag.sample(&mut engine, Duration::ZERO, target);
        assert_eq!(drag.ghost(drag.output_rect), Some(drag.preview));
        drag.mark_presented(&engine, Duration::ZERO);
        let middle_time = Duration::from_millis(100);
        drag.sample(&mut engine, middle_time, target);
        let middle = drag.ghost(drag.output_rect).unwrap();
        assert!(middle.width > target.width && middle.width < drag.preview.width);
        assert!(middle.x > drag.preview.x && middle.x < target.x);
        drag.mark_presented(&engine, middle_time);
        drag.position.x += 40.0;
        let changed = Rect::new(0, 0, 120, 90);
        drag.sample(&mut engine, middle_time, changed);
        assert_eq!(
            drag.ghost(drag.output_rect),
            Some(Rect::new(
                middle.x + 40,
                middle.y,
                middle.width,
                middle.height
            )),
            "scale retargets preserve the shown shape while pointer motion is immediate"
        );
        drag.sample(&mut engine, Duration::from_millis(300), changed);
        assert_eq!(
            drag.ghost(drag.output_rect),
            Some(drag.anchored_rect(drag_pose(changed)))
        );
        assert_eq!(engine.len(), 1);
        drag.clear_track(&mut engine);
        assert_eq!(engine.len(), 0);
    }

    #[test]
    fn pointer_translation_is_immediate_during_pickup_scaling_and_capped_samples() {
        let mut drag = drag();
        drag.output_rect = Rect::new(0, 0, 2000, 1600);
        let mut engine = engine();
        let layout = layout(drag.output_rect);
        let target = drag.target(&layout, drag.preview, layout.output);
        drag.sample(&mut engine, Duration::ZERO, target);
        drag.mark_presented(&engine, Duration::ZERO);
        let pickup_time = Duration::from_millis(50);
        drag.sample(&mut engine, pickup_time, target);
        let mut previous = drag.ghost(layout.output).unwrap();
        let pose = drag.motion.sampled_pose;
        for dx in [40.0, -70.0, 100.0] {
            drag.position.x += dx;
            let target = drag.target(&layout, drag.preview, layout.output);
            // Same animation timestamp models redraws between capped samples.
            drag.sample(&mut engine, pickup_time, target);
            let shown = drag.ghost(layout.output).unwrap();
            assert_eq!(shown.x - previous.x, dx as i32);
            assert_eq!(shown.y, previous.y);
            assert_eq!(drag.motion.sampled_pose, pose);
            previous = shown;
        }
        let settled_time = Duration::from_millis(200);
        drag.sample(&mut engine, settled_time, target);
        drag.mark_presented(&engine, settled_time);
        let small = Rect::new(0, 0, 80, 60);
        drag.sample(&mut engine, settled_time, small);
        let scaling_time = Duration::from_millis(300);
        drag.sample(&mut engine, scaling_time, small);
        previous = drag.ghost(layout.output).unwrap();
        assert!(previous.width > small.width && previous.width < target.width);
        for dx in [60.0, -40.0] {
            drag.position.x += dx;
            drag.sample(&mut engine, scaling_time, small);
            let shown = drag.ghost(layout.output).unwrap();
            assert_eq!(shown.x - previous.x, dx as i32);
            assert!(
                (f64::from(shown.x) + 0.5 * f64::from(shown.width) - drag.position.x).abs() <= 0.5
            );
            assert!(
                (f64::from(shown.y) + 0.5 * f64::from(shown.height) - drag.position.y).abs() <= 0.5
            );
            previous = shown;
        }
        assert_eq!(engine.len(), 1);
    }

    #[test]
    fn continuous_pointer_updates_advance_instead_of_restarting_at_zero() {
        let mut drag = drag();
        let mut engine = engine();
        let layout = layout(drag.output_rect);
        let target = drag.target(&layout, drag.preview, layout.output);
        drag.sample(&mut engine, Duration::ZERO, target);
        drag.mark_presented(&engine, Duration::ZERO);
        let mut previous_width = drag.preview.width;
        for frame in 1..=10 {
            drag.position.x += 4.0;
            let target = drag.target(&layout, drag.preview, layout.output);
            let now = Duration::from_millis(frame * 16);
            drag.sample(&mut engine, now, target);
            let shown = drag.ghost(layout.output).unwrap();
            assert!(
                shown.width < previous_width,
                "pickup must advance during pointer motion"
            );
            assert_eq!(engine.len(), 1);
            drag.mark_presented(&engine, now);
            previous_width = shown.width;
        }
        assert!(previous_width > target.width);
    }

    #[test]
    fn pickup_uses_the_latest_submitted_card_after_entry_moves_it() {
        let mut drag = drag();
        drag.active = false;
        drag.position = (600.0, 340.0).into();
        let latest_card = Rect::new(400, 200, 240, 180);
        drag.begin_pickup(latest_card);
        assert!(drag.active);
        assert_eq!(drag.ghost(drag.output_rect), Some(latest_card));
        assert_eq!(drag.origin, Point::from((520.0, 290.0)));
        let layout = layout(drag.output_rect);
        let target = drag.target(&layout, latest_card, layout.output);
        assert_eq!(target, Rect::new(494, 260, 213, 160));
    }

    #[test]
    fn approaching_desktops_shrinks_to_the_same_fitted_miniature_size() {
        let mut drag = drag();
        drag.output_rect = Rect::new(0, 0, 1024, 768);
        drag.preview = Rect::new(272, 224, 480, 360);
        drag.origin = (392.0, 494.0).into(); // Grab at one-quarter width, three-quarter height.
        let mut layout = layout(drag.output_rect);
        let tile = Rect::new(400, 24, 120, 72);
        layout.items.push(OverviewItem {
            target: OverviewTarget::Workspace(WorkspaceId(2)),
            rect: Rect::new(390, 14, 140, 110),
            preview: Some(tile),
        });
        let frame = Rect::new(100, 80, 800, 600);
        drag.position = (460.0, 300.0).into();
        let far = drag.target(&layout, frame, layout.output);
        drag.position.y = 196.0; // Half the 144-pixel approach band below the tile hit box.
        let halfway = drag.target(&layout, frame, layout.output);
        drag.position.y = 90.0;
        let near = drag.target(&layout, frame, layout.output);
        let miniature = fit(
            drag.preview,
            miniature_frame(frame, layout.output, tile).unwrap(),
        );
        assert_eq!(
            (near.width, near.height),
            (miniature.width, miniature.height)
        );
        assert!(near.width < halfway.width && halfway.width < far.width);
        assert!(near.height < halfway.height && halfway.height < far.height);
        assert!((f64::from(near.x) + 0.25 * f64::from(near.width) - drag.position.x).abs() <= 0.5);
        assert!((f64::from(near.y) + 0.75 * f64::from(near.height) - drag.position.y).abs() <= 0.5);
        drag.position.y = 300.0;
        assert_eq!(
            drag.target(&layout, frame, layout.output),
            far,
            "moving away restores the normal drag size"
        );
    }

    #[test]
    fn instant_motion_and_edge_clamps_keep_the_ghost_bounded() {
        let mut drag = drag();
        let mut engine = AnimationEngine::new(AnimationsConfig::default(), Duration::ZERO).unwrap();
        let layout = layout(drag.output_rect);
        for point in [(319.0, 11.0), (950.0, 490.0), (500.0, 240.0)] {
            drag.position = point.into();
            let target = drag.target(&layout, drag.preview, layout.output);
            drag.sample(&mut engine, Duration::ZERO, target);
            let ghost = drag.ghost(layout.output).unwrap();
            assert_eq!(ghost, target);
            assert_eq!(ghost.intersection(layout.output), Some(ghost));
            assert!(ghost.width <= 240 && ghost.height <= 160);
            assert!((ghost.width * 3 - ghost.height * 4).abs() <= 1);
        }
        assert_eq!(engine.len(), 0);
        drag.active = false;
        assert!(drag.ghost(layout.output).is_none());
    }

    #[test]
    fn speed_reload_and_reduced_motion_apply_to_an_active_pickup() {
        let mut drag = drag();
        let mut engine = engine();
        let target = drag.target(&layout(drag.output_rect), drag.preview, drag.output_rect);
        drag.sample(&mut engine, Duration::ZERO, target);
        drag.sample(&mut engine, Duration::from_millis(50), target);
        let before = drag.ghost(drag.output_rect).unwrap();
        drag.mark_presented(&engine, Duration::from_millis(50));
        let mut config = engine.config().clone();
        config.speed = 2.0;
        engine
            .apply_config(config.clone(), Duration::from_millis(50))
            .unwrap();
        drag.sample(&mut engine, Duration::from_millis(50), target);
        assert_eq!(drag.ghost(drag.output_rect), Some(before));
        drag.sample(&mut engine, Duration::from_millis(125), target);
        assert_eq!(drag.ghost(drag.output_rect), Some(target));
        config.reduced_motion = true;
        engine
            .apply_config(config, Duration::from_millis(125))
            .unwrap();
        drag.sample(&mut engine, Duration::from_millis(125), target);
        assert_eq!(engine.len(), 0);
        assert_eq!(drag.ghost(drag.output_rect), Some(target));
    }
}
