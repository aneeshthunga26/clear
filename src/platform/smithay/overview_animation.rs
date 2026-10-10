//! Overview presentation state; desktop policy and renderer textures stay elsewhere.

use super::{overview::OverviewLayout, state::Compositor};
use crate::{
    config::AnimationEffect,
    core::{Rect, WindowId},
    runtime::{
        OverviewSession, OverviewTarget,
        animation::{AnimationPose, AnimationRect},
    },
};
use std::collections::BTreeMap;

/// One bounded transition retains only IDs/layout and at most twelve desktop poses.
#[derive(Default)]
pub(super) struct OverviewAnimation {
    session: Option<OverviewSession>,
    target: Option<OverviewLayout>,
    desktop: BTreeMap<WindowId, Rect>,
    transition_from: Option<OverviewLayout>,
    transition_start: f64,
    from_opacities: BTreeMap<WindowId, f32>,
    track: Option<u64>,
    open: bool,
    sampled: Option<OverviewLayout>,
    displayed: Option<OverviewLayout>,
    progress: f64,
    displayed_progress: f64,
    displayed_phase: f64,
    sampled_revision: Option<u64>,
    finished: bool,
}

impl OverviewAnimation {
    fn window_opacity_at(&self, id: WindowId, progress: f64) -> f32 {
        let target = if self.open || self.desktop.contains_key(&id) {
            1.0
        } else {
            0.0
        };
        let from = self.from_opacities.get(&id).copied().unwrap_or(target);
        let fraction = transition_fraction(self.open, progress, self.transition_start) as f32;
        from + (target - from) * fraction
    }
    fn closing_frame_complete(&self, current_revision: Option<u64>) -> bool {
        !self.open && self.finished && current_revision == self.sampled_revision
    }
}

impl Compositor {
    /// Include the retained closing scene in render and protocol input ownership.
    pub fn overview_present(&self) -> bool {
        self.runtime.overview.is_some() || self.overview_animation.session.is_some()
    }
    pub fn overview_render_session(&self) -> Option<&OverviewSession> {
        self.runtime
            .overview
            .as_ref()
            .or(self.overview_animation.session.as_ref())
    }
    /// Last submitted geometry drives pointer discovery; a prediction cannot change hits.
    pub fn overview_displayed_layout(&self) -> Option<OverviewLayout> {
        self.overview_animation.displayed.clone()
    }
    pub fn overview_sampled_layout(&self) -> Option<OverviewLayout> {
        self.overview_animation.sampled.clone()
    }
    pub fn overview_progress(&self) -> f64 {
        if self.overview_animation.session.is_some() {
            self.overview_animation.progress
        } else {
            1.0
        }
    }
    pub fn overview_window_opacity(&self, id: WindowId) -> f32 {
        self.overview_animation
            .window_opacity_at(id, self.overview_progress())
    }
    fn overview_desktop_frames(
        &self,
        layout: &OverviewLayout,
        presented: bool,
    ) -> BTreeMap<WindowId, Rect> {
        layout
            .items
            .iter()
            .filter_map(|item| {
                let OverviewTarget::Window(id) = item.target else {
                    return None;
                };
                let placement = self.placements.iter().find(|p| p.window == id)?;
                if presented && let Some(visual) = self.window_animations.presented_visual(id) {
                    return Some((id, rounded_destination(visual.destination())));
                }
                let entry = self.windows.get(&id).filter(|e| e.mapped)?;
                let size = entry.window.geometry().size;
                let frame = Rect::new(
                    placement.rect.x,
                    placement.rect.y,
                    size.w.max(1),
                    size.h.max(1).saturating_add(if entry.uses_ssd() {
                        self.runtime.config.theme.titlebar.height
                    } else {
                        0
                    }),
                );
                Some((
                    id,
                    entry.outline(frame, &self.runtime.config.theme).outer.rect,
                ))
            })
            .collect()
    }
    /// Reconcile once after placements: capture entry/exit endpoints without policy work per frame.
    pub fn sync_overview_animation(&mut self) {
        let config = self.runtime.animations.config();
        let motion = config.enabled && !config.reduced_motion && config.overview.enabled;
        if !self.host_focused || self.outputs.is_empty() {
            self.clear_overview_animation();
            return;
        }
        let now = self.runtime.animation_time_at(self.start + self.frame_time);
        let session = self.runtime.overview.clone();
        if session.is_none()
            && self.overview_animation.session.as_ref().is_some_and(|old| {
                self.outputs
                    .iter()
                    .find(|output| output.id == old.output)
                    .is_none_or(|output| {
                        self.overview_animation
                            .target
                            .as_ref()
                            .is_none_or(|layout| layout.output != output.rect)
                    })
            })
        {
            self.clear_overview_animation();
            return;
        }
        if session.is_none() && (!motion || self.overview_animation.session.is_none()) {
            self.clear_overview_animation();
            return;
        }
        let layout = session
            .as_ref()
            .and_then(|s| self.overview_target_layout(s));
        if session.is_some() && layout.is_none() {
            self.clear_overview_animation();
            return;
        }
        let opening = session.is_some();
        let changed = opening != self.overview_animation.open;
        let new_scene = self.overview_animation.session.is_none();
        let mut geometry_reset = false;
        let shown_layout = self
            .overview_animation
            .displayed
            .clone()
            .or_else(|| self.overview_animation.transition_from.clone());
        let shown_opacities: BTreeMap<_, _> = shown_layout
            .iter()
            .flat_map(|layout| &layout.items)
            .filter_map(|item| match item.target {
                OverviewTarget::Window(id) => Some((
                    id,
                    self.overview_animation
                        .window_opacity_at(id, self.overview_animation.displayed_progress),
                )),
                _ => None,
            })
            .collect();
        if let (Some(session), Some(layout)) = (session, layout) {
            let topology_changed = self
                .overview_animation
                .target
                .as_ref()
                .is_some_and(|old| old.output != layout.output)
                || self
                    .overview_animation
                    .session
                    .as_ref()
                    .is_some_and(|old| old.output != session.output);
            if new_scene || topology_changed {
                geometry_reset = topology_changed;
                if topology_changed && let Some(track) = self.overview_animation.track.take() {
                    self.runtime.animations.remove(track);
                }
                self.overview_animation.desktop = self.overview_desktop_frames(&layout, true);
                self.overview_animation.displayed = None;
                self.overview_animation.displayed_progress =
                    if topology_changed { 1.0 } else { 0.0 };
                self.overview_animation.displayed_phase = self.runtime.animations.clock.sample(now);
            }
            self.overview_animation.session = Some(session);
            self.overview_animation.target = Some(layout);
        } else if changed {
            if let Some(layout) = self.overview_animation.target.as_ref() {
                self.overview_animation.desktop = self.overview_desktop_frames(layout, false);
            }
        }
        if changed || new_scene || geometry_reset {
            self.overview_animation.transition_start = self.overview_animation.displayed_progress;
            let target = self
                .overview_animation
                .target
                .as_ref()
                .expect("retained overview target");
            self.overview_animation.transition_from = Some(if geometry_reset {
                target.clone()
            } else {
                shown_layout.unwrap_or_else(|| {
                    transition_layout(target, &self.overview_animation.desktop, 0.0)
                })
            });
            self.overview_animation.from_opacities = if new_scene {
                target
                    .items
                    .iter()
                    .filter_map(|item| match item.target {
                        OverviewTarget::Window(id) => Some((
                            id,
                            if self.overview_animation.desktop.contains_key(&id) {
                                self.window_animations
                                    .presented_visual(id)
                                    .map_or(1.0, |v| v.sampled.pose.opacity as f32)
                            } else {
                                0.0
                            },
                        )),
                        _ => None,
                    })
                    .collect()
            } else {
                shown_opacities
            };
            let track = *self.overview_animation.track.get_or_insert_with(|| {
                (1..=u64::MAX)
                    .rev()
                    .find(|id| !self.runtime.animations.contains(*id))
                    .expect("bounded engine has a free identity")
            });
            let from = progress_pose(self.overview_animation.displayed_progress);
            let to = progress_pose(if opening { 1.0 } else { 0.0 });
            if self
                .runtime
                .animations
                .retarget_presented(
                    track,
                    track,
                    AnimationEffect::Overview,
                    from,
                    to,
                    now,
                    1.0,
                    self.overview_animation.displayed_phase,
                )
                .is_err()
            {
                self.overview_animation.progress = if opening { 1.0 } else { 0.0 };
                self.overview_animation.finished = true;
                self.overview_animation.track = None;
            }
        }
        self.overview_animation.open = opening;
        self.sample_overview_animation();
    }
    /// Sample at the backend's shared capped animation timestamp, before composition.
    pub fn sample_overview_animation(&mut self) {
        if self.overview_animation.session.is_none() {
            return;
        }
        let now = self
            .runtime
            .animation_time_at(self.start + self.animation_sample_time);
        if let Some(track) = self.overview_animation.track {
            if let Some(sample) = self.runtime.animations.sample(track, now) {
                self.overview_animation.progress = sample.pose.opacity;
                self.overview_animation.finished = sample.finished;
                self.overview_animation.sampled_revision = self.runtime.animations.revision(track);
            }
        }
        self.overview_animation.sampled = self.overview_animation.target.as_ref().map(|layout| {
            let target = if self.overview_animation.open {
                layout.clone()
            } else {
                transition_layout(layout, &self.overview_animation.desktop, 0.0)
            };
            let from = self
                .overview_animation
                .transition_from
                .as_ref()
                .unwrap_or(&target);
            blend_layouts(
                from,
                &target,
                transition_fraction(
                    self.overview_animation.open,
                    self.overview_animation.progress,
                    self.overview_animation.transition_start,
                ),
            )
        });
    }
    /// Commit only a submitted frame; retain closing input ownership through its terminal frame.
    pub fn mark_overview_presented(&mut self) {
        self.overview_animation.displayed = self.overview_animation.sampled.clone();
        self.overview_animation.displayed_progress = self.overview_animation.progress;
        let now = self
            .runtime
            .animation_time_at(self.start + self.animation_sample_time);
        self.overview_animation.displayed_phase = self.runtime.animations.clock.sample(now);
        let current = self
            .overview_animation
            .track
            .and_then(|id| self.runtime.animations.revision(id));
        if self.overview_animation.closing_frame_complete(current) {
            self.clear_overview_animation();
            self.dirty = true;
        }
    }
    /// Renderer/topology/host teardown releases the one engine track and retained geometry.
    pub fn clear_overview_animation(&mut self) {
        let was_present = self.overview_animation.session.is_some();
        if let Some(track) = self.overview_animation.track {
            self.runtime.animations.remove(track);
        }
        self.overview_animation = OverviewAnimation::default();
        if was_present {
            self.dirty = true;
        }
    }
}

fn progress_pose(progress: f64) -> AnimationPose {
    AnimationPose {
        rect: AnimationRect {
            x: progress * 4096.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        opacity: progress,
    }
}
fn rounded_destination(
    destination: smithay::utils::Rectangle<f64, smithay::utils::Logical>,
) -> Rect {
    Rect::new(
        destination.loc.x.round() as i32,
        destination.loc.y.round() as i32,
        destination.size.w.round().max(1.0) as i32,
        destination.size.h.round().max(1.0) as i32,
    )
}
fn interpolate(from: Rect, to: Rect, progress: f64) -> Rect {
    let p = progress.clamp(0.0, 1.0);
    let blend = |a: i32, b: i32| (f64::from(a) + (f64::from(b) - f64::from(a)) * p).round() as i32;
    Rect::new(
        blend(from.x, to.x),
        blend(from.y, to.y),
        blend(from.width, to.width).max(1),
        blend(from.height, to.height).max(1),
    )
}

fn transition_fraction(open: bool, progress: f64, start: f64) -> f64 {
    if open {
        if start >= 1.0 {
            1.0
        } else {
            ((progress - start) / (1.0 - start)).clamp(0.0, 1.0)
        }
    } else if start <= 0.0 {
        1.0
    } else {
        ((start - progress) / start).clamp(0.0, 1.0)
    }
}

fn blend_layouts(from: &OverviewLayout, target: &OverviewLayout, progress: f64) -> OverviewLayout {
    let mut layout = target.clone();
    layout.canvas = interpolate(from.canvas, target.canvas, progress);
    for item in &mut layout.items {
        if let OverviewTarget::Window(_) = item.target
            && let Some(old) = from.items.iter().find(|old| old.target == item.target)
        {
            item.preview = old
                .preview
                .zip(item.preview)
                .map(|(old, target)| interpolate(old, target, progress));
            item.rect = interpolate(old.rect, item.rect, progress)
                .intersection(target.output)
                .unwrap_or(Rect::new(target.output.x, target.output.y, 0, 0));
        }
    }
    layout
}
fn transition_layout(
    target: &OverviewLayout,
    desktop: &BTreeMap<WindowId, Rect>,
    progress: f64,
) -> OverviewLayout {
    let mut layout = target.clone();
    layout.canvas = interpolate(target.output, target.canvas, progress);
    for item in &mut layout.items {
        if let OverviewTarget::Window(id) = item.target {
            if let Some(dest) = item.preview {
                if let Some(from) = desktop.get(&id) {
                    let preview = interpolate(*from, dest, progress);
                    item.preview = Some(preview);
                    item.rect = Rect::new(
                        preview.x,
                        preview.y,
                        preview.width,
                        preview.height + (40.0 * progress).round() as i32,
                    )
                    .intersection(target.output)
                    .unwrap_or(Rect::new(
                        target.output.x,
                        target.output.y,
                        0,
                        0,
                    ));
                }
            }
        }
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AnimationsConfig, runtime::animation::AnimationEngine};
    use std::time::Duration;
    #[test]
    fn entry_midpoint_and_exit_geometry_share_the_same_motion() {
        let mut desktop = crate::core::Desktop::new();
        desktop.add_output(
            crate::core::OutputId(1),
            "left".into(),
            Rect::new(319, 11, 641, 481),
        );
        desktop.add_window(WindowId(1), "app".into(), "app".into());
        let session = OverviewSession::new(&desktop).unwrap();
        let target = OverviewLayout::new(&session, Rect::new(319, 11, 641, 481));
        let from = Rect::new(330, 35, 600, 400);
        let endpoints = BTreeMap::from([(WindowId(1), from)]);
        let preview = |layout: OverviewLayout| {
            layout
                .items
                .iter()
                .find(|i| i.target == OverviewTarget::Window(WindowId(1)))
                .unwrap()
                .preview
                .unwrap()
        };
        assert_eq!(preview(transition_layout(&target, &endpoints, 0.0)), from);
        assert_eq!(
            preview(transition_layout(&target, &endpoints, 1.0)),
            preview(target.clone())
        );
        assert_eq!(
            transition_layout(&target, &endpoints, 0.0).canvas,
            target.output
        );
        let middle = preview(transition_layout(&target, &endpoints, 0.5));
        assert!(middle.width < from.width && middle.width > preview(target).width);
    }
    #[test]
    fn reverse_starts_from_submitted_phase_and_speed_reload_preserves_pose() {
        let mut config = AnimationsConfig::default();
        config.enabled = true;
        let mut engine = AnimationEngine::new(config.clone(), Duration::ZERO).unwrap();
        engine
            .retarget(
                1,
                1,
                AnimationEffect::Overview,
                progress_pose(0.0),
                progress_pose(1.0),
                Duration::ZERO,
                1.0,
            )
            .unwrap();
        let shown = Duration::from_millis(80);
        let sample = engine.sample(1, shown).unwrap();
        let phase = engine.clock.sample(shown);
        let now = Duration::from_millis(100);
        config.speed = 2.0;
        engine.apply_config(config, now).unwrap();
        engine
            .retarget_presented(
                1,
                1,
                AnimationEffect::Overview,
                sample.pose,
                progress_pose(0.0),
                now,
                1.0,
                phase,
            )
            .unwrap();
        assert_eq!(engine.sample(1, now).unwrap().pose, sample.pose);
        assert_eq!(
            engine
                .sample(1, Duration::from_secs(3))
                .unwrap()
                .pose
                .opacity,
            0.0
        );
    }

    #[test]
    fn closing_ownership_retires_only_after_current_terminal_frame_is_submitted() {
        let mut state = OverviewAnimation {
            open: false,
            finished: false,
            sampled_revision: Some(7),
            ..Default::default()
        };
        assert!(!state.closing_frame_complete(Some(7)));
        state.finished = true;
        assert!(
            !state.closing_frame_complete(Some(8)),
            "retarget makes sampled terminal pose stale"
        );
        assert!(state.closing_frame_complete(Some(7)));
        state.open = true;
        assert!(
            !state.closing_frame_complete(Some(7)),
            "open terminal poses retain session"
        );
    }

    #[test]
    fn interrupted_entry_activation_rebases_cards_and_wallpaper_from_displayed_geometry() {
        let mut desktop = crate::core::Desktop::new();
        desktop.add_output(
            crate::core::OutputId(1),
            "left".into(),
            Rect::new(0, 0, 1024, 768),
        );
        desktop.add_window(WindowId(1), "app".into(), "app".into());
        let session = OverviewSession::new(&desktop).unwrap();
        let target = OverviewLayout::new(&session, Rect::new(0, 0, 1024, 768));
        let old = BTreeMap::from([(WindowId(1), Rect::new(20, 40, 800, 600))]);
        let new = BTreeMap::from([(WindowId(1), Rect::new(320, 40, 600, 500))]);
        let shown = transition_layout(&target, &old, 0.4);
        let closing_target = transition_layout(&target, &new, 0.0);
        let first = blend_layouts(
            &shown,
            &closing_target,
            transition_fraction(false, 0.4, 0.4),
        );
        let last = blend_layouts(
            &shown,
            &closing_target,
            transition_fraction(false, 0.0, 0.4),
        );
        let preview = |layout: &OverviewLayout| {
            layout
                .items
                .iter()
                .find(|item| item.target == OverviewTarget::Window(WindowId(1)))
                .unwrap()
                .preview
        };
        assert_eq!(
            preview(&first),
            preview(&shown),
            "new desktop endpoint must not change first reverse frame"
        );
        assert_eq!(first.canvas, shown.canvas);
        assert_eq!(preview(&last), Some(new[&WindowId(1)]));
        assert_eq!(last.canvas, target.output);
        let halfway = blend_layouts(
            &shown,
            &closing_target,
            transition_fraction(false, 0.2, 0.4),
        );
        assert_eq!(
            preview(&halfway),
            Some(interpolate(
                preview(&shown).unwrap(),
                new[&WindowId(1)],
                0.5
            ))
        );
    }

    #[test]
    fn presented_endpoint_retains_outer_border_geometry_and_rounds_once() {
        let outer = smithay::utils::Rectangle::new((98.2, 47.6).into(), (207.6, 107.6).into());
        assert_eq!(rounded_destination(outer), Rect::new(98, 48, 208, 108));
        assert_ne!(rounded_destination(outer), Rect::new(100, 50, 200, 100));
    }
}
