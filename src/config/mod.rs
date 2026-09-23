//! TOML configuration with deterministic defaults and all-or-default startup loading.
//!
//! `gaps` and `script` are top-level keys. Arrays use `[[outputs]]`,
//! `[[workspaces]]`, and `[[bindings]]`; each supplied array replaces its default.
//! Bindings use a flattened snake-case `action` tag and action-specific fields.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

mod shell;
pub use shell::{PanelLayer, PanelRule, ShellConfig};

use crate::{
    decoration::Theme,
    input::{self, Binding, Bindings},
};

/// A virtual output's logical dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    /// Unique output name, also used by workspace output-mode overrides.
    pub name: String,
    /// Logical width in pixels.
    pub width: i32,
    /// Logical height in pixels.
    pub height: i32,
}

/// A workspace and its default and per-output layout selections.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Stable positive workspace ID.
    pub id: u64,
    /// Human-readable workspace label.
    pub name: String,
    /// Built-in mode or `script:function_name` selected by the platform.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Output names mapped to layout mode overrides.
    #[serde(default)]
    pub output_modes: BTreeMap<String, String>,
}

fn default_mode() -> String {
    "columns".into()
}

/// Runtime configuration, independent of the compositor's platform objects.
#[derive(Debug, Clone)]
pub struct Config {
    /// Virtual outputs created at startup.
    pub outputs: Vec<OutputConfig>,
    /// Available workspaces, in presentation order.
    pub workspaces: Vec<WorkspaceConfig>,
    /// Uncompiled shortcuts, ready for `Bindings::new`.
    pub bindings: Vec<Binding>,
    /// Layout gap in logical pixels.
    pub gaps: i32,
    /// Optional Rhai extension file; relative paths are resolved when loading a file.
    pub script: Option<PathBuf>,
    /// Static renderer styling.
    pub theme: Theme,
    /// Launcher roles and namespace-specific panel policies.
    pub shell: ShellConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            outputs: (1..=2)
                .map(|n| OutputConfig {
                    name: format!("virtual-{n}"),
                    width: 800,
                    height: 600,
                })
                .collect(),
            workspaces: (1..=9)
                .map(|id| WorkspaceConfig {
                    id,
                    name: id.to_string(),
                    mode: default_mode(),
                    output_modes: BTreeMap::new(),
                })
                .collect(),
            bindings: input::default_bindings(),
            gaps: 8,
            script: None,
            theme: Theme::default(),
            shell: ShellConfig::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Source {
    outputs: Vec<OutputConfig>,
    workspaces: Vec<WorkspaceConfig>,
    bindings: Vec<Binding>,
    gaps: i32,
    script: Option<PathBuf>,
    theme: Theme,
    shell: ShellConfig,
    keys: Keys,
}

impl Default for Source {
    fn default() -> Self {
        let config = Config::default();
        Self {
            outputs: config.outputs,
            workspaces: config.workspaces,
            bindings: config.bindings,
            gaps: config.gaps,
            script: config.script,
            theme: config.theme,
            shell: config.shell,
            keys: Keys::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Keys {
    leader: String,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            leader: "Super".into(),
        }
    }
}

impl Config {
    /// Load the XDG config file, logging unreadable or invalid files to stderr.
    ///
    /// An absolute, nonempty `XDG_CONFIG_HOME` takes precedence over
    /// `$HOME/.config`. Missing files and invalid configuration use all defaults.
    pub fn load() -> Self {
        let path = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join(".config"))
            })
            .map(|base| base.join("clear/config.toml"));
        let Some(path) = path else {
            eprintln!("clear: no absolute XDG_CONFIG_HOME or HOME; using default configuration");
            return Self::default();
        };
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "clear: cannot read {}: {error}; using defaults",
                        path.display()
                    );
                }
                return Self::default();
            }
        };
        match Self::from_source(&source) {
            Ok(mut config) => {
                if let Some(script) = config.script.as_mut() {
                    if script.is_relative() {
                        *script = path.parent().unwrap_or(Path::new(".")).join(&*script);
                    }
                }
                config
            }
            Err(error) => {
                eprintln!("clear: invalid {}: {error}; using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Parse and validate TOML without filesystem access or environment mutation.
    ///
    /// An empty script path disables extensions. Relative script paths are left
    /// relative here; `load` resolves them against the config file directory.
    pub fn from_source(source: &str) -> Result<Self, String> {
        let mut source: Source = toml::from_str(source).map_err(|error| error.to_string())?;
        input::resolve_leader(&mut source.bindings, &source.keys.leader)?;
        let config = Self {
            outputs: source.outputs,
            workspaces: source.workspaces,
            bindings: source.bindings,
            gaps: source.gaps,
            script: source.script.filter(|p| !p.as_os_str().is_empty()),
            theme: source.theme,
            shell: source.shell,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.outputs.is_empty() || self.outputs.len() > 16 {
            return Err("configure between 1 and 16 outputs".into());
        }
        let mut outputs = BTreeSet::new();
        for output in &self.outputs {
            if output.name.trim().is_empty() || !outputs.insert(output.name.as_str()) {
                return Err(format!("empty or duplicate output name {:?}", output.name));
            }
            if !(1..=32_768).contains(&output.width) || !(1..=32_768).contains(&output.height) {
                return Err(format!(
                    "output {:?}: dimensions must be in 1..=32768",
                    output.name
                ));
            }
        }
        if self.workspaces.is_empty() || self.workspaces.len() > 256 {
            return Err("configure between 1 and 256 workspaces".into());
        }
        let mut workspaces = BTreeSet::new();
        for workspace in &self.workspaces {
            if workspace.id == 0
                || !workspaces.insert(workspace.id)
                || workspace.name.trim().is_empty()
            {
                return Err("workspaces require unique positive IDs and nonempty names".into());
            }
            validate_mode(&workspace.mode)?;
            for (output, mode) in &workspace.output_modes {
                if !outputs.contains(output.as_str()) {
                    return Err(format!(
                        "workspace {} refers to unknown output {output:?}",
                        workspace.id
                    ));
                }
                validate_mode(mode)?;
            }
        }
        if !(0..=4096).contains(&self.gaps) {
            return Err("gaps must be in 0..=4096".into());
        }
        self.theme.validate()?;
        self.shell.validate()?;
        Bindings::new(&self.bindings)?;
        Ok(())
    }
}

fn validate_mode(mode: &str) -> Result<(), String> {
    if mode.trim().is_empty() || mode.len() > 256 {
        return Err("workspace modes must be nonempty and at most 256 bytes".into());
    }
    if let Some(name) = mode.strip_prefix("script:") {
        if !input::valid_function_name(name) {
            return Err("script mode must be script:function_name".into());
        }
    }
    Ok(())
}
