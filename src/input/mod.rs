//! Platform-independent shortcut parsing and typed compositor requests.

use std::collections::HashMap;

use serde::Deserialize;

/// A requested operation; execution and authorization belong to the compositor.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Advance the Alt-Tab selection; focus changes when Alt is released.
    AltTab,
    FocusNext,
    FocusPrevious,
    CycleOutput,
    SwitchWorkspace {
        workspace: u64,
    },
    MoveToWorkspace {
        workspace: u64,
    },
    MoveToOutput {
        output: u64,
    },
    SetWorkspaceMode {
        mode: String,
    },
    SetOutputMode {
        mode: String,
    },
    ClearOutputMode,
    CycleMode,
    StretchAll,
    Unstretch,
    ToggleFloating,
    /// Toggle maximization without changing the saved layout or floating rectangle.
    ToggleMaximized,
    /// Hide the focused window; explicit focus or Alt-Tab restores it.
    Minimize,
    Scroll {
        amount: i32,
    },
    CloseFocused,
    Spawn {
        command: Vec<String>,
    },
    Quit,
    Reload,
    Script {
        name: String,
    },
}

impl Action {
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::SwitchWorkspace { workspace } | Self::MoveToWorkspace { workspace }
                if *workspace == 0 =>
            {
                Err("workspace IDs must be positive".into())
            }
            Self::SetWorkspaceMode { mode } | Self::SetOutputMode { mode }
                if mode.trim().is_empty() || mode.len() > 256 =>
            {
                Err("mode must be nonempty and at most 256 bytes".into())
            }
            Self::Spawn { command }
                if command.is_empty()
                    || command[0].trim().is_empty()
                    || command.len() > 256
                    || command
                        .iter()
                        .any(|arg| arg.contains('\0') || arg.len() > 16_384) =>
            {
                Err("spawn requires a nonempty executable and bounded, NUL-free arguments".into())
            }
            Self::Script { name } if !valid_function_name(name) => {
                Err("script name must be an ASCII function identifier".into())
            }
            _ => Ok(()),
        }
    }
}

pub(crate) fn valid_function_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
}

/// A shortcut with a flattened, snake-case action tag, e.g. `action = "quit"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Modifier names separated by `+`, followed by one key name.
    pub key: String,
    /// The operation requested when the shortcut is pressed.
    pub action: Action,
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        // Serde's flattened internally-tagged enum does not consume unknown
        // fields consistently for unit variants, so validate this flat map first.
        let mut fields = toml::Table::deserialize(deserializer)?;
        let key = fields
            .remove("key")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| D::Error::custom("binding requires a string key"))?;
        let action: Action = toml::Value::Table(fields.clone())
            .try_into()
            .map_err(D::Error::custom)?;
        let parameter = match &action {
            Action::SwitchWorkspace { .. } | Action::MoveToWorkspace { .. } => "workspace",
            Action::MoveToOutput { .. } => "output",
            Action::SetWorkspaceMode { .. } | Action::SetOutputMode { .. } => "mode",
            Action::Scroll { .. } => "amount",
            Action::Spawn { .. } => "command",
            Action::Script { .. } => "name",
            _ => "",
        };
        if let Some(field) = fields
            .keys()
            .find(|field| field.as_str() != "action" && field.as_str() != parameter)
        {
            return Err(D::Error::custom(format!("unknown binding field {field:?}")));
        }
        Ok(Self { key, action })
    }
}

/// Keyboard modifiers, independent of a particular input backend.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Control modifier.
    pub ctrl: bool,
    /// Alt modifier.
    pub alt: bool,
    /// Shift modifier.
    pub shift: bool,
    /// Logo/Super modifier.
    pub logo: bool,
}

/// Validated shortcuts indexed by normalized modifiers and key name.
#[derive(Debug, Clone)]
pub struct Bindings {
    actions: HashMap<(Modifiers, String), Action>,
}

impl Bindings {
    /// Parse shortcuts, rejecting malformed keys, actions and duplicate chords.
    pub fn new(bindings: &[Binding]) -> Result<Self, String> {
        let mut actions = HashMap::new();
        for binding in bindings {
            let chord = parse_chord(&binding.key)
                .map_err(|error| format!("binding {:?}: {error}", binding.key))?;
            binding
                .action
                .validate()
                .map_err(|error| format!("binding {:?}: {error}", binding.key))?;
            if actions.insert(chord, binding.action.clone()).is_some() {
                return Err(format!("duplicate shortcut {:?}", binding.key));
            }
        }
        Ok(Self { actions })
    }

    /// Resolve a key name case-insensitively; unknown keys do not match.
    ///
    /// Backends should supply the unshifted key name (e.g. `1`, not `!`).
    pub fn action(&self, modifiers: Modifiers, key: &str) -> Option<Action> {
        self.actions.get(&(modifiers, normalize_key(key)?)).cloned()
    }
}

fn parse_chord(chord: &str) -> Result<(Modifiers, String), String> {
    let parts: Vec<_> = chord.split('+').map(str::trim).collect();
    let (key, modifiers) = parts.split_last().ok_or("empty shortcut")?;
    let mut parsed = Modifiers::default();
    for modifier in modifiers {
        let slot = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => &mut parsed.ctrl,
            "alt" | "mod1" => &mut parsed.alt,
            "shift" => &mut parsed.shift,
            "super" | "logo" | "win" | "mod4" | "leader" => &mut parsed.logo,
            _ => return Err(format!("unknown modifier {modifier:?}")),
        };
        if *slot {
            return Err(format!("repeated modifier {modifier:?}"));
        }
        *slot = true;
    }
    let key = normalize_key(key).ok_or_else(|| format!("unknown key {key:?}"))?;
    Ok((parsed, key))
}

fn normalize_key(key: &str) -> Option<String> {
    let key = key.trim().to_ascii_lowercase();
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() {
        return Some(key);
    }
    if let Some(number) = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=35).contains(&number) && key == format!("f{number}") {
            return Some(key);
        }
    }
    let canonical = match key.as_str() {
        "return" | "enter" => "return",
        "escape" | "esc" => "escape",
        "space" | "spacebar" => "space",
        "tab" | "iso_left_tab" => "tab",
        "backspace" | "back_space" => "backspace",
        "delete" | "del" => "delete",
        "insert" | "ins" => "insert",
        "home" => "home",
        "end" => "end",
        "left" => "left",
        "right" => "right",
        "up" => "up",
        "down" => "down",
        "pageup" | "page_up" | "prior" => "page_up",
        "pagedown" | "page_down" | "next" => "page_down",
        "print" | "printscreen" | "print_screen" | "sys_req" => "print",
        "pause" | "break" => "pause",
        "menu" => "menu",
        "grave" | "quoteleft" | "`" => "grave",
        "minus" | "-" => "minus",
        "equal" | "=" => "equal",
        "plus" => "plus",
        "bracketleft" | "[" => "bracketleft",
        "bracketright" | "]" => "bracketright",
        "backslash" | "\\" => "backslash",
        "semicolon" | ";" => "semicolon",
        "apostrophe" | "quoteright" | "'" => "apostrophe",
        "comma" | "," => "comma",
        "period" | "." => "period",
        "slash" | "/" => "slash",
        "caps_lock" | "capslock" => "caps_lock",
        "num_lock" | "numlock" => "num_lock",
        "scroll_lock" | "scrolllock" => "scroll_lock",
        "xf86audiomute" => "xf86audiomute",
        "xf86audiolowervolume" => "xf86audiolowervolume",
        "xf86audioraisevolume" => "xf86audioraisevolume",
        "xf86audioplay" => "xf86audioplay",
        "xf86audiostop" => "xf86audiostop",
        "xf86audioprev" => "xf86audioprev",
        "xf86audionext" => "xf86audionext",
        "xf86monbrightnessup" => "xf86monbrightnessup",
        "xf86monbrightnessdown" => "xf86monbrightnessdown",
        _ => return None,
    };
    Some(canonical.into())
}

/// Default shortcuts; a configured binding array replaces this entire list.
pub fn default_bindings() -> Vec<Binding> {
    let mut bindings = vec![
        ("Alt+Tab", Action::AltTab),
        (
            "leader+Return",
            Action::Spawn {
                command: vec!["foot".into()],
            },
        ),
        ("leader+q", Action::CloseFocused),
        ("leader+Escape", Action::Quit),
        ("leader+m", Action::CycleMode),
        ("leader+o", Action::CycleOutput),
        ("leader+s", Action::StretchAll),
        ("leader+Shift+s", Action::Unstretch),
        ("leader+f", Action::ToggleFloating),
        ("leader+Up", Action::ToggleMaximized),
        ("leader+Down", Action::Minimize),
        ("leader+j", Action::FocusNext),
        ("leader+k", Action::FocusPrevious),
        ("leader+Shift+r", Action::Reload),
    ]
    .into_iter()
    .map(|(key, action)| Binding {
        key: key.into(),
        action,
    })
    .collect::<Vec<_>>();
    for workspace in 1..=9 {
        bindings.push(Binding {
            key: format!("leader+{workspace}"),
            action: Action::SwitchWorkspace { workspace },
        });
        bindings.push(Binding {
            key: format!("leader+Shift+{workspace}"),
            action: Action::MoveToWorkspace { workspace },
        });
    }
    bindings
}

/// Resolve `leader` in configured chords; leaders may combine modifiers.
pub(crate) fn resolve_leader(bindings: &mut [Binding], leader: &str) -> Result<(), String> {
    if leader.trim().is_empty()
        || leader
            .split('+')
            .any(|p| p.trim().eq_ignore_ascii_case("leader"))
    {
        return Err("keys.leader must contain explicit modifiers, not leader".into());
    }
    parse_chord(&format!("{leader}+Return")).map_err(|error| format!("keys.leader: {error}"))?;
    for binding in bindings {
        binding.key = binding
            .key
            .split('+')
            .map(|part| {
                if part.trim().eq_ignore_ascii_case("leader") {
                    leader
                } else {
                    part.trim()
                }
            })
            .collect::<Vec<_>>()
            .join("+");
    }
    Ok(())
}
