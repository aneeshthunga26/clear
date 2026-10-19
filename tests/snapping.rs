use clear::core::{Command, Desktop, Mode, OutputId, Rect, SnapTarget, WindowId, WindowRole};

fn desktop() -> Desktop {
    let mut d = Desktop::new();
    d.add_output_with_bounds(
        OutputId(1),
        "left".into(),
        Rect::new(0, 0, 801, 601),
        Rect::new(0, 32, 801, 569),
    );
    d.command(Command::SetWorkspaceMode(Mode::Floating));
    d.add_window(WindowId(1), "one".into(), "test".into());
    d.command(Command::SetFloatingRect(
        WindowId(1),
        Rect::new(160, 140, 300, 220),
    ));
    d
}
fn frame(d: &Desktop) -> Rect {
    d.placements()
        .iter()
        .find(|p| p.window == WindowId(1))
        .unwrap()
        .rect
}

#[test]
fn partitions_cover_odd_usable_areas_exactly_and_keep_corner_order() {
    for width in 2..25 {
        for height in 2..25 {
            let area = Rect::new(-319, 33, width, height);
            let rectangles: Vec<_> = [
                SnapTarget::TopLeft,
                SnapTarget::TopRight,
                SnapTarget::BottomLeft,
                SnapTarget::BottomRight,
            ]
            .map(|s| s.rect(area).unwrap())
            .into();
            assert_eq!(
                rectangles.iter().map(|r| r.width * r.height).sum::<i32>(),
                width * height
            );
            for (i, r) in rectangles.iter().enumerate() {
                assert_eq!(r.intersection(area), Some(*r));
                for other in &rectangles[..i] {
                    assert!(!r.intersects(*other));
                }
            }
            let left = SnapTarget::Left.rect(area).unwrap();
            let right = SnapTarget::Right.rect(area).unwrap();
            assert_eq!(left.width + right.width, width);
            assert_eq!(left.right(), right.x);
        }
    }
    for area in [Rect::new(0, 0, 0, 10), Rect::new(0, 0, 1, 10)] {
        assert!(SnapTarget::Left.rect(area).is_none());
    }
    assert!(SnapTarget::TopLeft.rect(Rect::new(0, 0, 10, 1)).is_none());
    assert_eq!(
        SnapTarget::Maximize.rect(Rect::new(0, 0, 1, 1)),
        Some(Rect::new(0, 0, 1, 1))
    );
}

#[test]
fn physical_edge_zones_are_output_local_corners_win_and_bottom_middle_is_free() {
    let b = Rect::new(801, -20, 640, 481);
    for ((x, y), expected) in [
        ((802., -19.), Some(SnapTarget::TopLeft)),
        ((1440., -19.), Some(SnapTarget::TopRight)),
        ((802., 460.), Some(SnapTarget::BottomLeft)),
        ((1440., 460.), Some(SnapTarget::BottomRight)),
        ((1100., -19.), Some(SnapTarget::Maximize)),
        ((802., 200.), Some(SnapTarget::Left)),
        ((1440., 200.), Some(SnapTarget::Right)),
        ((1100., 460.), None),
        ((1100., 200.), None),
        ((800., 0.), None),
        ((1441., 0.), None),
        ((f64::NAN, 0.), None),
    ] {
        assert_eq!(SnapTarget::at_pointer(b, x, y), expected);
    }
}

#[test]
fn snapping_preserves_free_geometry_tracks_reservations_and_survives_other_states() {
    let mut d = desktop();
    let saved = d.window(WindowId(1)).unwrap().floating_rect;
    assert!(d.snap_window(WindowId(1), SnapTarget::Right));
    assert_eq!(frame(&d), Rect::new(400, 32, 401, 569));
    d.set_output_area(OutputId(1), Rect::new(0, 64, 799, 500));
    assert_eq!(frame(&d), Rect::new(399, 64, 400, 500));
    d.command(Command::SetMaximized(WindowId(1), true));
    assert_eq!(frame(&d), Rect::new(0, 64, 799, 500));
    d.command(Command::SetMaximized(WindowId(1), false));
    assert_eq!(frame(&d), Rect::new(399, 64, 400, 500));
    d.command(Command::SetFullscreen(WindowId(1), true));
    assert_eq!(frame(&d), Rect::new(0, 0, 801, 601));
    d.command(Command::SetFullscreen(WindowId(1), false));
    d.command(Command::SetMinimized(WindowId(1), true));
    assert!(d.placements().is_empty());
    d.command(Command::Focus(WindowId(1)));
    assert_eq!(frame(&d), Rect::new(399, 64, 400, 500));
    assert_eq!(d.window(WindowId(1)).unwrap().floating_rect, saved);
    d.command(Command::SetWorkspaceMode(Mode::Columns));
    assert_eq!(d.placements().len(), 1);
    assert!(!d.placements()[0].tiled);
    d.command(Command::SetFloatingRect(WindowId(1), saved));
    assert_eq!(frame(&d), saved);
    assert!(d.window(WindowId(1)).unwrap().snapped.is_none());
}

#[test]
fn invalid_snap_requests_are_atomic_and_launcher_classification_clears_the_override() {
    let mut d = desktop();
    assert!(!d.snap_window(WindowId(99), SnapTarget::Left));
    d.set_output_area(OutputId(1), Rect::new(0, 32, 1, 10));
    let original = d.window(WindowId(1)).unwrap().clone();
    assert!(!d.snap_window(WindowId(1), SnapTarget::Left));
    assert_eq!(d.window(WindowId(1)), Some(&original));
    d.set_output_area(OutputId(1), Rect::new(0, 32, 801, 569));
    d.snap_window(WindowId(1), SnapTarget::TopLeft);
    d.set_window_role(WindowId(1), WindowRole::Launcher);
    assert!(d.window(WindowId(1)).unwrap().snapped.is_none());
    assert!(!d.snap_window(WindowId(1), SnapTarget::Right));
}

#[test]
fn release_applies_preview_but_drag_out_restores_size_under_pointer_immediately() {
    let mut d = desktop();
    let saved = frame(&d);
    let mut session = d.begin_move(WindowId(1), saved, (200., 150.)).unwrap();
    assert!(d.update_move(&mut session, (800., 300.)));
    let preview = d.snap_preview(&session).unwrap();
    assert_eq!(preview.rect, Rect::new(400, 32, 401, 569));
    assert_eq!(d.window(WindowId(1)).unwrap().snapped, None);
    assert_eq!(frame(&d).width, 300, "hovering must not resize the client");
    assert!(d.finish_move(session));
    assert_eq!(frame(&d), preview.rect);
    assert_eq!(d.window(WindowId(1)).unwrap().floating_rect, saved);
    let shown = frame(&d);
    let mut session = d.begin_move(WindowId(1), shown, (600., 40.)).unwrap();
    assert_eq!((frame(&d).width, frame(&d).height), (300, 220));
    let start = frame(&d);
    assert!(d.update_move(&mut session, (550., 90.)));
    assert_eq!(frame(&d).x, start.x - 50);
    assert_eq!(frame(&d).y, start.y + 50);
    assert!(d.snap_preview(&session).is_none());
    assert!(d.finish_move(session));
    assert_eq!(frame(&d).width, 300);
}

#[test]
fn top_maximize_drag_out_and_escape_preserve_the_original_restore_geometry() {
    let mut d = desktop();
    let saved = frame(&d);
    let mut s = d.begin_move(WindowId(1), saved, (200., 150.)).unwrap();
    d.update_move(&mut s, (400., 0.));
    assert!(d.finish_move(s));
    assert!(d.window(WindowId(1)).unwrap().maximized);
    assert_eq!(d.window(WindowId(1)).unwrap().floating_rect, saved);
    let mut s = d.begin_move(WindowId(1), frame(&d), (400., 40.)).unwrap();
    d.update_move(&mut s, (200., 200.));
    assert!(!d.window(WindowId(1)).unwrap().maximized);
    assert!(d.cancel_move(s));
    assert!(d.window(WindowId(1)).unwrap().maximized);
    d.command(Command::SetMaximized(WindowId(1), false));
    assert_eq!(frame(&d), saved);
}

#[test]
fn crossing_outputs_commits_to_the_target_or_cancels_back_to_original_ownership_and_order() {
    let mut d = desktop();
    d.add_window(WindowId(2), "two".into(), "test".into());
    d.command(Command::Focus(WindowId(1)));
    let original = d.window(WindowId(1)).unwrap().clone();
    let order = d.workspace(original.workspace).unwrap().windows().to_vec();
    d.add_output_with_bounds(
        OutputId(2),
        "right".into(),
        Rect::new(801, 0, 640, 481),
        Rect::new(801, 24, 640, 457),
    );
    let mut s = d.begin_move(WindowId(1), frame(&d), (200., 150.)).unwrap();
    d.update_move(&mut s, (1440., 480.));
    assert_eq!(d.window(WindowId(1)).unwrap().output, Some(OutputId(2)));
    assert_eq!(
        d.snap_preview(&s).unwrap().rect,
        Rect::new(1121, 252, 320, 229)
    );
    assert!(d.cancel_move(s));
    assert_eq!(d.window(WindowId(1)), Some(&original));
    assert_eq!(d.workspace(original.workspace).unwrap().windows(), order);
    let mut s = d.begin_move(WindowId(1), frame(&d), (200., 150.)).unwrap();
    d.update_move(&mut s, (1440., 480.));
    assert!(d.finish_move(s));
    assert_eq!(frame(&d), Rect::new(1121, 252, 320, 229));
    assert_eq!(d.window(WindowId(1)).unwrap().output, Some(OutputId(2)));
}

#[test]
fn commits_do_not_invalidate_but_policy_changes_cancel_without_undoing_newer_state() {
    let mut d = desktop();
    let mut s = d.begin_move(WindowId(1), frame(&d), (200., 150.)).unwrap();
    d.update_move(&mut s, (800., 200.));
    d.update_window_metadata(WindowId(1), "new title".into(), "new app".into());
    d.set_window_committed_size(WindowId(1), 10, 20);
    assert!(d.move_valid(&s));
    d.command(Command::SetMinimized(WindowId(1), true));
    assert!(!d.move_valid(&s));
    assert!(d.snap_preview(&s).is_none());
    assert!(!d.cancel_move(s));
    assert!(d.window(WindowId(1)).unwrap().minimized);
}

#[test]
fn changed_then_restored_output_area_invalidates_the_old_target() {
    let mut d = desktop();
    let s = d.begin_move(WindowId(1), frame(&d), (200., 150.)).unwrap();
    d.set_output_area(OutputId(1), Rect::new(0, 64, 801, 537));
    d.set_output_area(OutputId(1), Rect::new(0, 32, 801, 569));
    assert!(!d.move_valid(&s));
    assert!(!d.finish_move(s));
}

#[test]
fn snap_then_restore_invalidates_resize_and_tiny_output_hides_override_without_losing_it() {
    use clear::core::ResizeEdges;
    let mut d = desktop();
    d.command(Command::ToggleFloating);
    let saved = frame(&d);
    let resize = d
        .begin_resize(
            WindowId(1),
            ResizeEdges {
                right: true,
                ..Default::default()
            },
        )
        .unwrap();
    d.snap_window(WindowId(1), SnapTarget::Right);
    d.command(Command::SetFloatingRect(WindowId(1), saved));
    assert!(!d.update_resize(&resize, 20, 0));
    d.snap_window(WindowId(1), SnapTarget::Right);
    d.set_output_area(OutputId(1), Rect::new(0, 32, 1, 10));
    assert!(d.placements().is_empty());
    assert_eq!(
        d.window(WindowId(1)).unwrap().snapped,
        Some(SnapTarget::Right)
    );
    d.set_output_area(OutputId(1), Rect::new(0, 32, 801, 569));
    assert_eq!(frame(&d), Rect::new(400, 32, 401, 569));
    assert_eq!(d.window(WindowId(1)).unwrap().floating_rect, saved);
}

#[test]
fn explicit_snapping_is_nonrevealing_and_does_not_steal_focus() {
    let mut d = desktop();
    d.command(Command::SwitchWorkspace(clear::core::WorkspaceId(2)));
    d.add_window(WindowId(2), "two".into(), "test".into());
    let focused = d.focused_window();
    let workspace = d.workspace_for_output(OutputId(1));
    assert!(d.snap_window(WindowId(1), SnapTarget::Left));
    assert_eq!(d.focused_window(), focused);
    assert_eq!(d.workspace_for_output(OutputId(1)), workspace);
    assert!(d.placements().iter().all(|p| p.window != WindowId(1)));
}

#[test]
fn extreme_finite_pointer_positions_do_not_saturate_into_another_outputs_bounds() {
    let mut d = desktop();
    d.add_output(OutputId(2), "far".into(), Rect::new(i32::MIN, 0, 800, 600));
    let mut s = d.begin_move(WindowId(1), frame(&d), (200., 150.)).unwrap();
    assert!(d.update_move(&mut s, (-f64::MAX, 200.)));
    assert_eq!(d.window(WindowId(1)).unwrap().output, Some(OutputId(1)));
    assert!(d.snap_preview(&s).is_none());
}

#[test]
fn disabled_snapping_keeps_motion_and_cached_targets_but_cannot_snap_on_release() {
    let mut d = desktop();
    let id = WindowId(1);
    let mut s = d.begin_move(id, frame(&d), (200., 150.)).unwrap();
    s.set_snapping_enabled(false);
    assert!(d.update_move(&mut s, (0., 300.)));
    let moved = frame(&d);
    assert_eq!(moved, Rect::new(-40, 290, 300, 220));
    assert!(d.snap_preview(&s).is_none());
    s.set_snapping_enabled(true);
    assert_eq!(d.snap_preview(&s).unwrap().target, SnapTarget::Left);
    s.set_snapping_enabled(false);
    assert!(d.finish_move(s));
    assert_eq!(frame(&d), moved);
    assert_eq!(d.window(id).unwrap().snapped, None);
}

#[test]
fn generous_corner_zones_work_without_touching_edges_and_leave_edge_centers_available() {
    let b = Rect::new(801, -20, 640, 481);
    for ((x, y), target) in [
        ((851., 30.), Some(SnapTarget::TopLeft)),
        ((1391., 30.), Some(SnapTarget::TopRight)),
        ((851., 411.), Some(SnapTarget::BottomLeft)),
        ((1391., 411.), Some(SnapTarget::BottomRight)),
        ((864., 43.), Some(SnapTarget::TopLeft)),
        ((865., 44.), None),
        ((802., 60.), Some(SnapTarget::Left)),
        ((1440., 60.), Some(SnapTarget::Right)),
        ((900., -19.), Some(SnapTarget::Maximize)),
        ((1100., 460.), None),
    ] {
        assert_eq!(SnapTarget::at_pointer(b, x, y), target, "{x},{y}");
    }
    let tiny = Rect::new(0, 0, 30, 30);
    assert_eq!(
        SnapTarget::at_pointer(tiny, 14., 14.),
        Some(SnapTarget::TopLeft)
    );
    assert_eq!(
        SnapTarget::at_pointer(tiny, 15., 15.),
        Some(SnapTarget::BottomRight)
    );
    assert_eq!(SnapTarget::at_pointer(tiny, 30., 15.), None);
}
