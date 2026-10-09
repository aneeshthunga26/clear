use std::collections::{BTreeMap, HashMap};

use super::Rect;

/// Stable platform-assigned output identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputId(pub u64);

/// Stable platform-assigned window identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

/// Stable workspace identity, independent of its display name.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(pub u64);

/// A workspace default or a workspace-specific output override.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub enum Mode {
    /// All windows use their saved floating geometry.
    Floating,
    /// Horizontally scrolling columns, initially equal-width.
    Scrolling,
    /// One master window and a vertically divided stack.
    #[default]
    MasterStack,
    /// Weighted tiled columns, initially equal-width.
    Columns,
    /// Weighted tiled rows, initially equal-height.
    Rows,
    /// Tiled rows and columns with balanced cell counts.
    Grid,
    /// Successive shrinking splits that rotate around the remaining area.
    Spiral,
    /// Each tiled window fills the region; the focused one is stacked last.
    Monocle,
    /// A named layout delegated to the host, with master-stack fallback.
    Script(String),
}

impl Mode {
    /// Parses built-in names or `script:<name>`; unknown and empty names are rejected.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "floating" | "float" => Some(Self::Floating),
            "scrolling" | "scroll" => Some(Self::Scrolling),
            "master_stack" | "master-stack" | "masterstack" | "tiling" | "tile" => {
                Some(Self::MasterStack)
            }
            "columns" => Some(Self::Columns),
            "rows" => Some(Self::Rows),
            "grid" => Some(Self::Grid),
            "spiral" | "fibonacci" | "dwindle" => Some(Self::Spiral),
            "monocle" => Some(Self::Monocle),
            _ => {
                let (prefix, name) = value.split_once(':')?;
                let name = name.trim();
                (prefix.eq_ignore_ascii_case("script") && !name.is_empty())
                    .then(|| Self::Script(name.to_owned()))
            }
        }
    }

    /// Returns the canonical built-in name, or the script's name.
    pub fn name(&self) -> &str {
        match self {
            Self::Floating => "floating",
            Self::Scrolling => "scrolling",
            Self::MasterStack => "master_stack",
            Self::Columns => "columns",
            Self::Rows => "rows",
            Self::Grid => "grid",
            Self::Spiral => "spiral",
            Self::Monocle => "monocle",
            Self::Script(name) => name,
        }
    }

    pub(crate) fn next(&self) -> Self {
        match self {
            Self::Floating => Self::Scrolling,
            Self::Scrolling => Self::MasterStack,
            Self::MasterStack => Self::Columns,
            Self::Columns => Self::Rows,
            Self::Rows => Self::Grid,
            Self::Grid => Self::Spiral,
            Self::Spiral => Self::Monocle,
            Self::Monocle | Self::Script(_) => Self::Floating,
        }
    }
}

/// Placement policy independent of workspace mode or floating state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowRole {
    /// A regular window participating in its region's layout.
    #[default]
    Normal,
    /// A centered overlay above normal windows, excluded from layout policies.
    Launcher,
}

/// Declarative input to a layout policy; never contains platform objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutContext {
    /// Usable output area, already excluding host-managed reservations.
    pub area: Rect,
    /// Layout participants in stable order, excluding launchers, minimized/maximized/fullscreen
    /// windows, and tiled-mode floating exceptions.
    pub windows: Vec<LayoutWindow>,
    /// Globally focused window, if it belongs to this region.
    pub focused: Option<WindowId>,
    /// Horizontal logical-pixel offset, retained across mode changes.
    pub scroll_offset: i32,
    /// Nonnegative requested inner and outer gaps.
    pub gaps: i32,
}

/// Window metadata needed by the built-in geometry policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutWindow {
    /// Stable window identity.
    pub id: WindowId,
    /// Saved floating geometry, unaffected by tiling.
    pub floating_rect: Rect,
}

/// One window's desired geometry, in back-to-front stacking order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Window receiving the placement.
    pub window: WindowId,
    /// Desired geometry; floating windows and launchers may extend beyond the output.
    pub rect: Rect,
    /// Optional output-space clipping rectangle.
    pub clip: Option<Rect>,
    /// Whether this is the globally focused window.
    pub focused: bool,
    /// Whether interactive platform moves should first detach this window from tiling.
    pub tiled: bool,
}

/// Input-independent desktop operations. Invalid IDs are harmless no-ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Focus the next window in the active group's workspace order.
    FocusNext,
    /// Focus the previous window in the active group's workspace order.
    FocusPrevious,
    /// Reveal and focus a window, switching workspace if necessary.
    Focus(WindowId),
    /// Focus a region, restoring its remembered window if possible.
    FocusOutput(OutputId),
    /// Cycle connected outputs in ID order.
    CycleOutput,
    /// Present a workspace, swapping whole group presentations if already visible.
    SwitchWorkspace(WorkspaceId),
    /// Move the focused window without following it to the destination workspace.
    MoveToWorkspace(WorkspaceId),
    /// Move a specific window without revealing it or following its workspace.
    MoveWindowToWorkspace(WindowId, WorkspaceId),
    /// Move the focused window to an output's visible workspace and follow it.
    MoveToOutput(OutputId),
    /// Set the active workspace's default, retaining all region overrides.
    SetWorkspaceMode(Mode),
    /// Override the active region's mode for its current workspace.
    SetOutputMode(Mode),
    /// Remove the active region's override.
    ClearOutputMode,
    /// Cycle the active region's effective mode, storing an output override.
    CycleMode,
    /// Join all outputs to the active workspace, retaining other workspaces hidden.
    StretchAll,
    /// Split the active group; the focused region retains its workspace.
    Unstretch,
    /// Toggle a per-window floating exception without discarding its saved rectangle.
    ToggleFloating,
    /// Toggle maximization of the focused normal window within its usable output.
    ToggleMaximized,
    /// Toggle fullscreen on the focused normal window within its full home output.
    ToggleFullscreen,
    /// Hide the focused normal window without unmapping or discarding its state.
    MinimizeFocused,
    /// Set maximization without changing focus, floating state, or saved geometry.
    SetMaximized(WindowId, bool),
    /// Set fullscreen without changing focus or the underlying maximized restore state.
    SetFullscreen(WindowId, bool),
    /// Set minimization; restoring this way does not reveal its workspace or focus it.
    SetMinimized(WindowId, bool),
    /// Scroll the active scrolling region by logical pixels, clamped to its content.
    Scroll(i32),
    /// Update saved floating geometry without changing the window's floating flag.
    SetFloatingRect(WindowId, Rect),
    /// Ask the host to close the focused window; await actual removal.
    CloseFocused,
    /// Ask the host to spawn an executable and its arguments without a shell.
    Spawn(Vec<String>),
    /// Ask the host to exit.
    Quit,
}

/// Side effects interpreted by the platform after applying a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Request a graceful client close.
    Close(WindowId),
    /// Launch the supplied argument vector.
    Spawn(Vec<String>),
    /// End the compositor session.
    Quit,
}

/// A connected output with separate full and usable logical geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Stable platform identity.
    pub id: OutputId,
    /// Human-readable connector name.
    pub name: String,
    /// Full logical bounds including reservations; legacy empty outputs may lack bounds.
    pub bounds: Rect,
    /// Usable logical geometry after reservations; may be empty.
    pub area: Rect,
}

/// A set of outputs presenting exactly one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputGroup {
    /// Connected members, sorted by ID.
    pub outputs: Vec<OutputId>,
    /// Exclusively presented workspace.
    pub workspace: WorkspaceId,
}

/// Persistent window state, independent of its current layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// Stable platform identity.
    pub id: WindowId,
    /// Current client title.
    pub title: String,
    /// Current application identifier.
    pub app_id: String,
    /// Owning workspace, whether visible or hidden.
    pub workspace: WorkspaceId,
    /// Home region; may be disconnected only when there are no outputs.
    pub output: Option<OutputId>,
    /// Placement role, retained across mode changes.
    pub role: WindowRole,
    /// Last valid client-committed size, separate from requested floating geometry.
    pub committed_size: Option<(i32, i32)>,
    /// Per-window exception to nonfloating layouts.
    pub floating: bool,
    /// Fill the home output's usable area, independently of saved layout state.
    pub maximized: bool,
    /// Fill the full home output, preserving the underlying maximized restore state.
    pub fullscreen: bool,
    /// Hidden from placements and ordinary focus cycling; explicit focus restores it.
    pub minimized: bool,
    /// Saved geometry, not overwritten by tiling or temporary output constraints.
    pub floating_rect: Rect,
}

/// Persistent workspace policy and stable window order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Stable workspace identity.
    pub id: WorkspaceId,
    /// User-facing name.
    pub name: String,
    /// Default mode for regions without an override.
    pub mode: Mode,
    pub(crate) windows: Vec<WindowId>,
    pub(crate) regions: BTreeMap<OutputId, RegionState>,
}

impl Workspace {
    /// Returns the stable window order, including hidden and floating windows.
    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }

    /// Returns an explicit override, not the effective default.
    pub fn output_mode(&self, output: OutputId) -> Option<&Mode> {
        self.regions
            .get(&output)
            .and_then(|region| region.mode.as_ref())
    }

    /// Returns the retained scroll position for this output.
    pub fn scroll_offset(&self, output: OutputId) -> i32 {
        self.regions
            .get(&output)
            .map_or(0, |region| region.scroll_offset)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RegionState {
    pub mode: Option<Mode>,
    pub scroll_offset: i32,
    pub scroll_initialized: bool,
    pub focused: Option<WindowId>,
    pub sizing: HashMap<Mode, super::LayoutSizing>,
    pub resize_revision: u64,
    pub(super) resize_environment: Option<super::resize::ResizeEnvironment>,
}
