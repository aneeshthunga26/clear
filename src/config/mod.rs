use std::{env, fs, path::PathBuf};

use serde::Deserialize;
use smithay::input::keyboard::{Keysym, ModifiersState, keysyms};

pub mod app_launcher;
pub mod status_bar;

use app_launcher::AppLauncherConfig;
use status_bar::StatusBarConfig;

/// User-facing Clear configuration loaded from `config.toml`.
///
/// This file is located in the user's home directory at
/// `$XDG_CONFIG_HOME/clear/config.toml`, or `~/.config/clear/config.toml` if
/// `XDG_CONFIG_HOME` is not set.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Key aliases shared by shortcut definitions.
    pub keys: KeysConfig,
    /// Compositor-level keybindings.
    pub shortcuts: ShortcutsConfig,
    /// External applications Clear can launch.
    pub apps: AppsConfig,
}

/// Named keys that can be referenced from shortcuts.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    /// Modifier used as the shortcut leader. Defaults to the Super/Windows key.
    pub leader: String,
}

/// Configurable compositor shortcuts.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    /// Chord used to open the launcher.
    pub launcher: String,
    /// Chord used to toggle the status bar.
    pub status_bar: String,
}

/// External app commands and app ids used by compositor actions.
#[derive(Debug, Clone)]
pub struct AppsConfig {
    /// Launcher config in the new `[apps.launcher]` format.
    pub launcher: AppLauncherConfig,
    /// Status bar config in the new `[apps.status_bar]` format.
    pub status_bar: StatusBarConfig,
}

impl Config {
    /// Load config from the XDG config path, falling back to built-in defaults.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };

        match fs::read_to_string(path) {
            // Invalid config should not prevent the compositor from starting.
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Return true when the current key event matches the configured launcher chord.
    pub fn launcher_matches(&self, modifiers: &ModifiersState, keysym: Keysym) -> bool {
        self.shortcut_matches(&self.shortcuts.launcher, modifiers, keysym)
    }

    /// Return true when the current key event matches the configured status bar chord.
    pub fn status_bar_matches(&self, modifiers: &ModifiersState, keysym: Keysym) -> bool {
        self.shortcut_matches(&self.shortcuts.status_bar, modifiers, keysym)
    }

    /// Command that should be spawned for the launcher.
    pub fn launcher_command(&self) -> &str {
        self.apps.launcher.command()
    }

    /// App id expected from the launcher's XDG toplevel surface.
    pub fn launcher_app_id(&self) -> &str {
        self.apps.launcher.app_id()
    }

    fn shortcut_matches(&self, shortcut: &str, modifiers: &ModifiersState, keysym: Keysym) -> bool {
        let Some(chord) = KeyChord::parse(shortcut, &self.keys.leader) else {
            return false;
        };

        chord.matches(modifiers, keysym)
    }

    fn path() -> Option<PathBuf> {
        if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(config_home).join("clear/config.toml"));
        }

        env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/clear/config.toml"))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keys: KeysConfig::default(),
            shortcuts: ShortcutsConfig::default(),
            apps: AppsConfig::default(),
        }
    }
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            leader: "Super".to_string(),
        }
    }
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            launcher: "leader+Space".to_string(),
            status_bar: "leader+Grave".to_string(),
        }
    }
}

impl Default for AppsConfig {
    fn default() -> Self {
        Self {
            launcher: AppLauncherConfig::default(),
            status_bar: StatusBarConfig::default(),
        }
    }
}

impl<'de> Deserialize<'de> for AppsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(default)]
        struct RawAppsConfig {
            launcher: Option<AppLauncherConfig>,
            status_bar: StatusBarConfig,
            command: Option<String>,
            app_id: Option<String>,
        }

        impl Default for RawAppsConfig {
            fn default() -> Self {
                Self {
                    launcher: None,
                    status_bar: StatusBarConfig::default(),
                    command: None,
                    app_id: None,
                }
            }
        }

        let raw = RawAppsConfig::deserialize(deserializer)?;
        let mut launcher = raw.launcher.unwrap_or_default();
        if let Some(command) = raw.command {
            launcher.command = command;
        }
        if let Some(app_id) = raw.app_id {
            launcher.app_id = app_id;
        }

        Ok(Self {
            launcher,
            status_bar: raw.status_bar,
        })
    }
}

#[derive(Debug, Clone)]
struct KeyChord {
    leader: LeaderKey,
    key: ShortcutKey,
}

impl KeyChord {
    fn parse(chord: &str, leader: &str) -> Option<Self> {
        let mut parts = chord.split('+').map(str::trim);
        let first = parts.next()?;
        let second = parts.next()?;

        // Keep the parser small: Clear currently supports leader-based chords.
        if parts.next().is_some() || !first.eq_ignore_ascii_case("leader") {
            return None;
        }

        Some(Self {
            leader: LeaderKey::parse(leader)?,
            key: ShortcutKey::parse(second)?,
        })
    }

    fn matches(&self, modifiers: &ModifiersState, keysym: Keysym) -> bool {
        self.leader.matches(modifiers) && self.key.matches(keysym)
    }
}

#[derive(Debug, Clone)]
enum LeaderKey {
    Super,
    Ctrl,
    Alt,
    Shift,
}

impl LeaderKey {
    fn parse(key: &str) -> Option<Self> {
        match normalize_key_name(key).as_str() {
            "super" | "logo" | "windows" | "win" => Some(Self::Super),
            "ctrl" | "control" => Some(Self::Ctrl),
            "alt" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            _ => None,
        }
    }

    fn matches(&self, modifiers: &ModifiersState) -> bool {
        match self {
            Self::Super => modifiers.logo,
            Self::Ctrl => modifiers.ctrl,
            Self::Alt => modifiers.alt,
            Self::Shift => modifiers.shift,
        }
    }
}

#[derive(Debug, Clone)]
enum ShortcutKey {
    Space,
    B,
    Grave,
}

impl ShortcutKey {
    fn parse(key: &str) -> Option<Self> {
        match normalize_key_name(key).as_str() {
            "space" => Some(Self::Space),
            "b" => Some(Self::B),
            "grave" | "backtick" | "`" => Some(Self::Grave),
            _ => None,
        }
    }

    fn matches(&self, keysym: Keysym) -> bool {
        match self {
            Self::Space => keysym == keysyms::KEY_space.into(),
            Self::B => keysym == keysyms::KEY_b.into() || keysym == keysyms::KEY_B.into(),
            Self::Grave => keysym == keysyms::KEY_grave.into(),
        }
    }
}

fn normalize_key_name(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace(' ', "")
}
