//! Transient, ID-only overview selection. Preview never computes placements.

use crate::core::{
    Command, Desktop, OutputId, PresentationTarget, WindowId, WindowRole, WorkspaceId,
};

/// One global overview; authoritative ownership and focus remain in the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewSession {
    pub output: OutputId,
    pub original_focus: Option<WindowId>,
    pub workspace: WorkspaceId,
    pub selected: OverviewTarget,
    pub workspaces: Vec<WorkspaceId>,
    pub windows: Vec<WindowId>,
    pub fullscreen: Option<WindowId>,
    pub fullscreen_selected: bool,
}

/// A workspace tile or normal window card, including minimized windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewTarget {
    Workspace(WorkspaceId),
    Fullscreen,
    Window(WindowId),
}

/// Backend-neutral navigation; grid width comes from the rendered layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewNavigation {
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
    WorkspacePrevious,
    WorkspaceNext,
}

impl OverviewSession {
    /// Enter on the focused output without mutating desktop state.
    pub fn new(desktop: &Desktop) -> Option<Self> {
        let output = desktop.focused_output()?;
        let workspace = desktop.workspace_for_output(output)?;
        let original_focus = desktop.focused_window();
        let mut session = Self {
            output,
            original_focus,
            workspace,
            selected: original_focus
                .map_or(OverviewTarget::Workspace(workspace), OverviewTarget::Window),
            workspaces: Vec::new(),
            windows: Vec::new(),
            fullscreen: None,
            fullscreen_selected: desktop.presentation_for_output(output)
                == Some(PresentationTarget::Fullscreen),
        };
        session.refresh(desktop);
        Some(session)
    }

    /// Repair stale IDs, preserving the removed card's position when possible.
    /// False means there are no surviving outputs/workspaces and the UI must close.
    pub fn refresh(&mut self, desktop: &Desktop) -> bool {
        if desktop.output(self.output).is_none() {
            let Some(output) = desktop.outputs().next() else {
                return false;
            };
            self.output = output.id;
        }
        self.workspaces = desktop.workspaces().map(|w| w.id).collect();
        self.fullscreen = desktop
            .fullscreen_slot()
            .filter(|s| s.output == Some(self.output))
            .map(|s| s.window);
        if self.fullscreen_selected && self.fullscreen.is_none() {
            self.fullscreen_selected = false;
            self.selected = OverviewTarget::Workspace(self.workspace);
        }
        if !self.workspaces.contains(&self.workspace) {
            let Some(workspace) = desktop.workspace_for_output(self.output) else {
                return false;
            };
            self.workspace = workspace;
            self.selected = OverviewTarget::Workspace(workspace);
        }
        let old_index = match self.selected {
            OverviewTarget::Window(id) => self.windows.iter().position(|w| *w == id).unwrap_or(0),
            _ => 0,
        };
        self.windows = if self.fullscreen_selected {
            self.fullscreen
                .into_iter()
                .chain(
                    desktop
                        .windows()
                        .filter(|w| desktop.is_fullscreen_transient(w.id))
                        .map(|w| w.id),
                )
                .collect()
        } else {
            desktop
                .workspace(self.workspace)
                .map_or_else(Vec::new, |w| {
                    w.windows()
                        .iter()
                        .copied()
                        .filter(|id| {
                            desktop.window(*id).is_some_and(|w| {
                                w.role == WindowRole::Normal
                                    && !w.fullscreen
                                    && !desktop.is_fullscreen_transient(w.id)
                            })
                        })
                        .collect()
                })
        };
        match self.selected {
            OverviewTarget::Window(id) if !self.windows.contains(&id) => {
                self.selected = self
                    .windows
                    .get(old_index.min(self.windows.len().saturating_sub(1)))
                    .copied()
                    .map_or(self.desktop_target(), OverviewTarget::Window);
            }
            OverviewTarget::Workspace(_) => {
                self.selected = OverviewTarget::Workspace(self.workspace)
            }
            _ => {}
        }
        true
    }

    /// Preview a workspace or card only after validating its current identity.
    pub fn select(&mut self, desktop: &Desktop, target: OverviewTarget) {
        match target {
            OverviewTarget::Workspace(id) if desktop.workspace(id).is_some() => {
                self.fullscreen_selected = false;
                self.workspace = id;
                self.selected = target;
                self.refresh(desktop);
            }
            OverviewTarget::Fullscreen
                if desktop
                    .fullscreen_slot()
                    .is_some_and(|s| s.output == Some(self.output)) =>
            {
                self.fullscreen_selected = true;
                self.selected = target;
                self.refresh(desktop);
            }
            OverviewTarget::Window(id) if self.windows.contains(&id) => self.selected = target,
            _ => {}
        }
    }

    /// The desktop whose cards are being previewed, independent of card selection.
    pub fn desktop_target(&self) -> OverviewTarget {
        if self.fullscreen_selected {
            OverviewTarget::Fullscreen
        } else {
            OverviewTarget::Workspace(self.workspace)
        }
    }

    /// Ordered tiles, with the occupied fullscreen destination fixed at the end.
    pub fn desktop_targets(&self) -> Vec<OverviewTarget> {
        self.workspaces
            .iter()
            .copied()
            .map(OverviewTarget::Workspace)
            .chain(self.fullscreen.map(|_| OverviewTarget::Fullscreen))
            .collect()
    }

    /// Move the UI to another output, without focusing or switching the desktop.
    pub fn select_output(&mut self, desktop: &Desktop, output: OutputId) {
        if let Some(workspace) = desktop.workspace_for_output(output) {
            self.output = output;
            self.select(desktop, OverviewTarget::Workspace(workspace));
        }
    }

    /// Drop a previewed normal window on another workspace, staying in overview.
    pub fn move_window(
        &mut self,
        desktop: &mut Desktop,
        window: WindowId,
        destination: WorkspaceId,
    ) -> bool {
        if !self.refresh(desktop)
            || !self.windows.contains(&window)
            || desktop.is_fullscreen_transient(window)
            || (destination == self.workspace && !self.fullscreen_selected)
            || desktop.workspace(destination).is_none()
        {
            return false;
        }
        desktop.command(Command::MoveWindowToWorkspace(window, destination));
        self.refresh(desktop);
        true
    }

    /// Navigate every candidate, including cards/workspaces outside the visible page.
    pub fn navigate(&mut self, desktop: &Desktop, direction: OverviewNavigation, columns: usize) {
        if !self.refresh(desktop) {
            return;
        }
        let columns = columns.max(1);
        let window_index = match self.selected {
            OverviewTarget::Window(id) => self.windows.iter().position(|w| *w == id),
            _ => None,
        };
        let workspace_step = match direction {
            OverviewNavigation::WorkspacePrevious => Some(-1),
            OverviewNavigation::WorkspaceNext => Some(1),
            OverviewNavigation::Left if window_index.is_none() => Some(-1),
            OverviewNavigation::Right if window_index.is_none() => Some(1),
            _ => None,
        };
        if let Some(step) = workspace_step {
            let targets = self.desktop_targets();
            let index = targets
                .iter()
                .position(|t| *t == self.desktop_target())
                .unwrap_or(0);
            let index = (index as isize + step).rem_euclid(targets.len() as isize) as usize;
            self.select(desktop, targets[index]);
            return;
        }
        let count = self.windows.len();
        let index = match (direction, window_index) {
            (OverviewNavigation::Next, index) => Some(index.map_or(0, |i| i + 1)),
            (OverviewNavigation::Previous, index) => Some(index.map_or(
                count.saturating_sub(1),
                |i| if i == 0 { count } else { i - 1 },
            )),
            (OverviewNavigation::Down, None) => Some(0),
            (OverviewNavigation::Down, Some(i)) => Some((i + columns).min(count.saturating_sub(1))),
            (OverviewNavigation::Up, Some(i)) if i >= columns => Some(i - columns),
            (OverviewNavigation::Left, Some(i)) => Some(if i == 0 {
                count.saturating_sub(1)
            } else {
                i - 1
            }),
            (OverviewNavigation::Right, Some(i)) => Some(if i + 1 == count { 0 } else { i + 1 }),
            _ => None,
        };
        self.selected = index
            .and_then(|i| self.windows.get(i))
            .copied()
            .map_or(self.desktop_target(), OverviewTarget::Window);
    }

    /// Commit through the existing focus/restore or workspace-swap commands.
    pub fn activate(mut self, desktop: &mut Desktop) {
        if !self.refresh(desktop) {
            return;
        }
        match self.selected {
            OverviewTarget::Fullscreen => {
                desktop.command(Command::ActivateFullscreen);
            }
            OverviewTarget::Window(id) => {
                desktop.command(Command::Focus(id));
            }
            OverviewTarget::Workspace(id) => {
                desktop.command(Command::FocusOutput(self.output));
                desktop.command(Command::SwitchWorkspace(id));
            }
        }
    }
}
