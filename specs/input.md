# Input and typed actions

## Shortcut parsing

A shortcut consists of zero or more modifiers followed by one key, separated by
`+`. Parsing trims parts and normalizes names case-insensitively. Modifier aliases
are Ctrl/Control, Alt/Mod1, Shift, and Super/Logo/Win/Mod4. Configuration expands
`leader` to `keys.leader` before parsing; a leader must be a nonempty explicit
modifier combination and cannot refer to itself.

Repeated modifiers, unknown keys/modifiers, malformed chords, invalid actions,
and duplicate normalized chords MUST be rejected. Matching requires the exact
Ctrl/Alt/Shift/Logo combination. The adapter supplies unshifted key names so
Shift+1 resolves against `1`, not `!`.

Supported keys include ASCII letters/digits, F1–F35, navigation/editing keys,
Return/Enter, Escape/Esc, Tab/ISO_Left_Tab, Space/Spacebar, Print, Pause, Menu,
named punctuation, lock keys, and the supported XF86 audio/brightness keys.
Punctuation aliases and the complete accepted vocabulary are implemented by
`normalize_key` in [input](../src/input/mod.rs); arbitrary keysyms are not accepted.
Use the key name `plus` for a literal plus key in a `+`-separated chord.

## Action schema

Bindings use `key`, a snake-case `action` tag, and only that action's parameters.
Extra parameters MUST be rejected even for parameterless actions.

| Action tags | Additional fields |
| --- | --- |
| `alt_tab`, `focus_next`, `focus_previous`, `cycle_output` | None |
| `switch_workspace`, `move_to_workspace` | `workspace`: positive `u64` |
| `move_to_output` | `output`: `u64`; runtime target must exist |
| `set_workspace_mode`, `set_output_mode` | `mode`: nonblank string, at most 256 bytes |
| `clear_output_mode`, `cycle_mode`, `stretch_all`, `unstretch` | None |
| `toggle_floating`, `toggle_maximized`, `minimize` | None |
| `scroll` | `amount`: `i32` logical pixels |
| `close_focused`, `quit`, `reload` | None |
| `spawn` | `command`: executable and argument array |
| `script` | `name`: ASCII function identifier, at most 128 bytes |

Spawn permits at most 256 arguments, each at most 16384 bytes and NUL-free; the
executable must be nonblank. The adapter launches the argument vector without
shell interpolation. Mode actions with unrecognized names are ignored by runtime.
Actions requesting nonexistent desktop targets do not mutate policy. See
[desktop](desktop.md) for command semantics and [runtime](configuration.md#runtime-action-routing)
for effect execution and script expansion.

## Default shortcuts

A configured binding array replaces this entire list. The default leader is
Super; pointer gestures below do not use the configurable leader.

| Binding | Action |
| --- | --- |
| leader+Return | Spawn `foot` |
| leader+1…9 | Switch workspace |
| leader+Shift+1…9 | Move focused window to workspace |
| leader+s / leader+Shift+s | Stretch all / unstretch |
| leader+o | Cycle output |
| leader+m | Cycle output's workspace-specific mode override |
| leader+f | Toggle floating exception |
| leader+Up | Toggle maximize |
| leader+Down | Minimize |
| leader+j / leader+k | Focus next / previous |
| Alt+Tab | Advance switcher |
| leader+q | Request close |
| leader+Shift+r | Reload |
| leader+Escape | Quit |

Intercepted shortcut presses MUST suppress the matching release by physical key
code, even if modifiers changed before release. Repeated intercepted presses are
also suppressed while that code is held. Unhandled events are forwarded through
the seat's normal keyboard path.

## Alt-Tab

The first press snapshots normal windows in active-workspace order, including
minimized windows and windows on other outputs of the stretched group, excluding
launchers. It selects the next candidate after focus, or the first if focus is not
a candidate. Further presses cycle that candidate order without changing focus.

Physical Alt release commits the selection through explicit desktop focus, which
restores a minimized candidate. Escape cancels without changing focus or minimized
state. A shell only presents the transient
[switcher snapshot](shell.md#snapshot-fields); the compositor owns the gesture.

## Pointer gestures

Super+left drag moves, Super+right drag resizes, and Super+wheel scrolls a scrolling
region, independently of `keys.leader`. A move detaches a tiled window into a
floating exception. A resize MUST retain tiled membership and use the
[resize-session rules](layouts.md#resize-sessions). Floating movement can transfer
the window to the output crossed by the pointer.

Super+right selects a corner by the pointer's quadrant of the frame. Only supported
internal tile boundaries move. Launchers and maximized windows reject ordinary
move/resize gestures. Client XDG move/resize requests require a valid pointer grab
serial and a grab origin owned by the requesting client; merely sending a request
does not authorize it.

Hit-testing follows [scene clips and surface priority](platform.md#scene-and-hit-testing).
A layer hit cannot initiate a drag of an application underneath. Server-side
titlebar controls use compositor-owned hits and matching-release suppression;
their activation and movement rules are in [decorations](decorations.md#controls-and-input).
Super gestures take precedence over those controls.

## Implementation and evidence

- [Typed actions/parser/defaults](../src/input/mod.rs),
  [runtime switcher](../src/runtime/mod.rs),
  [physical input adapter](../src/platform/smithay/input.rs),
  [native request authorization](../src/platform/smithay/protocols.rs).
- [Extension tests](../tests/extensions.rs): normalized keys, exact modifiers,
  collisions, strict flat bindings, and action validation.
- [Shell IPC tests](../tests/shell_ipc.rs) and [window-state tests](../tests/window_state.rs):
  switcher commit/cancel and restoring minimized candidates.
- Inline adapter tests cover keysym translation, pointer quadrants, titlebar
  threshold, and control precedence. These are not physical gesture tests.
