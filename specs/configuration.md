# Configuration and runtime

## Loading and defaults

`--config PATH` selects an explicit TOML file. Otherwise Clear uses an absolute
`XDG_CONFIG_HOME` with `clear/config.toml`, or an absolute `HOME` with
`.config/clear/config.toml`. Without either base it uses defaults. Missing,
unreadable, or invalid startup configuration MUST fall back to the complete safe
default configuration rather than partly applying invalid declarations.

| Setting                  | Default                                         | Validation/meaning                                                  |
| ------------------------ | ----------------------------------------------- | ------------------------------------------------------------------- |
| `outputs`                | `virtual-1`, `virtual-2`, each 800×600          | 1–16 entries; unique nonblank names; integer width/height 1–32768   |
| `workspaces`             | IDs 1–9, names `1`–`9`, mode `columns`          | 1–256 declarations; unique positive IDs; nonblank names             |
| Workspace `mode`         | `columns`                                       | Nonblank mode string, at most 256 bytes                             |
| Workspace `output_modes` | Empty                                           | Configured output names mapped to mode strings                      |
| `gaps`                   | 8                                               | Integer logical pixels, 0–4096                                      |
| `script`                 | Absent                                          | Optional Rhai path; empty path disables extensions                  |
| `keys.leader`            | `Super`                                         | Explicit modifier combination; see [input](input.md)                |
| `bindings`               | [Default shortcuts](input.md#default-shortcuts) | Flat `key`/`action` maps with action-specific fields                |
| `theme`                  | [Rendering defaults](rendering.md#theme)        | Includes [titlebar](decorations.md#titlebar-configuration) settings |
| `wallpaper`              | No image, `fill`                                | [Wallpaper selection](wallpaper.md#selection)                       |
| `shell`                  | `wofi` launcher; `waybar` top-layer rule        | [Shell rules](#shell-rules)                                         |
| `overview.preview_workspace_on_hover` | `false` | Strict boolean; [overview interaction](overview.md#layout-and-input) |
| `animations` | Engine disabled, speed 1, refresh-rate sampling | Strict effect tables; [animation engine](animations.md#configuration); visual effects are not yet connected |

Unknown fields and malformed typed values MUST be rejected. Supplied output,
workspace, and binding arrays replace their default declarations. Empty bindings
disable shortcuts; empty output/workspace declarations are invalid. Workspace
declarations configure persistent IDs rather than deleting previously existing
workspaces or their windows.
The shipped liquid-glass example declares its own bindings, including `Alt+Tab`
for the switcher, because default shortcuts do not carry into a supplied array.

Mode strings starting with the literal `script:` prefix are checked for an ASCII
function identifier of at most 128 bytes. Other nonempty bounded mode names can
pass configuration parsing: runtime uses master/stack for an unknown workspace
default; an unparsed configured output override becomes absent. Thus configuration
validation MUST NOT be described as rejecting every unknown built-in mode name.
The available modes and core aliases are defined in [layouts](layouts.md).

Relative script, wallpaper, and titlebar control paths resolve against the config
file's directory when loading a file. Pure `Config::from_source` leaves them
relative. Paths do not expand `~` or environment variables. Wallpaper/control
paths MUST be nonblank and NUL-free. Resource contents are prepared separately
from TOML parsing.

## Monitor examples

[`single-monitor.toml`](../examples/single-monitor.toml) explicitly replaces the
output defaults with one `virtual-1` output, initially 1280×720, and retains three
workspaces without second-output overrides or multi-output shortcuts.
[`dual-virtual-monitors.toml`](../examples/dual-virtual-monitors.toml) retains the
two default virtual outputs and the multi-output controls. Both remain nested
configurations: output geometry follows the [host framebuffer](platform.md#virtual-outputs),
not physical monitor modes. Switching between them requires restart under the
[reload contract](#reload).

Evidence: `monitor_examples_select_their_intended_topologies` in
[configuration tests](../tests/extensions.rs).

## Startup resources

With otherwise valid configuration, a script load/compile failure leaves scripted
layouts using their built-in fallback. Wallpaper preparation failure warns and
uses solid theme backgrounds. Explicit titlebar control preparation failure warns
and uses built-in controls. These startup resource failures do not discard other
valid settings. Missing app icons use the generic glyph and are not fatal resource
failures. See the [Rhai](scripting.md), [wallpaper](wallpaper.md), and
[decoration](decorations.md) contracts for their bounds.

## Reload

Reload MUST prepare a complete candidate configuration, bindings, script host,
wallpaper set, and titlebar resources before publishing any of them. Parse,
validation, script, or explicit resource failures MUST retain the entire last-good
configuration and prepared resource set. Reload rereads resources even when the
path is unchanged. A reload without an available config path fails.

The configured output list must be identical to the running configuration:
changes to names, dimensions, order, or count require restart. Runtime does not
hot-apply topology declarations.

Successful reload updates gaps, declared workspace names/default modes and their
configured output overrides, bindings, theme, resources, and shell rules. It
preserves desktop windows and persistent workspace ownership. It clears disabled
script-function state and reclassifies all managed windows, including hidden ones.
The adapter reapplies panel rules, insets/configures, and scene state. A failed
reload MUST NOT partially reclassify windows or change those resources.
Animation clock/track reload behavior follows [animations](animations.md#reload).

## Shell rules

`shell.launcher_app_ids` defaults to `["wofi"]`. `shell.panels` defaults to one
entry with `namespace = "waybar"` and `layer = "top"`. Each supplied array replaces
only its own defaults; an empty array disables that set of rules. Names MUST be
nonblank and unique within each array. Matching is exact and case-sensitive.
An explicit panel entry requires both namespace and layer; accepted layers are
`background`, `bottom`, `top`, and `overlay`.

Launcher classification occurs on mapping, late app-ID changes, and successful
reload. It changes role without changing workspace ownership, order, or saved
floating geometry. See [desktop state](desktop.md#maximize-and-minimize) for the
state flags cleared by launcher classification and
[platform](platform.md#launcher-geometry) for placement.

Panel rules override only the layer; client anchors, margins, sizes, and exclusive
zones remain client-owned. Unmatched namespaces retain their requested layer.
These declarations MUST NOT autostart clients or provide launcher/panel toggle
actions. Layer-shell launchers remain protocol surfaces, distinct from classified
XDG windows.

## Runtime action routing

The `toggle_overview` action queues a request for adapter input authorization;
its session and lifecycle follow [overview](overview.md).

Runtime translates validated typed actions into desktop commands or explicit
platform effects. It owns reload and script calls, not Wayland objects. Script
action expansion preserves returned order and is bounded to 128 dispatched
actions per invocation, including nested calls. Ordinary action dispatch clears
the transient switcher. Only the physical input adapter starts an Alt-held switch
gesture; dispatching `AltTab` through ordinary runtime actions does not start one.

## Implementation and evidence

- [Config parsing](../src/config/mod.rs), [shell rules](../src/config/shell.rs),
  [runtime](../src/runtime/mod.rs), [example configuration](../examples/config.toml).
- [Extension/config tests](../tests/extensions.rs): defaults, malformed values,
  shortcut validation, XDG path selection, and startup fallback.
- [Runtime tests](../tests/runtime.rs): atomic parse/script/topology rejection,
  retained windows, launcher reclassification, and explicit effects.
- [Wallpaper](../tests/wallpaper.rs), [titlebar](../tests/titlebar_theme.rs),
  [rounded](../tests/rounded.rs), and [blur](../tests/blur.rs) tests cover their
  schema/resource reload cases; they do not establish rendered GPU behavior.
