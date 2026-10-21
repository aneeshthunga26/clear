//! One occupied-only presentation destination, independent of workspace ownership.

use super::*;

/// Requested presentation policy; nested low latency still uses synchronized host swaps.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FullscreenMode {
    #[default]
    Desktop,
    LowLatency,
}

impl FullscreenMode {
    /// Stable configuration and IPC spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::LowLatency => "low-latency",
        }
    }
}

/// A presentation destination; Fullscreen is never a numbered workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PresentationTarget {
    Workspace(WorkspaceId),
    Fullscreen,
}

/// The single fullscreen occupant and its return context. Home ownership stays on Window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullscreenSlot {
    /// Sole managed fullscreen occupant.
    pub window: WindowId,
    /// Requested mode, retained while dormant or using an overlay fallback.
    pub mode: FullscreenMode,
    /// Effective presentation output; absent while headless.
    pub output: Option<OutputId>,
    /// Whether the special destination is selected on its output.
    pub active: bool,
    /// Ownership epoch, independent of client ACK/commit timing.
    pub revision: u64,
    /// Ordinary destination restored when exiting a viewed slot.
    pub return_workspace: WorkspaceId,
    /// Output from which the fullscreen presentation was invoked.
    pub return_output: Option<OutputId>,
    /// Previous eligible normal focus, repaired if removed.
    pub return_focus: Option<WindowId>,
}

impl Desktop {
    /// Set a validated transient relationship without accepting cycles or unknown parents.
    pub fn set_window_parent(&mut self, window: WindowId, parent: Option<WindowId>) {
        if self.window(window).is_none() {
            return;
        }
        if parent.is_some_and(|p| p == window || self.window(p).is_none()) {
            return;
        }
        let mut ancestor = parent;
        for _ in 0..self.windows.len() {
            let Some(id) = ancestor else {
                break;
            };
            if id == window {
                return;
            }
            ancestor = self.window(id).and_then(|w| w.transient_for);
        }
        if self.window(window).unwrap().transient_for != parent {
            self.windows.get_mut(&window).unwrap().transient_for = parent;
            self.settle(false);
        }
    }

    /// A normal transient descendant of the occupant shares its special presentation.
    pub fn is_fullscreen_transient(&self, window: WindowId) -> bool {
        let Some(slot) = self.fullscreen.as_ref() else {
            return false;
        };
        if window == slot.window
            || self
                .window(window)
                .is_none_or(|w| w.role != WindowRole::Normal)
        {
            return false;
        }
        let mut ancestor = self.window(window).and_then(|w| w.transient_for);
        for _ in 0..self.windows.len() {
            let Some(id) = ancestor else {
                return false;
            };
            if id == slot.window {
                return true;
            }
            let Some(parent) = self
                .window(id)
                .filter(|w| !w.minimized && w.role == WindowRole::Normal)
            else {
                return false;
            };
            ancestor = parent.transient_for;
        }
        false
    }
    /// The occupied fullscreen desktop, including when another desktop is selected.
    pub fn fullscreen_slot(&self) -> Option<&FullscreenSlot> {
        self.fullscreen.as_ref()
    }

    /// Monotonic ownership epoch, also retained after the slot becomes empty.
    pub fn fullscreen_revision(&self) -> u64 {
        self.fullscreen_revision
    }

    /// Actual presentation on an output, without changing its underlying workspace.
    pub fn presentation_for_output(&self, output: OutputId) -> Option<PresentationTarget> {
        let workspace = self.workspace_for_output(output)?;
        Some(
            if self
                .fullscreen
                .as_ref()
                .is_some_and(|s| s.active && s.output == Some(output))
            {
                PresentationTarget::Fullscreen
            } else {
                PresentationTarget::Workspace(workspace)
            },
        )
    }

    /// Ordered navigation destinations; the special desktop exists only on its output.
    pub fn presentation_targets(&self, output: OutputId) -> Vec<PresentationTarget> {
        let mut targets: Vec<_> = self
            .workspaces()
            .map(|w| PresentationTarget::Workspace(w.id))
            .collect();
        if self
            .fullscreen
            .as_ref()
            .is_some_and(|s| s.output == Some(output))
        {
            targets.push(PresentationTarget::Fullscreen);
        }
        targets
    }

    /// Effective presentation output, distinct from a fullscreen window's saved home.
    pub fn presentation_output(&self, window: WindowId) -> Option<OutputId> {
        if let Some(slot) = self
            .fullscreen
            .as_ref()
            .filter(|s| s.window == window || self.is_fullscreen_transient(window))
        {
            slot.output
        } else {
            self.window(window).and_then(|w| w.output)
        }
    }

    /// Whether a window's presentation may be clipped to this output's viewport.
    pub fn window_presented_on(&self, window: WindowId, output: OutputId) -> bool {
        let Some(w) = self.window(window) else {
            return false;
        };
        if w.fullscreen || self.is_fullscreen_transient(window) {
            self.fullscreen
                .as_ref()
                .is_some_and(|s| s.active && s.output == Some(output))
        } else {
            self.presentation_for_output(output) == Some(PresentationTarget::Workspace(w.workspace))
        }
    }

    /// Replace the single occupant atomically, preserving home geometry and ownership.
    /// A background request installs a dormant occupant without revealing or focusing it.
    pub fn request_fullscreen(
        &mut self,
        window: WindowId,
        output: Option<OutputId>,
        activate: bool,
    ) -> bool {
        self.request_fullscreen_mode(window, output, activate, FullscreenMode::Desktop)
    }

    /// Enter or change mode in place without losing an existing occupant's return context.
    pub fn request_fullscreen_mode(
        &mut self,
        window: WindowId,
        output: Option<OutputId>,
        activate: bool,
        mode: FullscreenMode,
    ) -> bool {
        let Some(w) = self.window(window) else {
            return false;
        };
        if w.role != WindowRole::Normal || w.minimized {
            return false;
        }
        let target = output
            .filter(|o| self.output(*o).is_some())
            .or_else(|| {
                self.fullscreen
                    .as_ref()
                    .filter(|s| s.window == window)
                    .and_then(|s| s.output)
            })
            .or(w.output.filter(|o| self.output(*o).is_some()))
            .or(self.focused_output())
            .or_else(|| self.outputs().next().map(|o| o.id));
        if self
            .fullscreen
            .as_ref()
            .is_some_and(|s| s.window == window && s.output == target)
        {
            self.fullscreen.as_mut().unwrap().mode = mode;
            if activate {
                self.activate_fullscreen();
            }
            return true;
        }
        if let Some(old) = self.fullscreen.as_ref().map(|s| s.window) {
            self.leave_fullscreen(old);
        }
        self.fullscreen_revision = self
            .fullscreen_revision
            .checked_add(1)
            .expect("fullscreen revision exhausted");
        let return_output = self.focused_output.or(target);
        let return_workspace = return_output
            .and_then(|o| self.workspace_for_output(o))
            .unwrap_or(self.window(window).unwrap().workspace);
        let return_focus = self.focused_window.filter(|id| {
            self.window(*id)
                .is_some_and(|w| w.workspace == return_workspace && !w.fullscreen && !w.minimized)
        });
        self.fullscreen = Some(FullscreenSlot {
            window,
            mode,
            output: target,
            active: false,
            revision: self.fullscreen_revision,
            return_workspace,
            return_output,
            return_focus,
        });
        self.windows.get_mut(&window).unwrap().fullscreen = true;
        if activate {
            self.activate_fullscreen();
        }
        self.settle(false);
        true
    }

    /// Reveal the occupied slot on its target output. Empty/headless slots cannot activate.
    pub fn activate_fullscreen(&mut self) -> bool {
        let Some(output) = self
            .fullscreen
            .as_ref()
            .and_then(|s| s.output)
            .filter(|o| self.output(*o).is_some())
        else {
            return false;
        };
        // The return context never points back at Fullscreen itself.
        let return_output = self.focused_output.or(Some(output));
        let return_workspace = return_output
            .and_then(|o| self.workspace_for_output(o))
            .unwrap();
        let return_focus = self.focused_window.filter(|id| {
            self.window(*id)
                .is_some_and(|w| !w.fullscreen && !w.minimized && w.workspace == return_workspace)
        });
        let slot = self.fullscreen.as_mut().unwrap();
        if !slot.active || self.focused_output != slot.output {
            slot.return_workspace = return_workspace;
            slot.return_output = return_output;
            slot.return_focus = return_focus.or(slot.return_focus);
        }
        slot.active = true;
        self.focused_output = Some(output);
        self.focused_window = Some(slot.window);
        self.settle(false);
        true
    }

    /// Release only the named occupant; stale exit requests never evict a replacement.
    pub fn leave_fullscreen(&mut self, window: WindowId) -> bool {
        if !self.fullscreen.as_ref().is_some_and(|s| s.window == window) {
            return false;
        }
        let slot = self.fullscreen.take().unwrap();
        if let Some(w) = self.windows.get_mut(&window) {
            w.fullscreen = false;
        }
        self.fullscreen_revision = self
            .fullscreen_revision
            .checked_add(1)
            .expect("fullscreen revision exhausted");
        if slot.active && self.focused_output == slot.output {
            self.focused_output = slot
                .return_output
                .filter(|o| self.output(*o).is_some())
                .or(slot.output);
            if self.workspace(slot.return_workspace).is_some() {
                self.switch_workspace(slot.return_workspace);
            }
            self.focused_window = slot.return_focus.filter(|id| {
                self.window(*id)
                    .is_some_and(|w| !w.minimized && w.workspace == slot.return_workspace)
            });
        }
        self.settle(false);
        true
    }

    pub(super) fn hide_fullscreen_on(&mut self, output: OutputId) {
        if let Some(slot) = self
            .fullscreen
            .as_mut()
            .filter(|s| s.output == Some(output))
        {
            if slot.active && self.focused_output == slot.output {
                self.focused_window = slot.return_focus;
            }
            slot.active = false;
        }
    }

    pub(super) fn repair_fullscreen(&mut self) {
        let Some(slot) = self.fullscreen.as_ref() else {
            return;
        };
        if self
            .window(slot.window)
            .is_none_or(|w| w.role != WindowRole::Normal || w.minimized)
        {
            let id = slot.window;
            self.leave_fullscreen(id);
            return;
        }
        if slot.output.is_none_or(|o| self.output(o).is_none()) {
            let output = self
                .window(slot.window)
                .and_then(|w| w.output)
                .filter(|o| self.output(*o).is_some())
                .or(self.focused_output)
                .or_else(|| self.outputs().next().map(|o| o.id));
            let return_workspace = output.and_then(|o| self.workspace_for_output(o));
            let slot = self.fullscreen.as_mut().unwrap();
            slot.output = output;
            if slot.return_output.is_none()
                && let Some(workspace) = return_workspace
            {
                slot.return_workspace = workspace;
            }
        }
        let slot = self.fullscreen.as_ref().unwrap();
        if slot.return_output.is_none_or(|o| self.output(o).is_none()) {
            let output = slot.output.or(self.focused_output);
            let workspace = output.and_then(|o| self.workspace_for_output(o));
            let slot = self.fullscreen.as_mut().unwrap();
            slot.return_output = output;
            if let Some(workspace) = workspace {
                slot.return_workspace = workspace;
            }
        }
    }
}
