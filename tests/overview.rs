use clear::{
    config::Config,
    core::{Command, OutputId, Rect, WindowId, WindowRole, WorkspaceId},
    input::{Action, Bindings, Modifiers},
    runtime::{OverviewNavigation as Nav, OverviewTarget as Target, Runtime},
    shell,
};

fn runtime() -> Runtime {
    let mut r = Runtime::new(Config::default(), None).unwrap();
    r.desktop
        .add_output(OutputId(1), "one".into(), Rect::new(0, 0, 800, 600));
    r.desktop
        .add_output(OutputId(2), "two".into(), Rect::new(800, 0, 800, 600));
    r.desktop.command(Command::FocusOutput(OutputId(1)));
    for id in 1..=3 {
        r.desktop
            .add_window(WindowId(id), format!("Window {id}"), "test".into());
    }
    r
}

#[test]
fn workspace_hover_preference_defaults_off_and_rejects_invalid_schema() {
    assert!(!Config::default().overview.preview_workspace_on_hover);
    assert!(
        !Config::from_source("[overview]")
            .unwrap()
            .overview
            .preview_workspace_on_hover
    );
    assert!(
        Config::from_source("[overview]\npreview_workspace_on_hover=true")
            .unwrap()
            .overview
            .preview_workspace_on_hover
    );
    for text in [
        "[overview]\npreview_workspace_on_hover=1",
        "[overview]\nhover=true",
    ] {
        assert!(Config::from_source(text).is_err());
    }
}

#[test]
fn overview_drop_moves_an_unfocused_minimized_window_without_following_or_restoring() {
    let mut r = runtime();
    r.desktop.command(Command::SetFloatingRect(
        WindowId(2),
        Rect::new(-50, 70, 420, 310),
    ));
    r.desktop.command(Command::SetMaximized(WindowId(2), true));
    r.desktop.command(Command::SetMinimized(WindowId(2), true));
    let window = r.desktop.window(WindowId(2)).unwrap().clone();
    let groups = r.desktop.groups().to_vec();
    let focus = r.desktop.focused_window();
    r.toggle_overview();
    let session = r.overview.as_mut().unwrap();
    assert!(session.move_window(&mut r.desktop, WindowId(2), WorkspaceId(3)));
    assert_eq!(session.workspace, WorkspaceId(1));
    assert!(!session.windows.contains(&WindowId(2)));
    assert_eq!(r.desktop.groups(), groups);
    assert_eq!(r.desktop.focused_window(), focus);
    let moved = r.desktop.window(WindowId(2)).unwrap();
    assert_eq!(moved.workspace, WorkspaceId(3));
    assert_eq!(moved.floating_rect, window.floating_rect);
    assert_eq!(moved.floating, window.floating);
    assert_eq!(moved.maximized, window.maximized);
    assert_eq!(moved.minimized, window.minimized);
    r.cancel_overview();
    assert_eq!(
        r.desktop.window(WindowId(2)).unwrap().workspace,
        WorkspaceId(3)
    );
}

#[test]
fn invalid_or_stale_drops_are_noops_and_focused_drop_repairs_source_focus() {
    let mut r = runtime();
    r.toggle_overview();
    let before = shell::snapshot(&r);
    let session = r.overview.as_mut().unwrap();
    for (window, workspace) in [
        (WindowId(3), WorkspaceId(1)),
        (WindowId(3), WorkspaceId(999)),
        (WindowId(999), WorkspaceId(3)),
    ] {
        assert!(!session.move_window(&mut r.desktop, window, workspace));
    }
    assert_eq!(shell::snapshot(&r), before);
    r.desktop.set_window_role(WindowId(2), WindowRole::Launcher);
    assert!(
        !r.overview
            .as_mut()
            .unwrap()
            .move_window(&mut r.desktop, WindowId(2), WorkspaceId(3))
    );
    assert!(
        r.overview
            .as_mut()
            .unwrap()
            .move_window(&mut r.desktop, WindowId(3), WorkspaceId(2))
    );
    assert_eq!(
        r.desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(1))
    );
    assert_ne!(r.desktop.focused_window(), Some(WindowId(3)));
    assert_eq!(
        r.desktop.window(WindowId(3)).unwrap().output,
        Some(OutputId(2))
    );
    assert_eq!(r.overview.as_ref().unwrap().workspace, WorkspaceId(1));
    assert_eq!(
        r.overview.as_ref().unwrap().selected,
        Target::Window(WindowId(1))
    );
    // The explicit command also rejects invalid/same-workspace targets atomically.
    let before = shell::snapshot(&r);
    for command in [
        Command::MoveWindowToWorkspace(WindowId(3), WorkspaceId(2)),
        Command::MoveWindowToWorkspace(WindowId(999), WorkspaceId(1)),
        Command::MoveWindowToWorkspace(WindowId(1), WorkspaceId(999)),
    ] {
        assert!(r.desktop.command(command).is_empty());
        assert_eq!(shell::snapshot(&r), before);
    }
}

#[test]
fn entry_cancels_switcher_and_preview_cancel_preserves_all_desktop_state() {
    let mut r = runtime();
    r.advance_switcher();
    let mut before = shell::snapshot(&r);
    before.switcher = None;
    let placements = r.placements();
    r.toggle_overview();
    assert!(r.switcher.is_none());
    assert_eq!(
        r.overview.as_ref().unwrap().original_focus,
        Some(WindowId(3))
    );
    let s = r.overview.as_mut().unwrap();
    s.select(&r.desktop, Target::Workspace(WorkspaceId(2)));
    assert_eq!(s.windows, Vec::<WindowId>::new());
    s.navigate(&r.desktop, Nav::WorkspaceNext, 2);
    assert_eq!(s.workspace, WorkspaceId(3));
    assert_eq!(r.placements(), placements);
    r.cancel_overview();
    assert_eq!(shell::snapshot(&r), before);
}

#[test]
fn window_activation_reveals_hidden_workspace_and_restores_only_selected_window() {
    let mut r = runtime();
    r.desktop.command(Command::SetMinimized(WindowId(2), true));
    r.desktop.command(Command::SetMinimized(WindowId(3), true));
    r.desktop.command(Command::SwitchWorkspace(WorkspaceId(3)));
    r.toggle_overview();
    let s = r.overview.as_mut().unwrap();
    s.select(&r.desktop, Target::Workspace(WorkspaceId(1)));
    assert_eq!(s.windows, vec![WindowId(1), WindowId(2), WindowId(3)]);
    s.select(&r.desktop, Target::Window(WindowId(2)));
    assert!(r.desktop.window(WindowId(2)).unwrap().minimized);
    r.finish_overview();
    assert_eq!(
        r.desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(1))
    );
    assert_eq!(r.desktop.focused_window(), Some(WindowId(2)));
    assert!(!r.desktop.window(WindowId(2)).unwrap().minimized);
    assert!(r.desktop.window(WindowId(3)).unwrap().minimized);
}

#[test]
fn workspace_activation_swaps_groups_and_keeps_minimized_windows_hidden() {
    let mut r = runtime();
    r.desktop.command(Command::SetMinimized(WindowId(3), true));
    r.toggle_overview();
    r.overview
        .as_mut()
        .unwrap()
        .select(&r.desktop, Target::Workspace(WorkspaceId(2)));
    r.finish_overview();
    assert_eq!(
        r.desktop.workspace_for_output(OutputId(1)),
        Some(WorkspaceId(2))
    );
    assert_eq!(
        r.desktop.workspace_for_output(OutputId(2)),
        Some(WorkspaceId(1))
    );
    assert!(r.desktop.window(WindowId(3)).unwrap().minimized);
}

#[test]
fn stretched_workspace_has_one_candidate_order_and_launchers_are_excluded() {
    let mut r = runtime();
    r.desktop.set_window_role(WindowId(2), WindowRole::Launcher);
    r.desktop.command(Command::StretchAll);
    r.toggle_overview();
    let s = r.overview.as_ref().unwrap();
    assert_eq!(s.windows, vec![WindowId(1), WindowId(3)]);
    assert_eq!(
        s.workspaces
            .iter()
            .filter(|id| **id == WorkspaceId(1))
            .count(),
        1
    );
}

#[test]
fn selection_repairs_removal_role_and_workspace_changes() {
    let mut r = runtime();
    r.toggle_overview();
    r.overview
        .as_mut()
        .unwrap()
        .select(&r.desktop, Target::Window(WindowId(2)));
    r.desktop.remove_window(WindowId(2));
    r.refresh_overview();
    assert_eq!(
        r.overview.as_ref().unwrap().selected,
        Target::Window(WindowId(3))
    );
    r.desktop.set_window_role(WindowId(3), WindowRole::Launcher);
    r.refresh_overview();
    assert_eq!(
        r.overview.as_ref().unwrap().selected,
        Target::Window(WindowId(1))
    );
    r.desktop.command(Command::Focus(WindowId(1)));
    r.desktop.command(Command::MoveToWorkspace(WorkspaceId(3)));
    r.refresh_overview();
    assert_eq!(
        r.overview.as_ref().unwrap().selected,
        Target::Workspace(WorkspaceId(1))
    );
}

#[test]
fn output_loss_moves_session_then_closes_without_dropping_windows() {
    let mut r = runtime();
    r.toggle_overview();
    r.desktop.remove_output(OutputId(1));
    r.refresh_overview();
    assert_eq!(r.overview.as_ref().unwrap().output, OutputId(2));
    r.desktop.remove_output(OutputId(2));
    r.refresh_overview();
    assert!(r.overview.is_none());
    assert_eq!(r.desktop.windows().count(), 3);
    r.toggle_overview();
    assert!(r.overview.is_none());
}

#[test]
fn keyboard_navigation_visits_workspace_strip_all_cards_and_empty_workspaces() {
    let mut r = runtime();
    r.toggle_overview();
    let s = r.overview.as_mut().unwrap();
    s.select(&r.desktop, Target::Workspace(WorkspaceId(1)));
    for target in [
        Target::Window(WindowId(1)),
        Target::Window(WindowId(2)),
        Target::Window(WindowId(3)),
        Target::Workspace(WorkspaceId(1)),
    ] {
        s.navigate(&r.desktop, Nav::Next, 2);
        assert_eq!(s.selected, target);
    }
    s.navigate(&r.desktop, Nav::Previous, 2);
    assert_eq!(s.selected, Target::Window(WindowId(3)));
    s.navigate(&r.desktop, Nav::Up, 2);
    assert_eq!(s.selected, Target::Window(WindowId(1)));
    s.navigate(&r.desktop, Nav::Up, 2);
    assert_eq!(s.selected, Target::Workspace(WorkspaceId(1)));
    s.navigate(&r.desktop, Nav::Down, 2);
    assert_eq!(s.selected, Target::Window(WindowId(1)));
    s.navigate(&r.desktop, Nav::WorkspacePrevious, 2);
    assert_eq!(s.workspace, WorkspaceId(9));
    s.navigate(&r.desktop, Nav::Down, 0);
    assert_eq!(s.selected, Target::Workspace(WorkspaceId(9)));
}

#[test]
fn pointer_output_selection_does_not_focus_output_or_swap_workspace() {
    let mut r = runtime();
    let mut before = shell::snapshot(&r);
    before.overview_open = true;
    r.toggle_overview();
    r.overview
        .as_mut()
        .unwrap()
        .select_output(&r.desktop, OutputId(2));
    assert_eq!(r.overview.as_ref().unwrap().workspace, WorkspaceId(2));
    assert_eq!(shell::snapshot(&r), before);
}

#[test]
fn default_binding_and_strict_optional_ipc_only_request_adapter_authorization() {
    let mut r = runtime();
    assert_eq!(
        Bindings::new(&r.config.bindings).unwrap().action(
            Modifiers {
                logo: true,
                ..Default::default()
            },
            "w"
        ),
        Some(Action::ToggleOverview)
    );
    r.action(Action::ToggleOverview);
    assert!(r.overview_requested);
    assert!(r.overview.is_none());
    r.toggle_overview();
    assert!(shell::snapshot(&r).overview_open);
    let request =
        shell::parse_request(br#"{"version":1,"id":1,"request":{"type":"toggle_overview"}}"#)
            .unwrap();
    shell::execute(&mut r, &request).unwrap();
    assert!(r.overview_requested);
    r.toggle_overview();
    assert!(!shell::snapshot(&r).overview_open);
    for bad in [
        br#"{"version":1,"id":1,"request":{"type":"toggle_overview","output":"1"}}"#.as_slice(),
        br#"{"version":2,"id":1,"request":{"type":"toggle_overview"}}"#.as_slice(),
    ] {
        assert!(shell::parse_request(bad).is_err());
    }
    assert!(
        Config::from_source("[[bindings]]\nkey='leader+w'\naction='toggle_overview'\nwindow=1")
            .is_err()
    );
}

#[test]
fn declarative_script_toggle_obeys_the_same_queued_input_authorization() {
    let mut host = clear::scripting::ScriptHost::from_source(
        "fn overview() { [#{action: \"toggle_overview\"}] }",
    )
    .unwrap();
    assert_eq!(
        host.action("overview").unwrap(),
        vec![Action::ToggleOverview]
    );
    let mut invalid = clear::scripting::ScriptHost::from_source(
        "fn overview() { [#{action: \"toggle_overview\", output: 1}] }",
    )
    .unwrap();
    assert!(invalid.action("overview").is_err());
}
