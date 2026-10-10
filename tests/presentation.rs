use clear::{
    config::{AnimationEffect, AnimationsConfig},
    core::{OutputId, Placement, Rect, WindowId, WorkspaceId},
    runtime::{animation::AnimationEngine, presentation::*},
};
use std::time::Duration;
fn t(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}
fn key(id: u64) -> WindowKey {
    WindowKey {
        window: WindowId(id),
        generation: 1,
    }
}
fn window(id: u64, workspace: u64, rect: Option<Rect>) -> WindowState {
    WindowState {
        key: key(id),
        live: true,
        mapped: true,
        minimized: false,
        maximized: false,
        fullscreen: false,
        home_output: OutputId(1),
        workspace: WorkspaceId(workspace),
        placement: rect.map(|rect| Placement {
            window: WindowId(id),
            rect,
            clip: Some(Rect::new(0, 0, 800, 600)),
            focused: id == 1,
            tiled: true,
        }),
        minimize_target: None,
    }
}
fn snapshot(windows: Vec<WindowState>) -> PresentationSnapshot {
    PresentationSnapshot {
        windows,
        outputs: vec![OutputState {
            id: OutputId(1),
            bounds: Rect::new(0, 0, 800, 600),
            panel: None,
            physical_scale: 1.0,
        }],
        groups: vec![GroupState {
            id: 1,
            workspace: WorkspaceId(1),
            outputs: vec![OutputId(1)],
        }],
    }
}
fn enabled() -> AnimationsConfig {
    AnimationsConfig {
        enabled: true,
        ..Default::default()
    }
}
fn setup(s: PresentationSnapshot) -> (PresentationPlanner, AnimationEngine) {
    let mut p = PresentationPlanner::new();
    let mut e = AnimationEngine::new(enabled(), Duration::ZERO).unwrap();
    p.reconcile(s, &[TransitionCause::Settle], Duration::ZERO, &mut e)
        .unwrap();
    let frame = p.sample(Duration::ZERO, &e);
    p.mark_presented(&frame).unwrap();
    (p, e)
}
fn rect() -> Rect {
    Rect::new(100, 100, 200, 100)
}

#[test]
fn opening_scales_about_center_and_fades_to_exact_policy_placement() {
    let (mut p, mut e) = setup(snapshot(vec![]));
    let s = snapshot(vec![window(1, 1, Some(rect()))]);
    assert!(
        p.reconcile(s.clone(), &[], t(0.0), &mut e)
            .unwrap()
            .retain
            .is_empty()
    );
    let frame = p.sample(t(0.0), &e);
    let w = &frame.windows[0];
    assert_eq!(w.effect, Some(AnimationEffect::WindowOpen));
    assert_eq!(w.pose.opacity, 0.0);
    assert!((w.pose.rect.width - 208.0).abs() < 1e-9);
    assert_eq!(w.pose.rect.x, 96.0);
    let middle = p.sample(t(0.075), &e);
    assert!(middle.windows[0].pose.opacity > 0.0 && middle.windows[0].pose.opacity < 1.0);
    let last = p.sample(t(0.15), &e);
    assert_eq!(last.windows[0].pose.rect.x, 100.0);
    assert_eq!(last.windows[0].pose.rect.width, 200.0);
    assert_eq!(last.windows[0].pose.opacity, 1.0);
    assert!(last.windows[0].finished);
    assert_eq!(s.windows[0].placement.as_ref().unwrap().rect, rect());
    p.mark_presented(&last).unwrap();
    p.retire_finished(t(0.15), &mut e);
    assert!(e.is_empty());
}

#[test]
fn destruction_uses_one_retained_generation_and_is_never_interactive() {
    let (mut p, mut e) = setup(snapshot(vec![window(1, 1, Some(rect()))]));
    let update = p.reconcile(snapshot(vec![]), &[], t(0.0), &mut e).unwrap();
    assert_eq!(update.retain.len(), 1);
    assert_eq!(update.retain[0].key, key(1));
    let w = &p.sample(t(0.05), &e).windows[0];
    assert_eq!(w.source, PresentationSource::Retained(update.retain[0].id));
    assert!(!w.input_eligible && w.outgoing);
    assert_eq!(w.effect, Some(AnimationEffect::WindowClose));
    assert!(w.pose.rect.width > 200.0 && w.pose.opacity < 1.0);
    assert!(
        p.reconcile(snapshot(vec![]), &[], t(0.06), &mut e)
            .unwrap()
            .retain
            .is_empty()
    );
    assert!(p.sample(t(0.2), &e).windows.is_empty());
    p.mark_presented(&p.sample(t(0.2), &e)).unwrap();
    assert_eq!(p.retire_finished(t(0.2), &mut e), vec![update.retain[0].id]);
    assert!(e.is_empty());
}

#[test]
fn remapping_keeps_old_retained_generation_distinct_from_new_live_source() {
    let (mut p, mut e) = setup(snapshot(vec![window(1, 1, Some(rect()))]));
    let mut remapped = window(1, 1, Some(rect()));
    remapped.key.generation = 2;
    let u = p
        .reconcile(snapshot(vec![remapped.clone()]), &[], t(0.01), &mut e)
        .unwrap();
    assert_eq!(u.retain.len(), 1);
    let frame = p.sample(t(0.02), &e);
    assert_eq!(frame.windows.len(), 2);
    assert_eq!(
        frame.windows[0].source,
        PresentationSource::Live(remapped.key)
    );
    assert_eq!(frame.windows[1].key, key(1));
    assert_ne!(frame.windows[0].key, frame.windows[1].key);
}

#[test]
fn reflow_and_retarget_begin_at_last_displayed_pose_with_one_group_timestamp() {
    let base = snapshot(vec![
        window(1, 1, Some(rect())),
        window(2, 1, Some(Rect::new(400, 100, 200, 100))),
    ]);
    let (mut p, mut e) = setup(base.clone());
    let mut moved = base.clone();
    moved.windows[0].placement.as_mut().unwrap().rect.x = 200;
    moved.windows[1].placement.as_mut().unwrap().rect.x = 500;
    p.reconcile(moved.clone(), &[], t(0.01), &mut e).unwrap();
    let shown = p.sample(t(0.04), &e);
    p.mark_presented(&shown).unwrap();
    assert!((shown.windows[1].pose.rect.x - shown.windows[0].pose.rect.x - 300.0).abs() < 1e-9);
    moved.windows[0].placement.as_mut().unwrap().rect.x = 50;
    p.reconcile(moved, &[], t(0.1), &mut e).unwrap();
    let restarted = p.sample(t(0.1), &e);
    assert_eq!(restarted.windows[0].pose, shown.windows[0].pose);
}

#[test]
fn minimize_uses_icon_hint_then_reverses_from_displayed_outgoing_pose() {
    let mut base = window(1, 1, Some(rect()));
    base.minimize_target = Some(Rect::new(400, 570, 20, 20));
    let (mut p, mut e) = setup(snapshot(vec![base.clone()]));
    let mut hidden = base.clone();
    hidden.minimized = true;
    hidden.placement = None;
    let u = p
        .reconcile(snapshot(vec![hidden]), &[], t(0.01), &mut e)
        .unwrap();
    assert_eq!(u.retain.len(), 1);
    let middle = p.sample(t(0.1), &e);
    p.mark_presented(&middle).unwrap();
    assert!(!middle.windows[0].input_eligible);
    assert_eq!(middle.windows[0].effect, Some(AnimationEffect::Minimize));
    assert!(middle.windows[0].pose.rect.width < 200.0);
    assert!(middle.windows[0].pose.rect.y > 100.0);
    let u = p
        .reconcile(snapshot(vec![base]), &[], t(0.12), &mut e)
        .unwrap();
    assert_eq!(u.release.len(), 1);
    let restore = p.sample(t(0.12), &e);
    assert_eq!(restore.windows.len(), 1);
    assert_eq!(restore.windows[0].pose, middle.windows[0].pose);
    assert_eq!(restore.windows[0].source, PresentationSource::Live(key(1)));
}

#[test]
fn completed_minimize_restore_uses_panel_or_bottom_fallback_not_stale_desktop() {
    for panel in [Some(Rect::new(0, 570, 800, 30)), None] {
        let base = window(1, 1, Some(rect()));
        let mut s = snapshot(vec![base.clone()]);
        s.outputs[0].panel = panel;
        let (mut p, mut e) = setup(s.clone());
        s.windows[0].minimized = true;
        s.windows[0].placement = None;
        p.reconcile(s.clone(), &[], t(0.01), &mut e).unwrap();
        let frame = p.sample(t(0.1), &e);
        p.mark_presented(&frame).unwrap();
        p.mark_presented(&p.sample(t(0.3), &e)).unwrap();
        p.retire_finished(t(0.3), &mut e);
        s.windows[0] = base;
        p.reconcile(s, &[], t(0.4), &mut e).unwrap();
        let frame = p.sample(t(0.4), &e);
        assert_eq!(frame.windows[0].pose.opacity, 0.0);
        assert!(frame.windows[0].pose.rect.y >= 570.0);
        assert!(frame.windows[0].pose.rect.width < 100.0);
    }
}

#[test]
fn maximize_fullscreen_and_restore_select_their_own_geometry_laws() {
    let base = snapshot(vec![window(1, 1, Some(rect()))]);
    let (mut p, mut e) = setup(base.clone());
    let mut s = base.clone();
    s.windows[0].maximized = true;
    s.windows[0].placement.as_mut().unwrap().rect = Rect::new(0, 0, 800, 570);
    p.reconcile(s.clone(), &[], t(0.0), &mut e).unwrap();
    assert_eq!(
        p.sample(t(0.01), &e).windows[0].effect,
        Some(AnimationEffect::Maximize)
    );
    s.windows[0].fullscreen = true;
    s.windows[0].placement.as_mut().unwrap().rect = Rect::new(0, 0, 800, 600);
    p.reconcile(s, &[], t(0.1), &mut e).unwrap();
    assert_eq!(
        p.sample(t(0.11), &e).windows[0].effect,
        Some(AnimationEffect::Fullscreen)
    );
    p.reconcile(base, &[], t(0.2), &mut e).unwrap();
    assert_eq!(
        p.sample(t(0.21), &e).windows[0].effect,
        Some(AnimationEffect::Fullscreen)
    );
    assert_eq!(p.sample(t(3.0), &e).windows[0].pose.rect.width, 200.0);
}

fn workspace_states() -> (PresentationSnapshot, PresentationSnapshot) {
    let before = snapshot(vec![window(1, 1, Some(rect())), window(2, 2, None)]);
    let mut after = before.clone();
    after.groups[0].workspace = WorkspaceId(2);
    after.windows[0].placement = None;
    after.windows[1].placement = Some(Placement {
        window: WindowId(2),
        rect: rect(),
        clip: Some(Rect::new(0, 0, 800, 600)),
        focused: true,
        tiled: true,
    });
    (before, after)
}

#[test]
fn workspace_slides_expose_output_local_live_and_noninteractive_outgoing_records() {
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before);
    let update = p.reconcile(after, &[], t(0.0), &mut e).unwrap();
    assert_eq!(update.retain.len(), 1);
    let frame = p.sample(t(0.0), &e);
    assert_eq!(frame.windows.len(), 2);
    assert_eq!(frame.windows[0].key, key(2));
    assert_eq!(frame.windows[0].pose.rect.x, 900.0);
    assert!(!frame.windows[0].input_eligible);
    assert!(frame.windows[1].outgoing && !frame.windows[1].input_eligible);
    assert_eq!(frame.windows[1].pose.rect.x, 100.0);
    assert_eq!(frame.workspaces[0].incoming_offset, (800.0, 0.0));
    let final_frame = p.sample(t(2.0), &e);
    assert_eq!(final_frame.windows.len(), 1);
    assert_eq!(final_frame.windows[0].pose.rect.x, 100.0);
    assert!(final_frame.windows[0].input_eligible);
}

#[test]
fn workspace_direction_extends_to_vertical_and_reversal_uses_last_shown_pose() {
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before.clone());
    p.reconcile(
        after,
        &[TransitionCause::WorkspaceSwitch {
            group: 1,
            direction: WorkspaceDirection { x: 0.0, y: 1.0 },
        }],
        t(0.0),
        &mut e,
    )
    .unwrap();
    let frame = p.sample(t(0.08), &e);
    assert!(frame.windows[0].pose.rect.y > 100.0);
    p.mark_presented(&frame).unwrap();
    let outgoing_pose = frame.windows[1].pose;
    let update = p
        .reconcile(
            before,
            &[TransitionCause::WorkspaceSwitch {
                group: 1,
                direction: WorkspaceDirection { x: 0.0, y: -1.0 },
            }],
            t(0.1),
            &mut e,
        )
        .unwrap();
    assert_eq!(update.release.len(), 1);
    assert_eq!(p.sample(t(0.1), &e).windows[0].pose, outgoing_pose);
    assert!(e.len() <= 256);
}

#[test]
fn disabled_reduced_motion_and_track_budget_never_hide_final_live_windows() {
    for config in [
        AnimationsConfig::default(),
        AnimationsConfig {
            reduced_motion: true,
            ..enabled()
        },
        enabled(),
    ] {
        let mut p = PresentationPlanner::new();
        let mut e = AnimationEngine::new(config.clone(), t(0.0)).unwrap();
        let s = snapshot((1..=300).map(|id| window(id, 1, Some(rect()))).collect());
        let update = p.reconcile(s, &[], t(0.0), &mut e).unwrap();
        assert!(update.retain.is_empty());
        let frame = p.sample(t(0.0), &e);
        assert_eq!(frame.windows.len(), 300);
        assert!(e.len() <= 256);
        if !config.enabled || config.reduced_motion {
            assert!(frame.windows.iter().all(|w| w.pose.opacity == 1.0));
            assert!(!p.is_animating(t(0.0), &e));
        }
        let update = p.reconcile(snapshot(vec![]), &[], t(0.01), &mut e).unwrap();
        assert!(update.retain.len() <= 256);
        assert!(e.len() <= 256);
    }
}

#[test]
fn direct_motion_settle_and_output_loss_release_retained_sources() {
    let base = snapshot(vec![window(1, 1, Some(rect()))]);
    let (mut p, mut e) = setup(base.clone());
    let mut moved = base.clone();
    moved.windows[0].placement.as_mut().unwrap().rect.x = 400;
    p.reconcile(
        moved,
        &[TransitionCause::Window {
            key: key(1),
            kind: WindowTransitionKind::Direct,
        }],
        t(0.0),
        &mut e,
    )
    .unwrap();
    assert!(e.is_empty());
    assert_eq!(p.sample(t(0.0), &e).windows[0].pose.rect.x, 400.0);
    let update = p.reconcile(snapshot(vec![]), &[], t(0.01), &mut e).unwrap();
    assert_eq!(update.retain.len(), 1);
    let update = p
        .reconcile(PresentationSnapshot::default(), &[], t(0.02), &mut e)
        .unwrap();
    assert_eq!(update.release.len(), 1);
    assert!(p.sample(t(0.02), &e).windows.is_empty());
    p.reconcile(base, &[TransitionCause::Settle], t(0.03), &mut e)
        .unwrap();
    assert!(e.is_empty());
    assert_eq!(p.sample(t(0.03), &e).windows[0].pose.opacity, 1.0);
}

#[test]
fn malformed_snapshot_and_direction_leave_presentation_unchanged() {
    let base = snapshot(vec![window(1, 1, Some(rect()))]);
    let (mut p, mut e) = setup(base.clone());
    let frame = p.sample(t(0.0), &e);
    let mut invalid = base.clone();
    invalid.windows.push(invalid.windows[0].clone());
    assert!(p.reconcile(invalid, &[], t(0.0), &mut e).is_err());
    assert!(
        p.reconcile(
            base,
            &[TransitionCause::WorkspaceSwitch {
                group: 1,
                direction: WorkspaceDirection {
                    x: f64::NAN,
                    y: 0.0
                }
            }],
            t(0.0),
            &mut e
        )
        .is_err()
    );
    assert_eq!(p.sample(t(0.0), &e), frame);
}

#[test]
fn displayed_frame_rejects_stale_duplicate_invalid_and_backwards_records_atomically() {
    let base = snapshot(vec![window(1, 1, Some(rect()))]);
    let (mut p, mut e) = setup(base.clone());
    let valid = p.sample(t(0.1), &e);
    p.mark_presented(&valid).unwrap();
    let mut duplicate = valid.clone();
    duplicate.windows.push(duplicate.windows[0].clone());
    assert!(p.mark_presented(&duplicate).is_err());
    let mut invalid = valid.clone();
    invalid.windows[0].pose.rect.width = f64::NAN;
    assert!(p.mark_presented(&invalid).is_err());
    let mut backwards = valid.clone();
    backwards.timestamp = t(0.05);
    assert!(p.mark_presented(&backwards).is_err());
    let mut phase = valid.clone();
    phase.animation_time = f64::NAN;
    assert!(p.mark_presented(&phase).is_err());
    let mut s = base;
    s.windows[0].key.generation = 2;
    p.reconcile(s, &[], t(0.2), &mut e).unwrap();
    assert!(p.mark_presented(&valid).is_err());
}

#[test]
fn disabled_reload_settles_all_visuals_and_reenable_does_not_replay() {
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before);
    let u = p.reconcile(after.clone(), &[], t(0.0), &mut e).unwrap();
    assert_eq!(u.retain.len(), 1);
    let unfinished = p.sample(t(0.025), &e);
    p.mark_presented(&unfinished).unwrap();
    e.apply_config(AnimationsConfig::default(), t(0.05))
        .unwrap();
    assert!(p.is_animating(t(0.05), &e));
    assert!(p.retire_finished(t(0.05), &mut e).is_empty());
    // A delayed acknowledgement from before the reload is still not terminal.
    p.mark_presented(&unfinished).unwrap();
    assert!(p.is_animating(t(0.05), &e));
    assert!(p.retire_finished(t(0.05), &mut e).is_empty());
    let final_frame = p.sample(t(0.05), &e);
    assert_eq!(final_frame.windows.len(), 1);
    assert_eq!(final_frame.windows[0].pose.rect.x, 100.0);
    assert!(final_frame.workspaces.is_empty());
    p.mark_presented(&final_frame).unwrap();
    assert!(!p.is_animating(t(0.05), &e));
    assert_eq!(p.retire_finished(t(0.05), &mut e).len(), 1);
    e.apply_config(enabled(), t(0.1)).unwrap();
    assert!(
        p.reconcile(after, &[], t(0.1), &mut e)
            .unwrap()
            .retain
            .is_empty()
    );
    assert!(!p.is_animating(t(0.1), &e));
}

#[test]
fn group_identity_repair_drops_obsolete_workspace_visuals() {
    let (before, mut after) = workspace_states();
    let (mut p, mut e) = setup(before);
    let u = p.reconcile(after.clone(), &[], t(0.0), &mut e).unwrap();
    assert_eq!(u.retain.len(), 1);
    after.groups[0].id = 99;
    let u = p.reconcile(after, &[], t(0.05), &mut e).unwrap();
    assert_eq!(u.release.len(), 1);
    let frame = p.sample(t(0.05), &e);
    assert_eq!(frame.windows.len(), 1);
    assert!(frame.workspaces.is_empty());
    assert_eq!(frame.windows[0].pose.rect.x, 100.0);
}

#[test]
fn invalid_group_coverage_duplicate_generation_and_raw_overflow_rectangles_are_rejected() {
    let base = snapshot(vec![window(1, 1, Some(rect()))]);
    let (mut p, mut e) = setup(base.clone());
    let mut s = base.clone();
    s.groups.clear();
    assert!(p.reconcile(s, &[], t(0.0), &mut e).is_err());
    let mut s = base.clone();
    let mut duplicate = s.windows[0].clone();
    duplicate.key.generation = 2;
    s.windows.push(duplicate);
    assert!(p.reconcile(s, &[], t(0.0), &mut e).is_err());
    let mut s = base.clone();
    s.outputs[0].bounds = Rect {
        x: i32::MAX,
        y: 0,
        width: 100,
        height: 100,
    };
    assert!(p.reconcile(s, &[], t(0.0), &mut e).is_err());
    let mut s = base;
    s.windows[0].mapped = false;
    assert!(p.reconcile(s, &[], t(0.0), &mut e).is_err());
}

#[test]
fn shared_engine_track_ids_and_visible_workspace_ownership_are_preserved() {
    use clear::runtime::animation::{AnimationPose, AnimationRect};
    let mut e = AnimationEngine::new(enabled(), t(0.0)).unwrap();
    let from = AnimationPose {
        rect: AnimationRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
        opacity: 1.0,
    };
    let mut to = from;
    to.rect.x = 200.0;
    e.retarget(
        1,
        999,
        AnimationEffect::WindowMovement,
        from,
        to,
        t(0.0),
        1.0,
    )
    .unwrap();
    let before = e.sample(1, t(0.05));
    let mut p = PresentationPlanner::new();
    let base = snapshot(vec![window(1, 1, Some(rect()))]);
    p.reconcile(base.clone(), &[], t(0.0), &mut e).unwrap();
    assert_eq!(e.sample(1, t(0.05)), before);
    p.clear(&mut e);
    assert!(e.contains(1));
    let mut invalid = base;
    invalid.windows[0].workspace = WorkspaceId(2);
    assert!(p.reconcile(invalid, &[], t(0.1), &mut e).is_err());
}

#[test]
fn workspace_offsets_and_spring_velocities_reverse_continuously_after_speed_reload() {
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before.clone());
    p.reconcile(after, &[], t(0.0), &mut e).unwrap();
    let dt = 0.0000001;
    let shown = p.sample(t(0.04), &e);
    let next = p.sample(t(0.04 + dt), &e);
    p.mark_presented(&shown).unwrap();
    let old_group = &shown.workspaces[0];
    let group_velocity = (next.workspaces[0].outgoing_offset.0 - old_group.outgoing_offset.0) / dt;
    let old_window = shown.windows.iter().find(|w| w.key == key(1)).unwrap();
    let old_window_velocity = (next
        .windows
        .iter()
        .find(|w| w.key == key(1))
        .unwrap()
        .pose
        .rect
        .x
        - old_window.pose.rect.x)
        / dt;
    let mut config = enabled();
    config.speed = 2.0;
    e.apply_config(config, t(0.05)).unwrap();
    p.reconcile(before, &[], t(0.06), &mut e).unwrap();
    let reversed = p.sample(t(0.06), &e);
    let group = &reversed.workspaces[0];
    assert_eq!(group.incoming_offset, old_group.outgoing_offset);
    assert_eq!(group.outgoing_offset, old_group.incoming_offset);
    let incoming = reversed.windows.iter().find(|w| w.key == key(1)).unwrap();
    assert_eq!(incoming.pose, old_window.pose);
    let step = p.sample(t(0.06 + dt / 2.0), &e);
    let new_group_velocity = (step.workspaces[0].incoming_offset.0 - group.incoming_offset.0) / dt;
    let new_window_velocity = (step
        .windows
        .iter()
        .find(|w| w.key == key(1))
        .unwrap()
        .pose
        .rect
        .x
        - incoming.pose.rect.x)
        / dt;
    assert!(
        (group_velocity - new_group_velocity).abs() < 0.25,
        "{group_velocity} vs {new_group_velocity}"
    );
    assert!((old_window_velocity - new_window_velocity).abs() < 0.25);
}

#[test]
fn missed_terminal_frame_retains_offsets_and_requests_one_more_frame_without_stale_replay() {
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before.clone());
    p.reconcile(after.clone(), &[], t(0.0), &mut e).unwrap();
    let shown = p.sample(t(0.04), &e);
    p.mark_presented(&shown).unwrap();
    assert!(p.sample(t(3.0), &e).workspaces.is_empty());
    assert!(p.is_animating(t(3.0), &e));
    assert!(p.retire_finished(t(3.0), &mut e).is_empty());
    p.reconcile(before.clone(), &[], t(3.0), &mut e).unwrap();
    let reversed = p.sample(t(3.0), &e);
    assert_eq!(
        reversed.workspaces[0].incoming_offset,
        shown.workspaces[0].outgoing_offset
    );
    assert_eq!(reversed.windows[0].pose, shown.windows[1].pose);
    let terminal = p.sample(t(6.0), &e);
    p.mark_presented(&terminal).unwrap();
    assert!(!p.is_animating(t(6.0), &e));
    p.retire_finished(t(6.0), &mut e);
    p.reconcile(after, &[], t(6.01), &mut e).unwrap();
    let fresh = p.sample(t(6.01), &e);
    assert_eq!(fresh.workspaces[0].incoming_offset, (800.0, 0.0));
    assert_eq!(fresh.workspaces[0].outgoing_offset, (0.0, 0.0));
}

#[test]
fn minimize_restore_preserves_critical_spring_velocity_after_analytic_deadline_gap() {
    use clear::config::AnimationKind;
    let base = window(1, 1, Some(rect()));
    let (mut p, mut e) = setup(snapshot(vec![base.clone()]));
    let mut config = enabled();
    config.minimize.kind = AnimationKind::Spring { stiffness: 800.0 };
    e.apply_config(config, t(0.0)).unwrap();
    let mut hidden = base.clone();
    hidden.minimized = true;
    hidden.placement = None;
    p.reconcile(snapshot(vec![hidden]), &[], t(0.0), &mut e)
        .unwrap();
    let dt = 0.0000001;
    let shown = p.sample(t(0.04), &e);
    let step = p.sample(t(0.04 + dt), &e);
    p.mark_presented(&shown).unwrap();
    let velocity = (step.windows[0].pose.rect.x - shown.windows[0].pose.rect.x) / dt;
    assert!(p.retire_finished(t(3.0), &mut e).is_empty());
    p.reconcile(snapshot(vec![base]), &[], t(3.0), &mut e)
        .unwrap();
    let restored = p.sample(t(3.0), &e);
    assert_eq!(restored.windows[0].pose, shown.windows[0].pose);
    let new_velocity =
        (p.sample(t(3.0 + dt), &e).windows[0].pose.rect.x - restored.windows[0].pose.rect.x) / dt;
    assert!((velocity - new_velocity).abs() < 0.25);
}

#[test]
fn bottom_center_anchor_is_exact_and_stays_inside_tiny_or_odd_outputs() {
    for (width, height) in [(800, 600), (17, 9), (1, 1)] {
        let base = window(1, 1, Some(rect()));
        let mut s = snapshot(vec![base.clone()]);
        s.outputs[0].bounds = Rect::new(21, 31, width, height);
        let (mut p, mut e) = setup(s.clone());
        s.windows[0].minimized = true;
        s.windows[0].placement = None;
        p.reconcile(s.clone(), &[], t(0.0), &mut e).unwrap();
        p.mark_presented(&p.sample(t(0.3), &e)).unwrap();
        p.retire_finished(t(0.3), &mut e);
        s.windows[0] = base;
        p.reconcile(s, &[], t(0.4), &mut e).unwrap();
        let pose = p.sample(t(0.4), &e).windows[0].pose;
        assert_eq!(pose.opacity, 0.0);
        assert!(
            (pose.rect.x + pose.rect.width / 2.0 - (21.0 + f64::from(width) / 2.0)).abs() < 1e-9
        );
        assert!(pose.rect.width <= f64::from(width) && pose.rect.height <= f64::from(height));
        assert!(pose.rect.y >= 31.0 && pose.rect.y + pose.rect.height <= 31.0 + f64::from(height));
    }
}

#[test]
fn direct_restore_releases_ghost_and_group_budget_fallback_settles_whole_workspace_effect() {
    let base = window(1, 1, Some(rect()));
    let (mut p, mut e) = setup(snapshot(vec![base.clone()]));
    let mut hidden = base.clone();
    hidden.minimized = true;
    hidden.placement = None;
    p.reconcile(snapshot(vec![hidden]), &[], t(0.0), &mut e)
        .unwrap();
    let u = p
        .reconcile(
            snapshot(vec![base]),
            &[TransitionCause::Window {
                key: key(1),
                kind: WindowTransitionKind::Direct,
            }],
            t(0.01),
            &mut e,
        )
        .unwrap();
    assert_eq!(u.release.len(), 1);
    let frame = p.sample(t(0.01), &e);
    assert_eq!(frame.windows.len(), 1);
    assert!(!frame.windows[0].outgoing);
    assert_eq!(frame.windows[0].pose.opacity, 1.0);
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before);
    use clear::runtime::animation::{AnimationPose, AnimationRect};
    let pose = AnimationPose {
        rect: AnimationRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        opacity: 1.0,
    };
    for id in 1..=255 {
        e.retarget(
            id,
            1,
            AnimationEffect::WindowMovement,
            pose,
            pose,
            t(0.0),
            1.0,
        )
        .unwrap();
    }
    let u = p.reconcile(after, &[], t(0.0), &mut e).unwrap();
    assert!(u.retain.is_empty());
    let frame = p.sample(t(0.0), &e);
    assert!(frame.workspaces.is_empty());
    assert_eq!(frame.windows.len(), 1);
    assert_eq!(frame.windows[0].pose.rect.x, 100.0);
    assert!(frame.windows[0].input_eligible);
    assert_eq!(e.len(), 255);
}

#[test]
fn group_acknowledgement_bounds_and_tiny_direction_progress_are_finite() {
    let (before, after) = workspace_states();
    let (mut p, mut e) = setup(before.clone());
    p.reconcile(after, &[], t(0.0), &mut e).unwrap();
    let shown = p.sample(t(0.04), &e);
    p.mark_presented(&shown).unwrap();
    for invalid in [1e100, f64::NAN] {
        let mut frame = shown.clone();
        frame.workspaces[0].incoming_offset.0 = invalid;
        assert!(p.mark_presented(&frame).is_err());
        let mut frame = shown.clone();
        frame.windows[0].pose.rect.x = invalid;
        assert!(p.mark_presented(&frame).is_err());
    }
    let mut duplicate = shown.clone();
    duplicate.workspaces.push(duplicate.workspaces[0].clone());
    assert!(p.mark_presented(&duplicate).is_err());
    p.reconcile(
        before,
        &[TransitionCause::WorkspaceSwitch {
            group: 1,
            direction: WorkspaceDirection { x: -1e-308, y: 0.0 },
        }],
        t(0.05),
        &mut e,
    )
    .unwrap();
    let frame = p.sample(t(0.05), &e);
    assert!(frame.workspaces[0].progress.is_finite());
    assert_eq!(
        frame.workspaces[0].incoming_offset,
        shown.workspaces[0].outgoing_offset
    );
}

#[test]
fn explicit_settlement_requires_a_drawn_live_terminal_frame_at_its_revision() {
    let (mut p, mut e) = setup(snapshot(vec![]));
    let s = snapshot(vec![window(1, 1, Some(rect()))]);
    p.reconcile(s, &[], t(0.0), &mut e).unwrap();
    let unfinished = p.sample(t(0.025), &e);
    p.mark_presented(&unfinished).unwrap();
    // Planner allocation starts at one in this isolated engine.
    e.settle(1);
    assert!(p.is_animating(t(0.025), &e));
    assert!(p.retire_finished(t(0.025), &mut e).is_empty());
    assert_eq!(e.len(), 1);

    let final_frame = p.sample(t(0.025), &e);
    let mut unavailable_draw = final_frame.clone();
    unavailable_draw.windows.clear();
    p.mark_presented(&unavailable_draw).unwrap();
    assert!(p.is_animating(t(0.025), &e));
    p.retire_finished(t(0.025), &mut e);
    assert_eq!(e.len(), 1);

    p.mark_presented(&final_frame).unwrap();
    assert!(!p.is_animating(t(0.025), &e));
    p.retire_finished(t(0.025), &mut e);
    assert!(e.is_empty());
    assert_eq!(p.sample(t(0.025), &e).windows[0].pose.rect.x, 100.0);
}
