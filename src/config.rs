use std::{env, fs, path::PathBuf};

use serde::Deserialize;
use smithay::input::keyboard::{Keysym, ModifiersState, keysyms};

/// User-facing Clear configuration loaded from `config.toml`.
/// This file is located in the user's home directory at `$XDG_CONFIG_HOME/clear/config.toml`, or
/// `~/.config/clear/config.toml` if `XDG_CONFIG_HOME` is not set.
///
/// Each top-level field maps to a TOML section. Missing sections or fields use
/// defaults so users can override only the pieces they care about.
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
    /// Chord used to open the launcher. Currently supports `leader+Space`.
    pub launcher: String,
}

/// External app commands and app ids used by compositor actions.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppsConfig {
    /// Shell command used to launch the app launcher.
    pub launcher: String,
    /// XDG app id used to recognize and center the launcher window.
    pub launcher_app_id: String,
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
        let Some(chord) = KeyChord::parse(&self.shortcuts.launcher, &self.keys.leader) else {
            return Self::default().launcher_matches(modifiers, keysym);
        };

        chord.matches(modifiers, keysym)
    }

    /// Command that should be spawned for the launcher.
    pub fn launcher_command(&self) -> &str {
        self.apps.launcher.trim()
    }

    /// App id expected from the launcher's XDG toplevel surface.
    pub fn launcher_app_id(&self) -> &str {
        self.apps.launcher_app_id.trim()
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
        }
    }
}

impl Default for AppsConfig {
    fn default() -> Self {
        Self {
            launcher: "wofi --show drun".to_string(),
            launcher_app_id: "wofi".to_string(),
        }
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

        // Keep the first parser deliberately small until Clear has more bindings.
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
    /// Space is enough for the initial `leader+Space` launcher binding.
    Space,
}

impl ShortcutKey {
    fn parse(key: &str) -> Option<Self> {
        match normalize_key_name(key).as_str() {
            "space" => Some(Self::Space),
            _ => None,
        }
    }

    fn matches(&self, keysym: Keysym) -> bool {
        match self {
            Self::Space => keysym == keysyms::KEY_space.into(),
        }
    }
}

fn normalize_key_name(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace(' ', "")
}
