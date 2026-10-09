# Clear behavior specifications

These specifications are the canonical reference for Clear's implemented behavior.
They describe the current nested compositor, not a proposed native desktop session.
New or changed implementation behavior must be documented here in the same change.

MUST and MUST NOT identify required behavior. MAY identifies permitted behavior;
it does not promise an unimplemented feature. Unless marked as a limitation,
defaults, tables, and validation rules are part of the contract. A specification
change and its implementation must agree; report discrepancies rather than
treating either an outdated document or an accidental implementation detail as
permission to change behavior silently.

## Components

| Specification | Contract |
| --- | --- |
| [Desktop state](desktop.md) | Workspace ownership, output groups, focus, floating state, maximize/minimize |
| [Layouts and resizing](layouts.md) | Built-in geometry, scrolling, persistent proportions, resize sessions |
| [Desktop overview](overview.md) | Compositor-owned preview, navigation, input ownership and bounded live cards |
| [Input and actions](input.md) | Shortcut parsing, action schema, default bindings, gestures, Alt-Tab |
| [Animation foundation](animations.md) | Strict preferences, clock and bounded pose tracks; visual effects remain pending |
| [Configuration and runtime](configuration.md) | TOML schema, defaults, startup fallback, atomic reload, orchestration |
| [Rhai extensions](scripting.md) | Layout/action interfaces, validation, execution limits, fallback |
| [Platform and surfaces](platform.md) | CLI, nested outputs, mapping/configures, clipping, layer-shell lifecycle |
| [Shell IPC](shell.md) | Optional shell boundary, v1 messages, snapshots, validation, transport limits |
| [Decorations](decorations.md) | XDG/KDE negotiation, titlebar style, controls, SVG and app-icon resources |
| [Rendering](rendering.md) | Theme, rounded outlines, alpha, backdrop filters, liquid glass, renderer resource bounds |
| [Wallpapers](wallpaper.md) | Image selection, scaling, preparation, reload and failure behavior |

## Reading and evidence

Each specification links to implementation and relevant tests. These references
identify where behavior is defined and checked; they are not a claim that tests
were run when editing the specification. CPU geometry/schema tests, protocol/GPU
fixtures, and physical input verification establish different things.

The [architecture guide](../docs/architecture.md) explains module ownership and
data flow. The [shell integration guide](../docs/shell-integration.md) explains
how to run a companion client. The [testing guide](../docs/vm-testing.md) retains
portable commands, dependencies, coverage limitations, and historical validation
records. The [project README](../README.md) is the starting point for setup.
