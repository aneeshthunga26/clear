//! Bounded owned window images bridge pure presentation tracks to the nested scene.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

use super::{
    rounded::RoundedShaders,
    state::Compositor,
    titlebar::{TitlebarCache, content_rect},
    window_image::{
        MAX_IMAGE_EDGE, PreparedWindowImage, WindowImage, WindowImageComposer, WindowImageIdentity,
    },
};
use crate::{
    core::{OutputId, Rect, WindowId, WindowRole, WorkspaceId},
    decoration::{Theme, TitlebarTheme},
    runtime::presentation::{
        GroupState, OutputState, PresentationFrame, PresentationPlanner, PresentationSnapshot,
        PresentationSource, RetainedId, SampledWindow, TransitionCause, WindowKey, WindowState,
        WorkspaceMotion,
    },
};
use smithay::{
    backend::renderer::gles::{GlesError, GlesRenderer},
    utils::{Logical, Rectangle},
};

const MAX_IMAGES: usize = 128;
const MAX_BYTES: usize = 64 * 1024 * 1024;

struct CachedImage {
    image: WindowImage,
    identity: WindowImageIdentity,
    content: Rect,
    titlebar: TitlebarTheme,
    ssd: bool,
}

/// One committed body/SSD image transformed through an unrounded presentation frame.
#[derive(Clone)]
pub(super) struct AnimatedWindow {
    pub sampled: SampledWindow,
    pub image: WindowImage,
    pub committed_geometry_origin: (i32, i32),
    pub committed_content: Rect,
    pub committed_titlebar: TitlebarTheme,
    pub committed_ssd: bool,
    pub fullscreen_priority: bool,
    pub clip_outputs: Vec<Rect>,
    // Keep ownership identity for budget accounting and exact submitted-image retention.
    cached: Arc<CachedImage>,
}

impl AnimatedWindow {
    /// The complete original border/source is scaled relative to the committed inner frame.
    pub fn destination(&self) -> Rectangle<f64, Logical> {
        image_destination(self.image.source, self.image.frame, self.sampled.pose.rect)
    }
}

/// Cache ownership is local to this adapter; no retained image can own a client handle.
#[derive(Default)]
pub(super) struct WindowAnimations {
    planner: PresentationPlanner,
    images: BTreeMap<WindowKey, Arc<CachedImage>>,
    submitted_images: BTreeMap<WindowKey, Arc<CachedImage>>,
    submitted_priority: BTreeMap<WindowKey, bool>,
    retained_priority: BTreeMap<RetainedId, bool>,
    frame_priority: BTreeMap<WindowKey, bool>,
    retained: BTreeMap<RetainedId, Arc<CachedImage>>,
    visuals: BTreeMap<WindowId, AnimatedWindow>,
    outgoing_visuals: Vec<AnimatedWindow>,
    presented_visuals: BTreeMap<WindowId, AnimatedWindow>,
    presented_workspaces: Vec<WorkspaceMotion>,
    held: BTreeSet<WindowKey>,
    frame: Option<PresentationFrame>,
    drawn: BTreeSet<PresentationSource>,
    drawn_groups: BTreeSet<u64>,
    last_use: BTreeMap<WindowKey, u64>,
    generation: u64,
    reset_pending: bool,
    theme: Option<Theme>,
    topology: Vec<(OutputId, Rect)>,
    policy_workspaces: BTreeMap<OutputId, WorkspaceId>,
    pending_workspace_outputs: BTreeSet<OutputId>,
}

impl WindowAnimations {
    pub fn visual(&self, id: WindowId) -> Option<&AnimatedWindow> {
        self.visuals.get(&id)
    }
    pub fn presented_visual(&self, id: WindowId) -> Option<&AnimatedWindow> {
        self.presented_visuals.get(&id)
    }
    pub fn outgoing(&self) -> impl Iterator<Item = &AnimatedWindow> {
        self.outgoing_visuals.iter()
    }
    pub fn wallpaper_offsets(&self, output: OutputId) -> Option<&WorkspaceMotion> {
        self.frame
            .as_ref()?
            .workspaces
            .iter()
            .find(|g| g.outputs.contains(&output))
    }
    pub fn workspace_input_blocked(&self, output: OutputId) -> bool {
        self.pending_workspace_outputs.contains(&output)
            || self
                .presented_workspaces
                .iter()
                .any(|g| g.outputs.contains(&output))
    }
    #[cfg(test)]
    pub fn test_workspace_motion(&mut self, output: OutputId, active: bool) {
        if active {
            self.pending_workspace_outputs.insert(output);
        } else {
            self.pending_workspace_outputs.remove(&output);
        }
    }
    /// Record only a source whose elements were successfully included in the submitted scene.
    pub fn drawn(&mut self, source: PresentationSource) {
        self.drawn.insert(source);
    }
    pub fn drawn_workspace(&mut self, group: u64) {
        self.drawn_groups.insert(group);
    }
    /// The next reconciliation baselines policy instantly; renderer loss must not replay old tracks.
    pub fn reset(&mut self) {
        self.reset_pending = true;
    }

    fn drop_images(&mut self) {
        self.images.clear();
        self.submitted_images.clear();
        self.submitted_priority.clear();
        self.retained_priority.clear();
        self.frame_priority.clear();
        self.retained.clear();
        self.visuals.clear();
        self.outgoing_visuals.clear();
        self.presented_visuals.clear();
        self.presented_workspaces.clear();
        self.held.clear();
        self.frame = None;
        self.drawn.clear();
        self.drawn_groups.clear();
        self.last_use.clear();
        self.pending_workspace_outputs.clear();
    }
    fn release(&mut self, ids: impl IntoIterator<Item = RetainedId>) {
        for id in ids {
            self.retained.remove(&id);
            self.retained_priority.remove(&id);
        }
    }
    fn apply_update(
        &mut self,
        update: crate::runtime::presentation::PresentationUpdate,
        engine: &mut crate::runtime::animation::AnimationEngine,
    ) {
        self.release(update.release);
        for request in update.retain {
            if let Some(image) = self.submitted_images.get(&request.key).cloned() {
                self.retained.insert(request.id, image);
                self.pin_priority(request.id, request.key);
            } else {
                // A request never authorizes accessing a missing/destroyed client.
                let released = self.planner.settle_window(request.key, engine);
                self.release(released);
            }
        }
    }
    fn pin_priority(&mut self, id: RetainedId, key: WindowKey) {
        self.retained_priority.insert(
            id,
            self.submitted_priority.get(&key).copied().unwrap_or(false),
        );
    }
    fn remember_priority(&mut self, key: WindowKey) {
        self.submitted_priority
            .insert(key, self.frame_priority.get(&key).copied().unwrap_or(false));
    }
    fn resident(&self) -> (usize, usize) {
        let mut unique = BTreeMap::new();
        for image in self
            .images
            .values()
            .chain(self.submitted_images.values())
            .chain(self.retained.values())
            .chain(self.visuals.values().map(|w| &w.cached))
            .chain(self.presented_visuals.values().map(|w| &w.cached))
            .chain(self.outgoing_visuals.iter().map(|w| &w.cached))
        {
            unique.insert(Arc::as_ptr(image) as usize, image.image.bytes());
        }
        (unique.len(), unique.values().sum())
    }
    fn make_room(&mut self, bytes: usize, keep: WindowKey) -> bool {
        loop {
            let (count, used) = self.resident();
            if image_fits(count, used, bytes) {
                return true;
            }
            let victim = self
                .last_use
                .iter()
                .filter(|(key, _)| **key != keep)
                .min_by_key(|(_, use_time)| **use_time)
                .map(|(key, _)| *key);
            let Some(victim) = victim else {
                return false;
            };
            self.last_use.remove(&victim);
            self.images.remove(&victim);
            self.submitted_images.remove(&victim);
            self.submitted_priority.remove(&victim);
            self.frame_priority.remove(&victim);
        }
    }
    fn forget_window(&mut self, key: WindowKey) {
        self.images.remove(&key);
        self.submitted_images.remove(&key);
        self.submitted_priority.remove(&key);
        self.frame_priority.remove(&key);
        self.held.remove(&key);
        self.visuals.remove(&key.window);
        self.presented_visuals.remove(&key.window);
        self.outgoing_visuals.retain(|w| w.sampled.key != key);
        self.last_use.remove(&key);
    }
}

fn image_destination(
    source: Rect,
    frame: Rect,
    target: crate::runtime::animation::AnimationRect,
) -> Rectangle<f64, Logical> {
    let sx = target.width / f64::from(frame.width);
    let sy = target.height / f64::from(frame.height);
    Rectangle::new(
        (
            target.x + f64::from(source.x - frame.x) * sx,
            target.y + f64::from(source.y - frame.y) * sy,
        )
            .into(),
        (f64::from(source.width) * sx, f64::from(source.height) * sy).into(),
    )
}
fn image_fits(count: usize, used: usize, bytes: usize) -> bool {
    count < MAX_IMAGES && bytes <= MAX_BYTES && used <= MAX_BYTES - bytes
}
fn image_size(source: Rect) -> (i32, i32) {
    let scale = (f64::from(MAX_IMAGE_EDGE) / f64::from(source.width.max(1)))
        .min(f64::from(MAX_IMAGE_EDGE) / f64::from(source.height.max(1)))
        .min(1.0);
    (
        (f64::from(source.width) * scale).round().max(1.0) as i32,
        (f64::from(source.height) * scale).round().max(1.0) as i32,
    )
}
fn committed_matches(image: &CachedImage, sample: &SampledWindow, fullscreen: bool) -> bool {
    frame_matches(
        image.image.frame,
        image.identity.committed_fullscreen,
        sample.pose.rect,
        fullscreen,
    )
}
fn frame_matches(
    frame: Rect,
    committed_fullscreen: bool,
    target: crate::runtime::animation::AnimationRect,
    fullscreen: bool,
) -> bool {
    (f64::from(frame.width) - target.width).abs() <= 1.0
        && (f64::from(frame.height) - target.height).abs() <= 1.0
        && committed_fullscreen == fullscreen
}

impl Compositor {
    /// Reconcile full normal-window policy once, after placements and shell targets are refreshed.
    pub fn reconcile_window_animations(&mut self) {
        let now = self.runtime.animation_time_at(Instant::now());
        let snapshot = self.window_animation_snapshot();
        let enabled = self.runtime.config.animations.enabled
            && !self.runtime.config.animations.reduced_motion;
        let suppressed = !enabled || self.overview_present();
        let topology: Vec<_> = self.outputs.iter().map(|o| (o.id, o.rect)).collect();
        let mut animations = std::mem::take(&mut self.window_animations);
        let reset = animations.reset_pending
            || animations
                .theme
                .as_ref()
                .is_some_and(|theme| theme != &self.runtime.config.theme)
            || (!animations.topology.is_empty() && animations.topology != topology);
        animations.reset_pending = false;
        animations.theme = Some(self.runtime.config.theme.clone());
        animations.topology = topology;
        let grabbed = self
            .seat
            .get_pointer()
            .is_some_and(|pointer| pointer.is_grabbed())
            || self.drag.is_some()
            || self.titlebar_drag.is_some()
            || self.titlebar_press.is_some()
            || !self.suppressed_buttons.is_empty();
        let workspaces: BTreeMap<_, _> = snapshot
            .groups
            .iter()
            .flat_map(|g| g.outputs.iter().map(move |o| (*o, g.workspace)))
            .collect();
        if !suppressed
            && !reset
            && !grabbed
            && self.runtime.config.animations.workspace_switch.enabled
        {
            for (output, workspace) in &workspaces {
                if animations
                    .policy_workspaces
                    .get(output)
                    .is_some_and(|old| old != workspace)
                {
                    animations.pending_workspace_outputs.insert(*output);
                }
            }
        }
        animations.policy_workspaces = workspaces;
        let causes: Vec<_> = if suppressed || reset || grabbed {
            vec![TransitionCause::Settle]
        } else {
            Vec::new()
        };
        if suppressed || reset || grabbed {
            animations.drop_images();
        }
        match animations.planner.reconcile(
            snapshot.clone(),
            &causes,
            now,
            &mut self.runtime.animations,
        ) {
            Ok(update) => animations.apply_update(update, &mut self.runtime.animations),
            Err(error) => {
                eprintln!("clear: animation snapshot rejected, settling: {error}");
                let released = animations.planner.clear(&mut self.runtime.animations);
                animations.release(released);
                animations.drop_images();
                let _ = animations.planner.reconcile(
                    snapshot,
                    &[TransitionCause::Settle],
                    now,
                    &mut self.runtime.animations,
                );
            }
        }
        let keys: BTreeSet<_> = self
            .windows
            .iter()
            .filter(|(_, e)| e.mapped)
            .map(|(id, e)| WindowKey {
                window: *id,
                generation: e.mapping_generation,
            })
            .collect();
        animations.images.retain(|key, _| keys.contains(key));
        animations
            .submitted_images
            .retain(|key, _| keys.contains(key));
        animations.last_use.retain(|key, _| keys.contains(key));
        animations
            .submitted_priority
            .retain(|key, _| animations.submitted_images.contains_key(key));
        animations.held.retain(|key| keys.contains(key));
        self.window_animations = animations;
    }

    fn window_animation_snapshot(&self) -> PresentationSnapshot {
        PresentationSnapshot {
            windows: self
                .runtime
                .desktop
                .windows()
                .filter(|w| w.role == WindowRole::Normal)
                .filter_map(|w| {
                    let entry = self.windows.get(&w.id)?;
                    let output = w.output?;
                    Some(WindowState {
                        key: WindowKey {
                            window: w.id,
                            generation: entry.mapping_generation,
                        },
                        live: true,
                        mapped: entry.mapped,
                        minimized: w.minimized,
                        maximized: w.maximized,
                        fullscreen: w.fullscreen,
                        home_output: output,
                        workspace: w.workspace,
                        placement: self.placements.iter().find(|p| p.window == w.id).cloned(),
                        minimize_target: self
                            .minimize_target(w.id)
                            .or_else(|| self.minimize_fallback(w.id)),
                    })
                })
                .collect(),
            outputs: self
                .outputs
                .iter()
                .map(|o| OutputState {
                    id: o.id,
                    bounds: o.rect,
                    panel: None,
                    physical_scale: o.output.current_scale().fractional_scale(),
                })
                .collect(),
            groups: self
                .runtime
                .desktop
                .groups()
                .iter()
                .filter_map(|g| {
                    Some(GroupState {
                        id: g.outputs.first()?.0,
                        workspace: g.workspace,
                        outputs: g.outputs.clone(),
                    })
                })
                .collect(),
        }
    }

    /// Compose live committed sources before the main framebuffer is bound. No readback occurs.
    pub fn prepare_window_animations(
        &mut self,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        shaders: &RoundedShaders,
        composer: &mut WindowImageComposer,
    ) -> Result<(), GlesError> {
        let now = self
            .runtime
            .animation_time_at(self.start + self.animation_sample_time);
        let mut animations = std::mem::take(&mut self.window_animations);
        if !self.runtime.config.animations.enabled
            || self.runtime.config.animations.reduced_motion
            || self.overview_present()
        {
            let released = animations.planner.clear(&mut self.runtime.animations);
            animations.release(released);
            animations.drop_images();
            self.window_animations = animations;
            return Ok(());
        }
        animations.generation = animations.generation.wrapping_add(1);
        animations.visuals.clear();
        animations.outgoing_visuals.clear();
        animations.drawn.clear();
        animations.drawn_groups.clear();
        let candidates: Vec<_> = self
            .placements
            .iter()
            .filter(|p| {
                self.runtime
                    .desktop
                    .window(p.window)
                    .is_some_and(|w| w.role == WindowRole::Normal)
            })
            .filter_map(|p| {
                self.windows.get(&p.window).map(|e| {
                    (
                        WindowKey {
                            window: p.window,
                            generation: e.mapping_generation,
                        },
                        p.focused,
                    )
                })
            })
            .collect();
        for (key, focused) in candidates {
            let prepared = match PreparedWindowImage::prepare(
                self, renderer, titlebars, key.window, focused,
            ) {
                Ok(Some(prepared)) => prepared,
                Ok(None) | Err(_) => {
                    let released = animations
                        .planner
                        .settle_window(key, &mut self.runtime.animations);
                    animations.release(released);
                    animations.forget_window(key);
                    continue;
                }
            };
            animations.last_use.insert(key, animations.generation);
            if animations
                .images
                .get(&key)
                .is_some_and(|image| image.identity == prepared.identity)
            {
                continue;
            }
            let size = image_size(prepared.identity.source);
            let bytes = size.0 as usize * size.1 as usize * 4;
            if !animations.make_room(bytes, key) {
                let released = animations
                    .planner
                    .settle_window(key, &mut self.runtime.animations);
                animations.release(released);
                animations.forget_window(key);
                continue;
            }
            let image = match composer.compose(renderer, shaders, &prepared, size, None) {
                Ok(image) => image,
                Err(_) => {
                    let released = animations
                        .planner
                        .settle_window(key, &mut self.runtime.animations);
                    animations.release(released);
                    animations.forget_window(key);
                    continue;
                }
            };
            let entry = &self.windows[&key.window];
            animations.images.insert(
                key,
                Arc::new(CachedImage {
                    content: content_rect(
                        image.frame,
                        entry.uses_ssd(),
                        self.runtime.config.theme.titlebar.height,
                    ),
                    titlebar: self.runtime.config.theme.titlebar.clone(),
                    ssd: entry.uses_ssd(),
                    image,
                    identity: prepared.identity,
                }),
            );
        }
        animations.frame_priority = animations
            .images
            .keys()
            .filter_map(|key| {
                self.runtime
                    .desktop
                    .window(key.window)
                    .map(|window| (*key, window.fullscreen))
            })
            .collect();
        let mut frame = animations.planner.sample(now, &self.runtime.animations);
        let mut unavailable = Vec::new();
        for sample in &frame.windows {
            let cached = match sample.source {
                PresentationSource::Live(key) => animations.images.get(&key).cloned(),
                PresentationSource::Retained(id) => animations.retained.get(&id).cloned(),
            };
            let Some(cached) = cached else {
                if sample.effect.is_some() {
                    unavailable.push(sample.key);
                }
                continue;
            };
            let fullscreen = self
                .runtime
                .desktop
                .window(sample.key.window)
                .is_some_and(|w| w.fullscreen);
            if !sample.outgoing && sample.effect.is_some() {
                // Restore/workspace entry can also reveal a client whose hidden size
                // has not caught up with its newly requested placement.
                animations.held.insert(sample.key);
            }
            let holds_commit = animations.held.contains(&sample.key)
                && !committed_matches(&cached, sample, fullscreen);
            if !sample.outgoing && sample.finished && !holds_commit {
                animations.held.remove(&sample.key);
                continue;
            }
            let clip_outputs = self
                .runtime
                .desktop
                .groups()
                .iter()
                .find(|g| g.outputs.contains(&sample.output))
                .map(|g| g.outputs.as_slice())
                .unwrap_or(std::slice::from_ref(&sample.output))
                .iter()
                .filter_map(|id| self.outputs.iter().find(|o| o.id == *id).map(|o| o.rect))
                .collect();
            let visual = AnimatedWindow {
                sampled: sample.clone(),
                image: cached.image.clone(),
                committed_geometry_origin: cached.identity.geometry_origin,
                committed_content: cached.content,
                committed_titlebar: cached.titlebar.clone(),
                committed_ssd: cached.ssd,
                fullscreen_priority: match sample.source {
                    PresentationSource::Live(_) => fullscreen,
                    PresentationSource::Retained(id) => animations
                        .retained_priority
                        .get(&id)
                        .copied()
                        .unwrap_or(false),
                },
                clip_outputs,
                cached,
            };
            if sample.outgoing {
                animations.outgoing_visuals.push(visual);
            } else {
                animations.visuals.insert(sample.key.window, visual);
            }
        }
        for key in unavailable {
            let released = animations
                .planner
                .settle_window(key, &mut self.runtime.animations);
            animations.release(released);
            animations.forget_window(key);
        }
        // A failed source is settled before any scene element can use its abandoned prediction.
        frame = animations.planner.sample(now, &self.runtime.animations);
        animations.frame = Some(frame);
        self.window_animations = animations;
        Ok(())
    }

    /// Acknowledge only the successful main-frame submission, then retire owned sources.
    pub fn acknowledge_window_animations(&mut self) {
        let now = self
            .runtime
            .animation_time_at(self.start + self.animation_sample_time);
        let animations = &mut self.window_animations;
        let Some(mut frame) = animations.frame.take() else {
            return;
        };
        let pending: BTreeSet<_> = frame
            .workspaces
            .iter()
            .flat_map(|g| g.outputs.iter().copied())
            .collect();
        frame
            .windows
            .retain(|w| animations.drawn.contains(&w.source));
        frame
            .workspaces
            .retain(|g| animations.drawn_groups.contains(&g.group));
        if animations.planner.mark_presented(&frame).is_err() {
            return;
        }
        animations.presented_visuals = animations
            .visuals
            .iter()
            .filter(|(_, visual)| animations.drawn.contains(&visual.sampled.source))
            .map(|(id, visual)| (*id, visual.clone()))
            .collect();
        animations.presented_workspaces = frame.workspaces.clone();
        animations.pending_workspace_outputs = pending;
        let submitted_keys: Vec<_> = animations
            .drawn
            .iter()
            .filter_map(|source| match source {
                PresentationSource::Live(key) => Some(*key),
                _ => None,
            })
            .collect();
        for key in submitted_keys {
            if let Some(image) = animations.images.get(&key).cloned() {
                animations.submitted_images.insert(key, image);
                animations.remember_priority(key);
            }
        }
        let released = animations
            .planner
            .retire_finished(now, &mut self.runtime.animations);
        animations.release(released);
        animations.drawn.clear();
        animations.drawn_groups.clear();
        animations.frame = Some(frame);
    }

    /// A physical move/resize owns geometry immediately and cancels its old visual source.
    pub fn settle_window_animation(&mut self, id: WindowId) {
        let Some(entry) = self.windows.get(&id) else {
            return;
        };
        let key = WindowKey {
            window: id,
            generation: entry.mapping_generation,
        };
        let released = self
            .window_animations
            .planner
            .settle_window(key, &mut self.runtime.animations);
        self.window_animations.release(released);
        self.window_animations.forget_window(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_admission_accounts_bytes_and_entries_without_overflow() {
        assert!(image_fits(127, MAX_BYTES - 4, 4));
        assert!(!image_fits(128, 0, 4));
        assert!(!image_fits(0, MAX_BYTES, 4));
        assert!(!image_fits(0, usize::MAX, 4));
        assert!(!image_fits(0, 0, usize::MAX));
    }
    #[test]
    fn bounded_live_source_resolution_keeps_original_aspect_ratio() {
        assert_eq!(image_size(Rect::new(-2, -2, 4096, 2048)), (2048, 1024));
        assert_eq!(image_size(Rect::new(0, 0, 1, 8192)), (1, 2048));
    }
    #[test]
    fn committed_border_and_ssd_source_scale_relative_to_the_inner_frame() {
        use crate::runtime::animation::AnimationRect;
        let target = AnimationRect {
            x: 10.25,
            y: 20.5,
            width: 200.0,
            height: 264.0,
        };
        let dest = image_destination(
            Rect::new(-2, -2, 104, 136),
            Rect::new(0, 0, 100, 132),
            target,
        );
        assert_eq!(dest.loc, (6.25, 16.5).into());
        assert_eq!(dest.size, (208.0, 272.0).into());
        // An ACK/state request cannot drop SSD or old geometry before the actual buffer commit.
        assert!(!frame_matches(
            Rect::new(0, 0, 100, 132),
            false,
            target,
            true
        ));
        assert!(!frame_matches(
            Rect::new(0, 0, 200, 264),
            false,
            target,
            true
        ));
        assert!(frame_matches(Rect::new(0, 0, 200, 264), true, target, true));
        assert!(!frame_matches(
            Rect::new(0, 0, 200, 230),
            true,
            target,
            true
        ));
    }
    #[test]
    fn destruction_without_a_previously_submitted_image_settles_without_surface_access() {
        use crate::{
            config::AnimationsConfig,
            core::{Placement, WorkspaceId},
            runtime::animation::AnimationEngine,
        };
        use std::time::Duration;
        let mut bridge = WindowAnimations::default();
        let mut engine = AnimationEngine::new(
            AnimationsConfig {
                enabled: true,
                ..Default::default()
            },
            Duration::ZERO,
        )
        .unwrap();
        let key = WindowKey {
            window: WindowId(1),
            generation: 1,
        };
        let bounds = Rect::new(0, 0, 800, 600);
        let mut snapshot = PresentationSnapshot {
            windows: vec![WindowState {
                key,
                live: true,
                mapped: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                home_output: OutputId(1),
                workspace: WorkspaceId(1),
                placement: Some(Placement {
                    window: key.window,
                    rect: Rect::new(100, 100, 200, 132),
                    clip: Some(bounds),
                    focused: true,
                    tiled: true,
                }),
                minimize_target: None,
            }],
            outputs: vec![OutputState {
                id: OutputId(1),
                bounds,
                panel: None,
                physical_scale: 1.0,
            }],
            groups: vec![GroupState {
                id: 1,
                workspace: WorkspaceId(1),
                outputs: vec![OutputId(1)],
            }],
        };
        bridge
            .planner
            .reconcile(
                snapshot.clone(),
                &[TransitionCause::Settle],
                Duration::ZERO,
                &mut engine,
            )
            .unwrap();
        snapshot.windows.clear();
        let update = bridge
            .planner
            .reconcile(snapshot, &[], Duration::ZERO, &mut engine)
            .unwrap();
        assert_eq!(update.retain.len(), 1);
        assert_eq!(engine.len(), 1);
        bridge.apply_update(update, &mut engine);
        assert!(engine.is_empty());
        assert!(bridge.retained.is_empty());
        assert!(
            bridge
                .planner
                .sample(Duration::ZERO, &engine)
                .windows
                .is_empty()
        );
    }
    #[test]
    fn retained_layer_priority_uses_last_submission_and_mapping_generation() {
        let mut bridge = WindowAnimations::default();
        let old = WindowKey {
            window: WindowId(1),
            generation: 1,
        };
        let remapped = WindowKey {
            generation: 2,
            ..old
        };
        bridge.submitted_priority.insert(old, false);
        bridge.frame_priority.insert(old, true);
        bridge.pin_priority(RetainedId(1), old);
        assert!(
            !bridge.retained_priority[&RetainedId(1)],
            "an unsubmitted fullscreen request does not change the source layer"
        );
        // The submitted fullscreen scene may still use a pre-fullscreen SSD image.
        bridge.remember_priority(old);
        bridge.pin_priority(RetainedId(2), old);
        assert!(bridge.retained_priority[&RetainedId(2)]);
        bridge.frame_priority.insert(remapped, false);
        bridge.remember_priority(remapped);
        bridge.pin_priority(RetainedId(3), remapped);
        assert!(!bridge.retained_priority[&RetainedId(3)]);
        bridge.forget_window(old);
        assert!(
            bridge.retained_priority[&RetainedId(2)],
            "retained policy priority survives source generation removal"
        );
        bridge.release([RetainedId(2)]);
        assert!(!bridge.retained_priority.contains_key(&RetainedId(2)));
        assert!(!bridge.submitted_priority.contains_key(&old));
        assert_eq!(bridge.submitted_priority.len(), 1);
    }
}
