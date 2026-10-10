//! Snapshot differences and immutable presentation frames, without renderer ownership.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use super::animation::{AnimationEngine, AnimationPose, AnimationRect};
use crate::{
    config::AnimationEffect,
    core::{OutputId, Placement, Rect, WindowId, WorkspaceId},
};

/// Identity of a single mapped generation; remapping never reuses a retained source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowKey {
    pub window: WindowId,
    pub generation: u64,
}

/// Backend-neutral normal-window state, including hidden windows.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    pub key: WindowKey,
    pub live: bool,
    pub mapped: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub home_output: OutputId,
    pub workspace: WorkspaceId,
    pub placement: Option<Placement>,
    pub minimize_target: Option<Rect>,
}

impl WindowState {
    fn visible(&self) -> bool {
        self.live && self.mapped && !self.minimized && self.placement.is_some()
    }
}

/// Output bounds and optional mapped-panel fallback for minimize destinations.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputState {
    pub id: OutputId,
    pub bounds: Rect,
    pub panel: Option<Rect>,
    pub physical_scale: f64,
}

/// Current core presentation ownership. Motion never mutates this record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupState {
    pub id: u64,
    pub workspace: WorkspaceId,
    pub outputs: Vec<OutputId>,
}

/// Reconciled policy and mapping state, in back-to-front placement order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresentationSnapshot {
    pub windows: Vec<WindowState>,
    pub outputs: Vec<OutputState>,
    pub groups: Vec<GroupState>,
}

/// Direction vector for workspace motion; horizontal helpers are the initial policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkspaceDirection {
    pub x: f64,
    pub y: f64,
}
impl WorkspaceDirection {
    pub const LEFT: Self = Self { x: -1.0, y: 0.0 };
    pub const RIGHT: Self = Self { x: 1.0, y: 0.0 };
}

/// Explicit intent when snapshot changes alone cannot identify the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTransitionKind {
    Open,
    Close,
    Minimize,
    Restore,
    Movement,
    Maximize,
    Fullscreen,
    Direct,
}

/// Optional cause hints. Workspace ownership and final geometry still come from snapshots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransitionCause {
    Window {
        key: WindowKey,
        kind: WindowTransitionKind,
    },
    WorkspaceSwitch {
        group: u64,
        direction: WorkspaceDirection,
    },
    /// Settle all motion for an output/topology/resource reset.
    Settle,
}

/// Opaque retained-image identity assigned by the planner; the adapter owns its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RetainedId(pub u64);

/// Which committed generation or retained image a sampled visual must use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PresentationSource {
    Live(WindowKey),
    Retained(RetainedId),
}

/// Request to pin an already owned prior committed image. A disappeared mapping
/// must never be dereferenced to satisfy this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionRequest {
    pub id: RetainedId,
    pub key: WindowKey,
}

/// Resource ownership changes produced only by reconciliation/retirement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresentationUpdate {
    pub retain: Vec<RetentionRequest>,
    pub release: Vec<RetainedId>,
}

/// Immutable presentation geometry for one window/image, with explicit input eligibility.
#[derive(Debug, Clone, PartialEq)]
pub struct SampledWindow {
    pub key: WindowKey,
    pub source: PresentationSource,
    pub pose: AnimationPose,
    pub clip: Option<Rect>,
    pub output: OutputId,
    pub input_eligible: bool,
    pub outgoing: bool,
    pub effect: Option<AnimationEffect>,
    pub finished: bool,
}

/// Per-group wallpaper/scene offsets; pixels and layer ordering belong to the adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMotion {
    pub group: u64,
    pub from_workspace: WorkspaceId,
    pub to_workspace: WorkspaceId,
    pub outputs: Vec<OutputId>,
    pub incoming_offset: (f64, f64),
    pub outgoing_offset: (f64, f64),
    pub progress: f64,
}

/// One common-timestamp frame. Sampling performs no commands, IO, or script calls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresentationFrame {
    pub timestamp: Duration,
    /// Captured analytic phase; retain this unchanged when acknowledging the frame.
    pub animation_time: f64,
    pub windows: Vec<SampledWindow>,
    pub workspaces: Vec<WorkspaceMotion>,
    // Opaque track identity/terminal status must survive caller filtering of unavailable draws.
    sampled_tracks: BTreeMap<u64, (u64, bool)>,
}

#[derive(Debug, Clone)]
struct Motion {
    track: u64,
    key: WindowKey,
    source: PresentationSource,
    clip: Option<Rect>,
    output: OutputId,
    effect: AnimationEffect,
    workspace_group: Option<u64>,
}
#[derive(Debug, Clone)]
struct GroupMotion {
    incoming_track: u64,
    outgoing_track: u64,
    from: WorkspaceId,
    to: WorkspaceId,
    outputs: Vec<OutputId>,
    delta: (f64, f64),
}

/// Bounded presentation bookkeeping; no compositor surface or image is retained here.
#[derive(Debug, Default)]
pub struct PresentationPlanner {
    snapshot: PresentationSnapshot,
    live: BTreeMap<WindowKey, Motion>,
    outgoing: BTreeMap<RetainedId, Motion>,
    groups: BTreeMap<u64, GroupMotion>,
    presented: BTreeMap<WindowKey, (AnimationPose, Duration, f64)>,
    presented_groups: BTreeMap<u64, (WorkspaceMotion, Duration, f64)>,
    acknowledged: Option<(Duration, f64)>,
    acknowledged_tracks: BTreeMap<u64, (u64, bool)>,
    next_id: u64,
}

impl PresentationPlanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Diff a validated snapshot once; sampling never recalculates desktop policy.
    pub fn reconcile(
        &mut self,
        snapshot: PresentationSnapshot,
        causes: &[TransitionCause],
        now: Duration,
        engine: &mut AnimationEngine,
    ) -> Result<PresentationUpdate, String> {
        validate_snapshot(&snapshot)?;
        for cause in causes {
            if let TransitionCause::WorkspaceSwitch { direction, .. } = cause {
                if !direction.x.is_finite()
                    || !direction.y.is_finite()
                    || direction.x.abs() > 1.0
                    || direction.y.abs() > 1.0
                    || (direction.x == 0.0 && direction.y == 0.0)
                {
                    return Err(
                        "workspace direction must be finite, nonzero, and in -1..=1 per axis"
                            .into(),
                    );
                }
            }
        }
        let mut update = PresentationUpdate::default();
        if causes.contains(&TransitionCause::Settle) {
            update.release.extend(self.clear(engine));
            self.snapshot = snapshot;
            return Ok(update);
        }
        update.release.extend(self.retire_finished(now, engine));
        let connected: BTreeSet<_> = snapshot.outputs.iter().map(|o| o.id).collect();
        let stale: Vec<_> = self
            .outgoing
            .iter()
            .filter_map(|(id, m)| (!connected.contains(&m.output)).then_some(*id))
            .collect();
        for id in stale {
            self.remove_outgoing(id, engine);
            update.release.push(id);
        }
        let stale_groups: Vec<_> = self
            .groups
            .iter()
            .filter_map(|(id, m)| {
                snapshot
                    .groups
                    .iter()
                    .find(|g| g.id == *id)
                    .is_none_or(|g| g.outputs != m.outputs)
                    .then_some(*id)
            })
            .collect();
        for id in stale_groups {
            self.remove_group(id, engine);
        }
        let stale_outgoing: Vec<_> = self
            .outgoing
            .iter()
            .filter_map(|(id, m)| {
                m.workspace_group
                    .is_some_and(|id| {
                        snapshot
                            .groups
                            .iter()
                            .find(|g| g.id == id)
                            .is_none_or(|g| !g.outputs.contains(&m.output))
                    })
                    .then_some(*id)
            })
            .collect();
        for id in stale_outgoing {
            self.remove_outgoing(id, engine);
            update.release.push(id);
        }
        let stale_live: Vec<_> = self
            .live
            .iter()
            .filter_map(|(key, m)| {
                m.workspace_group
                    .is_some_and(|id| {
                        snapshot
                            .groups
                            .iter()
                            .find(|g| g.id == id)
                            .is_none_or(|g| !g.outputs.contains(&m.output))
                    })
                    .then_some(*key)
            })
            .collect();
        for key in stale_live {
            self.remove_live(key, engine);
        }
        let previous = self.snapshot.clone();
        let old: BTreeMap<_, _> = previous.windows.iter().map(|w| (w.key, w)).collect();
        let next: BTreeMap<_, _> = snapshot.windows.iter().map(|w| (w.key, w)).collect();
        let settings = engine.config().clone();
        let mut switches = BTreeMap::new();
        for group in &snapshot.groups {
            if let Some(prior) = previous.groups.iter().find(|g| g.id == group.id) {
                if prior.workspace != group.workspace {
                    let direction = causes
                        .iter()
                        .find_map(|c| match c {
                            TransitionCause::WorkspaceSwitch {
                                group: id,
                                direction,
                            } if *id == group.id => Some(*direction),
                            _ => None,
                        })
                        .unwrap_or(if group.workspace > prior.workspace {
                            WorkspaceDirection::RIGHT
                        } else {
                            WorkspaceDirection::LEFT
                        });
                    let bounds = group_bounds(group, &snapshot);
                    switches.insert(
                        group.id,
                        (
                            prior.workspace,
                            group.workspace,
                            (
                                direction.x * f64::from(bounds.width),
                                direction.y * f64::from(bounds.height),
                            ),
                        ),
                    );
                }
            }
        }
        let mut inherited_tracks = BTreeMap::new();
        for (group, (from, to, delta)) in &switches {
            let superseded: Vec<_> = self
                .outgoing
                .iter()
                .filter_map(|(id, m)| (m.workspace_group == Some(*group)).then_some(*id))
                .collect();
            for id in superseded {
                if let Some(m) = self.outgoing.remove(&id) {
                    if next.get(&m.key).is_some_and(|w| w.visible()) {
                        inherited_tracks.insert(m.key, m.track);
                    } else {
                        engine.remove(m.track);
                    }
                }
                update.release.push(id);
            }
            let prior = self.groups.remove(group);
            let shown = self.presented_groups.get(group).cloned();
            let incoming_old = prior.as_ref().and_then(|m| {
                if m.to == *to {
                    Some(m.incoming_track)
                } else if m.from == *to {
                    Some(m.outgoing_track)
                } else {
                    None
                }
            });
            let outgoing_old = prior.as_ref().and_then(|m| {
                if m.to == *from {
                    Some(m.incoming_track)
                } else if m.from == *from {
                    Some(m.outgoing_track)
                } else {
                    None
                }
            });
            if let Some(m) = &prior {
                for track in [m.incoming_track, m.outgoing_track] {
                    if Some(track) != incoming_old && Some(track) != outgoing_old {
                        engine.remove(track);
                    }
                }
            }
            let shown_offset = |workspace: WorkspaceId| {
                shown.as_ref().and_then(|(m, _, phase)| {
                    if m.to_workspace == workspace {
                        Some((m.incoming_offset, *phase))
                    } else if m.from_workspace == workspace {
                        Some((m.outgoing_offset, *phase))
                    } else {
                        None
                    }
                })
            };
            let phase = engine.clock.sample(now);
            let incoming = shown_offset(*to).unwrap_or((*delta, phase));
            let outgoing = shown_offset(*from).unwrap_or(((0.0, 0.0), phase));
            let incoming_track = incoming_old.unwrap_or_else(|| self.allocate_track(engine));
            let outgoing_track = outgoing_old.unwrap_or_else(|| self.allocate_track(engine));
            let enabled = motion_enabled(engine, AnimationEffect::WorkspaceSwitch);
            let incoming_started = enabled
                && engine
                    .retarget_presented(
                        incoming_track,
                        *group,
                        AnimationEffect::WorkspaceSwitch,
                        offset_pose(incoming.0),
                        offset_pose((0.0, 0.0)),
                        now,
                        1.0,
                        incoming.1,
                    )
                    .is_ok();
            let outgoing_started = incoming_started
                && engine
                    .retarget_presented(
                        outgoing_track,
                        *group,
                        AnimationEffect::WorkspaceSwitch,
                        offset_pose(outgoing.0),
                        offset_pose((-delta.0, -delta.1)),
                        now,
                        1.0,
                        outgoing.1,
                    )
                    .is_ok();
            if outgoing_started {
                self.groups.insert(
                    *group,
                    GroupMotion {
                        incoming_track,
                        outgoing_track,
                        from: *from,
                        to: *to,
                        outputs: snapshot
                            .groups
                            .iter()
                            .find(|g| g.id == *group)
                            .unwrap()
                            .outputs
                            .clone(),
                        delta: *delta,
                    },
                );
            } else {
                engine.remove(incoming_track);
                engine.remove(outgoing_track);
                self.presented_groups.remove(group);
            }
        }
        for prior in previous.windows.iter().filter(|w| w.visible()) {
            if !connected.contains(&prior.home_output) {
                self.remove_live(prior.key, engine);
                continue;
            }
            let current = next.get(&prior.key).copied();
            let old_group_switch = group_for_output(&previous, prior.home_output)
                .is_some_and(|g| switches.contains_key(&g.id));
            if current.is_some_and(|w| {
                w.visible() && (!old_group_switch || w.home_output == prior.home_output)
            }) {
                continue;
            }
            let hint = window_hint(causes, prior.key);
            if hint == Some(WindowTransitionKind::Direct) {
                update.release.extend(self.settle_window(prior.key, engine));
                continue;
            }
            let kind = hint.unwrap_or_else(|| {
                if current.is_none_or(|w| !w.live || !w.mapped) {
                    WindowTransitionKind::Close
                } else if current.is_some_and(|w| w.minimized) {
                    WindowTransitionKind::Minimize
                } else {
                    WindowTransitionKind::Movement
                }
            });
            let switch = group_for_output(&previous, prior.home_output)
                .and_then(|g| switches.get(&g.id).map(|s| (g.id, s.2)));
            let (effect, target) = match kind {
                WindowTransitionKind::Close => {
                    let p = pose(prior.placement.as_ref().unwrap().rect);
                    (
                        AnimationEffect::WindowClose,
                        scaled(p, settings.window_close.scale, 0.0),
                    )
                }
                WindowTransitionKind::Minimize => (
                    AnimationEffect::Minimize,
                    minimize_pose(
                        current.unwrap_or(prior),
                        &snapshot,
                        prior.placement.as_ref().unwrap().rect,
                    ),
                ),
                _ if switch.is_some() => {
                    let (_, delta) = switch.unwrap();
                    (
                        AnimationEffect::WorkspaceSwitch,
                        translated(
                            pose(prior.placement.as_ref().unwrap().rect),
                            -delta.0,
                            -delta.1,
                        ),
                    )
                }
                _ => {
                    self.remove_live(prior.key, engine);
                    continue;
                }
            };
            let from = self.last_pose(
                prior.key,
                pose(prior.placement.as_ref().unwrap().rect),
                engine.clock.sample(now),
            );
            let track = self
                .live
                .remove(&prior.key)
                .map(|m| m.track)
                .unwrap_or_else(|| self.allocate_track(engine));
            if motion_enabled(engine, effect)
                && (effect != AnimationEffect::WorkspaceSwitch
                    || switch.is_some_and(|s| self.groups.contains_key(&s.0)))
                && engine
                    .retarget_presented(
                        track,
                        switch.map_or(prior.home_output.0, |s| s.0),
                        effect,
                        from.0,
                        target,
                        now,
                        output_scale(&snapshot, prior.home_output),
                        from.1,
                    )
                    .is_ok()
            {
                let id = RetainedId(self.allocate());
                self.outgoing.insert(
                    id,
                    Motion {
                        track,
                        key: prior.key,
                        source: PresentationSource::Retained(id),
                        clip: if effect == AnimationEffect::Minimize {
                            snapshot
                                .outputs
                                .iter()
                                .find(|o| o.id == prior.home_output)
                                .map(|o| o.bounds)
                        } else {
                            prior.placement.as_ref().unwrap().clip
                        },
                        output: prior.home_output,
                        effect,
                        workspace_group: switch.map(|s| s.0),
                    },
                );
                update.retain.push(RetentionRequest { id, key: prior.key });
            } else {
                engine.remove(track);
            }
        }
        for current in snapshot.windows.iter().filter(|w| w.visible()) {
            let prior = old.get(&current.key).copied();
            let placement = current.placement.as_ref().unwrap();
            let hint = window_hint(causes, current.key);
            if hint == Some(WindowTransitionKind::Direct) {
                update
                    .release
                    .extend(self.settle_window(current.key, engine));
                continue;
            }
            let group = group_for_output(&snapshot, current.home_output);
            let switched = group.and_then(|g| switches.get(&g.id).map(|s| (g.id, s.2)));
            let target = pose(placement.rect);
            let kind = hint.or_else(|| match prior {
                None => Some(WindowTransitionKind::Open),
                Some(w) if !w.live || !w.mapped => Some(WindowTransitionKind::Open),
                Some(w) if w.minimized => Some(WindowTransitionKind::Restore),
                Some(w) if w.fullscreen != current.fullscreen => {
                    Some(WindowTransitionKind::Fullscreen)
                }
                Some(w) if w.maximized != current.maximized => Some(WindowTransitionKind::Maximize),
                Some(w) if !w.visible() && switched.is_some() => {
                    Some(WindowTransitionKind::Movement)
                }
                Some(w) if w.placement.as_ref().map(|p| p.rect) != Some(placement.rect) => {
                    Some(WindowTransitionKind::Movement)
                }
                _ => None,
            });
            let Some(kind) = kind else {
                continue;
            };
            let (effect, initial) = match kind {
                WindowTransitionKind::Open => (
                    AnimationEffect::WindowOpen,
                    scaled(target, settings.window_open.scale, 0.0),
                ),
                WindowTransitionKind::Restore => (
                    AnimationEffect::Minimize,
                    minimize_pose(prior.unwrap_or(current), &snapshot, placement.rect),
                ),
                WindowTransitionKind::Maximize => (
                    AnimationEffect::Maximize,
                    prior
                        .and_then(|w| w.placement.as_ref())
                        .map_or(target, |p| pose(p.rect)),
                ),
                WindowTransitionKind::Fullscreen => (
                    AnimationEffect::Fullscreen,
                    prior
                        .and_then(|w| w.placement.as_ref())
                        .map_or(target, |p| pose(p.rect)),
                ),
                _ if switched.is_some()
                    && prior
                        .is_none_or(|w| !w.visible() || w.home_output != current.home_output) =>
                {
                    let (_, delta) = switched.unwrap();
                    (
                        AnimationEffect::WorkspaceSwitch,
                        translated(target, delta.0, delta.1),
                    )
                }
                _ => (
                    AnimationEffect::WindowMovement,
                    prior
                        .and_then(|w| w.placement.as_ref())
                        .map_or(target, |p| pose(p.rect)),
                ),
            };
            let restored: Vec<_> = self
                .outgoing
                .iter()
                .filter_map(|(id, m)| (m.key == current.key).then_some(*id))
                .collect();
            for id in restored {
                if let Some(m) = self.outgoing.remove(&id) {
                    inherited_tracks.insert(m.key, m.track);
                }
                update.release.push(id);
            }
            let from = if self.live.contains_key(&current.key)
                || prior.is_some_and(WindowState::visible)
                || kind == WindowTransitionKind::Restore
                || (effect == AnimationEffect::WorkspaceSwitch
                    && self.presented.contains_key(&current.key))
            {
                self.last_pose(current.key, initial, engine.clock.sample(now))
            } else {
                (initial, engine.clock.sample(now))
            };
            let track = self
                .live
                .get(&current.key)
                .map(|m| m.track)
                .or_else(|| inherited_tracks.remove(&current.key))
                .unwrap_or_else(|| self.allocate_track(engine));
            if motion_enabled(engine, effect)
                && (effect != AnimationEffect::WorkspaceSwitch
                    || switched.is_some_and(|s| self.groups.contains_key(&s.0)))
                && engine
                    .retarget_presented(
                        track,
                        group.map_or(current.home_output.0, |g| g.id),
                        effect,
                        from.0,
                        target,
                        now,
                        output_scale(&snapshot, current.home_output),
                        from.1,
                    )
                    .is_ok()
            {
                self.live.insert(
                    current.key,
                    Motion {
                        track,
                        key: current.key,
                        source: PresentationSource::Live(current.key),
                        clip: placement.clip,
                        output: current.home_output,
                        effect,
                        workspace_group: switched.map(|s| s.0),
                    },
                );
            } else {
                self.remove_live(current.key, engine);
                engine.remove(track);
            }
        }
        let live_keys: BTreeSet<_> = snapshot
            .windows
            .iter()
            .filter(|w| w.visible())
            .map(|w| w.key)
            .collect();
        for track in inherited_tracks.into_values() {
            engine.remove(track);
        }
        let stale: Vec<_> = self
            .live
            .keys()
            .copied()
            .filter(|k| !live_keys.contains(k))
            .collect();
        for key in stale {
            self.remove_live(key, engine);
        }
        let retained_keys: BTreeSet<_> = self.outgoing.values().map(|m| m.key).collect();
        self.presented
            .retain(|key, _| live_keys.contains(key) || retained_keys.contains(key));
        self.snapshot = snapshot;
        Ok(update)
    }

    /// Sample all final live placements plus bounded outgoing visual-only records.
    pub fn sample(&self, now: Duration, engine: &AnimationEngine) -> PresentationFrame {
        let mut frame = PresentationFrame {
            timestamp: now,
            animation_time: engine.clock.sample(now),
            ..Default::default()
        };
        for track in self
            .live
            .values()
            .chain(self.outgoing.values())
            .map(|m| m.track)
            .chain(
                self.groups
                    .values()
                    .flat_map(|m| [m.incoming_track, m.outgoing_track]),
            )
        {
            if let (Some(revision), Some(sample)) =
                (engine.revision(track), engine.sample(track, now))
            {
                frame
                    .sampled_tracks
                    .insert(track, (revision, sample.finished));
            }
        }
        for (group, m) in &self.groups {
            let incoming = engine.sample(m.incoming_track, now);
            let outgoing = engine.sample(m.outgoing_track, now);
            if let (Some(incoming), Some(outgoing)) = (incoming, outgoing) {
                if !incoming.finished || !outgoing.finished {
                    let incoming_offset = (incoming.pose.rect.x, incoming.pose.rect.y);
                    let outgoing_offset = (outgoing.pose.rect.x, outgoing.pose.rect.y);
                    let remaining = if m.delta.0.abs() >= m.delta.1.abs() {
                        incoming_offset.0 / m.delta.0
                    } else {
                        incoming_offset.1 / m.delta.1
                    };
                    let progress = (1.0 - remaining).clamp(0.0, 1.0);
                    frame.workspaces.push(WorkspaceMotion {
                        group: *group,
                        from_workspace: m.from,
                        to_workspace: m.to,
                        outputs: m.outputs.clone(),
                        incoming_offset,
                        outgoing_offset,
                        progress,
                    });
                }
            }
        }
        for w in self.snapshot.windows.iter().filter(|w| w.visible()) {
            let placement = w.placement.as_ref().unwrap();
            let motion = self.live.get(&w.key);
            let sample = motion.and_then(|m| engine.sample(m.track, now));
            let moving_workspace = motion
                .is_some_and(|m| m.effect == AnimationEffect::WorkspaceSwitch)
                && sample.is_some_and(|s| !s.finished);
            frame.windows.push(SampledWindow {
                key: w.key,
                source: PresentationSource::Live(w.key),
                pose: sample.map_or_else(|| pose(placement.rect), |s| s.pose),
                clip: placement.clip,
                output: w.home_output,
                input_eligible: !moving_workspace,
                outgoing: false,
                effect: motion.map(|m| m.effect),
                finished: sample.is_none_or(|s| s.finished),
            });
        }
        for m in self.outgoing.values() {
            if let Some(s) = engine.sample(m.track, now).filter(|s| !s.finished) {
                frame.windows.push(SampledWindow {
                    key: m.key,
                    source: m.source,
                    pose: s.pose,
                    clip: m.clip,
                    output: m.output,
                    input_eligible: false,
                    outgoing: true,
                    effect: Some(m.effect),
                    finished: false,
                });
            }
        }
        frame
    }

    /// Record actually submitted/presented poses, never predictions. The adapter
    /// must remove visuals it could not draw. Unknown/stale sources, malformed poses,
    /// duplicates, and per-window backwards timestamps fail without changing state.
    pub fn mark_presented(&mut self, frame: &PresentationFrame) -> Result<(), String> {
        if !frame.animation_time.is_finite() || frame.animation_time < 0.0 {
            return Err("presented animation phase must be finite and nonnegative".into());
        }
        if self
            .acknowledged
            .is_some_and(|(time, phase)| time > frame.timestamp || phase > frame.animation_time)
        {
            return Err("presented frame timestamp and phase must not go backwards".into());
        }
        let mut sources = BTreeSet::new();
        for w in &frame.windows {
            let valid_source = match w.source {
                PresentationSource::Live(key) => {
                    key == w.key
                        && !w.outgoing
                        && self
                            .snapshot
                            .windows
                            .iter()
                            .any(|s| s.key == key && s.visible() && s.home_output == w.output)
                }
                PresentationSource::Retained(id) => {
                    w.outgoing
                        && self
                            .outgoing
                            .get(&id)
                            .is_some_and(|m| m.key == w.key && m.output == w.output)
                }
            };
            if !valid_source
                || !sources.insert(w.source)
                || w.pose.validate().is_err()
                || self.presented.get(&w.key).is_some_and(|(_, t, phase)| {
                    *t > frame.timestamp || *phase > frame.animation_time
                })
            {
                return Err("presented frame has an unknown/duplicate source, invalid pose, or backwards timestamp".into());
            }
        }
        let mut groups = BTreeSet::new();
        for g in &frame.workspaces {
            let valid = self.groups.get(&g.group).is_some_and(|m| {
                m.from == g.from_workspace && m.to == g.to_workspace && m.outputs == g.outputs
            });
            if !valid
                || !groups.insert(g.group)
                || offset_pose(g.incoming_offset).validate().is_err()
                || offset_pose(g.outgoing_offset).validate().is_err()
                || !g.progress.is_finite()
                || !(0.0..=1.0).contains(&g.progress)
                || self
                    .presented_groups
                    .get(&g.group)
                    .is_some_and(|(_, timestamp, phase)| {
                        *timestamp > frame.timestamp || *phase > frame.animation_time
                    })
            {
                return Err("presented frame has a stale/duplicate group, invalid offset, or backwards timestamp".into());
            }
        }
        let live: BTreeSet<_> = frame
            .windows
            .iter()
            .filter(|w| !w.outgoing)
            .map(|w| w.key)
            .collect();
        for w in &frame.windows {
            if !w.outgoing || !live.contains(&w.key) {
                self.presented
                    .insert(w.key, (w.pose, frame.timestamp, frame.animation_time));
            }
        }
        for g in &frame.workspaces {
            self.presented_groups
                .insert(g.group, (g.clone(), frame.timestamp, frame.animation_time));
        }
        self.acknowledged_tracks.clear();
        for motion in self.live.values() {
            if sources.contains(&motion.source) {
                if let Some(sample) = frame.sampled_tracks.get(&motion.track) {
                    self.acknowledged_tracks.insert(motion.track, *sample);
                }
            }
        }
        for track in self.outgoing.values().map(|m| m.track).chain(
            self.groups
                .values()
                .flat_map(|m| [m.incoming_track, m.outgoing_track]),
        ) {
            if let Some(sample) = frame.sampled_tracks.get(&track) {
                self.acknowledged_tracks.insert(track, *sample);
            }
        }
        self.acknowledged = Some((frame.timestamp, frame.animation_time));
        Ok(())
    }

    /// Drop finished tracks and return retained images safe for adapter release.
    pub fn retire_finished(
        &mut self,
        now: Duration,
        engine: &mut AnimationEngine,
    ) -> Vec<RetainedId> {
        let finished = |track: u64| {
            engine.sample(track, now).is_none_or(|s| s.finished)
                && engine.revision(track).is_none_or(|revision| {
                    self.acknowledged_tracks.get(&track) == Some(&(revision, true))
                })
        };
        let keys: Vec<_> = self
            .live
            .iter()
            .filter_map(|(k, m)| finished(m.track).then_some(*k))
            .collect();
        let outgoing: Vec<_> = self
            .outgoing
            .iter()
            .filter_map(|(id, m)| finished(m.track).then_some(*id))
            .collect();
        let groups: Vec<_> = self
            .groups
            .iter()
            .filter_map(|(id, m)| {
                [m.incoming_track, m.outgoing_track]
                    .iter()
                    .all(|track| finished(*track))
                    .then_some(*id)
            })
            .collect();
        for key in keys {
            self.remove_live(key, engine);
        }
        for id in &outgoing {
            self.remove_outgoing(*id, engine);
        }
        for id in groups {
            self.remove_group(id, engine);
        }
        let live_keys: BTreeSet<_> = self
            .snapshot
            .windows
            .iter()
            .filter(|w| w.visible())
            .map(|w| w.key)
            .collect();
        let retained_keys: BTreeSet<_> = self.outgoing.values().map(|m| m.key).collect();
        self.presented
            .retain(|key, _| live_keys.contains(key) || retained_keys.contains(key));
        outgoing
    }

    /// Settle a directly manipulated window and cancel any retained outgoing image.
    pub fn settle_window(
        &mut self,
        key: WindowKey,
        engine: &mut AnimationEngine,
    ) -> Vec<RetainedId> {
        self.remove_live(key, engine);
        let ids: Vec<_> = self
            .outgoing
            .iter()
            .filter_map(|(id, m)| (m.key == key).then_some(*id))
            .collect();
        for id in &ids {
            self.remove_outgoing(*id, engine);
        }
        ids
    }

    /// Release all presentation bookkeeping on renderer/topology reset or shutdown.
    pub fn clear(&mut self, engine: &mut AnimationEngine) -> Vec<RetainedId> {
        let mut tracks = BTreeSet::new();
        tracks.extend(self.live.values().map(|m| m.track));
        tracks.extend(self.outgoing.values().map(|m| m.track));
        tracks.extend(
            self.groups
                .values()
                .flat_map(|m| [m.incoming_track, m.outgoing_track]),
        );
        for track in tracks {
            engine.remove(track);
        }
        let release = self.outgoing.keys().copied().collect();
        self.live.clear();
        self.outgoing.clear();
        self.groups.clear();
        self.presented.clear();
        self.presented_groups.clear();
        self.acknowledged = None;
        self.acknowledged_tracks.clear();
        release
    }

    /// Whether motion or an unacknowledged terminal pose still needs a frame.
    pub fn is_animating(&self, now: Duration, engine: &AnimationEngine) -> bool {
        let needs_frame = |track: u64| {
            engine.sample(track, now).is_some_and(|s| !s.finished)
                || engine.revision(track).is_some_and(|revision| {
                    self.acknowledged_tracks.get(&track) != Some(&(revision, true))
                })
        };
        self.live
            .values()
            .chain(self.outgoing.values())
            .any(|m| needs_frame(m.track))
            || self.groups.values().any(|m| {
                [m.incoming_track, m.outgoing_track]
                    .iter()
                    .any(|track| needs_frame(*track))
            })
    }

    fn allocate(&mut self) -> u64 {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("presentation identity exhausted");
        self.next_id
    }
    fn allocate_track(&mut self, engine: &AnimationEngine) -> u64 {
        loop {
            let id = self.allocate();
            if !engine.contains(id) {
                return id;
            }
        }
    }
    fn last_pose(
        &self,
        key: WindowKey,
        fallback: AnimationPose,
        animation_time: f64,
    ) -> (AnimationPose, f64) {
        self.presented
            .get(&key)
            .map(|(pose, _, phase)| (*pose, *phase))
            .unwrap_or((fallback, animation_time))
    }
    fn remove_live(&mut self, key: WindowKey, engine: &mut AnimationEngine) {
        if let Some(m) = self.live.remove(&key) {
            engine.remove(m.track);
        }
    }
    fn remove_group(&mut self, id: u64, engine: &mut AnimationEngine) {
        if let Some(m) = self.groups.remove(&id) {
            engine.remove(m.incoming_track);
            engine.remove(m.outgoing_track);
        }
        self.presented_groups.remove(&id);
    }
    fn remove_outgoing(&mut self, id: RetainedId, engine: &mut AnimationEngine) {
        if let Some(m) = self.outgoing.remove(&id) {
            engine.remove(m.track);
        }
    }
}

fn motion_enabled(engine: &AnimationEngine, effect: AnimationEffect) -> bool {
    let c = engine.config();
    c.enabled && !c.reduced_motion && c.effect(effect).enabled
}
fn window_hint(causes: &[TransitionCause], key: WindowKey) -> Option<WindowTransitionKind> {
    causes.iter().rev().find_map(|c| match c {
        TransitionCause::Window { key: k, kind } if *k == key => Some(*kind),
        _ => None,
    })
}
fn pose(r: Rect) -> AnimationPose {
    AnimationPose {
        rect: AnimationRect {
            x: f64::from(r.x),
            y: f64::from(r.y),
            width: f64::from(r.width),
            height: f64::from(r.height),
        },
        opacity: 1.0,
    }
}
fn offset_pose(offset: (f64, f64)) -> AnimationPose {
    AnimationPose {
        rect: AnimationRect {
            x: offset.0,
            y: offset.1,
            width: 1.0,
            height: 1.0,
        },
        opacity: 1.0,
    }
}
fn scaled(p: AnimationPose, scale: f64, opacity: f64) -> AnimationPose {
    AnimationPose {
        rect: AnimationRect {
            x: p.rect.x + p.rect.width * (1.0 - scale) * 0.5,
            y: p.rect.y + p.rect.height * (1.0 - scale) * 0.5,
            width: p.rect.width * scale,
            height: p.rect.height * scale,
        },
        opacity,
    }
}
fn translated(mut p: AnimationPose, x: f64, y: f64) -> AnimationPose {
    p.rect.x += x;
    p.rect.y += y;
    p
}
fn output_scale(s: &PresentationSnapshot, id: OutputId) -> f64 {
    s.outputs
        .iter()
        .find(|o| o.id == id)
        .map_or(1.0, |o| o.physical_scale)
}
fn group_for_output(s: &PresentationSnapshot, id: OutputId) -> Option<&GroupState> {
    s.groups.iter().find(|g| g.outputs.contains(&id))
}
fn group_bounds(g: &GroupState, s: &PresentationSnapshot) -> Rect {
    let outputs: Vec<_> = s
        .outputs
        .iter()
        .filter(|o| g.outputs.contains(&o.id))
        .collect();
    let left = outputs
        .iter()
        .map(|o| i64::from(o.bounds.x))
        .min()
        .unwrap_or(0);
    let top = outputs
        .iter()
        .map(|o| i64::from(o.bounds.y))
        .min()
        .unwrap_or(0);
    let right = outputs
        .iter()
        .map(|o| i64::from(o.bounds.x) + i64::from(o.bounds.width))
        .max()
        .unwrap_or(1);
    let bottom = outputs
        .iter()
        .map(|o| i64::from(o.bounds.y) + i64::from(o.bounds.height))
        .max()
        .unwrap_or(1);
    Rect::new(
        left as i32,
        top as i32,
        (right - left).min(i64::from(i32::MAX)) as i32,
        (bottom - top).min(i64::from(i32::MAX)) as i32,
    )
}
fn minimize_pose(w: &WindowState, s: &PresentationSnapshot, rect: Rect) -> AnimationPose {
    let output = s.outputs.iter().find(|o| o.id == w.home_output);
    let explicit = w
        .minimize_target
        .and_then(|r| output.and_then(|o| r.intersection(o.bounds)))
        .or_else(|| output.and_then(|o| o.panel.and_then(|r| r.intersection(o.bounds))));
    let anchor = explicit.unwrap_or_else(|| {
        output.map_or(
            Rect::new(
                rect.x,
                rect.y.saturating_add(rect.height.saturating_sub(1)),
                16,
                16,
            ),
            |o| {
                let width = o.bounds.width.min(16);
                let height = o.bounds.height.min(16);
                Rect::new(
                    o.bounds.x.saturating_add((o.bounds.width - width) / 2),
                    o.bounds.bottom().saturating_sub(height),
                    width,
                    height,
                )
            },
        )
    });
    let center_x = if explicit.is_none() {
        output.map_or(f64::from(anchor.x) + f64::from(anchor.width) * 0.5, |o| {
            f64::from(o.bounds.x) + f64::from(o.bounds.width) * 0.5
        })
    } else {
        f64::from(anchor.x) + f64::from(anchor.width) * 0.5
    };
    let ratio = (f64::from(anchor.width) / f64::from(rect.width))
        .min(f64::from(anchor.height) / f64::from(rect.height))
        .min(1.0);
    let width = f64::from(rect.width) * ratio;
    let height = f64::from(rect.height) * ratio;
    AnimationPose {
        rect: AnimationRect {
            x: center_x - width * 0.5,
            y: f64::from(anchor.y) + f64::from(anchor.height) * 0.5 - height * 0.5,
            width,
            height,
        },
        opacity: 0.0,
    }
}
fn validate_snapshot(s: &PresentationSnapshot) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    let mut windows = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    let mut groups = BTreeSet::new();
    let mut workspaces = BTreeSet::new();
    let mut memberships = BTreeSet::new();
    for o in &s.outputs {
        if !outputs.insert(o.id)
            || !valid_rect(o.bounds)
            || !o.physical_scale.is_finite()
            || o.physical_scale <= 0.0
            || o.panel.is_some_and(|r| !valid_rect(r))
        {
            return Err("presentation outputs need unique IDs, positive geometry, and finite positive scale".into());
        }
    }
    for g in &s.groups {
        if !groups.insert(g.id)
            || !workspaces.insert(g.workspace)
            || g.outputs.is_empty()
            || g.outputs
                .iter()
                .any(|id| !outputs.contains(id) || !memberships.insert(*id))
        {
            return Err(
                "presentation groups need unique IDs and exclusive connected outputs".into(),
            );
        }
    }
    if memberships.len() != outputs.len() {
        return Err("presentation groups must cover every connected output".into());
    }
    for w in &s.windows {
        if !keys.insert(w.key)
            || !windows.insert(w.key.window)
            || w.placement.as_ref().is_some_and(|p| {
                p.window != w.key.window
                    || !valid_rect(p.rect)
                    || p.clip.is_some_and(|r| !valid_rect(r))
            })
            || w.minimize_target.is_some_and(|r| !valid_rect(r))
            || (w.mapped && !w.live)
            || (w.placement.is_some() && (!w.live || !w.mapped || w.minimized))
        {
            return Err(
                "presentation windows need unique generations and valid placement/anchor geometry"
                    .into(),
            );
        }
        if w.visible() && !outputs.contains(&w.home_output) {
            return Err("visible presentation window needs a connected home output".into());
        }
        if w.visible()
            && group_for_output(s, w.home_output).is_none_or(|g| g.workspace != w.workspace)
        {
            return Err(
                "visible presentation window must belong to its home output's presented workspace"
                    .into(),
            );
        }
    }
    Ok(())
}

fn valid_rect(rect: Rect) -> bool {
    !rect.is_empty() && rect.normalized() == rect
}
