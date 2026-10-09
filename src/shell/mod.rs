//! Backend-independent shell state and versioned JSON messages.
//!
//! The transport owns newline framing, connection limits, subscriptions, and backend
//! reconciliation. This module neither performs IO nor exposes process/script actions.

pub mod server;

use serde::{Deserialize, Serialize};

use crate::{
    core::{Command, Mode, OutputId, WindowRole},
    runtime::Runtime,
};

/// Version carried by every request and response.
pub const PROTOCOL_VERSION: u32 = 1;

const MAX_MODE_BYTES: usize = 256;

/// A single request; desktop identities are decimal strings, not JSON numbers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: u32,
    pub request: RequestKind,
}

/// The complete shell capability allowlist. No arbitrary actions are accepted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestKind {
    #[serde(deserialize_with = "empty_request")]
    Snapshot,
    #[serde(deserialize_with = "empty_request")]
    Subscribe,
    #[serde(deserialize_with = "empty_request")]
    ToggleOverview,
    FocusOutput {
        output: String,
    },
    SwitchWorkspace {
        output: String,
        workspace: String,
    },
    FocusWindow {
        window: String,
    },
    SetMaximized {
        window: String,
        maximized: bool,
    },
    SetMinimized {
        window: String,
        minimized: bool,
    },
    SetMode {
        output: String,
        mode: String,
    },
    ClearMode {
        output: String,
    },
    Stretch {
        output: String,
    },
    Split {
        output: String,
    },
}

fn empty_request<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    // Serde's internally tagged unit variants otherwise ignore extra fields, even
    // with deny_unknown_fields on the enum. An empty struct enforces the shape.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct EmptyRequest {}

    EmptyRequest::deserialize(deserializer).map(|_| ())
}

/// A complete, owned view of desktop policy and pending switch selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub outputs: Vec<OutputSnapshot>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub groups: Vec<GroupSnapshot>,
    pub windows: Vec<WindowSnapshot>,
    pub focused_output: Option<String>,
    pub focused_window: Option<String>,
    pub switcher: Option<SwitcherSnapshot>,
    /// Whether compositor-owned overview input/UI is active.
    pub overview_open: bool,
}

/// Pending Alt-Tab candidates and selected window, without changing focus yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SwitcherSnapshot {
    pub output: String,
    pub windows: Vec<String>,
    pub selected: String,
}

/// Usable logical output area, after reservations; not the physical output bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A connected output and the mode of its currently presented workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputSnapshot {
    pub id: String,
    pub name: String,
    pub area: Area,
    pub workspace: String,
    pub effective_mode: String,
    pub mode_override: Option<String>,
}

/// Persistent workspace policy and window IDs in core workspace order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceSnapshot {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub windows: Vec<String>,
}

/// Outputs jointly presenting one workspace, with no duplicated presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupSnapshot {
    pub outputs: Vec<String>,
    pub workspace: String,
}

/// Client metadata and saved policy, not a rendered placement or visibility test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowSnapshot {
    pub id: String,
    pub title: String,
    pub app_id: String,
    pub workspace: String,
    /// Home output, including for hidden windows; null when no home is assigned.
    pub output: Option<String>,
    /// Either `normal` or `launcher`.
    pub role: String,
    /// Saved per-window floating flag, independent of workspace mode or role.
    pub floating: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub focused: bool,
}

/// Copy policy and switcher state without computing placements or changing focus.
pub fn snapshot(runtime: &Runtime) -> Snapshot {
    let desktop = &runtime.desktop;
    Snapshot {
        outputs: desktop
            .outputs()
            .map(|output| {
                // Core guarantees that each connected output presents one workspace.
                let workspace = desktop
                    .workspace_for_output(output.id)
                    .and_then(|id| desktop.workspace(id))
                    .expect("connected output must present an existing workspace");
                let mode_override = workspace.output_mode(output.id);
                OutputSnapshot {
                    id: output.id.0.to_string(),
                    name: output.name.clone(),
                    area: Area {
                        x: output.area.x,
                        y: output.area.y,
                        width: output.area.width,
                        height: output.area.height,
                    },
                    workspace: workspace.id.0.to_string(),
                    effective_mode: mode_name(mode_override.unwrap_or(&workspace.mode)),
                    mode_override: mode_override.map(mode_name),
                }
            })
            .collect(),
        workspaces: desktop
            .workspaces()
            .map(|workspace| WorkspaceSnapshot {
                id: workspace.id.0.to_string(),
                name: workspace.name.clone(),
                mode: mode_name(&workspace.mode),
                windows: workspace
                    .windows()
                    .iter()
                    .map(|id| id.0.to_string())
                    .collect(),
            })
            .collect(),
        groups: desktop
            .groups()
            .iter()
            .map(|group| GroupSnapshot {
                outputs: group.outputs.iter().map(|id| id.0.to_string()).collect(),
                workspace: group.workspace.0.to_string(),
            })
            .collect(),
        windows: desktop
            .windows()
            .map(|window| WindowSnapshot {
                id: window.id.0.to_string(),
                title: window.title.clone(),
                app_id: window.app_id.clone(),
                workspace: window.workspace.0.to_string(),
                output: window.output.map(|id| id.0.to_string()),
                role: match window.role {
                    WindowRole::Normal => "normal",
                    WindowRole::Launcher => "launcher",
                }
                .into(),
                floating: window.floating,
                maximized: window.maximized,
                minimized: window.minimized,
                focused: desktop.focused_window() == Some(window.id),
            })
            .collect(),
        focused_output: desktop.focused_output().map(|id| id.0.to_string()),
        focused_window: desktop.focused_window().map(|id| id.0.to_string()),
        overview_open: runtime.overview.is_some(),
        switcher: runtime.switcher.as_ref().map(|switcher| SwitcherSnapshot {
            output: switcher.output.0.to_string(),
            windows: switcher.windows.iter().map(|id| id.0.to_string()).collect(),
            selected: switcher.selected.0.to_string(),
        }),
    }
}

fn mode_name(mode: &Mode) -> String {
    match mode {
        Mode::Script(name) => format!("script:{name}"),
        _ => mode.name().to_owned(),
    }
}

/// Decode one JSON message (with or without its trailing newline).
/// Framing and maximum message size are the transport's responsibility.
pub fn parse_request(bytes: &[u8]) -> Result<Request, String> {
    let request: Request = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    validate_version(request.version)?;
    Ok(request)
}

fn validate_version(version: u32) -> Result<(), String> {
    if version != PROTOCOL_VERSION {
        return Err(format!(
            "unsupported protocol version {version}; expected {PROTOCOL_VERSION}"
        ));
    }
    Ok(())
}

fn output_id(runtime: &Runtime, id: &str) -> Result<OutputId, String> {
    runtime
        .desktop
        .outputs()
        .find(|output| output.id.0.to_string() == id)
        .map(|output| output.id)
        .ok_or_else(|| format!("unknown output {id:?}"))
}

/// Apply only allowlisted policy operations, validating every target before mutation.
///
/// IDs must match the canonical decimal strings in snapshots. Mode names are at
/// most 256 UTF-8 bytes and use core parsing (including aliases and `script:NAME`
/// with core's normal layout fallback). Mode changes do not select/focus outputs.
/// Switch/stretch/split select their output first; window focus may reveal a hidden
/// workspace. Snapshot/subscribe are no-ops here; the transport manages replies.
pub fn execute(runtime: &mut Runtime, request: &Request) -> Result<(), String> {
    validate_version(request.version)?;
    match &request.request {
        RequestKind::Snapshot | RequestKind::Subscribe => {}
        RequestKind::ToggleOverview => {
            runtime.overview_requested = !runtime.overview_requested;
        }
        RequestKind::FocusOutput { output } => {
            let output = output_id(runtime, output)?;
            runtime.desktop.command(Command::FocusOutput(output));
        }
        RequestKind::SwitchWorkspace { output, workspace } => {
            let output = output_id(runtime, output)?;
            let workspace = runtime
                .desktop
                .workspaces()
                .find(|candidate| candidate.id.0.to_string() == *workspace)
                .map(|workspace| workspace.id)
                .ok_or_else(|| format!("unknown workspace {workspace:?}"))?;
            runtime.desktop.command(Command::FocusOutput(output));
            runtime.desktop.command(Command::SwitchWorkspace(workspace));
        }
        RequestKind::FocusWindow { window } => {
            let window = runtime
                .desktop
                .windows()
                .find(|candidate| candidate.id.0.to_string() == *window)
                .map(|window| window.id)
                .ok_or_else(|| format!("unknown window {window:?}"))?;
            runtime.desktop.command(Command::Focus(window));
        }
        RequestKind::SetMaximized { window, .. } | RequestKind::SetMinimized { window, .. } => {
            let window = runtime
                .desktop
                .windows()
                .find(|candidate| candidate.id.0.to_string() == *window)
                .ok_or_else(|| format!("unknown window {window:?}"))?;
            if window.role != WindowRole::Normal {
                return Err("launcher windows remain client-sized and cannot be minimized".into());
            }
            let command = match &request.request {
                RequestKind::SetMaximized { maximized, .. } => {
                    Command::SetMaximized(window.id, *maximized)
                }
                RequestKind::SetMinimized { minimized, .. } => {
                    Command::SetMinimized(window.id, *minimized)
                }
                _ => unreachable!(),
            };
            runtime.desktop.command(command);
        }
        RequestKind::SetMode { output, .. } | RequestKind::ClearMode { output } => {
            let output = output_id(runtime, output)?;
            let workspace = runtime
                .desktop
                .workspace_for_output(output)
                .ok_or("output has no workspace")?;
            let mode = if let RequestKind::SetMode { mode, .. } = &request.request {
                if mode.len() > MAX_MODE_BYTES {
                    return Err(format!("mode exceeds {MAX_MODE_BYTES} bytes"));
                }
                Some(Mode::parse(mode).ok_or_else(|| format!("unknown or empty mode {mode:?}"))?)
            } else {
                None
            };
            runtime
                .desktop
                .set_workspace_output_mode(workspace, output, mode);
        }
        RequestKind::Stretch { output } | RequestKind::Split { output } => {
            let output = output_id(runtime, output)?;
            let command = if matches!(request.request, RequestKind::Stretch { .. }) {
                Command::StretchAll
            } else {
                Command::Unstretch
            };
            runtime.desktop.command(Command::FocusOutput(output));
            runtime.desktop.command(command);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Response<'a> {
    version: u32,
    #[serde(flatten)]
    message: Message<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message<'a> {
    Snapshot { id: u32, state: &'a Snapshot },
    State { state: &'a Snapshot },
    Ok { id: u32 },
    Error { id: Option<u32>, message: &'a str },
}

fn encode(message: Message<'_>) -> Vec<u8> {
    // These concrete models contain only infallibly serializable JSON data.
    let mut bytes = serde_json::to_vec(&Response {
        version: PROTOCOL_VERSION,
        message,
    })
    .expect("shell response must serialize");
    bytes.push(b'\n');
    bytes
}

/// Encode a solicited full snapshot, followed by exactly one newline.
pub fn encode_snapshot(id: u32, state: &Snapshot) -> Vec<u8> {
    encode(Message::Snapshot { id, state })
}

/// Encode an unsolicited full state update without a request ID.
pub fn encode_update(state: &Snapshot) -> Vec<u8> {
    encode(Message::State { state })
}

/// Encode successful execution of a mutating request.
pub fn encode_ok(id: u32) -> Vec<u8> {
    encode(Message::Ok { id })
}

/// Encode an error; an unavailable request ID is represented as JSON null.
pub fn encode_error(id: Option<u32>, message: &str) -> Vec<u8> {
    encode(Message::Error { id, message })
}
