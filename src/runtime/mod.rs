//! Configuration and scripting orchestration, without Wayland or renderer types.

use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{Duration, Instant},
};

pub mod animation;
pub mod overview;
pub use overview::{OverviewNavigation, OverviewSession, OverviewTarget};
pub mod titlebar;
pub mod wallpaper;
pub use titlebar::TitlebarAssets;
use wallpaper::Wallpapers;

use crate::{
    config::Config,
    core::{
        Command, Desktop, Effect, Mode, OutputId, Placement, Rect, WindowId, WindowRole,
        WorkspaceId,
    },
    input::{Action, Bindings},
    scripting::{ScriptContext, ScriptHost, ScriptRect, ScriptWindow},
};

/// Startup options shared by the CLI and platform adapter.
#[derive(Default)]
pub struct Options {
    /// Explicit configuration file; otherwise use the XDG location.
    pub config_path: Option<PathBuf>,
    /// Optional child executable and arguments, without shell expansion.
    pub command: Vec<String>,
    /// Gracefully stop after this duration, for bounded integration tests.
    pub exit_after: Option<Duration>,
    /// Explicit Wayland socket name, useful for external test clients.
    pub socket_name: Option<String>,
    /// Optional PPM framebuffer capture for visual smoke tests.
    pub capture: Option<PathBuf>,
    /// Disable the optional local shell state/command interface.
    pub no_shell_ipc: bool,
}

/// Owns desktop policy, compiled shortcuts, and the optional extension host.
pub struct Runtime {
    /// Backend-neutral desktop state.
    pub desktop: Desktop,
    /// Last successfully loaded configuration.
    pub config: Config,
    /// CPU wallpaper resources from the last successful preparation.
    pub wallpapers: Wallpapers,
    /// Bounded CPU titlebar resources; the adapter owns any uploaded textures.
    pub titlebar_assets: TitlebarAssets,
    /// Compiled bindings for backend-normalized key events.
    pub bindings: Bindings,
    /// Pending Alt-Tab selection, committed by the physical modifier release.
    pub switcher: Option<Switcher>,
    /// Current compositor-owned preview, independent of shell IPC.
    pub overview: Option<OverviewSession>,
    /// Toggle request awaiting adapter grab/held-input authorization.
    pub overview_requested: bool,
    /// Pure presentation tracks; the adapter does not yet create visual effects.
    pub animations: animation::AnimationEngine,
    animation_origin: Instant,
    config_path: Option<PathBuf>,
    script: Option<ScriptHost>,
    failed_functions: BTreeSet<String>,
}

/// Stable candidate order and selection for one keyboard switch gesture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switcher {
    pub output: OutputId,
    pub windows: Vec<WindowId>,
    pub selected: WindowId,
}

impl Runtime {
    /// Load startup policy, using defaults rather than refusing an invalid config.
    pub fn load(config_path: Option<PathBuf>) -> Result<Self, String> {
        let config_path = config_path.or_else(default_config_path);
        let config = config_path.as_ref().map_or_else(Config::default, |path| {
            read_config(path).unwrap_or_else(|error| {
                eprintln!("clear: {error}; using default configuration");
                Config::default()
            })
        });
        Self::new(config, config_path)
    }

    /// Construct policy from validated configuration, without creating any outputs.
    pub fn new(config: Config, config_path: Option<PathBuf>) -> Result<Self, String> {
        let animations =
            animation::AnimationEngine::new(config.animations.clone(), Duration::ZERO)?;
        let bindings = Bindings::new(&config.bindings)?;
        let script = load_script(&config).unwrap_or_else(|error| {
            eprintln!("clear: {error}; scripted layouts will use master_stack");
            None
        });
        let wallpapers = Wallpapers::prepare(&config).unwrap_or_else(|error| {
            eprintln!("clear: {error}; using solid theme background");
            Wallpapers::default()
        });
        let titlebar_assets =
            TitlebarAssets::prepare(&config.theme.titlebar).unwrap_or_else(|error| {
                eprintln!("clear: {error}; using built-in titlebar controls");
                TitlebarAssets::builtins(&config.theme.titlebar)
            });
        let mut runtime = Self {
            desktop: Desktop::new(),
            wallpapers,
            titlebar_assets,
            config,
            bindings,
            switcher: None,
            overview: None,
            overview_requested: false,
            animations,
            animation_origin: Instant::now(),
            config_path,
            script,
            failed_functions: BTreeSet::new(),
        };
        runtime.apply_config();
        Ok(runtime)
    }

    fn apply_config(&mut self) {
        self.desktop.set_gaps(self.config.gaps);
        for workspace in &self.config.workspaces {
            let mode = Mode::parse(&workspace.mode).unwrap_or_else(|| {
                eprintln!(
                    "clear: unknown mode {:?}; using master_stack",
                    workspace.mode
                );
                Mode::MasterStack
            });
            self.desktop.configure_workspace(
                WorkspaceId(workspace.id),
                workspace.name.clone(),
                mode,
            );
        }
        self.configure_output_modes();
        let windows: Vec<_> = self
            .desktop
            .windows()
            .map(|w| (w.id, w.app_id.clone()))
            .collect();
        for (id, app_id) in windows {
            self.classify_window(id, &app_id);
        }
    }

    /// Apply launcher policy after mapping, late app-ID changes, or config reload.
    pub fn classify_window(&mut self, id: crate::core::WindowId, app_id: &str) {
        self.titlebar_assets.prepare_app_icon(app_id);
        let role = if self.config.shell.is_launcher(app_id) {
            crate::core::WindowRole::Launcher
        } else {
            crate::core::WindowRole::Normal
        };
        self.desktop.set_window_role(id, role);
    }

    /// Resolve configured connector names after the backend creates its outputs.
    pub fn configure_output_modes(&mut self) {
        let outputs: Vec<_> = self
            .desktop
            .outputs()
            .map(|o| (o.id, o.name.clone()))
            .collect();
        for workspace in &self.config.workspaces {
            for (id, name) in &outputs {
                let mode = workspace
                    .output_modes
                    .get(name)
                    .and_then(|name| Mode::parse(name));
                self.desktop
                    .set_workspace_output_mode(WorkspaceId(workspace.id), *id, mode);
            }
        }
    }

    /// Reload atomically. Output topology changes require restarting the backend.
    pub fn reload(&mut self) -> Result<(), String> {
        let path = self
            .config_path
            .as_ref()
            .ok_or("no configuration path available")?;
        let config = read_config(path)?;
        if config.outputs != self.config.outputs {
            return Err("output topology changes require restarting Clear".into());
        }
        let bindings = Bindings::new(&config.bindings)?;
        let script = load_script(&config)?;
        let wallpapers = Wallpapers::prepare(&config)?;
        let titlebar_assets = TitlebarAssets::prepare(&config.theme.titlebar)?;
        self.animations.apply_config(
            config.animations.clone(),
            self.animation_time_at(Instant::now()),
        )?;
        self.config = config;
        self.wallpapers = wallpapers;
        self.titlebar_assets = titlebar_assets;
        self.bindings = bindings;
        self.script = script;
        self.failed_functions.clear();
        self.apply_config();
        eprintln!("clear: configuration reloaded");
        Ok(())
    }

    /// Convert an adapter's monotonic target timestamp into the engine's time domain.
    pub fn animation_time_at(&self, target: Instant) -> Duration {
        target.saturating_duration_since(self.animation_origin)
    }

    /// Execute a typed action, returning only the effects requiring platform IO.
    pub fn action(&mut self, action: Action) -> Vec<Effect> {
        let mut pending = vec![action];
        let mut effects = Vec::new();
        let mut budget = 128;
        while let Some(action) = pending.pop() {
            if budget == 0 {
                eprintln!("clear: script action expansion limit reached");
                break;
            }
            budget -= 1;
            eprintln!("clear: action {action:?}");
            self.switcher = None;
            let command = match action {
                // Only the physical-key adapter can start a modifier-held gesture.
                Action::AltTab => continue,
                Action::ToggleOverview => {
                    self.overview_requested = !self.overview_requested;
                    continue;
                }
                Action::FocusNext => Command::FocusNext,
                Action::FocusPrevious => Command::FocusPrevious,
                Action::CycleOutput => Command::CycleOutput,
                Action::SwitchWorkspace { workspace } => {
                    Command::SwitchWorkspace(WorkspaceId(workspace))
                }
                Action::MoveToWorkspace { workspace } => {
                    Command::MoveToWorkspace(WorkspaceId(workspace))
                }
                Action::MoveToOutput { output } => Command::MoveToOutput(OutputId(output)),
                Action::SetWorkspaceMode { mode } => {
                    let Some(mode) = Mode::parse(&mode) else {
                        eprintln!("clear: unknown mode {mode:?}");
                        continue;
                    };
                    Command::SetWorkspaceMode(mode)
                }
                Action::SetOutputMode { mode } => {
                    let Some(mode) = Mode::parse(&mode) else {
                        eprintln!("clear: unknown mode {mode:?}");
                        continue;
                    };
                    Command::SetOutputMode(mode)
                }
                Action::ClearOutputMode => Command::ClearOutputMode,
                Action::CycleMode => Command::CycleMode,
                Action::StretchAll => Command::StretchAll,
                Action::Unstretch => Command::Unstretch,
                Action::ToggleFloating => Command::ToggleFloating,
                Action::ToggleMaximized => Command::ToggleMaximized,
                Action::Minimize => Command::MinimizeFocused,
                Action::Scroll { amount } => Command::Scroll(amount),
                Action::CloseFocused => Command::CloseFocused,
                Action::Spawn { command } => Command::Spawn(command),
                Action::Quit => Command::Quit,
                Action::Reload => {
                    if let Err(error) = self.reload() {
                        eprintln!("clear: reload rejected: {error}; keeping current configuration");
                    }
                    continue;
                }
                Action::Script { name } => {
                    if self.failed_functions.contains(&name) {
                        continue;
                    }
                    if let Some(script) = self.script.as_mut() {
                        match script.action(&name) {
                            Ok(actions) => pending.extend(actions.into_iter().rev()),
                            Err(_) => {
                                self.failed_functions.insert(name);
                            }
                        }
                    }
                    continue;
                }
            };
            effects.extend(self.desktop.command(command));
        }
        effects
    }

    /// Start/close the overview after the adapter authorizes input ownership.
    pub fn toggle_overview(&mut self) {
        self.overview_requested = false;
        self.cancel_switcher();
        if self.overview.take().is_none() {
            self.overview = OverviewSession::new(&self.desktop);
        }
    }

    /// Cancel preview without changing desktop focus, geometry, or presentation.
    pub fn cancel_overview(&mut self) {
        self.overview = None;
        self.overview_requested = false;
    }

    /// Commit a validated transient selection through core commands.
    pub fn finish_overview(&mut self) {
        if let Some(session) = self.overview.take() {
            session.activate(&mut self.desktop);
        }
        self.overview_requested = false;
    }

    /// Remove stale candidates after desktop/client lifecycle changes.
    pub fn refresh_overview(&mut self) {
        if self
            .overview
            .as_mut()
            .is_some_and(|s| !s.refresh(&self.desktop))
        {
            self.cancel_overview();
        }
    }

    /// Advance a selection among normal windows on the focused workspace.
    pub fn advance_switcher(&mut self) {
        if let Some(switcher) = &mut self.switcher {
            if let Some(index) = switcher
                .windows
                .iter()
                .position(|id| *id == switcher.selected)
            {
                switcher.selected = switcher.windows[(index + 1) % switcher.windows.len()];
            }
            return;
        }
        let desktop = &self.desktop;
        let Some(output) = desktop.focused_output() else {
            return;
        };
        let Some(workspace) = desktop
            .workspace_for_output(output)
            .and_then(|id| desktop.workspace(id))
        else {
            return;
        };
        let windows: Vec<_> = workspace
            .windows()
            .iter()
            .copied()
            .filter(|id| {
                desktop
                    .window(*id)
                    .is_some_and(|window| window.role == WindowRole::Normal)
            })
            .collect();
        if windows.is_empty() {
            return;
        }
        let current = desktop.focused_window();
        let next = current
            .and_then(|id| windows.iter().position(|candidate| *candidate == id))
            .map_or(0, |index| (index + 1) % windows.len());
        self.switcher = Some(Switcher {
            output,
            selected: windows[next],
            windows,
        });
    }

    /// Commit the selected window when Alt is physically released.
    pub fn finish_switcher(&mut self) {
        if let Some(switcher) = self.switcher.take() {
            self.desktop.command(Command::Focus(switcher.selected));
        }
    }

    /// Cancel a pending switch without changing focus.
    pub fn cancel_switcher(&mut self) {
        self.switcher = None;
    }

    /// Compute a scene; a faulty script is disabled until reload, with built-in fallback.
    pub fn placements(&mut self) -> Vec<Placement> {
        let script = &mut self.script;
        let failed = &mut self.failed_functions;
        self.desktop.placements_with(|mode, ctx| {
            let Mode::Script(name) = mode else {
                return None;
            };
            if failed.contains(name) {
                return None;
            }
            let host = script.as_mut()?;
            let result = host.layout(
                name,
                ScriptContext {
                    area: to_script_rect(ctx.area),
                    windows: ctx
                        .windows
                        .iter()
                        .map(|w| ScriptWindow {
                            id: w.id.0,
                            rect: to_script_rect(w.floating_rect),
                        })
                        .collect(),
                    focused: ctx.focused.map(|id| id.0),
                    gaps: ctx.gaps,
                    scroll_offset: ctx.scroll_offset,
                },
            );
            match result {
                Ok(placements) => Some(
                    placements
                        .into_iter()
                        .map(|p| Placement {
                            window: crate::core::WindowId(p.window),
                            rect: Rect::new(p.rect.x, p.rect.y, p.rect.width, p.rect.height),
                            clip: Some(ctx.area),
                            focused: ctx.focused.map(|id| id.0) == Some(p.window),
                            tiled: true,
                        })
                        .collect(),
                ),
                Err(_) => {
                    failed.insert(name.clone());
                    None
                }
            }
        })
    }
}

fn to_script_rect(rect: Rect) -> ScriptRect {
    ScriptRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn load_script(config: &Config) -> Result<Option<ScriptHost>, String> {
    config.script.as_deref().map(ScriptHost::load).transpose()
}

fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".config"))
        })
        .map(|p| p.join("clear/config.toml"))
}

fn read_config(path: &std::path::Path) -> Result<Config, String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut config = Config::from_source(&source)?;
    config.resolve_paths(path.parent().unwrap_or(std::path::Path::new(".")));
    Ok(config)
}
