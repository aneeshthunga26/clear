use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::management;

/// Deterministic desktop model, independent of rendering, protocols, and scripting.
#[derive(Debug, Clone)]
pub struct Desktop {
    outputs: BTreeMap<OutputId, Output>,
    pub(super) windows: BTreeMap<WindowId, Window>,
    pub(super) workspaces: BTreeMap<WorkspaceId, Workspace>,
    groups: Vec<OutputGroup>,
    detached_workspaces: BTreeMap<OutputId, WorkspaceId>,
    pub(super) focused_output: Option<OutputId>,
    pub(super) focused_window: Option<WindowId>,
    pub(super) fullscreen: Option<FullscreenSlot>,
    pub(super) fullscreen_revision: u64,
    pending_workspace: WorkspaceId,
    pub(super) gaps: i32,
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Desktop {
    /// Creates nine master-stack workspaces (IDs 1–9), with no connected outputs.
    pub fn new() -> Self {
        let mut desktop = Self {
            outputs: BTreeMap::new(),
            windows: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            groups: Vec::new(),
            detached_workspaces: BTreeMap::new(),
            focused_output: None,
            focused_window: None,
            fullscreen: None,
            fullscreen_revision: 0,
            pending_workspace: WorkspaceId(1),
            gaps: 8,
        };
        for id in 1..=9 {
            desktop.insert_workspace(WorkspaceId(id), id.to_string(), Mode::default());
        }
        desktop
    }

    /// Returns connected outputs in ID order.
    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.outputs.values()
    }

    /// Returns all windows, including hidden ones, in ID order.
    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    /// Returns workspaces in ID order.
    pub fn workspaces(&self) -> impl Iterator<Item = &Workspace> {
        self.workspaces.values()
    }

    /// Returns the disjoint output groups and their exclusive workspace presentations.
    pub fn groups(&self) -> &[OutputGroup] {
        &self.groups
    }

    /// Looks up a connected output.
    pub fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.get(&id)
    }

    /// Looks up a managed window, including hidden windows.
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    /// Looks up persistent workspace state.
    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.get(&id)
    }

    /// Returns the underlying normal workspace, including beneath active fullscreen.
    pub fn workspace_for_output(&self, output: OutputId) -> Option<WorkspaceId> {
        self.group_index(output)
            .map(|index| self.groups[index].workspace)
    }

    /// Returns an output override or its workspace default, even for hidden workspaces.
    pub fn effective_mode(&self, workspace: WorkspaceId, output: OutputId) -> Option<&Mode> {
        let workspace = self.workspaces.get(&workspace)?;
        Some(workspace.output_mode(output).unwrap_or(&workspace.mode))
    }

    /// Returns the globally focused window; never references a hidden or removed window.
    pub fn focused_window(&self) -> Option<WindowId> {
        self.focused_window
    }

    /// Returns the focused connected output, including empty regions.
    pub fn focused_output(&self) -> Option<OutputId> {
        self.focused_output
    }

    /// Connects an output with an unused workspace, restoring its disconnected presentation if free.
    /// Full bounds initially equal the usable area; an empty update retains prior bounds.
    /// Repeated IDs update metadata without changing topology. A newly connected empty
    /// area has no full bounds until positive geometry is supplied.
    pub fn add_output(&mut self, id: OutputId, name: String, area: Rect) {
        let area = area.normalized();
        let bounds = if area.is_empty() {
            self.outputs.get(&id).map_or(area, |output| output.bounds)
        } else {
            area
        };
        self.connect_output(id, name, bounds, area);
    }

    /// Connects or updates an output with full bounds separate from its usable area.
    /// Empty normalized full bounds are ignored without changing metadata or topology.
    pub fn add_output_with_bounds(&mut self, id: OutputId, name: String, bounds: Rect, area: Rect) {
        let bounds = bounds.normalized();
        if !bounds.is_empty() {
            self.connect_output(id, name, bounds, area);
        }
    }

    fn connect_output(&mut self, id: OutputId, name: String, bounds: Rect, area: Rect) {
        if let Some(output) = self.outputs.get_mut(&id) {
            output.name = name;
            output.bounds = bounds;
            output.area = area.normalized();
            self.settle(true);
            return;
        }
        let preferred = self
            .detached_workspaces
            .remove(&id)
            .or_else(|| self.outputs.is_empty().then_some(self.pending_workspace));
        let workspace = preferred
            .filter(|id| {
                self.workspaces.contains_key(id)
                    && !self.groups.iter().any(|group| group.workspace == *id)
            })
            .unwrap_or_else(|| self.unused_workspace());
        self.outputs.insert(
            id,
            Output {
                id,
                name,
                bounds,
                area: area.normalized(),
            },
        );
        self.groups.push(OutputGroup {
            outputs: vec![id],
            workspace,
        });
        if self.focused_output.is_none() {
            self.focused_output = Some(id);
        }
        self.settle(true);
    }

    /// Disconnects an output, migrating orphaned windows to a surviving region.
    /// With no outputs, all workspace, geometry, and per-region state is retained.
    pub fn remove_output(&mut self, id: OutputId) {
        if !self.outputs.contains_key(&id) {
            return;
        }
        if let Some(workspace) = self.workspace_for_output(id) {
            self.detached_workspaces.insert(id, workspace);
        }
        self.outputs.remove(&id);
        for group in &mut self.groups {
            group.outputs.retain(|output| *output != id);
        }
        self.groups.retain(|group| !group.outputs.is_empty());
        self.settle(true);
    }

    /// Updates usable geometry without overwriting saved floating rectangles.
    pub fn set_output_area(&mut self, id: OutputId, area: Rect) {
        if let Some(output) = self.outputs.get_mut(&id) {
            output.area = area.normalized();
            self.settle(true);
        }
    }

    /// Updates full logical bounds while retaining usable area and saved window state.
    /// Unknown outputs and empty normalized bounds are ignored.
    pub fn set_output_bounds(&mut self, id: OutputId, bounds: Rect) {
        let bounds = bounds.normalized();
        if !bounds.is_empty() {
            if let Some(output) = self.outputs.get_mut(&id) {
                output.bounds = bounds;
                self.settle(false);
            }
        }
    }

    /// Adds and focuses a window in the active workspace and region.
    /// Repeated IDs update metadata without resetting persistent window state.
    pub fn add_window(&mut self, id: WindowId, title: String, app_id: String) {
        if self.windows.contains_key(&id) {
            self.update_window_metadata(id, title, app_id);
            return;
        }
        let workspace = self.active_workspace();
        let floating_rect = self
            .focused_output
            .and_then(|output| self.outputs.get(&output))
            .map_or(Rect::new(0, 0, 800, 600), |output| {
                output
                    .area
                    .centered(output.area.width.min(800), output.area.height.min(600))
            });
        self.windows.insert(
            id,
            Window {
                id,
                title,
                app_id,
                workspace,
                output: self.focused_output,
                role: WindowRole::default(),
                transient_for: None,
                committed_size: None,
                floating: false,
                snapped: None,
                maximized: false,
                fullscreen: false,
                minimized: false,
                floating_rect,
            },
        );
        self.workspaces
            .get_mut(&workspace)
            .unwrap()
            .windows
            .push(id);
        self.focused_window = Some(id);
        self.settle(true);
    }

    /// Removes a window and any remembered focus references to it.
    pub fn remove_window(&mut self, id: WindowId) {
        self.leave_fullscreen(id);
        for child in self
            .windows
            .values_mut()
            .filter(|w| w.transient_for == Some(id))
        {
            child.transient_for = None;
        }
        if let Some(window) = self.windows.remove(&id) {
            let workspace = self.workspaces.get_mut(&window.workspace).unwrap();
            workspace.windows.retain(|window| *window != id);
            for region in workspace.regions.values_mut() {
                if region.focused == Some(id) {
                    region.focused = None;
                }
            }
            for workspace in self.workspaces.values_mut() {
                for region in workspace.regions.values_mut() {
                    for sizing in region.sizing.values_mut() {
                        sizing.remove(id);
                    }
                }
            }
            self.settle(true);
        }
    }

    /// Updates client metadata without affecting focus, ownership, or order.
    pub fn update_window_metadata(&mut self, id: WindowId, title: String, app_id: String) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.title = title;
            window.app_id = app_id;
        }
    }

    /// Changes placement role without changing ownership, order, or saved floating state.
    /// Unknown windows are ignored.
    pub fn set_window_role(&mut self, id: WindowId, role: WindowRole) {
        if role == WindowRole::Launcher {
            self.leave_fullscreen(id);
        }
        if let Some(window) = self.windows.get_mut(&id) {
            if window.role != role {
                window.role = role;
                if role == WindowRole::Launcher {
                    window.snapped = None;
                    window.maximized = false;
                    window.fullscreen = false;
                    window.minimized = false;
                }
                self.settle(false);
            }
        }
    }

    /// Records a positive client size without changing requested floating geometry.
    /// Invalid sizes and unknown windows are ignored, retaining the last valid size.
    pub fn set_window_committed_size(&mut self, id: WindowId, width: i32, height: i32) {
        if width > 0 && height > 0 {
            if let Some(window) = self.windows.get_mut(&id) {
                window.committed_size = Some((width, height));
            }
        }
    }

    /// Sets nonnegative layout gaps, safely clamped to each region's available space.
    pub fn set_gaps(&mut self, gaps: i32) {
        self.gaps = gaps.max(0);
        self.settle(true);
    }

    /// Adds or updates workspace metadata and its default without resetting windows or overrides.
    pub fn configure_workspace(&mut self, id: WorkspaceId, name: String, mode: Mode) {
        if let Some(workspace) = self.workspaces.get_mut(&id) {
            workspace.name = name;
            workspace.mode = mode;
        } else {
            self.insert_workspace(id, name, mode);
        }
        self.settle(false);
    }

    /// Sets or clears a workspace-specific output override, including for disconnected outputs.
    /// Unknown workspaces are ignored.
    pub fn set_workspace_output_mode(
        &mut self,
        workspace: WorkspaceId,
        output: OutputId,
        mode: Option<Mode>,
    ) {
        if let Some(workspace) = self.workspaces.get_mut(&workspace) {
            workspace.regions.entry(output).or_default().mode = mode;
            self.settle(false);
        }
    }

    /// Applies a command, returning only host-owned side effects.
    pub fn command(&mut self, command: Command) -> Vec<Effect> {
        let valid = match &command {
            Command::Focus(id)
            | Command::SetFloatingRect(id, _)
            | Command::SetMaximized(id, _)
            | Command::SetFullscreen(id, _)
            | Command::SetMinimized(id, _) => self.windows.contains_key(id),
            Command::FocusOutput(id) | Command::MoveToOutput(id) => self.outputs.contains_key(id),
            Command::SwitchWorkspace(id) | Command::MoveToWorkspace(id) => {
                self.workspaces.contains_key(id)
            }
            Command::MoveWindowToWorkspace(window, workspace) => {
                self.windows
                    .get(window)
                    .is_some_and(|w| w.workspace != *workspace || w.fullscreen)
                    && self.workspaces.contains_key(workspace)
            }
            _ => true,
        };
        if !valid {
            return Vec::new();
        }
        let mut reveal = !matches!(
            &command,
            Command::SwitchWorkspace(_)
                | Command::MoveWindowToWorkspace(_, _)
                | Command::SetWorkspaceMode(_)
                | Command::SetOutputMode(_)
                | Command::ClearOutputMode
                | Command::CycleMode
        );
        match command {
            Command::CloseFocused => {
                return self.focused_window.map(Effect::Close).into_iter().collect();
            }
            Command::Spawn(arguments) => return vec![Effect::Spawn(arguments)],
            Command::Quit => return vec![Effect::Quit],
            Command::FocusNext => self.cycle_focus(false),
            Command::FocusPrevious => self.cycle_focus(true),
            Command::Focus(id) => self.focus(id),
            Command::FocusOutput(id) => {
                if self.outputs.contains_key(&id) {
                    self.focused_output = Some(id);
                }
            }
            Command::CycleOutput => {
                let next = self
                    .outputs
                    .keys()
                    .copied()
                    .find(|id| Some(*id) > self.focused_output)
                    .or_else(|| self.outputs.keys().next().copied());
                self.focused_output = next;
            }
            Command::SwitchWorkspace(id) => self.switch_workspace(id),
            Command::ActivateFullscreen => {
                self.activate_fullscreen();
            }
            Command::CycleDesktop(backwards) => {
                if let Some(output) = self.focused_output {
                    let targets = self.presentation_targets(output);
                    let current = self.presentation_for_output(output);
                    let index = targets
                        .iter()
                        .position(|t| Some(*t) == current)
                        .unwrap_or(0);
                    let next = if backwards {
                        (index + targets.len() - 1) % targets.len()
                    } else {
                        (index + 1) % targets.len()
                    };
                    match targets[next] {
                        PresentationTarget::Workspace(id) => self.switch_workspace(id),
                        PresentationTarget::Fullscreen => {
                            self.activate_fullscreen();
                        }
                    }
                }
            }
            Command::MoveToWorkspace(id) => {
                if self.workspaces.contains_key(&id)
                    && let Some(window) = self.focused_window
                {
                    self.transfer(window, id, None);
                }
            }
            Command::MoveWindowToWorkspace(window, workspace) => {
                self.transfer(window, workspace, None);
            }
            Command::MoveToOutput(output) => {
                if let (Some(workspace), Some(window)) =
                    (self.workspace_for_output(output), self.focused_window)
                {
                    self.transfer(window, workspace, Some(output));
                    self.focused_output = Some(output);
                    self.focused_window = Some(window);
                }
            }
            Command::SetWorkspaceMode(mode) => {
                let id = self.active_workspace();
                self.workspaces.get_mut(&id).unwrap().mode = mode;
            }
            Command::SetOutputMode(mode) => {
                if let Some(output) = self.focused_output {
                    let id = self.active_workspace();
                    self.workspaces
                        .get_mut(&id)
                        .unwrap()
                        .regions
                        .entry(output)
                        .or_default()
                        .mode = Some(mode);
                }
            }
            Command::ClearOutputMode => {
                if let Some(output) = self.focused_output {
                    let id = self.active_workspace();
                    self.workspaces
                        .get_mut(&id)
                        .unwrap()
                        .regions
                        .entry(output)
                        .or_default()
                        .mode = None;
                }
            }
            Command::CycleMode => {
                if let Some(output) = self.focused_output {
                    let id = self.active_workspace();
                    let mode = self.effective_mode(id, output).unwrap().next();
                    self.workspaces
                        .get_mut(&id)
                        .unwrap()
                        .regions
                        .entry(output)
                        .or_default()
                        .mode = Some(mode);
                }
            }
            Command::StretchAll => self.stretch(),
            Command::Unstretch => self.unstretch(),
            Command::ToggleFloating => {
                if let Some(window) = self.focused_window.and_then(|id| self.windows.get_mut(&id)) {
                    window.floating = !window.floating;
                    window.snapped = None;
                }
            }
            Command::ToggleMaximized => {
                if let Some(window) = self.focused_window.and_then(|id| self.windows.get_mut(&id)) {
                    if window.role == WindowRole::Normal {
                        window.maximized = !window.maximized;
                    }
                }
            }
            Command::ToggleFullscreen => {
                if let Some(id) = self.focused_window {
                    if self.window(id).is_some_and(|w| w.fullscreen) {
                        self.leave_fullscreen(id);
                    } else {
                        self.request_fullscreen(id, None, true);
                    }
                }
            }
            Command::MinimizeFocused => {
                if let Some(id) = self.focused_window {
                    self.leave_fullscreen(id);
                    if let Some(w) = self
                        .windows
                        .get_mut(&id)
                        .filter(|w| w.role == WindowRole::Normal)
                    {
                        w.minimized = true;
                    }
                }
            }
            Command::SetMaximized(id, maximized) => {
                reveal = false;
                if let Some(window) = self.windows.get_mut(&id) {
                    if window.role == WindowRole::Normal {
                        window.maximized = maximized;
                    }
                }
            }
            Command::SetFullscreen(id, fullscreen) => {
                reveal = false;
                if fullscreen {
                    self.request_fullscreen(id, None, self.focused_window == Some(id));
                } else {
                    self.leave_fullscreen(id);
                }
            }
            Command::SetMinimized(id, minimized) => {
                reveal = false;
                if minimized {
                    self.leave_fullscreen(id);
                }
                if let Some(window) = self.windows.get_mut(&id) {
                    if window.role == WindowRole::Normal {
                        window.minimized = minimized;
                    }
                }
            }
            Command::Scroll(delta) => {
                reveal = false;
                if let Some(output) = self.focused_output {
                    let workspace = self.active_workspace();
                    if self.effective_mode(workspace, output) == Some(&Mode::Scrolling) {
                        let region = self
                            .workspaces
                            .get_mut(&workspace)
                            .unwrap()
                            .regions
                            .entry(output)
                            .or_default();
                        region.scroll_offset = region.scroll_offset.saturating_add(delta).max(0);
                    }
                }
            }
            Command::SetFloatingRect(id, rect) => {
                reveal = false;
                if let Some(window) = self.windows.get_mut(&id) {
                    window.snapped = None;
                    window.floating_rect = rect.normalized();
                }
            }
        }
        self.settle(reveal);
        Vec::new()
    }

    /// Computes visible placements with built-in fallback for script modes.
    pub fn placements(&self) -> Vec<Placement> {
        self.placements_with(|_, _| None)
    }

    /// Computes each region independently, calling the host only for script modes.
    /// `None` or a result with missing, duplicate, or foreign windows falls back to master-stack.
    /// The host owns script evaluation and detailed geometry validation. Floating exceptions
    /// and launchers bypass the callback, as do maximized, fullscreen, and minimized windows.
    /// Tiles precede floats, maximized windows, and fullscreen; launchers remain above them all.
    /// Focus can raise a normal window above a maximized neighbor on its output.
    pub fn placements_with(
        &self,
        mut custom: impl FnMut(&Mode, &LayoutContext) -> Option<Vec<Placement>>,
    ) -> Vec<Placement> {
        let mut placements = Vec::new();
        for (&output, metadata) in &self.outputs {
            if self.presentation_for_output(output) == Some(PresentationTarget::Fullscreen) {
                let slot = self.fullscreen.as_ref().unwrap();
                if !metadata.bounds.is_empty() {
                    placements.push(Placement {
                        window: slot.window,
                        rect: metadata.bounds,
                        clip: Some(metadata.bounds),
                        focused: self.focused_window == Some(slot.window),
                        tiled: false,
                    });
                    placements.extend(
                        self.windows
                            .values()
                            .filter(|w| !w.minimized && self.is_fullscreen_transient(w.id))
                            .map(|w| {
                                let home = w
                                    .output
                                    .and_then(|o| self.output(o))
                                    .map_or(metadata.bounds, |o| o.bounds);
                                let rect = Rect::new(
                                    w.floating_rect
                                        .x
                                        .saturating_add(metadata.bounds.x.saturating_sub(home.x)),
                                    w.floating_rect
                                        .y
                                        .saturating_add(metadata.bounds.y.saturating_sub(home.y)),
                                    w.floating_rect.width,
                                    w.floating_rect.height,
                                );
                                Placement {
                                    window: w.id,
                                    rect,
                                    clip: Some(metadata.bounds),
                                    focused: self.focused_window == Some(w.id),
                                    tiled: false,
                                }
                            }),
                    );
                }
                continue;
            }
            let workspace = self.workspace_for_output(output).unwrap();
            let mode = self.effective_mode(workspace, output).unwrap();
            let context = self.layout_context(workspace, output);
            let scripted = if matches!(mode, Mode::Script(_)) {
                custom(mode, &context).filter(|result| {
                    let expected: BTreeSet<_> =
                        context.windows.iter().map(|window| window.id).collect();
                    let actual: BTreeSet<_> =
                        result.iter().map(|placement| placement.window).collect();
                    result.len() == expected.len() && actual == expected
                })
            } else {
                None
            };
            let mut region = scripted.unwrap_or_else(|| {
                management::arrange_with_sizing(
                    mode,
                    &context,
                    self.layout_sizing(workspace, output, mode),
                )
            });
            for placement in &mut region {
                placement.focused = self.focused_window == Some(placement.window);
                placement.tiled = !matches!(mode, Mode::Floating);
                placement.rect = placement.rect.normalized();
                placement.clip = placement.clip.map(Rect::normalized);
            }
            region.extend(
                self.region_windows(workspace, output)
                    .filter(|window| !window.fullscreen || !metadata.bounds.is_empty())
                    .filter(|window| {
                        window.maximized
                            || window.fullscreen
                            || window
                                .snapped
                                .is_none_or(|t| t.rect(metadata.area).is_some())
                    })
                    .filter(|window| {
                        window.role == WindowRole::Launcher
                            || window.maximized
                            || window.snapped.is_some()
                            || window.fullscreen
                            || (window.floating && !matches!(mode, Mode::Floating))
                    })
                    .map(|window| {
                        let rect = if window.role == WindowRole::Launcher {
                            let (width, height) = window.committed_size.unwrap_or((
                                window.floating_rect.width,
                                window.floating_rect.height,
                            ));
                            metadata.area.centered_unbounded(width, height)
                        } else if window.fullscreen {
                            metadata.bounds
                        } else if window.maximized {
                            metadata.area
                        } else {
                            window
                                .snapped
                                .and_then(|t| t.rect(metadata.area))
                                .unwrap_or(window.floating_rect.normalized())
                        };
                        Placement {
                            window: window.id,
                            rect,
                            clip: if window.fullscreen {
                                Some(metadata.bounds)
                            } else {
                                (window.maximized || window.snapped.is_some())
                                    .then_some(metadata.area)
                            },
                            focused: self.focused_window == Some(window.id),
                            tiled: false,
                        }
                    }),
            );
            placements.extend(region);
        }
        // A focused normal window must remain reachable above a maximized neighbor.
        let maximized_outputs: BTreeSet<_> = self
            .windows
            .values()
            .filter(|window| {
                window.maximized
                    && !window.minimized
                    && window.output.and_then(|id| self.workspace_for_output(id))
                        == Some(window.workspace)
            })
            .filter_map(|window| window.output)
            .collect();
        // Stable sorting preserves policy-defined tile order, notably monocle's focused tile.
        placements.sort_by_key(|placement| {
            let window = &self.windows[&placement.window];
            if window.role == WindowRole::Launcher {
                (6, placement.focused)
            } else if self.is_fullscreen_transient(window.id) {
                (5, placement.focused)
            } else if window.fullscreen {
                (4, placement.focused)
            } else if placement.focused
                && window
                    .output
                    .is_some_and(|id| maximized_outputs.contains(&id))
            {
                (3, true)
            } else if window.maximized {
                (2, placement.focused)
            } else if !placement.tiled {
                (1, placement.focused)
            } else {
                (0, false)
            }
        });
        placements
    }

    fn insert_workspace(&mut self, id: WorkspaceId, name: String, mode: Mode) {
        self.workspaces.insert(
            id,
            Workspace {
                id,
                name,
                mode,
                windows: Vec::new(),
                regions: BTreeMap::new(),
            },
        );
    }

    fn unused_workspace(&mut self) -> WorkspaceId {
        if let Some(id) = self
            .workspaces
            .keys()
            .find(|id| !self.groups.iter().any(|group| group.workspace == **id))
            .copied()
        {
            return id;
        }
        // A finite in-memory map cannot occupy every u64 identity.
        let id = (1..=u64::MAX)
            .map(WorkspaceId)
            .find(|id| !self.workspaces.contains_key(id))
            .unwrap();
        self.insert_workspace(id, id.0.to_string(), Mode::default());
        id
    }

    fn group_index(&self, output: OutputId) -> Option<usize> {
        self.groups
            .iter()
            .position(|group| group.outputs.contains(&output))
    }

    fn active_workspace(&self) -> WorkspaceId {
        self.focused_output
            .and_then(|output| self.workspace_for_output(output))
            .unwrap_or(self.pending_workspace)
    }

    fn region_windows(
        &self,
        workspace: WorkspaceId,
        output: OutputId,
    ) -> impl Iterator<Item = &Window> {
        self.workspaces[&workspace]
            .windows
            .iter()
            .filter_map(|id| self.windows.get(id))
            .filter(move |window| {
                window.output == Some(output)
                    && !window.minimized
                    && !window.fullscreen
                    && !self.is_fullscreen_transient(window.id)
            })
    }

    pub(super) fn layout_context(&self, workspace: WorkspaceId, output: OutputId) -> LayoutContext {
        let mode = self.effective_mode(workspace, output).unwrap();
        let windows: Vec<_> = self
            .region_windows(workspace, output)
            .filter(|window| {
                window.role == WindowRole::Normal
                    && !window.maximized
                    && window.snapped.is_none()
                    && !window.fullscreen
                    && (!window.floating || matches!(mode, Mode::Floating))
            })
            .map(|window| LayoutWindow {
                id: window.id,
                floating_rect: window.floating_rect,
            })
            .collect();
        let focused = self
            .focused_window
            .filter(|id| windows.iter().any(|window| window.id == *id));
        LayoutContext {
            area: self.outputs[&output].area,
            windows,
            focused,
            scroll_offset: self.workspaces[&workspace].scroll_offset(output),
            gaps: self.gaps,
        }
    }

    pub(super) fn switch_workspace(&mut self, workspace: WorkspaceId) {
        if !self.workspaces.contains_key(&workspace) {
            return;
        }
        if let Some(output) = self.focused_output {
            self.hide_fullscreen_on(output);
        }
        let Some(active) = self
            .focused_output
            .and_then(|output| self.group_index(output))
        else {
            self.pending_workspace = workspace;
            return;
        };
        let old = self.groups[active].workspace;
        if old == workspace {
            return;
        }
        if let Some(other) = self
            .groups
            .iter()
            .position(|group| group.workspace == workspace)
        {
            self.groups[other].workspace = old;
        }
        self.groups[active].workspace = workspace;
        self.focused_window = None;
    }

    fn transfer(&mut self, id: WindowId, workspace: WorkspaceId, output: Option<OutputId>) {
        self.leave_fullscreen(id);
        let old = self.windows[&id].workspace;
        if old != workspace {
            let source = self.workspaces.get_mut(&old).unwrap();
            source.windows.retain(|window| *window != id);
            for region in source.regions.values_mut() {
                if region.focused == Some(id) {
                    region.focused = None;
                }
            }
            self.workspaces
                .get_mut(&workspace)
                .unwrap()
                .windows
                .push(id);
            self.windows.get_mut(&id).unwrap().workspace = workspace;
            if self.focused_window == Some(id) {
                self.focused_window = None;
            }
        }
        if let Some(output) = output {
            self.windows.get_mut(&id).unwrap().output = Some(output);
        }
    }

    fn focus(&mut self, id: WindowId) {
        if self.is_fullscreen_transient(id) {
            self.activate_fullscreen();
            if let Some(w) = self.windows.get_mut(&id) {
                w.minimized = false;
            }
            self.focused_window = Some(id);
            return;
        }
        if self.fullscreen.as_ref().is_some_and(|s| s.window == id) {
            self.activate_fullscreen();
            return;
        }
        let Some(workspace) = self.windows.get(&id).map(|window| window.workspace) else {
            return;
        };
        if self.outputs.is_empty() {
            return;
        }
        if !self.groups.iter().any(|group| group.workspace == workspace) {
            self.switch_workspace(workspace);
        }
        self.windows.get_mut(&id).unwrap().minimized = false;
        self.repair_homes();
        if let Some(output) = self.windows[&id].output {
            self.hide_fullscreen_on(output);
        }
        self.focused_output = self.windows[&id].output;
        self.focused_window = Some(id);
    }

    fn cycle_focus(&mut self, backwards: bool) {
        if self.focused_output.is_none() {
            return;
        }
        let windows: Vec<_> = self.workspaces[&self.active_workspace()]
            .windows
            .iter()
            .copied()
            .filter(|id| !self.windows[id].minimized && !self.windows[id].fullscreen)
            .chain(
                self.fullscreen
                    .as_ref()
                    .filter(|s| s.active && s.output == self.focused_output)
                    .map(|s| s.window),
            )
            .collect();
        if windows.is_empty() {
            return;
        }
        let index = match windows
            .iter()
            .position(|id| Some(*id) == self.focused_window)
        {
            Some(0) if backwards => windows.len() - 1,
            Some(index) if backwards => index - 1,
            Some(index) => (index + 1) % windows.len(),
            None if backwards => windows.len() - 1,
            None => 0,
        };
        self.focus(windows[index]);
    }

    fn stretch(&mut self) {
        if self.outputs.is_empty() {
            return;
        }
        let workspace = self.active_workspace();
        self.groups = vec![OutputGroup {
            outputs: self.outputs.keys().copied().collect(),
            workspace,
        }];
    }

    fn unstretch(&mut self) {
        let Some(output) = self.focused_output else {
            return;
        };
        let index = self.group_index(output).unwrap();
        if self.groups[index].outputs.len() <= 1 {
            return;
        }
        let group = self.groups.remove(index);
        self.groups.push(OutputGroup {
            outputs: vec![output],
            workspace: group.workspace,
        });
        for other in group.outputs.into_iter().filter(|id| *id != output) {
            let workspace = self.unused_workspace();
            self.groups.push(OutputGroup {
                outputs: vec![other],
                workspace,
            });
        }
    }

    fn repair_homes(&mut self) {
        let fallback = self
            .focused_output
            .filter(|id| self.outputs.contains_key(id))
            .or_else(|| self.outputs.keys().next().copied());
        let Some(fallback) = fallback else {
            return;
        };
        for window in self.windows.values_mut() {
            if let Some(group) = self
                .groups
                .iter()
                .find(|group| group.workspace == window.workspace)
            {
                if !window
                    .output
                    .is_some_and(|output| group.outputs.contains(&output))
                {
                    window.output = Some(if group.outputs.contains(&fallback) {
                        fallback
                    } else {
                        group.outputs[0]
                    });
                }
            } else if !window
                .output
                .is_some_and(|output| self.outputs.contains_key(&output))
            {
                window.output = Some(fallback);
            }
        }
    }

    pub(super) fn settle(&mut self, reveal: bool) {
        self.groups.sort_by_key(|group| group.outputs[0]);
        if !self
            .focused_output
            .is_some_and(|id| self.outputs.contains_key(&id))
        {
            self.focused_output = self.outputs.keys().next().copied();
        }
        self.repair_homes();
        self.repair_fullscreen();
        self.refresh_resize_environments();
        let Some(output) = self.focused_output else {
            self.focused_window = None;
            return;
        };
        let workspace = self.workspace_for_output(output).unwrap();
        self.pending_workspace = workspace;
        let belongs = |id: WindowId| {
            self.windows.get(&id).is_some_and(|window| {
                window.workspace == workspace
                    && window.output == Some(output)
                    && !window.minimized
                    && !window.fullscreen
            })
        };
        let remembered = self.workspaces[&workspace]
            .regions
            .get(&output)
            .and_then(|region| region.focused);
        let transient_focus = self.focused_window.filter(|id| {
            self.is_fullscreen_transient(*id) && self.window(*id).is_some_and(|w| !w.minimized)
        });
        self.focused_window = self
            .focused_window
            .filter(|id| belongs(*id))
            .or_else(|| remembered.filter(|id| belongs(*id)))
            .or_else(|| {
                self.region_windows(workspace, output)
                    .next()
                    .map(|window| window.id)
            });
        if let Some(slot) = self
            .fullscreen
            .as_ref()
            .filter(|s| s.active && s.output == Some(output))
        {
            self.focused_window = transient_focus.or(Some(slot.window));
        }
        if self.presentation_for_output(output) != Some(PresentationTarget::Fullscreen) {
            self.workspaces
                .get_mut(&workspace)
                .unwrap()
                .regions
                .entry(output)
                .or_default()
                .focused = self.focused_window;
        }
        let scrolling: Vec<_> = self
            .outputs
            .keys()
            .copied()
            .filter_map(|output| {
                let workspace = self.workspace_for_output(output).unwrap();
                (self.effective_mode(workspace, output) == Some(&Mode::Scrolling))
                    .then_some((workspace, output))
            })
            .collect();
        for (workspace, output) in scrolling {
            let context = self.layout_context(workspace, output);
            let region = self
                .workspaces
                .get_mut(&workspace)
                .unwrap()
                .regions
                .entry(output)
                .or_default();
            // First activation reveals focus; later mode/workspace switches restore manual scrolling.
            region.scroll_offset = management::scrolling_offset_with_sizing(
                &context,
                reveal || !region.scroll_initialized,
                region.sizing.get(&Mode::Scrolling),
            );
            region.scroll_initialized = true;
        }
    }
}
