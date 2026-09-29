//! Pure resize policy tests, also runnable with `rustc --edition=2024 --test tests/resize.rs`.
#[allow(dead_code)]
#[path = "../src/core/mod.rs"]
mod core;
#[allow(dead_code)]
#[path = "../src/management/mod.rs"]
mod management;

use core::*;

const LEFT: ResizeEdges = ResizeEdges {
    left: true,
    right: false,
    top: false,
    bottom: false,
};
const RIGHT: ResizeEdges = ResizeEdges {
    left: false,
    right: true,
    top: false,
    bottom: false,
};
const TOP: ResizeEdges = ResizeEdges {
    left: false,
    right: false,
    top: true,
    bottom: false,
};
const BOTTOM: ResizeEdges = ResizeEdges {
    left: false,
    right: false,
    top: false,
    bottom: true,
};

fn desktop(mode: Mode, count: u64) -> Desktop {
    let mut desktop = Desktop::new();
    desktop.set_gaps(0);
    desktop.add_output(OutputId(1), "one".into(), Rect::new(0, 0, 1000, 800));
    desktop.command(Command::SetWorkspaceMode(mode));
    for id in 1..=count {
        desktop.add_window(WindowId(id), id.to_string(), "test".into());
    }
    desktop.command(Command::Focus(WindowId(1)));
    desktop
}

fn rect(desktop: &Desktop, id: u64) -> Rect {
    desktop
        .placements()
        .into_iter()
        .find(|p| p.window == WindowId(id))
        .unwrap()
        .rect
}

fn resize(desktop: &mut Desktop, id: u64, edges: ResizeEdges, dx: i32, dy: i32) -> ResizeSession {
    let session = desktop
        .begin_resize(WindowId(id), edges)
        .expect("supported edge");
    assert!(desktop.update_resize(&session, dx, dy));
    session
}

#[test]
fn floating_edges_fix_opposite_edge_and_reuse_baseline() {
    for (edges, dx, dy, expected) in [
        (LEFT, 50, 0, Rect::new(-150, -100, 350, 300)),
        (RIGHT, 50, 0, Rect::new(-200, -100, 450, 300)),
        (TOP, 0, 50, Rect::new(-200, -50, 400, 250)),
        (BOTTOM, 0, 50, Rect::new(-200, -100, 400, 350)),
        (
            ResizeEdges {
                left: true,
                top: true,
                ..Default::default()
            },
            50,
            50,
            Rect::new(-150, -50, 350, 250),
        ),
    ] {
        let mut d = desktop(Mode::Floating, 1);
        let baseline = Rect::new(-200, -100, 400, 300);
        d.command(Command::SetFloatingRect(WindowId(1), baseline));
        let session = resize(&mut d, 1, edges, dx, dy);
        assert_eq!(rect(&d, 1), expected);
        assert!(d.update_resize(&session, dx, dy));
        assert_eq!(rect(&d, 1), expected);
        assert!(d.update_resize(&session, 0, 0));
        assert_eq!(rect(&d, 1), baseline);
        assert!(!d.window(WindowId(1)).unwrap().floating);
    }
}

#[test]
fn floating_minimum_and_offscreen_growth_are_unconstrained() {
    let mut d = desktop(Mode::Floating, 1);
    let baseline = Rect::new(-200, -100, 400, 300);
    d.command(Command::SetFloatingRect(WindowId(1), baseline));
    let session = resize(
        &mut d,
        1,
        ResizeEdges {
            left: true,
            top: true,
            ..Default::default()
        },
        i32::MAX,
        i32::MAX,
    );
    assert_eq!(rect(&d, 1), Rect::new(136, 152, 64, 48));
    assert!(d.update_resize(&session, -10000, -10000));
    assert_eq!(rect(&d, 1), Rect::new(-10200, -10100, 10400, 10300));
    let session = resize(
        &mut d,
        1,
        ResizeEdges {
            right: true,
            bottom: true,
            ..Default::default()
        },
        i32::MIN,
        i32::MIN,
    );
    assert_eq!(rect(&d, 1).width, 64);
    assert_eq!(rect(&d, 1).height, 48);
    assert!(d.update_resize(&session, i32::MAX, i32::MAX));
    assert_eq!(rect(&d, 1), rect(&d, 1).normalized());
}

#[test]
fn unsupported_tiles_refuse_but_float_exceptions_resize_in_every_mode() {
    for mode in [
        Mode::Floating,
        Mode::Scrolling,
        Mode::MasterStack,
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Monocle,
        Mode::Script("test".into()),
    ] {
        let mut d = desktop(mode.clone(), 3);
        if matches!(mode, Mode::Spiral | Mode::Monocle | Mode::Script(_)) {
            for edge in [LEFT, RIGHT, TOP, BOTTOM] {
                assert!(d.begin_resize(WindowId(1), edge).is_none());
            }
        }
        d.command(Command::ToggleFloating);
        let before = d.window(WindowId(1)).unwrap().floating_rect;
        resize(&mut d, 1, RIGHT, 90, 0);
        assert_eq!(rect(&d, 1).width, before.width + 90);
        assert!(d.window(WindowId(1)).unwrap().floating);
        d.set_window_role(WindowId(1), WindowRole::Launcher);
        assert!(d.begin_resize(WindowId(1), RIGHT).is_none());
    }
}

#[test]
fn empty_unknown_conflicting_and_single_tile_edges() {
    let d = desktop(Mode::Floating, 1);
    assert!(d.begin_resize(WindowId(99), RIGHT).is_none());
    assert!(
        d.begin_resize(WindowId(1), ResizeEdges::default())
            .is_none()
    );
    assert!(
        d.begin_resize(
            WindowId(1),
            ResizeEdges {
                left: true,
                right: true,
                ..Default::default()
            }
        )
        .is_none()
    );
    for mode in [Mode::Columns, Mode::Rows, Mode::Grid, Mode::MasterStack] {
        let d = desktop(mode, 1);
        for edges in [LEFT, RIGHT, TOP, BOTTOM] {
            assert!(d.begin_resize(WindowId(1), edges).is_none());
        }
    }
}

#[test]
fn columns_and_rows_resize_only_adjacent_pair_from_either_side() {
    for (mode, leading, trailing, horizontal) in [
        (Mode::Columns, LEFT, RIGHT, true),
        (Mode::Rows, TOP, BOTTOM, false),
    ] {
        for (id, edge) in [(1, trailing), (2, leading)] {
            let mut d = desktop(mode.clone(), 3);
            let before: Vec<_> = (1..=3).map(|id| rect(&d, id)).collect();
            let session = resize(&mut d, id, edge, 50, 50);
            let after: Vec<_> = (1..=3).map(|id| rect(&d, id)).collect();
            assert_eq!(after[2], before[2]);
            if horizontal {
                assert_eq!(after[0].width, before[0].width + 50);
                assert_eq!(after[1].width, before[1].width - 50);
                assert_eq!(after[0].right(), after[1].x);
            } else {
                assert_eq!(after[0].height, before[0].height + 50);
                assert_eq!(after[1].height, before[1].height - 50);
                assert_eq!(after[0].bottom(), after[1].y);
            }
            assert!(d.update_resize(&session, 50, 50));
            assert_eq!((1..=3).map(|id| rect(&d, id)).collect::<Vec<_>>(), after);
            assert!(d.update_resize(&session, 0, 0));
            assert_eq!((1..=3).map(|id| rect(&d, id)).collect::<Vec<_>>(), before);
        }
        let d = desktop(mode, 3);
        assert!(d.begin_resize(WindowId(1), leading).is_none());
        assert!(d.begin_resize(WindowId(3), trailing).is_none());
        assert!(
            d.begin_resize(WindowId(2), if horizontal { TOP } else { LEFT })
                .is_none()
        );
    }
}

#[test]
fn pair_minimums_clamp_both_neighbors() {
    for (mode, edge, horizontal) in [
        (Mode::Columns, RIGHT, true),
        (Mode::Rows, BOTTOM, false),
        (Mode::MasterStack, RIGHT, true),
        (Mode::Grid, RIGHT, true),
    ] {
        let mut d = desktop(mode, 4);
        let session = resize(&mut d, 1, edge, i32::MAX, i32::MAX);
        let minimum = if horizontal { 64 } else { 48 };
        assert_eq!(
            if horizontal {
                rect(&d, 2).width
            } else {
                rect(&d, 2).height
            },
            minimum
        );
        assert!(d.update_resize(&session, i32::MIN, i32::MIN));
        assert_eq!(
            if horizontal {
                rect(&d, 1).width
            } else {
                rect(&d, 1).height
            },
            minimum
        );
    }
    let mut d = desktop(Mode::MasterStack, 4);
    let session = resize(&mut d, 2, BOTTOM, 0, i32::MAX);
    assert_eq!(rect(&d, 3).height, 48);
    assert!(d.update_resize(&session, 0, i32::MIN));
    assert_eq!(rect(&d, 2).height, 48);
}

#[test]
fn master_divider_from_any_stack_and_adjacent_stack_heights() {
    for (id, edge) in [(1, RIGHT), (2, LEFT), (3, LEFT), (4, LEFT)] {
        let mut d = desktop(Mode::MasterStack, 4);
        let session = resize(&mut d, id, edge, 75, 0);
        assert_eq!(rect(&d, 1), Rect::new(0, 0, 675, 800));
        for stack in 2..=4 {
            assert_eq!(rect(&d, stack).x, 675);
            assert_eq!(rect(&d, stack).width, 325);
        }
        assert!(d.update_resize(&session, 75, 0));
        assert_eq!(rect(&d, 1).width, 675);
    }
    for (id, edge) in [(2, BOTTOM), (3, TOP)] {
        let mut d = desktop(Mode::MasterStack, 4);
        let before = rect(&d, 4);
        resize(&mut d, id, edge, 0, 50);
        assert_eq!(rect(&d, 2).height, 317);
        assert_eq!(rect(&d, 3).height, 217);
        assert_eq!(rect(&d, 4), before);
    }
    let mut d = desktop(Mode::MasterStack, 4);
    resize(
        &mut d,
        2,
        ResizeEdges {
            left: true,
            bottom: true,
            ..Default::default()
        },
        50,
        40,
    );
    assert_eq!(rect(&d, 1).width, 650);
    assert_eq!(rect(&d, 2).height, 307);
    for (id, edge) in [
        (1, LEFT),
        (1, TOP),
        (1, BOTTOM),
        (2, TOP),
        (4, BOTTOM),
        (2, RIGHT),
    ] {
        assert!(d.begin_resize(WindowId(id), edge).is_none());
    }
}

#[test]
fn grid_resizes_whole_row_and_only_adjacent_cells_within_row() {
    for (id, edge) in [(1, RIGHT), (2, LEFT)] {
        let mut d = desktop(Mode::Grid, 5);
        let before: Vec<_> = (3..=5).map(|id| rect(&d, id)).collect();
        resize(&mut d, id, edge, 70, 0);
        assert_eq!(rect(&d, 1).width, 404);
        assert_eq!(rect(&d, 2).width, 263);
        assert_eq!((3..=5).map(|id| rect(&d, id)).collect::<Vec<_>>(), before);
    }
    for (id, edge) in [(1, BOTTOM), (2, BOTTOM), (3, BOTTOM), (4, TOP), (5, TOP)] {
        let mut d = desktop(Mode::Grid, 5);
        let session = resize(&mut d, id, edge, 0, 80);
        for first in 1..=3 {
            assert_eq!(rect(&d, first).height, 480);
        }
        for second in 4..=5 {
            assert_eq!(rect(&d, second).y, 480);
            assert_eq!(rect(&d, second).height, 320);
        }
        assert!(d.update_resize(&session, 0, i32::MAX));
        assert_eq!(rect(&d, 4).height, 48);
        assert!(d.update_resize(&session, 0, i32::MIN));
        assert_eq!(rect(&d, 1).height, 48);
    }
    let mut d = desktop(Mode::Grid, 5);
    resize(
        &mut d,
        2,
        ResizeEdges {
            left: true,
            bottom: true,
            ..Default::default()
        },
        70,
        80,
    );
    assert_eq!(rect(&d, 1), Rect::new(0, 0, 404, 480));
    resize(&mut d, 5, LEFT, -100, 0);
    assert_eq!(rect(&d, 4).width, 400);
    assert_eq!(rect(&d, 5).width, 600);
    for (id, edge) in [
        (1, LEFT),
        (3, RIGHT),
        (4, LEFT),
        (5, RIGHT),
        (1, TOP),
        (4, BOTTOM),
    ] {
        assert!(d.begin_resize(WindowId(id), edge).is_none());
    }
}

#[test]
fn tiled_corner_ignores_unsupported_axis() {
    let mut d = desktop(Mode::Columns, 2);
    resize(
        &mut d,
        1,
        ResizeEdges {
            right: true,
            top: true,
            ..Default::default()
        },
        40,
        500,
    );
    assert_eq!(rect(&d, 1), Rect::new(0, 0, 540, 800));
    let mut d = desktop(Mode::Grid, 4);
    resize(
        &mut d,
        1,
        ResizeEdges {
            left: true,
            bottom: true,
            ..Default::default()
        },
        500,
        40,
    );
    assert_eq!(rect(&d, 1), Rect::new(0, 0, 500, 440));
}

#[test]
fn scrolling_independent_widths_prefix_positions_and_total_displacement() {
    for (edge, delta) in [(RIGHT, 100), (LEFT, -100)] {
        let mut d = desktop(Mode::Scrolling, 3);
        assert_eq!(rect(&d, 1).x, 0);
        let session = resize(&mut d, 2, edge, delta, 0);
        assert_eq!(rect(&d, 1), Rect::new(0, 0, 666, 800));
        assert_eq!(rect(&d, 2), Rect::new(666, 0, 766, 800));
        assert_eq!(rect(&d, 3), Rect::new(1432, 0, 666, 800));
        assert!(d.update_resize(&session, delta, 0));
        assert_eq!(rect(&d, 2).width, 766);
        assert!(d.update_resize(&session, 0, 0));
        assert_eq!(rect(&d, 2).width, 666);
        assert!(d.begin_resize(WindowId(2), TOP).is_none());
        assert!(d.begin_resize(WindowId(2), BOTTOM).is_none());
    }
    let mut d = desktop(Mode::Scrolling, 1);
    let session = resize(&mut d, 1, RIGHT, i32::MIN, 0);
    assert_eq!(rect(&d, 1).width, 64);
    assert!(d.update_resize(&session, i32::MAX, 0));
    assert_eq!(rect(&d, 1).width, 1000);
}

#[test]
fn scrolling_reveal_and_content_clamp_use_resized_widths() {
    let mut d = desktop(Mode::Scrolling, 3);
    resize(&mut d, 1, RIGHT, -566, 0);
    resize(&mut d, 2, RIGHT, -466, 0);
    d.command(Command::Focus(WindowId(3)));
    assert_eq!(rect(&d, 3).x, 300);
    d.command(Command::Scroll(i32::MAX));
    assert_eq!(
        d.workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        0
    );
    resize(&mut d, 2, RIGHT, 500, 0);
    d.command(Command::Focus(WindowId(3)));
    assert_eq!(rect(&d, 3).right(), 1000);
    assert_eq!(
        d.workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        466
    );
    resize(&mut d, 2, RIGHT, -600, 0);
    assert_eq!(
        d.workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        0
    );
    assert_eq!(rect(&d, 3).x, 200);
}

#[test]
fn proportions_survive_area_changes_and_modes_without_touching_float_state() {
    for mode in [
        Mode::Columns,
        Mode::MasterStack,
        Mode::Grid,
        Mode::Scrolling,
    ] {
        let mut d = desktop(mode.clone(), 4);
        let floats: Vec<_> = d
            .windows()
            .map(|window| (window.id, window.floating_rect, window.floating))
            .collect();
        resize(&mut d, 1, RIGHT, 80, 0);
        let changed: Vec<_> = d.placements().into_iter().map(|p| p.rect).collect();
        d.command(Command::SetWorkspaceMode(Mode::Rows));
        resize(&mut d, 1, BOTTOM, 0, 30);
        d.command(Command::SetWorkspaceMode(mode));
        assert_eq!(
            d.placements()
                .into_iter()
                .map(|p| p.rect)
                .collect::<Vec<_>>(),
            changed
        );
        d.set_output_area(OutputId(1), Rect::new(0, 0, 2000, 1600));
        assert_eq!(rect(&d, 1).width, changed[0].width * 2);
        d.set_output_area(OutputId(1), Rect::new(0, 0, 1000, 800));
        assert_eq!(rect(&d, 1), changed[0]);
        assert_eq!(
            d.windows()
                .map(|window| (window.id, window.floating_rect, window.floating))
                .collect::<Vec<_>>(),
            floats
        );
    }
}

#[test]
fn workspace_and_output_sizing_are_independent_and_persistent() {
    let mut d = desktop(Mode::Columns, 2);
    resize(&mut d, 1, RIGHT, 100, 0);
    d.command(Command::SwitchWorkspace(WorkspaceId(2)));
    d.command(Command::SetWorkspaceMode(Mode::Columns));
    for id in 3..=4 {
        d.add_window(WindowId(id), "".into(), "".into());
    }
    assert_eq!(rect(&d, 3).width, 500);
    resize(&mut d, 3, RIGHT, -100, 0);
    d.command(Command::SwitchWorkspace(WorkspaceId(1)));
    assert_eq!(rect(&d, 1).width, 600);
    d.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert_eq!(rect(&d, 3).width, 400);
    d.add_output(OutputId(2), "two".into(), Rect::new(1000, 0, 1000, 800));
    d.command(Command::StretchAll);
    d.command(Command::FocusOutput(OutputId(2)));
    for id in 5..=6 {
        d.add_window(WindowId(id), "".into(), "".into());
    }
    assert_eq!(rect(&d, 5).width, 500);
    resize(&mut d, 5, RIGHT, 200, 0);
    assert_eq!(rect(&d, 3).width, 400);
    assert_eq!(rect(&d, 5).width, 700);
    d.set_workspace_output_mode(WorkspaceId(2), OutputId(2), Some(Mode::Rows));
    resize(&mut d, 5, BOTTOM, 0, 100);
    d.set_workspace_output_mode(WorkspaceId(2), OutputId(2), None);
    assert_eq!(rect(&d, 5).width, 700);
}

#[test]
fn same_windows_recover_each_outputs_own_proportions() {
    let mut d = desktop(Mode::Columns, 2);
    resize(&mut d, 1, RIGHT, 100, 0);
    d.add_output(OutputId(2), "two".into(), Rect::new(1000, 0, 1000, 800));
    d.command(Command::StretchAll);
    for id in 1..=2 {
        d.command(Command::Focus(WindowId(id)));
        d.command(Command::MoveToOutput(OutputId(2)));
    }
    assert_eq!(rect(&d, 1).width, 500);
    resize(&mut d, 1, RIGHT, 200, 0);
    for id in 1..=2 {
        d.command(Command::Focus(WindowId(id)));
        d.command(Command::MoveToOutput(OutputId(1)));
    }
    assert_eq!(rect(&d, 1).width, 600);
    for id in 1..=2 {
        d.command(Command::Focus(WindowId(id)));
        d.command(Command::MoveToOutput(OutputId(2)));
    }
    assert_eq!(rect(&d, 1).width, 700);
}

#[test]
fn scrolling_nonzero_gaps_and_area_changes_share_prefix_metrics() {
    let mut d = desktop(Mode::Scrolling, 3);
    d.set_gaps(10);
    resize(&mut d, 1, RIGHT, -153, 0);
    assert_eq!(rect(&d, 1), Rect::new(10, 10, 500, 780));
    assert_eq!(rect(&d, 2).x, 520);
    assert_eq!(rect(&d, 3).x, 1183);
    d.command(Command::Focus(WindowId(3)));
    assert_eq!(rect(&d, 3).right(), 990);
    assert_eq!(
        d.workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        846
    );
    d.set_output_area(OutputId(1), Rect::new(0, 0, 1980, 800));
    assert_eq!(rect(&d, 1).width, 1000);
    assert_eq!(rect(&d, 3).right(), 1970);
    d.command(Command::Focus(WindowId(1)));
    assert_eq!(rect(&d, 1).x, 10);
    assert_eq!(rect(&d, 2).x, 1020);
}

#[test]
fn float_extreme_coordinates_preserve_fixed_edges_without_overflow() {
    for baseline in [
        Rect::new(i32::MIN, i32::MIN, 400, 300),
        Rect::new(i32::MAX - 400, i32::MAX - 300, 400, 300),
    ] {
        let mut d = desktop(Mode::Floating, 1);
        d.command(Command::SetFloatingRect(WindowId(1), baseline));
        let session = d
            .begin_resize(
                WindowId(1),
                ResizeEdges {
                    left: true,
                    top: true,
                    ..Default::default()
                },
            )
            .unwrap();
        for delta in [i32::MIN, i32::MAX, 0] {
            assert!(d.update_resize(&session, delta, delta));
            let resized = rect(&d, 1);
            assert_eq!(resized, resized.normalized());
            assert_eq!(resized.right(), baseline.right());
            assert_eq!(resized.bottom(), baseline.bottom());
            assert!(resized.width >= 64 && resized.height >= 48);
        }
    }
}

#[test]
fn invalid_sessions_reject_lifecycle_changes_even_when_reversed() {
    let changes: Vec<Box<dyn Fn(&mut Desktop)>> = vec![
        Box::new(|d| {
            d.remove_window(WindowId(1));
        }),
        Box::new(|d| {
            d.remove_window(WindowId(1));
            d.add_window(WindowId(1), "".into(), "".into());
        }),
        Box::new(|d| {
            d.command(Command::SetWorkspaceMode(Mode::Rows));
        }),
        Box::new(|d| {
            d.command(Command::SetWorkspaceMode(Mode::Rows));
            d.command(Command::SetWorkspaceMode(Mode::Columns));
        }),
        Box::new(|d| {
            d.set_output_area(OutputId(1), Rect::new(0, 40, 1000, 760));
        }),
        Box::new(|d| {
            d.set_output_area(OutputId(1), Rect::new(0, 40, 1000, 760));
            d.set_output_area(OutputId(1), Rect::new(0, 0, 1000, 800));
        }),
        Box::new(|d| {
            d.set_gaps(10);
            d.set_gaps(0);
        }),
        Box::new(|d| {
            d.add_window(WindowId(4), "".into(), "".into());
            d.remove_window(WindowId(4));
        }),
        Box::new(|d| {
            d.command(Command::ToggleFloating);
            d.command(Command::ToggleFloating);
        }),
        Box::new(|d| {
            d.set_window_role(WindowId(2), WindowRole::Launcher);
        }),
        Box::new(|d| {
            d.command(Command::MoveToWorkspace(WorkspaceId(2)));
        }),
        Box::new(|d| {
            d.command(Command::SwitchWorkspace(WorkspaceId(2)));
            d.command(Command::SwitchWorkspace(WorkspaceId(1)));
        }),
        Box::new(|d| {
            d.remove_output(OutputId(1));
            d.add_output(OutputId(1), "one".into(), Rect::new(0, 0, 1000, 800));
        }),
        Box::new(|d| {
            d.add_output(OutputId(2), "two".into(), Rect::new(1000, 0, 1000, 800));
            d.command(Command::StretchAll);
        }),
    ];
    for change in changes {
        let mut d = desktop(Mode::Columns, 3);
        let session = d.begin_resize(WindowId(1), RIGHT).unwrap();
        change(&mut d);
        let before = d.placements();
        assert!(!d.update_resize(&session, 80, 0), "{d:?}");
        assert_eq!(d.placements(), before);
    }
}

#[test]
fn float_sessions_cancel_on_mode_area_and_membership_changes_too() {
    for command in [
        Command::SetWorkspaceMode(Mode::Spiral),
        Command::ToggleFloating,
        Command::MoveToWorkspace(WorkspaceId(2)),
    ] {
        let mut d = desktop(Mode::Floating, 1);
        let session = d.begin_resize(WindowId(1), RIGHT).unwrap();
        d.command(command);
        assert!(!d.update_resize(&session, 50, 0));
    }
    let mut d = desktop(Mode::Floating, 1);
    let session = d.begin_resize(WindowId(1), RIGHT).unwrap();
    d.set_output_area(OutputId(1), Rect::new(0, 0, 900, 700));
    assert!(!d.update_resize(&session, 50, 0));
}

#[test]
fn focus_metadata_commits_and_other_regions_do_not_cancel() {
    let mut d = desktop(Mode::Columns, 3);
    let session = d.begin_resize(WindowId(1), RIGHT).unwrap();
    d.command(Command::Focus(WindowId(2)));
    d.update_window_metadata(WindowId(1), "updated".into(), "test".into());
    d.set_window_committed_size(WindowId(1), 10, 10);
    d.add_output(OutputId(2), "two".into(), Rect::new(1000, 0, 1000, 800));
    d.command(Command::FocusOutput(OutputId(2)));
    d.add_window(WindowId(4), "".into(), "".into());
    d.command(Command::SetWorkspaceMode(Mode::Scrolling));
    assert!(d.update_resize(&session, 100, 0));
    assert_eq!(rect(&d, 1).width, 434);
}

#[test]
fn removed_window_sizing_is_cleaned_even_in_old_workspaces_and_modes() {
    let mut d = desktop(Mode::Scrolling, 3);
    resize(&mut d, 1, RIGHT, -500, 0);
    d.command(Command::MoveToWorkspace(WorkspaceId(2)));
    d.remove_window(WindowId(1));
    d.add_window(WindowId(1), "new".into(), "".into());
    assert_eq!(rect(&d, 1).width, 666);
    d.command(Command::SetWorkspaceMode(Mode::Columns));
    resize(&mut d, 2, RIGHT, 100, 0);
    d.remove_window(WindowId(2));
    d.remove_window(WindowId(3));
    d.remove_window(WindowId(1));
    d.add_window(WindowId(2), "new".into(), "".into());
    d.add_window(WindowId(3), "new".into(), "".into());
    assert_eq!(rect(&d, 2).width, 500);
}

fn assert_tiles(d: &Desktop, scrolling: bool) {
    let placements = d.placements();
    let area = d.output(OutputId(1)).unwrap().area;
    for (index, p) in placements.iter().enumerate() {
        assert_eq!(p.rect, p.rect.normalized());
        if !scrolling {
            assert!(p.rect.x >= area.x && p.rect.y >= area.y, "{p:?} {area:?}");
            assert!(
                p.rect.right() <= area.right() && p.rect.bottom() <= area.bottom(),
                "{p:?} {area:?}"
            );
        }
        for other in &placements[index + 1..] {
            assert!(!p.rect.intersects(other.rect), "overlap: {p:?} {other:?}");
        }
    }
}

#[test]
fn deterministic_random_resizes_preserve_nonoverlap_and_normalized_geometry() {
    let mut random = 7u64;
    let mut next = || {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        (random >> 32) as u32
    };
    for mode in [
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::MasterStack,
        Mode::Scrolling,
    ] {
        let mut d = desktop(mode.clone(), 30);
        for step in 0..300 {
            if step % 7 == 0 {
                let width = (next() % 1400) as i32;
                let height = (next() % 1000) as i32;
                d.set_output_area(OutputId(1), Rect::new(-500, -300, width, height));
                d.set_gaps((next() % 100) as i32);
            }
            let id = WindowId(u64::from(next() % 30 + 1));
            let edge = [
                LEFT,
                RIGHT,
                TOP,
                BOTTOM,
                ResizeEdges {
                    right: true,
                    bottom: true,
                    ..Default::default()
                },
            ][next() as usize % 5];
            if let Some(session) = d.begin_resize(id, edge) {
                assert!(d.update_resize(&session, next() as i32, next() as i32));
                assert_tiles(&d, mode == Mode::Scrolling);
                let placements = d.placements();
                assert!(d.update_resize(&session, 0, 0));
                assert_tiles(&d, mode == Mode::Scrolling);
                assert!(d.update_resize(&session, i32::MIN, i32::MAX));
                assert_tiles(&d, mode == Mode::Scrolling);
                assert_eq!(placements.len(), 30);
            }
        }
        for size in 0..=8 {
            d.set_output_area(OutputId(1), Rect::new(0, 0, size, size));
            d.set_gaps(0);
            for id in 1..=30 {
                for edge in [LEFT, RIGHT, TOP, BOTTOM] {
                    if let Some(session) = d.begin_resize(WindowId(id), edge) {
                        assert!(d.update_resize(&session, i32::MAX, i32::MIN));
                        assert_tiles(&d, mode == Mode::Scrolling);
                    }
                }
            }
        }
    }
}
