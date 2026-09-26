//! Standalone policy tests: also runnable with `rustc --edition=2024 --test tests/core.rs`.
//! Importing the modules directly keeps these tests independent of platform dependencies.
#[allow(dead_code)]
#[path = "../src/core/mod.rs"]
mod core;
#[allow(dead_code)]
#[path = "../src/management/mod.rs"]
mod management;

use std::collections::BTreeSet;

use core::*;

fn output(desktop: &mut Desktop, id: u64) {
    desktop.add_output(
        OutputId(id),
        format!("output-{id}"),
        Rect::new((id as i32 - 1) * 1000, 0, 1000, 800),
    );
}

fn window(desktop: &mut Desktop, id: u64) {
    desktop.add_window(WindowId(id), format!("window-{id}"), "test".into());
}

fn populated(count: u64) -> Desktop {
    let mut desktop = Desktop::new();
    output(&mut desktop, 1);
    for id in 1..=count {
        window(&mut desktop, id);
    }
    desktop
}

fn ids(placements: &[Placement]) -> BTreeSet<WindowId> {
    placements
        .iter()
        .map(|placement| placement.window)
        .collect()
}

fn assert_invariants(desktop: &Desktop) {
    let mut presented = BTreeSet::new();
    let mut connected = BTreeSet::new();
    for group in desktop.groups() {
        assert!(!group.outputs.is_empty());
        assert!(
            presented.insert(group.workspace),
            "workspace presented twice"
        );
        assert!(desktop.workspace(group.workspace).is_some());
        assert!(group.outputs.windows(2).all(|pair| pair[0] < pair[1]));
        for output in &group.outputs {
            assert!(connected.insert(*output), "output in two groups");
            assert!(desktop.output(*output).is_some());
        }
    }
    assert_eq!(
        connected,
        desktop.outputs().map(|output| output.id).collect()
    );
    let mut owned = BTreeSet::new();
    for workspace in desktop.workspaces() {
        for id in workspace.windows() {
            assert!(owned.insert(*id), "window owned twice");
            assert_eq!(desktop.window(*id).unwrap().workspace, workspace.id);
        }
    }
    assert_eq!(owned, desktop.windows().map(|window| window.id).collect());
    for window in desktop.windows() {
        if !connected.is_empty() {
            assert!(
                window
                    .output
                    .is_some_and(|output| connected.contains(&output))
            );
        }
        if let Some(group) = desktop
            .groups()
            .iter()
            .find(|group| group.workspace == window.workspace)
        {
            assert!(
                window
                    .output
                    .is_some_and(|output| group.outputs.contains(&output))
            );
        }
    }
    let placements = desktop.placements();
    assert_eq!(
        ids(&placements).len(),
        placements.len(),
        "duplicate placement"
    );
    let visible: BTreeSet<_> = desktop
        .windows()
        .filter(|window| presented.contains(&window.workspace))
        .map(|window| window.id)
        .collect();
    assert_eq!(ids(&placements), visible, "hidden or lost placement");
    let focused: Vec<_> = placements
        .iter()
        .filter(|placement| placement.focused)
        .map(|placement| placement.window)
        .collect();
    assert_eq!(
        focused,
        desktop.focused_window().into_iter().collect::<Vec<_>>()
    );
    for placement in placements {
        assert_eq!(placement.rect, placement.rect.normalized());
        if let Some(clip) = placement.clip {
            assert_eq!(clip, clip.normalized());
        }
    }
    if connected.is_empty() {
        assert_eq!(desktop.focused_output(), None);
        assert_eq!(desktop.focused_window(), None);
    } else {
        let output = desktop.focused_output().unwrap();
        assert!(connected.contains(&output));
        if let Some(id) = desktop.focused_window() {
            let window = desktop.window(id).unwrap();
            assert_eq!(window.output, Some(output));
            assert_eq!(Some(window.workspace), desktop.workspace_for_output(output));
        } else {
            assert!(!desktop.windows().any(|window| {
                window.output == Some(output)
                    && Some(window.workspace) == desktop.workspace_for_output(output)
            }));
        }
    }
}

#[test]
fn initial_state_and_exact_public_api() {
    let desktop = Desktop::default();
    assert_eq!(
        desktop
            .workspaces()
            .map(|workspace| workspace.id)
            .collect::<Vec<_>>(),
        (1..=9).map(WorkspaceId).collect::<Vec<_>>()
    );
    assert!(desktop.outputs().next().is_none());
    assert!(desktop.placements().is_empty());
    assert_invariants(&desktop);
    let context = LayoutContext {
        area: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
        windows: vec![LayoutWindow {
            id: WindowId(1),
            floating_rect: Rect::new(0, 0, 50, 50),
        }],
        focused: Some(WindowId(1)),
        scroll_offset: 0,
        gaps: 0,
    };
    let result = management::arrange(&Mode::Columns, &context);
    assert_eq!(
        result,
        vec![Placement {
            window: WindowId(1),
            rect: context.area,
            clip: None,
            focused: true,
            tiled: true
        }]
    );
}

#[test]
fn mode_names_and_parsing() {
    for (name, mode) in [
        ("floating", Mode::Floating),
        ("scrolling", Mode::Scrolling),
        ("master_stack", Mode::MasterStack),
        ("columns", Mode::Columns),
        ("rows", Mode::Rows),
        ("grid", Mode::Grid),
        ("spiral", Mode::Spiral),
        ("monocle", Mode::Monocle),
    ] {
        assert_eq!(Mode::parse(name), Some(mode.clone()));
        assert_eq!(mode.name(), name);
    }
    assert_eq!(Mode::parse(" MASTER-STACK "), Some(Mode::MasterStack));
    assert_eq!(Mode::parse("Fibonacci"), Some(Mode::Spiral));
    assert_eq!(Mode::parse("dwindle"), Some(Mode::Spiral));
    assert_eq!(
        Mode::parse(" Script:MyLayout "),
        Some(Mode::Script("MyLayout".into()))
    );
    assert_eq!(Mode::Script("custom".into()).name(), "custom");
    for invalid in ["", "unknown", "script", "script: ", "floating:bad"] {
        assert_eq!(Mode::parse(invalid), None);
    }
}

#[test]
fn mode_cycle_visits_every_builtin() {
    let mut desktop = populated(1);
    desktop.command(Command::SetOutputMode(Mode::Floating));
    for expected in [
        Mode::Scrolling,
        Mode::MasterStack,
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Monocle,
        Mode::Floating,
    ] {
        desktop.command(Command::CycleMode);
        assert_eq!(
            desktop.effective_mode(WorkspaceId(1), OutputId(1)),
            Some(&expected)
        );
    }
}

#[test]
fn geometry_half_open_clamping_and_extremes() {
    let rect = Rect::new(-10, 20, 100, 80);
    assert!(rect.contains(-10, 20));
    assert!(!rect.contains(90, 20));
    assert!(!rect.contains(0, 100));
    assert_eq!(
        rect.intersection(Rect::new(80, 90, 100, 100)),
        Some(Rect::new(80, 90, 10, 10))
    );
    assert!(!rect.intersects(Rect::new(90, 20, 10, 10)));
    assert_eq!(Rect::new(0, 0, -1, -9), Rect::default());
    assert_eq!(rect.inset(i32::MAX), Rect::new(40, 60, 0, 0));
    assert_eq!(
        Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX).right(),
        -1
    );
    assert_eq!(
        Rect::new(i32::MAX - 2, i32::MAX - 3, 10, 20),
        Rect::new(i32::MAX - 2, i32::MAX - 3, 2, 3)
    );
    assert_eq!(
        Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX).clamped_to(rect),
        rect
    );
    assert_eq!(rect.centered(20, 10), Rect::new(30, 55, 20, 10));
    assert_eq!(
        Rect {
            x: i32::MAX,
            y: i32::MIN,
            width: i32::MAX,
            height: -1
        }
        .normalized(),
        Rect::new(i32::MAX, i32::MIN, 0, 0)
    );
}

#[test]
fn each_output_gets_a_distinct_workspace_and_can_exceed_nine() {
    let mut desktop = Desktop::new();
    for id in 1..=12 {
        output(&mut desktop, id);
        assert_eq!(
            desktop.workspace_for_output(OutputId(id)),
            Some(WorkspaceId(id))
        );
    }
    assert_eq!(desktop.workspaces().count(), 12);
    assert_invariants(&desktop);
}

#[test]
fn repeated_ids_only_update_metadata() {
    let mut desktop = populated(2);
    desktop.command(Command::ToggleFloating);
    let before = desktop.window(WindowId(2)).unwrap().clone();
    desktop.add_window(WindowId(2), "new title".into(), "new app".into());
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().floating,
        before.floating
    );
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().floating_rect,
        before.floating_rect
    );
    assert_eq!(desktop.window(WindowId(2)).unwrap().title, "new title");
    desktop.command(Command::SwitchWorkspace(WorkspaceId(4)));
    desktop.add_output(OutputId(1), "renamed".into(), Rect::new(-100, 0, 300, 200));
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(4))
    );
    assert_eq!(desktop.groups().len(), 1);
    assert_eq!(desktop.output(OutputId(1)).unwrap().name, "renamed");
    assert_invariants(&desktop);
}

#[test]
fn workspace_switch_hides_and_restores_focus_and_order() {
    let mut desktop = populated(3);
    desktop.command(Command::Focus(WindowId(2)));
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert_eq!(desktop.focused_window(), None);
    assert!(desktop.placements().is_empty());
    window(&mut desktop, 4);
    desktop.command(Command::SwitchWorkspace(WorkspaceId(1)));
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    assert_eq!(
        desktop.workspace(WorkspaceId(1)).unwrap().windows(),
        &[WindowId(1), WindowId(2), WindowId(3)]
    );
    assert_eq!(
        ids(&desktop.placements()),
        [WindowId(1), WindowId(2), WindowId(3)].into()
    );
    assert_invariants(&desktop);
}

#[test]
fn visible_workspace_switch_swaps_presentations() {
    let mut desktop = populated(2);
    output(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(2)));
    window(&mut desktop, 3);
    desktop.command(Command::FocusOutput(OutputId(1)));
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(2))
    );
    assert_eq!(
        desktop.workspace_for_output(OutputId(2)),
        Some(WorkspaceId(1))
    );
    assert_eq!(desktop.focused_window(), Some(WindowId(3)));
    assert_eq!(
        desktop.window(WindowId(1)).unwrap().output,
        Some(OutputId(2))
    );
    assert_eq!(
        desktop.window(WindowId(3)).unwrap().output,
        Some(OutputId(1))
    );
    assert_eq!(desktop.windows().count(), 3);
    assert_invariants(&desktop);
}

#[test]
fn stretched_group_swaps_as_a_whole_with_a_single_output() {
    let mut desktop = populated(1);
    output(&mut desktop, 2);
    desktop.command(Command::StretchAll);
    output(&mut desktop, 3);
    desktop.command(Command::FocusOutput(OutputId(3)));
    window(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(1)));
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(2))
    );
    assert_eq!(
        desktop.workspace_for_output(OutputId(2)),
        Some(WorkspaceId(2))
    );
    assert_eq!(
        desktop.workspace_for_output(OutputId(3)),
        Some(WorkspaceId(1))
    );
    assert_eq!(
        desktop.window(WindowId(1)).unwrap().output,
        Some(OutputId(3))
    );
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().output,
        Some(OutputId(1))
    );
    assert_invariants(&desktop);
}

#[test]
fn stretch_hides_other_workspaces_and_empty_region_accepts_moves() {
    let mut desktop = populated(2);
    output(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(2)));
    window(&mut desktop, 3);
    desktop.command(Command::FocusOutput(OutputId(1)));
    desktop.command(Command::StretchAll);
    assert_eq!(desktop.groups().len(), 1);
    assert_eq!(
        desktop.window(WindowId(3)).unwrap().workspace,
        WorkspaceId(2)
    );
    assert_eq!(
        desktop.window(WindowId(1)).unwrap().output,
        Some(OutputId(1))
    );
    assert_eq!(
        ids(&desktop.placements()),
        [WindowId(1), WindowId(2)].into()
    );
    desktop.command(Command::MoveToOutput(OutputId(2)));
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().workspace,
        WorkspaceId(1)
    );
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().output,
        Some(OutputId(2))
    );
    assert_eq!(desktop.focused_output(), Some(OutputId(2)));
    assert_eq!(
        desktop
            .placements()
            .iter()
            .find(|p| p.window == WindowId(2))
            .unwrap()
            .rect
            .x,
        1008
    );
    let homes: Vec<_> = desktop.windows().map(|window| window.output).collect();
    desktop.command(Command::StretchAll);
    assert_eq!(
        desktop
            .windows()
            .map(|window| window.output)
            .collect::<Vec<_>>(),
        homes
    );
    assert_invariants(&desktop);
}

#[test]
fn unstretch_retains_workspace_on_focused_region_and_restores_hidden_windows() {
    let mut desktop = populated(2);
    output(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(2)));
    window(&mut desktop, 3);
    desktop.command(Command::FocusOutput(OutputId(1)));
    desktop.command(Command::StretchAll);
    desktop.command(Command::MoveToOutput(OutputId(2)));
    desktop.command(Command::Unstretch);
    assert_eq!(
        desktop.workspace_for_output(OutputId(2)),
        Some(WorkspaceId(1))
    );
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(2))
    );
    assert_eq!(
        desktop.window(WindowId(1)).unwrap().output,
        Some(OutputId(2))
    );
    assert_eq!(
        desktop.window(WindowId(3)).unwrap().output,
        Some(OutputId(1))
    );
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    assert_eq!(desktop.placements().len(), 3);
    assert_invariants(&desktop);
}

#[test]
fn removing_stretched_region_migrates_windows_without_losing_focus() {
    let mut desktop = populated(2);
    output(&mut desktop, 2);
    desktop.command(Command::StretchAll);
    desktop.command(Command::MoveToOutput(OutputId(2)));
    desktop.remove_output(OutputId(2));
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().output,
        Some(OutputId(1))
    );
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    assert_eq!(desktop.placements().len(), 2);
    assert_invariants(&desktop);
}

#[test]
fn removing_separate_output_hides_its_workspace_but_keeps_windows_reachable() {
    let mut desktop = populated(1);
    output(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(2)));
    window(&mut desktop, 2);
    desktop.remove_output(OutputId(2));
    assert_eq!(desktop.focused_window(), Some(WindowId(1)));
    assert_eq!(desktop.windows().count(), 2);
    assert_eq!(ids(&desktop.placements()), [WindowId(1)].into());
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    assert_invariants(&desktop);
}

#[test]
fn headless_windows_and_reconnection_preserve_state() {
    let mut desktop = Desktop::new();
    window(&mut desktop, 1);
    assert_eq!(desktop.focused_window(), None);
    assert!(desktop.placements().is_empty());
    output(&mut desktop, 1);
    assert_eq!(desktop.focused_window(), Some(WindowId(1)));
    desktop.command(Command::SwitchWorkspace(WorkspaceId(6)));
    window(&mut desktop, 2);
    desktop.command(Command::ToggleFloating);
    let saved = Rect::new(71, 43, 350, 210);
    desktop.command(Command::SetFloatingRect(WindowId(2), saved));
    desktop.command(Command::SetOutputMode(Mode::Columns));
    desktop.remove_output(OutputId(1));
    window(&mut desktop, 3);
    assert_eq!(
        desktop.window(WindowId(3)).unwrap().workspace,
        WorkspaceId(6)
    );
    assert_invariants(&desktop);
    output(&mut desktop, 1);
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(6))
    );
    assert_eq!(desktop.window(WindowId(2)).unwrap().floating_rect, saved);
    assert_eq!(
        desktop.effective_mode(WorkspaceId(6), OutputId(1)),
        Some(&Mode::Columns)
    );
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    assert_eq!(
        ids(&desktop.placements()),
        [WindowId(2), WindowId(3)].into()
    );
    assert_invariants(&desktop);
}

#[test]
fn reconnect_does_not_duplicate_workspace_now_shown_elsewhere() {
    let mut desktop = populated(1);
    output(&mut desktop, 2);
    desktop.remove_output(OutputId(2));
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    output(&mut desktop, 2);
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(2))
    );
    assert_ne!(
        desktop.workspace_for_output(OutputId(2)),
        Some(WorkspaceId(2))
    );
    assert_invariants(&desktop);
}

#[test]
fn modes_are_workspace_defaults_with_persistent_per_output_overrides() {
    let mut desktop = populated(1);
    output(&mut desktop, 2);
    desktop.command(Command::StretchAll);
    desktop.command(Command::SetWorkspaceMode(Mode::Columns));
    desktop.command(Command::SetOutputMode(Mode::Monocle));
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(1)),
        Some(&Mode::Monocle)
    );
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(2)),
        Some(&Mode::Columns)
    );
    desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(1)),
        Some(&Mode::Monocle)
    );
    desktop.command(Command::SwitchWorkspace(WorkspaceId(3)));
    assert_eq!(
        desktop.effective_mode(WorkspaceId(3), OutputId(1)),
        Some(&Mode::MasterStack)
    );
    desktop.command(Command::SwitchWorkspace(WorkspaceId(1)));
    desktop.command(Command::ClearOutputMode);
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(1)),
        Some(&Mode::Scrolling)
    );
    desktop.command(Command::CycleMode);
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(1)),
        Some(&Mode::MasterStack)
    );
    assert_eq!(
        desktop.workspace(WorkspaceId(1)).unwrap().mode,
        Mode::Scrolling
    );
    desktop.set_workspace_output_mode(WorkspaceId(1), OutputId(99), Some(Mode::Floating));
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(99)),
        Some(&Mode::Floating)
    );
    assert_invariants(&desktop);
}

#[test]
fn configure_workspace_retains_windows_and_overrides() {
    let mut desktop = populated(2);
    desktop.command(Command::SetOutputMode(Mode::Columns));
    desktop.configure_workspace(WorkspaceId(1), "code".into(), Mode::Monocle);
    desktop.configure_workspace(WorkspaceId(100), "extra".into(), Mode::Floating);
    assert_eq!(desktop.workspace(WorkspaceId(1)).unwrap().name, "code");
    assert_eq!(
        desktop.workspace(WorkspaceId(1)).unwrap().windows(),
        &[WindowId(1), WindowId(2)]
    );
    assert_eq!(
        desktop.effective_mode(WorkspaceId(1), OutputId(1)),
        Some(&Mode::Columns)
    );
    desktop.command(Command::SwitchWorkspace(WorkspaceId(100)));
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(100))
    );
    assert_invariants(&desktop);
}

#[test]
fn floating_exceptions_do_not_consume_tiles_and_keep_geometry() {
    let mut desktop = populated(2);
    desktop.set_gaps(10);
    desktop.command(Command::SetWorkspaceMode(Mode::Columns));
    let saved = Rect::new(100, 150, 200, 300);
    desktop.command(Command::SetFloatingRect(WindowId(2), saved));
    assert!(!desktop.window(WindowId(2)).unwrap().floating);
    desktop.command(Command::ToggleFloating);
    let placements = desktop.placements();
    assert_eq!(placements[0].window, WindowId(1));
    assert_eq!(placements[0].rect, Rect::new(10, 10, 980, 780));
    assert!(placements[0].tiled);
    assert_eq!(placements[1].rect, saved);
    assert!(!placements[1].tiled);
    let order = desktop
        .workspace(WorkspaceId(1))
        .unwrap()
        .windows()
        .to_vec();
    for mode in [
        Mode::Scrolling,
        Mode::Monocle,
        Mode::Floating,
        Mode::MasterStack,
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
    ] {
        desktop.command(Command::SetWorkspaceMode(mode));
        assert_eq!(desktop.window(WindowId(2)).unwrap().floating_rect, saved);
        assert_eq!(desktop.workspace(WorkspaceId(1)).unwrap().windows(), order);
    }
    desktop.command(Command::ToggleFloating);
    desktop.command(Command::ToggleFloating);
    assert_eq!(desktop.placements().last().unwrap().rect, saved);
    assert_invariants(&desktop);
}

#[test]
fn floating_geometry_restores_after_temporary_output_shrink() {
    let mut desktop = populated(1);
    desktop.command(Command::SetWorkspaceMode(Mode::Floating));
    let saved = Rect::new(500, 300, 400, 400);
    desktop.command(Command::SetFloatingRect(WindowId(1), saved));
    desktop.set_output_area(OutputId(1), Rect::new(0, 0, 200, 100));
    assert_eq!(desktop.placements()[0].rect, saved);
    assert_eq!(desktop.window(WindowId(1)).unwrap().floating_rect, saved);
    desktop.set_output_area(OutputId(1), Rect::new(0, 0, 1000, 800));
    assert_eq!(desktop.placements()[0].rect, saved);
}

#[test]
fn panel_reservation_changes_and_removal_retile_and_recenter_without_moving_saved_floats() {
    let mut desktop = populated(4);
    desktop.set_gaps(0);
    desktop.command(Command::SetWorkspaceMode(Mode::Columns));
    desktop.command(Command::Focus(WindowId(3)));
    desktop.command(Command::ToggleFloating);
    for id in 1..=4 {
        desktop.command(Command::SetFloatingRect(
            WindowId(id),
            Rect::new(-300, 900, 350, 220),
        ));
    }
    desktop.set_window_role(WindowId(4), WindowRole::Launcher);
    desktop.set_window_committed_size(WindowId(4), 400, 240);
    output(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(2)));
    window(&mut desktop, 5);
    desktop.command(Command::Focus(WindowId(3)));
    let saved: Vec<_> = desktop.windows().cloned().collect();
    let before = desktop.placements();
    let other_output = before.iter().find(|p| p.window == WindowId(5)).unwrap();

    // These are the usable areas supplied by the adapter after panel arrangement:
    // top panel appears, grows, a left panel joins, then both are removed.
    for (area, tiles, launcher) in [
        (
            Rect::new(0, 40, 1000, 760),
            [Rect::new(0, 40, 500, 760), Rect::new(500, 40, 500, 760)],
            Rect::new(300, 300, 400, 240),
        ),
        (
            Rect::new(0, 80, 1000, 720),
            [Rect::new(0, 80, 500, 720), Rect::new(500, 80, 500, 720)],
            Rect::new(300, 320, 400, 240),
        ),
        (
            Rect::new(60, 80, 940, 720),
            [Rect::new(60, 80, 470, 720), Rect::new(530, 80, 470, 720)],
            Rect::new(330, 320, 400, 240),
        ),
        (
            Rect::new(0, 0, 1000, 800),
            [Rect::new(0, 0, 500, 800), Rect::new(500, 0, 500, 800)],
            Rect::new(300, 280, 400, 240),
        ),
    ] {
        desktop.set_output_area(OutputId(1), area);
        let placements = desktop.placements();
        for (id, rect) in [(WindowId(1), tiles[0]), (WindowId(2), tiles[1])] {
            let placement = placements.iter().find(|p| p.window == id).unwrap();
            assert_eq!(placement.rect, rect);
            assert!(placement.tiled);
        }
        assert_eq!(
            placements.iter().find(|p| p.window == WindowId(5)),
            Some(other_output)
        );
        let float = placements.iter().find(|p| p.window == WindowId(3)).unwrap();
        assert_eq!(float.rect, Rect::new(-300, 900, 350, 220));
        assert_eq!(float.clip, None);
        assert!(!float.tiled);
        let overlay = placements.last().unwrap();
        assert_eq!(overlay.window, WindowId(4));
        assert_eq!(overlay.rect, launcher);
        assert_eq!(overlay.clip, None);
        assert!(!overlay.tiled);
        assert_eq!(desktop.windows().cloned().collect::<Vec<_>>(), saved);
        assert_eq!(desktop.focused_window(), Some(WindowId(3)));
        assert_eq!(
            desktop.workspace(WorkspaceId(1)).unwrap().windows(),
            &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)]
        );
        assert_invariants(&desktop);
    }
    assert_eq!(desktop.placements(), before);
    desktop.command(Command::SetWorkspaceMode(Mode::Floating));
    for placement in desktop.placements().iter().filter(|p| p.window.0 <= 3) {
        assert_eq!(placement.rect, Rect::new(-300, 900, 350, 220));
        assert!(!placement.tiled);
    }
}

#[test]
fn floating_windows_and_exceptions_preserve_unbounded_normalized_geometry() {
    for exception in [false, true] {
        let mut desktop = populated(2);
        if exception {
            desktop.command(Command::ToggleFloating);
        } else {
            desktop.command(Command::SetWorkspaceMode(Mode::Floating));
        }
        for rect in [
            Rect::new(-250, -100, 400, 300),
            Rect::new(2500, 1700, 400, 300),
            Rect::new(-2000, -1800, 1400, 1200),
            Rect::new(-100, -200, 1600, 1200),
            Rect {
                x: -2000,
                y: 1700,
                width: -1,
                height: 300,
            },
        ] {
            let saved = rect.normalized();
            desktop.command(Command::SetFloatingRect(WindowId(2), rect));
            desktop.set_window_committed_size(WindowId(2), 50, 40);
            for area in [Rect::new(0, 0, 1000, 800), Rect::new(100, 50, 100, 80)] {
                desktop.set_output_area(OutputId(1), area);
                let placement = desktop
                    .placements()
                    .into_iter()
                    .find(|placement| placement.window == WindowId(2))
                    .unwrap();
                assert_eq!(placement.rect, saved);
                assert!(!placement.tiled);
                assert_eq!(placement.clip, None);
                assert_eq!(desktop.window(WindowId(2)).unwrap().floating_rect, saved);
                assert_invariants(&desktop);
            }
        }
    }
}

#[test]
fn initial_floating_placement_fits_usable_area_and_mode_round_trips_restore_offscreen_rect() {
    let mut desktop = Desktop::new();
    let area = Rect::new(-1600, 140, 500, 350);
    desktop.add_output(OutputId(1), "offset".into(), area);
    desktop.command(Command::SetWorkspaceMode(Mode::Floating));
    window(&mut desktop, 1);
    assert_eq!(desktop.placements()[0].rect, area);
    assert_eq!(
        desktop.window(WindowId(1)).unwrap().role,
        WindowRole::Normal
    );
    assert_eq!(desktop.window(WindowId(1)).unwrap().committed_size, None);
    let saved = Rect::new(-2200, -900, 1300, 950);
    desktop.command(Command::SetFloatingRect(WindowId(1), saved));
    for mode in [
        Mode::Columns,
        Mode::Scrolling,
        Mode::MasterStack,
        Mode::Monocle,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Script("fallback".into()),
    ] {
        desktop.command(Command::SetWorkspaceMode(mode));
        assert!(desktop.placements()[0].tiled);
        desktop.set_window_committed_size(WindowId(1), 500, 350);
        desktop.command(Command::SetWorkspaceMode(Mode::Floating));
        assert_eq!(desktop.placements()[0].rect, saved);
        assert_invariants(&desktop);
    }
}

#[test]
fn launcher_commits_recenter_in_home_usable_area_without_changing_saved_state() {
    let mut desktop = populated(1);
    let usable = Rect::new(1200, -260, 1000, 760);
    desktop.add_output(OutputId(2), "offset-with-top-panel".into(), usable);
    desktop.command(Command::FocusOutput(OutputId(2)));
    window(&mut desktop, 2);
    desktop.command(Command::ToggleFloating);
    let saved = Rect::new(2400, -500, 300, 200);
    desktop.command(Command::SetFloatingRect(WindowId(2), saved));
    let mut expected = desktop.window(WindowId(2)).unwrap().clone();
    let order = desktop
        .workspace(expected.workspace)
        .unwrap()
        .windows()
        .to_vec();
    desktop.set_window_role(WindowId(2), WindowRole::Launcher);
    expected.role = WindowRole::Launcher;
    desktop.set_window_committed_size(WindowId(2), 0, 200);
    assert_eq!(desktop.window(WindowId(2)), Some(&expected));
    assert_eq!(
        desktop.placements().last().unwrap().rect,
        Rect::new(1550, 20, 300, 200)
    );
    for (width, height, rect) in [
        (400, 240, Rect::new(1500, 0, 400, 240)),
        (700, 500, Rect::new(1350, -130, 700, 500)),
        (1400, 1000, Rect::new(1000, -380, 1400, 1000)),
    ] {
        desktop.set_window_committed_size(WindowId(2), width, height);
        expected.committed_size = Some((width, height));
        assert_eq!(desktop.window(WindowId(2)), Some(&expected));
        let placement = desktop.placements().last().unwrap().clone();
        assert_eq!(placement.rect, rect);
        assert!(!placement.tiled);
        assert_eq!(placement.clip, None);
    }
    desktop.set_window_committed_size(WindowId(2), -1, 50);
    desktop.set_window_committed_size(WindowId(2), 50, 0);
    assert_eq!(desktop.window(WindowId(2)), Some(&expected));
    desktop.command(Command::Focus(WindowId(1)));
    assert_eq!(
        desktop.placements().last().unwrap().rect,
        Rect::new(1000, -380, 1400, 1000)
    );
    desktop.set_output_area(OutputId(2), Rect::new(1200, -220, 800, 600));
    assert_eq!(
        desktop.placements().last().unwrap().rect,
        Rect::new(900, -420, 1400, 1000)
    );
    assert_eq!(
        desktop.workspace(expected.workspace).unwrap().windows(),
        order
    );
    assert_eq!(
        desktop.effective_mode(expected.workspace, OutputId(2)),
        Some(&Mode::MasterStack)
    );
    desktop.set_window_role(WindowId(2), WindowRole::Normal);
    expected.role = WindowRole::Normal;
    assert_eq!(desktop.window(WindowId(2)), Some(&expected));
    assert_eq!(desktop.placements().last().unwrap().rect, saved);
    assert_invariants(&desktop);
}

#[test]
fn launchers_do_not_consume_tiles_or_scroll_content_in_any_mode() {
    for mode in [
        Mode::Floating,
        Mode::Scrolling,
        Mode::MasterStack,
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Monocle,
        Mode::Script("fallback".into()),
    ] {
        let mut control = populated(2);
        control.command(Command::SetWorkspaceMode(mode.clone()));
        let mut desktop = control.clone();
        window(&mut desktop, 3);
        desktop.set_window_role(WindowId(3), WindowRole::Launcher);
        desktop.set_window_committed_size(WindowId(3), 400, 200);
        desktop.command(Command::Focus(WindowId(2)));
        let placements = desktop.placements();
        assert_eq!(&placements[..2], control.placements());
        assert_eq!(placements[2].window, WindowId(3));
        assert_eq!(placements[2].rect, Rect::new(300, 300, 400, 200));
        assert!(!placements[2].tiled);
        desktop.command(Command::Focus(WindowId(3)));
        control.command(Command::Scroll(i32::MAX));
        desktop.command(Command::Scroll(i32::MAX));
        assert_eq!(
            desktop
                .workspace(WorkspaceId(1))
                .unwrap()
                .scroll_offset(OutputId(1)),
            control
                .workspace(WorkspaceId(1))
                .unwrap()
                .scroll_offset(OutputId(1))
        );
        desktop.command(Command::SetWorkspaceMode(Mode::Floating));
        desktop.command(Command::SetWorkspaceMode(mode));
        assert_eq!(
            desktop.window(WindowId(3)).unwrap().role,
            WindowRole::Launcher
        );
        assert_eq!(
            desktop.placements().last().unwrap().rect,
            Rect::new(300, 300, 400, 200)
        );
        assert_eq!(
            desktop.workspace(WorkspaceId(1)).unwrap().windows(),
            &[WindowId(1), WindowId(2), WindowId(3)]
        );
        assert_invariants(&desktop);
    }
}

#[test]
fn launcher_role_updates_remove_and_restore_script_membership_without_reordering() {
    let mut desktop = populated(3);
    desktop.command(Command::SetWorkspaceMode(Mode::Script("custom".into())));
    let saved = desktop.window(WindowId(2)).unwrap().clone();
    desktop.set_window_role(WindowId(2), WindowRole::Launcher);
    desktop.command(Command::Focus(WindowId(2)));
    let mut calls = 0;
    desktop.placements_with(|_, context| {
        calls += 1;
        assert_eq!(
            context
                .windows
                .iter()
                .map(|window| window.id)
                .collect::<Vec<_>>(),
            vec![WindowId(1), WindowId(3)]
        );
        assert_eq!(context.focused, None);
        None
    });
    assert_eq!(calls, 1);
    desktop.set_window_role(WindowId(2), WindowRole::Normal);
    assert_eq!(desktop.window(WindowId(2)), Some(&saved));
    desktop.placements_with(|_, context| {
        assert_eq!(
            context
                .windows
                .iter()
                .map(|window| window.id)
                .collect::<Vec<_>>(),
            vec![WindowId(1), WindowId(2), WindowId(3)]
        );
        assert_eq!(context.focused, Some(WindowId(2)));
        None
    });
    assert_invariants(&desktop);
}

#[test]
fn global_stacking_keeps_launchers_above_floats_and_tiles_across_outputs() {
    let mut desktop = populated(3);
    desktop.set_window_role(WindowId(3), WindowRole::Launcher);
    desktop.command(Command::Focus(WindowId(2)));
    desktop.command(Command::ToggleFloating);
    output(&mut desktop, 2);
    desktop.command(Command::FocusOutput(OutputId(2)));
    for id in 4..=6 {
        window(&mut desktop, id);
    }
    desktop.set_window_role(WindowId(6), WindowRole::Launcher);
    desktop.command(Command::Focus(WindowId(5)));
    desktop.command(Command::ToggleFloating);
    output(&mut desktop, 3);
    desktop.command(Command::FocusOutput(OutputId(3)));
    desktop.command(Command::SetWorkspaceMode(Mode::Floating));
    window(&mut desktop, 7);
    for (focus, expected) in [
        (2, vec![1, 4, 5, 7, 2, 3, 6]),
        (3, vec![1, 4, 2, 5, 7, 6, 3]),
        (7, vec![1, 4, 2, 5, 7, 3, 6]),
        (1, vec![1, 4, 2, 5, 7, 3, 6]),
    ] {
        desktop.command(Command::Focus(WindowId(focus)));
        assert_eq!(
            desktop
                .placements()
                .iter()
                .map(|placement| placement.window.0)
                .collect::<Vec<_>>(),
            expected
        );
        assert_invariants(&desktop);
    }
}

#[test]
fn launchers_keep_workspace_ownership_and_removal_restores_normal_focus() {
    let mut desktop = populated(2);
    let saved = desktop.window(WindowId(2)).unwrap().floating_rect;
    desktop.set_window_role(WindowId(2), WindowRole::Launcher);
    desktop.command(Command::MoveToWorkspace(WorkspaceId(3)));
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().workspace,
        WorkspaceId(3)
    );
    assert_eq!(ids(&desktop.placements()), [WindowId(1)].into());
    desktop.command(Command::Focus(WindowId(2)));
    assert_eq!(ids(&desktop.placements()), [WindowId(2)].into());
    output(&mut desktop, 2);
    desktop.command(Command::StretchAll);
    assert_invariants(&desktop);
    desktop.command(Command::MoveToOutput(OutputId(2)));
    desktop.command(Command::Unstretch);
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().workspace,
        WorkspaceId(3)
    );
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().role,
        WindowRole::Launcher
    );
    assert_eq!(desktop.window(WindowId(2)).unwrap().floating_rect, saved);
    assert_invariants(&desktop);
    window(&mut desktop, 3);
    desktop.command(Command::Focus(WindowId(2)));
    assert_eq!(
        desktop.command(Command::CloseFocused),
        vec![Effect::Close(WindowId(2))]
    );
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    desktop.remove_window(WindowId(2));
    assert_eq!(desktop.focused_window(), Some(WindowId(3)));
    assert_invariants(&desktop);
}

#[test]
fn unbounded_centering_preserves_oversized_extents_at_coordinate_limits() {
    for area in [
        Rect::new(i32::MIN, i32::MIN, 1, 1),
        Rect::new(i32::MAX - 1, i32::MAX - 1, 1, 1),
    ] {
        let rect = area.centered_unbounded(i32::MAX, i32::MAX);
        assert_eq!(rect.width, i32::MAX);
        assert_eq!(rect.height, i32::MAX);
        assert_eq!(rect, rect.normalized());
    }
}

#[test]
fn scrolling_reveals_focus_and_clamps_manual_scroll() {
    let mut desktop = populated(8);
    desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
    let workspace = WorkspaceId(1);
    let output = OutputId(1);
    assert!(desktop.workspace(workspace).unwrap().scroll_offset(output) > 0);
    let focused = desktop
        .placements()
        .into_iter()
        .find(|placement| placement.focused)
        .unwrap();
    assert_eq!(
        focused.rect.intersection(focused.clip.unwrap()),
        Some(focused.rect)
    );
    desktop.command(Command::Focus(WindowId(1)));
    assert_eq!(
        desktop.workspace(workspace).unwrap().scroll_offset(output),
        0
    );
    desktop.command(Command::Scroll(i32::MAX));
    let end = desktop.workspace(workspace).unwrap().scroll_offset(output);
    assert!(end > 0 && end < i32::MAX);
    desktop.command(Command::Scroll(i32::MAX));
    assert_eq!(
        desktop.workspace(workspace).unwrap().scroll_offset(output),
        end
    );
    desktop.command(Command::Scroll(i32::MIN));
    assert_eq!(
        desktop.workspace(workspace).unwrap().scroll_offset(output),
        0
    );
    desktop.command(Command::FocusPrevious);
    assert_eq!(desktop.focused_window(), Some(WindowId(8)));
    assert_eq!(
        desktop.workspace(workspace).unwrap().scroll_offset(output),
        end
    );
    for id in 2..=8 {
        desktop.remove_window(WindowId(id));
    }
    assert_eq!(
        desktop.workspace(workspace).unwrap().scroll_offset(output),
        0
    );
    assert_invariants(&desktop);
}

#[test]
fn scroll_state_is_independent_per_workspace_and_output_and_survives_modes() {
    let mut desktop = populated(6);
    output(&mut desktop, 2);
    desktop.command(Command::StretchAll);
    desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
    let saved = desktop
        .workspace(WorkspaceId(1))
        .unwrap()
        .scroll_offset(OutputId(1));
    assert!(saved > 0);
    desktop.command(Command::SetWorkspaceMode(Mode::Columns));
    assert_eq!(
        desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        saved
    );
    desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
    assert_eq!(
        desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        saved
    );
    desktop.command(Command::FocusOutput(OutputId(2)));
    assert_eq!(
        desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(2)),
        0
    );
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert_eq!(
        desktop
            .workspace(WorkspaceId(2))
            .unwrap()
            .scroll_offset(OutputId(1)),
        0
    );
    desktop.command(Command::SwitchWorkspace(WorkspaceId(1)));
    assert_eq!(
        desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        saved
    );
    assert_invariants(&desktop);
}

#[test]
fn manually_scrolled_position_survives_mode_and_workspace_round_trips() {
    let mut desktop = populated(6);
    desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
    desktop.command(Command::Scroll(i32::MIN));
    desktop.command(Command::Scroll(123));
    for mode in [
        Mode::Floating,
        Mode::Columns,
        Mode::Monocle,
        Mode::MasterStack,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Script("custom".into()),
    ] {
        desktop.command(Command::SetWorkspaceMode(mode));
        desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
        assert_eq!(
            desktop
                .workspace(WorkspaceId(1))
                .unwrap()
                .scroll_offset(OutputId(1)),
            123
        );
    }
    desktop.command(Command::SwitchWorkspace(WorkspaceId(2)));
    desktop.command(Command::SwitchWorkspace(WorkspaceId(1)));
    assert_eq!(
        desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1)),
        123
    );
    desktop.command(Command::Focus(WindowId(6)));
    assert!(
        desktop
            .workspace(WorkspaceId(1))
            .unwrap()
            .scroll_offset(OutputId(1))
            > 123
    );
}

#[test]
fn invalid_commands_do_not_reveal_focus_or_change_manual_scroll() {
    let mut desktop = populated(6);
    desktop.command(Command::SetWorkspaceMode(Mode::Scrolling));
    desktop.command(Command::Scroll(i32::MIN));
    for command in [
        Command::Focus(WindowId(999)),
        Command::FocusOutput(OutputId(999)),
        Command::MoveToOutput(OutputId(999)),
        Command::SwitchWorkspace(WorkspaceId(999)),
        Command::MoveToWorkspace(WorkspaceId(999)),
    ] {
        desktop.command(command);
        assert_eq!(
            desktop
                .workspace(WorkspaceId(1))
                .unwrap()
                .scroll_offset(OutputId(1)),
            0
        );
    }
}

#[test]
fn scripts_are_called_per_script_region_without_floating_exceptions() {
    let mut desktop = populated(3);
    output(&mut desktop, 2);
    desktop.command(Command::ToggleFloating);
    desktop.command(Command::StretchAll);
    desktop.command(Command::Focus(WindowId(2)));
    desktop.command(Command::MoveToOutput(OutputId(2)));
    desktop.command(Command::SetWorkspaceMode(Mode::Script("custom".into())));
    let mut calls = Vec::new();
    let placements = desktop.placements_with(|mode, context| {
        assert_eq!(mode, &Mode::Script("custom".into()));
        calls.push(context.clone());
        Some(management::arrange(&Mode::Monocle, context))
    });
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0]
            .windows
            .iter()
            .map(|window| window.id)
            .collect::<Vec<_>>(),
        vec![WindowId(1)]
    );
    assert_eq!(
        calls[1]
            .windows
            .iter()
            .map(|window| window.id)
            .collect::<Vec<_>>(),
        vec![WindowId(2)]
    );
    assert_eq!(calls[0].focused, None);
    assert_eq!(calls[1].focused, Some(WindowId(2)));
    assert_eq!(
        placements
            .iter()
            .filter(|placement| !placement.tiled)
            .count(),
        1
    );
    assert_eq!(
        ids(&placements),
        [WindowId(1), WindowId(2), WindowId(3)].into()
    );
    desktop.command(Command::SetOutputMode(Mode::Columns));
    let mut count = 0;
    desktop.placements_with(|_, _| {
        count += 1;
        None
    });
    assert_eq!(count, 1);
    assert_invariants(&desktop);
}

#[test]
fn malformed_script_membership_falls_back_and_host_cannot_forge_focus() {
    let mut desktop = populated(2);
    desktop.command(Command::SetWorkspaceMode(Mode::Script("broken".into())));
    let fallback = desktop.placements();
    assert_eq!(desktop.placements_with(|_, _| Some(Vec::new())), fallback);
    assert_eq!(
        desktop.placements_with(|_, context| {
            let mut result = management::arrange(&Mode::Columns, context);
            result[1].window = result[0].window;
            Some(result)
        }),
        fallback
    );
    assert_eq!(
        desktop.placements_with(|_, context| {
            let mut result = management::arrange(&Mode::Columns, context);
            result[0].window = WindowId(999);
            Some(result)
        }),
        fallback
    );
    let placements = desktop.placements_with(|_, context| {
        let mut result = management::arrange(&Mode::Columns, context);
        for placement in &mut result {
            placement.focused = true;
            placement.tiled = false;
        }
        Some(result)
    });
    assert_eq!(
        placements
            .iter()
            .filter(|placement| placement.focused)
            .count(),
        1
    );
    assert!(placements.iter().all(|placement| placement.tiled));
}

#[test]
fn builtins_never_call_custom_callback() {
    let mut desktop = populated(2);
    for mode in [
        Mode::Floating,
        Mode::Scrolling,
        Mode::MasterStack,
        Mode::Columns,
        Mode::Rows,
        Mode::Grid,
        Mode::Spiral,
        Mode::Monocle,
    ] {
        desktop.command(Command::SetWorkspaceMode(mode));
        desktop.placements_with(|_, _| panic!("unexpected script evaluation"));
    }
}

#[test]
fn moves_preserve_window_state_and_have_explicit_focus_semantics() {
    let mut desktop = populated(2);
    desktop.command(Command::ToggleFloating);
    let saved = desktop.window(WindowId(2)).unwrap().floating_rect;
    desktop.command(Command::MoveToWorkspace(WorkspaceId(3)));
    assert_eq!(desktop.focused_window(), Some(WindowId(1)));
    assert_eq!(
        desktop.window(WindowId(2)).unwrap().workspace,
        WorkspaceId(3)
    );
    desktop.command(Command::Focus(WindowId(2)));
    assert_eq!(
        desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(3))
    );
    output(&mut desktop, 2);
    let destination = desktop.workspace_for_output(OutputId(2)).unwrap();
    desktop.command(Command::MoveToOutput(OutputId(2)));
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    assert_eq!(desktop.focused_output(), Some(OutputId(2)));
    assert_eq!(desktop.window(WindowId(2)).unwrap().workspace, destination);
    assert!(desktop.window(WindowId(2)).unwrap().floating);
    assert_eq!(desktop.window(WindowId(2)).unwrap().floating_rect, saved);
    assert_invariants(&desktop);
}

#[test]
fn focus_cycles_across_stretched_regions_and_restores_each_region() {
    let mut desktop = populated(3);
    output(&mut desktop, 2);
    desktop.command(Command::StretchAll);
    desktop.command(Command::MoveToOutput(OutputId(2)));
    desktop.command(Command::FocusNext);
    assert_eq!(desktop.focused_window(), Some(WindowId(1)));
    assert_eq!(desktop.focused_output(), Some(OutputId(1)));
    desktop.command(Command::FocusNext);
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    desktop.command(Command::CycleOutput);
    assert_eq!(desktop.focused_window(), Some(WindowId(3)));
    desktop.command(Command::CycleOutput);
    assert_eq!(desktop.focused_window(), Some(WindowId(2)));
    desktop.command(Command::FocusPrevious);
    assert_eq!(desktop.focused_window(), Some(WindowId(1)));
    assert_invariants(&desktop);
}

#[test]
fn effects_do_not_remove_windows_or_execute_processes() {
    let mut desktop = populated(1);
    assert_eq!(
        desktop.command(Command::CloseFocused),
        vec![Effect::Close(WindowId(1))]
    );
    assert!(desktop.window(WindowId(1)).is_some());
    let args = vec![
        "app".into(),
        "argument with spaces".into(),
        "; not a shell".into(),
    ];
    assert_eq!(
        desktop.command(Command::Spawn(args.clone())),
        vec![Effect::Spawn(args)]
    );
    assert_eq!(desktop.command(Command::Quit), vec![Effect::Quit]);
    desktop.remove_window(WindowId(1));
    assert!(desktop.command(Command::CloseFocused).is_empty());
    assert_invariants(&desktop);
}

#[test]
fn invalid_ids_and_empty_desktop_commands_are_safe() {
    for mut desktop in [Desktop::new(), populated(2)] {
        let before = desktop.placements();
        for command in [
            Command::Focus(WindowId(999)),
            Command::FocusOutput(OutputId(999)),
            Command::SwitchWorkspace(WorkspaceId(999)),
            Command::MoveToWorkspace(WorkspaceId(999)),
            Command::MoveToOutput(OutputId(999)),
            Command::SetFloatingRect(WindowId(999), Rect::default()),
        ] {
            desktop.command(command);
        }
        desktop.remove_output(OutputId(999));
        desktop.remove_window(WindowId(999));
        desktop.update_window_metadata(WindowId(999), "ignored".into(), "ignored".into());
        desktop.set_window_role(WindowId(999), WindowRole::Launcher);
        desktop.set_window_committed_size(WindowId(999), 400, 300);
        desktop.set_output_area(OutputId(999), Rect::default());
        desktop.set_workspace_output_mode(WorkspaceId(999), OutputId(1), Some(Mode::Floating));
        assert_eq!(desktop.placements(), before);
        assert_invariants(&desktop);
    }
    let mut desktop = Desktop::new();
    for command in [
        Command::FocusNext,
        Command::FocusPrevious,
        Command::CycleOutput,
        Command::StretchAll,
        Command::Unstretch,
        Command::ToggleFloating,
        Command::Scroll(i32::MAX),
        Command::SetOutputMode(Mode::Scrolling),
        Command::ClearOutputMode,
        Command::CycleMode,
    ] {
        desktop.command(command);
        assert_invariants(&desktop);
    }
}

#[test]
fn tiled_layouts_cover_expected_geometry_and_monocle_raises_focus() {
    let mut desktop = populated(3);
    desktop.set_gaps(0);
    desktop.set_output_area(OutputId(1), Rect::new(-100, 50, 1001, 801));
    desktop.command(Command::SetWorkspaceMode(Mode::Columns));
    let columns = desktop.placements();
    assert_eq!(columns.iter().map(|p| p.rect.width).sum::<i32>(), 1001);
    assert_eq!(columns[0].rect.x, -100);
    assert_eq!(columns[0].rect.right(), columns[1].rect.x);
    assert_eq!(columns[1].rect.right(), columns[2].rect.x);
    assert_eq!(columns[2].rect.right(), 901);
    desktop.command(Command::SetWorkspaceMode(Mode::MasterStack));
    let master = desktop.placements();
    assert_eq!(master[0].rect, Rect::new(-100, 50, 600, 801));
    assert_eq!(master[1].rect, Rect::new(500, 50, 401, 401));
    assert_eq!(master[2].rect, Rect::new(500, 451, 401, 400));
    desktop.command(Command::SetWorkspaceMode(Mode::Monocle));
    desktop.command(Command::Focus(WindowId(1)));
    let monocle = desktop.placements();
    assert_eq!(monocle.last().unwrap().window, WindowId(1));
    assert!(
        monocle
            .iter()
            .all(|p| p.rect == Rect::new(-100, 50, 1001, 801))
    );
}

#[test]
fn rows_grid_and_spiral_place_windows_in_stable_order() {
    let mut desktop = populated(5);
    desktop.set_gaps(0);
    desktop.set_output_area(OutputId(1), Rect::new(-100, 50, 1001, 801));
    let expected = [
        (
            Mode::Rows,
            vec![
                Rect::new(-100, 50, 1001, 161),
                Rect::new(-100, 211, 1001, 160),
                Rect::new(-100, 371, 1001, 160),
                Rect::new(-100, 531, 1001, 160),
                Rect::new(-100, 691, 1001, 160),
            ],
        ),
        (
            Mode::Grid,
            vec![
                Rect::new(-100, 50, 334, 401),
                Rect::new(234, 50, 334, 401),
                Rect::new(568, 50, 333, 401),
                Rect::new(-100, 451, 501, 400),
                Rect::new(401, 451, 500, 400),
            ],
        ),
        (
            Mode::Spiral,
            vec![
                Rect::new(-100, 50, 600, 801),
                Rect::new(500, 50, 401, 480),
                Rect::new(661, 530, 240, 321),
                Rect::new(500, 659, 161, 192),
                Rect::new(500, 530, 161, 129),
            ],
        ),
    ];
    for (mode, rectangles) in expected {
        desktop.command(Command::SetWorkspaceMode(mode));
        let placements = desktop.placements();
        assert_eq!(
            placements
                .iter()
                .map(|placement| placement.window)
                .collect::<Vec<_>>(),
            (1..=5).map(WindowId).collect::<Vec<_>>()
        );
        assert_eq!(
            placements
                .iter()
                .map(|placement| placement.rect)
                .collect::<Vec<_>>(),
            rectangles
        );
    }
}

#[test]
fn new_layouts_respect_gaps_and_do_not_overlap() {
    let context = LayoutContext {
        area: Rect::new(-100, 20, 103, 83),
        windows: (1..=7)
            .map(|id| LayoutWindow {
                id: WindowId(id),
                floating_rect: Rect::default(),
            })
            .collect(),
        focused: None,
        scroll_offset: 0,
        gaps: 4,
    };
    let bounds = context.area.inset(context.gaps);
    for mode in [Mode::Rows, Mode::Grid, Mode::Spiral] {
        let placements = management::arrange(&mode, &context);
        assert_eq!(placements.len(), 7);
        for (index, placement) in placements.iter().enumerate() {
            assert!(placement.tiled);
            assert_eq!(placement.clip, None);
            assert!(placement.rect.x >= bounds.x && placement.rect.y >= bounds.y);
            assert!(placement.rect.right() <= bounds.right());
            assert!(placement.rect.bottom() <= bounds.bottom());
            for other in &placements[..index] {
                assert!(!placement.rect.intersects(other.rect), "{mode:?}");
            }
        }
    }
}

#[test]
fn layouts_handle_empty_tiny_negative_and_extreme_geometry_without_overflow() {
    let areas = [
        Rect::default(),
        Rect::new(0, 0, 1, 1),
        Rect::new(-3000, -2000, 17, 9),
        Rect {
            x: i32::MAX,
            y: i32::MIN,
            width: i32::MAX,
            height: -1,
        },
        Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        Rect::new(0, 0, i32::MAX, i32::MAX),
        Rect::new(i32::MAX - 5, i32::MAX - 3, 20, 20),
    ];
    for area in areas {
        for gaps in [i32::MIN, 0, 1, 8, i32::MAX] {
            for count in [0, 1, 2, 31] {
                let context = LayoutContext {
                    area,
                    gaps,
                    focused: Some(WindowId(count)),
                    scroll_offset: i32::MAX,
                    windows: (1..=count)
                        .map(|id| LayoutWindow {
                            id: WindowId(id),
                            floating_rect: Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
                        })
                        .collect(),
                };
                for mode in [
                    Mode::Floating,
                    Mode::Scrolling,
                    Mode::MasterStack,
                    Mode::Columns,
                    Mode::Rows,
                    Mode::Grid,
                    Mode::Spiral,
                    Mode::Monocle,
                    Mode::Script("fallback".into()),
                ] {
                    let result = management::arrange(&mode, &context);
                    assert_eq!(result.len(), count as usize);
                    for placement in &result {
                        assert_eq!(
                            placement.rect,
                            placement.rect.normalized(),
                            "{mode:?}: {context:?}"
                        );
                        if mode == Mode::Floating {
                            assert_eq!(
                                placement.rect,
                                context
                                    .windows
                                    .iter()
                                    .find(|window| window.id == placement.window)
                                    .unwrap()
                                    .floating_rect
                                    .normalized()
                            );
                            assert!(!placement.tiled);
                            assert_eq!(placement.clip, None);
                        } else if mode != Mode::Scrolling {
                            let rect = placement.rect;
                            let bounds = area.normalized();
                            assert!(rect.x >= bounds.x && rect.y >= bounds.y);
                            assert!(
                                rect.right() <= bounds.right() && rect.bottom() <= bounds.bottom()
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn deterministic_random_commands_preserve_all_global_invariants() {
    let mut desktop = Desktop::new();
    let mut state = 0x4d595df4d0f33173u64;
    for step in 0..4000 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let id = (state >> 32) % 30 + 1;
        let output_id = OutputId(id % 4 + 1);
        let workspace = WorkspaceId(id % 11 + 1);
        let mode = match id % 9 {
            0 => Mode::Floating,
            1 => Mode::Scrolling,
            2 => Mode::MasterStack,
            3 => Mode::Columns,
            4 => Mode::Rows,
            5 => Mode::Grid,
            6 => Mode::Spiral,
            7 => Mode::Monocle,
            _ => Mode::Script("test".into()),
        };
        match (state >> 16) % 24 {
            0 => output(&mut desktop, output_id.0),
            1 => desktop.remove_output(output_id),
            2 | 3 => window(&mut desktop, id),
            4 => desktop.remove_window(WindowId(id)),
            5 => {
                desktop.command(Command::Focus(WindowId(id)));
            }
            6 => {
                desktop.command(Command::FocusOutput(output_id));
            }
            7 => {
                desktop.command(Command::SwitchWorkspace(workspace));
            }
            8 => {
                desktop.command(Command::MoveToWorkspace(workspace));
            }
            9 => {
                desktop.command(Command::MoveToOutput(output_id));
            }
            10 => {
                desktop.command(Command::StretchAll);
            }
            11 => {
                desktop.command(Command::Unstretch);
            }
            12 => {
                desktop.command(Command::SetWorkspaceMode(mode));
            }
            13 => {
                desktop.command(Command::SetOutputMode(mode));
            }
            14 => {
                desktop.command(Command::ClearOutputMode);
            }
            15 => {
                desktop.command(Command::ToggleFloating);
            }
            16 => {
                desktop.command(Command::Scroll(state as i32));
            }
            17 => {
                desktop.command(Command::SetFloatingRect(
                    WindowId(id),
                    Rect::new(state as i32, (state >> 32) as i32, id as i32 * 100, 800),
                ));
            }
            18 => {
                desktop.command(Command::FocusNext);
            }
            19 => {
                desktop.command(Command::FocusPrevious);
            }
            20 => desktop.set_output_area(
                output_id,
                Rect::new(-500, -100, (step % 1300) as i32, (step % 900) as i32),
            ),
            21 => desktop.set_gaps((state % 2000) as i32 - 10),
            22 => desktop.configure_workspace(workspace, format!("workspace-{id}"), mode),
            _ => {
                desktop.command(Command::CycleOutput);
            }
        }
        assert_invariants(&desktop);
    }
}
