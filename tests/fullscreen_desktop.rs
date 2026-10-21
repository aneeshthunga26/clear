use clear::{
    config::Config,
    core::{
        Command, Desktop, Mode, OutputId, PresentationTarget, Rect, WindowId, WindowRole,
        WorkspaceId,
    },
    runtime::{OverviewSession, OverviewTarget, Runtime},
    shell::{Request, RequestKind, execute, snapshot},
};

fn desktop() -> Desktop {
    let mut d = Desktop::new();
    d.add_output_with_bounds(
        OutputId(1),
        "one".into(),
        Rect::new(0, 0, 801, 601),
        Rect::new(0, 32, 801, 569),
    );
    d.add_window(WindowId(1), "browser".into(), "browser".into());
    d.add_window(WindowId(2), "terminal".into(), "terminal".into());
    d
}

#[test]
fn one_slot_replaces_atomically_without_mutating_home_or_accepting_stale_exits() {
    let mut d = desktop();
    let original = d.window(WindowId(1)).unwrap().clone();
    let order = d.workspace(WorkspaceId(1)).unwrap().windows().to_vec();
    assert!(!d.activate_fullscreen());
    assert!(d.request_fullscreen(WindowId(1), None, true));
    assert_eq!(
        d.presentation_for_output(OutputId(1)),
        Some(PresentationTarget::Fullscreen)
    );
    assert_eq!(d.placements().len(), 1);
    assert_eq!(d.placements()[0].rect, Rect::new(0, 0, 801, 601));
    let revision = d.fullscreen_revision();
    assert!(d.request_fullscreen(WindowId(1), None, true));
    assert_eq!(d.fullscreen_revision(), revision);
    assert!(!d.request_fullscreen(WindowId(99), None, true));
    d.command(Command::SetMinimized(WindowId(2), true));
    assert!(!d.request_fullscreen(WindowId(2), None, true));
    assert_eq!(d.fullscreen_slot().unwrap().window, WindowId(1));
    d.command(Command::SetMinimized(WindowId(2), false));
    assert!(d.request_fullscreen(WindowId(2), None, true));
    assert!(!d.window(WindowId(1)).unwrap().fullscreen);
    assert_eq!(d.windows().filter(|w| w.fullscreen).count(), 1);
    assert!(!d.leave_fullscreen(WindowId(1)));
    assert_eq!(d.fullscreen_slot().unwrap().window, WindowId(2));
    let restored = d.window(WindowId(1)).unwrap();
    assert_eq!(restored.workspace, original.workspace);
    assert_eq!(restored.output, original.output);
    assert_eq!(restored.floating_rect, original.floating_rect);
    assert_eq!(d.workspace(WorkspaceId(1)).unwrap().windows(), order);
}

#[test]
fn presentation_output_and_stretched_group_are_independent_of_saved_home() {
    let mut d = desktop();
    d.add_output(OutputId(2), "two".into(), Rect::new(801, 0, 640, 480));
    d.command(Command::StretchAll);
    let home = d.window(WindowId(1)).unwrap().output;
    assert!(d.request_fullscreen(WindowId(1), Some(OutputId(2)), true));
    assert_eq!(d.window(WindowId(1)).unwrap().output, home);
    assert_eq!(d.presentation_output(WindowId(1)), Some(OutputId(2)));
    assert_eq!(d.groups()[0].outputs, vec![OutputId(1), OutputId(2)]);
    assert_eq!(
        d.presentation_for_output(OutputId(1)),
        Some(PresentationTarget::Workspace(WorkspaceId(1)))
    );
    assert_eq!(
        d.presentation_for_output(OutputId(2)),
        Some(PresentationTarget::Fullscreen)
    );
    assert!(d.placements().iter().any(|p| p.window == WindowId(2)));
    assert!(d.window_presented_on(WindowId(1), OutputId(2)));
    assert!(!d.window_presented_on(WindowId(1), OutputId(1)));
    assert!(!d.window_presented_on(WindowId(2), OutputId(2)));
    d.remove_output(OutputId(2));
    assert_eq!(d.fullscreen_slot().unwrap().output, Some(OutputId(1)));
    assert_eq!(
        d.fullscreen_slot().unwrap().return_workspace,
        WorkspaceId(1)
    );
    assert_eq!(d.placements()[0].window, WindowId(1));
    d.remove_output(OutputId(1));
    assert!(d.placements().is_empty());
    assert!(d.fullscreen_slot().is_some());
    d.add_output(OutputId(3), "three".into(), Rect::new(0, 0, 500, 400));
    assert_eq!(d.fullscreen_slot().unwrap().output, Some(OutputId(3)));
    assert_eq!(d.placements()[0].rect, Rect::new(0, 0, 500, 400));
}

#[test]
fn desktop_switches_hide_without_unfullscreening_and_explicit_focus_reactivates() {
    let mut d = desktop();
    d.request_fullscreen(WindowId(1), None, true);
    d.command(Command::SwitchWorkspace(WorkspaceId(1)));
    assert!(!d.fullscreen_slot().unwrap().active);
    assert!(d.window(WindowId(1)).unwrap().fullscreen);
    assert_eq!(
        d.placements().iter().map(|p| p.window).collect::<Vec<_>>(),
        vec![WindowId(2)]
    );
    d.command(Command::SwitchWorkspace(WorkspaceId(3)));
    assert!(d.placements().is_empty());
    d.command(Command::Focus(WindowId(1)));
    assert!(d.fullscreen_slot().unwrap().active);
    assert_eq!(d.workspace_for_output(OutputId(1)), Some(WorkspaceId(3)));
    d.command(Command::SetFullscreen(WindowId(1), false));
    assert_eq!(d.workspace_for_output(OutputId(1)), Some(WorkspaceId(3)));
    assert!(d.fullscreen_slot().is_none());
    assert!(d.placements().is_empty());
}

#[test]
fn minimize_transfer_and_reclassification_release_the_slot_and_preserve_restore_state() {
    for mode in [
        Mode::Floating,
        Mode::Columns,
        Mode::Scrolling,
        Mode::Monocle,
    ] {
        let mut d = desktop();
        d.command(Command::SetWorkspaceMode(mode));
        d.command(Command::Focus(WindowId(1)));
        let saved = d.window(WindowId(1)).unwrap().floating_rect;
        let normal = d.placements();
        d.command(Command::ToggleFullscreen);
        d.command(Command::ToggleFullscreen);
        assert_eq!(d.placements(), normal);
        d.command(Command::SetMaximized(WindowId(1), true));
        d.command(Command::ToggleFullscreen);
        d.command(Command::MinimizeFocused);
        assert!(d.fullscreen_slot().is_none());
        let w = d.window(WindowId(1)).unwrap();
        assert!(w.minimized && w.maximized && !w.fullscreen);
        assert_eq!(w.floating_rect, saved);
        d.command(Command::Focus(WindowId(1)));
        d.command(Command::ToggleFullscreen);
        d.command(Command::MoveWindowToWorkspace(WindowId(1), WorkspaceId(1)));
        assert!(d.fullscreen_slot().is_none());
        d.command(Command::Focus(WindowId(1)));
        d.command(Command::ToggleFullscreen);
        d.set_window_role(WindowId(1), WindowRole::Launcher);
        assert!(d.fullscreen_slot().is_none());
        assert!(!d.window(WindowId(1)).unwrap().fullscreen);
    }
}

#[test]
fn occupied_navigation_overview_and_ipc_do_not_fabricate_a_workspace() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    runtime.desktop = desktop();
    runtime.desktop.request_fullscreen(WindowId(1), None, true);
    let mut overview = OverviewSession::new(&runtime.desktop).unwrap();
    assert!(overview.fullscreen_selected);
    assert_eq!(overview.windows, vec![WindowId(1)]);
    assert_eq!(
        overview.desktop_targets().last(),
        Some(&OverviewTarget::Fullscreen)
    );
    overview.select(&runtime.desktop, OverviewTarget::Workspace(WorkspaceId(1)));
    assert_eq!(overview.windows, vec![WindowId(2)]);
    let state = snapshot(&runtime);
    assert_eq!(state.workspaces.len(), 9);
    assert_eq!(state.fullscreen_desktop.as_ref().unwrap().window, "1");
    assert!(state.fullscreen_desktop.unwrap().active);
    runtime
        .desktop
        .command(Command::SwitchWorkspace(WorkspaceId(9)));
    runtime.desktop.command(Command::CycleDesktop(false));
    assert_eq!(
        runtime.desktop.presentation_for_output(OutputId(1)),
        Some(PresentationTarget::Fullscreen)
    );
    runtime.desktop.command(Command::CycleDesktop(false));
    assert_eq!(
        runtime.desktop.presentation_for_output(OutputId(1)),
        Some(PresentationTarget::Workspace(WorkspaceId(1)))
    );
    let request = Request {
        version: 1,
        id: 1,
        request: RequestKind::ActivateFullscreen,
    };
    execute(&mut runtime, &request).unwrap();
    assert_eq!(runtime.desktop.focused_window(), Some(WindowId(1)));
    runtime.desktop.remove_window(WindowId(1));
    overview.refresh(&runtime.desktop);
    assert!(
        !overview
            .desktop_targets()
            .contains(&OverviewTarget::Fullscreen)
    );
    assert!(execute(&mut runtime, &request).is_err());
    assert!(snapshot(&runtime).fullscreen_desktop.is_none());
}

#[test]
fn alt_tab_on_fullscreen_includes_the_return_desktop_and_cancel_preserves_occupancy() {
    let mut runtime = Runtime::new(Config::default(), None).unwrap();
    runtime.desktop = desktop();
    runtime.desktop.request_fullscreen(WindowId(1), None, true);
    runtime.advance_switcher();
    let selected = snapshot(&runtime).switcher.unwrap();
    assert_eq!(selected.windows, vec!["2", "1"]);
    assert_eq!(selected.selected, "2");
    runtime.cancel_switcher();
    assert!(runtime.desktop.fullscreen_slot().unwrap().active);
    runtime.advance_switcher();
    runtime.finish_switcher();
    assert_eq!(runtime.desktop.focused_window(), Some(WindowId(2)));
    assert!(!runtime.desktop.fullscreen_slot().unwrap().active);
    assert!(runtime.desktop.window(WindowId(1)).unwrap().fullscreen);
}

#[test]
fn related_dialogs_share_fullscreen_without_becoming_another_occupant() {
    let mut d = desktop();
    d.add_output(OutputId(2), "two".into(), Rect::new(801, 0, 640, 480));
    d.request_fullscreen(WindowId(1), Some(OutputId(2)), true);
    d.add_window(WindowId(3), "dialog".into(), "browser".into());
    d.set_window_parent(WindowId(3), Some(WindowId(1)));
    d.command(Command::SetFloatingRect(
        WindowId(3),
        Rect::new(900, 100, 250, 180),
    ));
    d.command(Command::Focus(WindowId(3)));
    assert_eq!(d.focused_window(), Some(WindowId(3)));
    assert_eq!(d.fullscreen_slot().unwrap().window, WindowId(1));
    assert!(!d.window(WindowId(3)).unwrap().fullscreen);
    assert!(d.is_fullscreen_transient(WindowId(3)));
    assert_eq!(d.placements().last().unwrap().window, WindowId(3));
    assert!(
        d.begin_move(
            WindowId(3),
            d.placements().last().unwrap().rect,
            (920., 120.)
        )
        .is_none()
    );
    d.set_window_parent(WindowId(1), Some(WindowId(3)));
    assert_eq!(d.window(WindowId(1)).unwrap().transient_for, None);
    d.set_window_parent(WindowId(3), Some(WindowId(99)));
    assert_eq!(
        d.window(WindowId(3)).unwrap().transient_for,
        Some(WindowId(1))
    );
    d.command(Command::SwitchWorkspace(WorkspaceId(2)));
    assert!(!d.placements().iter().any(|p| p.window == WindowId(3)));
    d.command(Command::Focus(WindowId(3)));
    assert!(d.fullscreen_slot().unwrap().active);
    assert_eq!(d.focused_window(), Some(WindowId(3)));
    let overview = OverviewSession::new(&d).unwrap();
    assert_eq!(overview.windows, vec![WindowId(1), WindowId(3)]);
    d.command(Command::SetMinimized(WindowId(3), true));
    assert_eq!(
        OverviewSession::new(&d).unwrap().windows,
        vec![WindowId(1), WindowId(3)]
    );
    d.command(Command::Focus(WindowId(3)));
    assert!(!d.window(WindowId(3)).unwrap().minimized);
    d.remove_window(WindowId(1));
    assert_eq!(d.window(WindowId(3)).unwrap().transient_for, None);
    assert!(d.fullscreen_slot().is_none());
}

#[test]
fn exiting_on_another_output_returns_to_the_invoking_desktop_and_focus() {
    let mut d = desktop();
    d.add_output(OutputId(2), "two".into(), Rect::new(801, 0, 640, 480));
    d.command(Command::FocusOutput(OutputId(2)));
    d.add_window(WindowId(3), "other".into(), "other".into());
    d.command(Command::Focus(WindowId(1)));
    d.request_fullscreen(WindowId(1), Some(OutputId(2)), true);
    assert_eq!(
        d.fullscreen_slot().unwrap().return_output,
        Some(OutputId(1))
    );
    d.leave_fullscreen(WindowId(1));
    assert_eq!(d.focused_output(), Some(OutputId(1)));
    assert_eq!(d.focused_window(), Some(WindowId(1)));
    d.request_fullscreen(WindowId(1), Some(OutputId(2)), true);
    d.command(Command::SwitchWorkspace(WorkspaceId(2)));
    d.command(Command::ActivateFullscreen);
    assert_eq!(
        d.fullscreen_slot().unwrap().return_output,
        Some(OutputId(2))
    );
    d.leave_fullscreen(WindowId(1));
    assert_eq!(d.focused_output(), Some(OutputId(2)));
    assert_eq!(d.focused_window(), Some(WindowId(3)));
    d.command(Command::Focus(WindowId(1)));
    d.request_fullscreen(WindowId(1), Some(OutputId(2)), true);
    d.command(Command::FocusOutput(OutputId(1)));
    d.command(Command::SwitchWorkspace(WorkspaceId(3)));
    d.add_window(WindowId(4), "new return context".into(), "other".into());
    d.command(Command::Focus(WindowId(1)));
    d.leave_fullscreen(WindowId(1));
    assert_eq!(d.focused_output(), Some(OutputId(1)));
    assert_eq!(d.workspace_for_output(OutputId(1)), Some(WorkspaceId(3)));
    assert_eq!(d.focused_window(), Some(WindowId(4)));
}
