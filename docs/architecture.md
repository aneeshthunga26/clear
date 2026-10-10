# Architecture

This guide explains code organization and data flow. The
[behavior specifications](../specs/README.md) own component contracts, defaults,
lifecycle rules, failure behavior, and current limitations. Implementation changes
must update the relevant specification in the same change.

## Policy boundary

Keep Smithay imports under `src/platform/smithay/`. The rest of Clear works with
plain IDs and logical rectangles. The adapter translates protocol/input events;
runtime resolves actions, reloads configuration, and invokes Rhai; core owns
desktop state; management computes built-in geometry. Core returns explicit
effects for operations requiring platform IO.

The flow is:

1. A protocol event or physical input arrives at the adapter.
2. The adapter translates it into a desktop command or typed action.
3. `Runtime` resolves the action, reload, or extension call.
4. `Desktop` computes ordered placements for visible regions.
5. The adapter reconciles XDG state, maps scene elements, and synchronizes focus.
6. Rendering and hit-testing consume the same placement/clip descriptions.

This division makes policy testable without a display while keeping protocol
serials, committed buffers, and renderer ownership in the adapter. The
[platform specification](../specs/platform.md) defines the lifecycle and
asynchronous configure contract at that boundary.

## Source map

| Module | Responsibility | Specification |
| --- | --- | --- |
| `src/core/` | Desktop model, IDs, geometry, commands/effects, resize state | [Desktop](../specs/desktop.md), [layouts](../specs/layouts.md) |
| `src/management/` | Pure built-in geometry policies | [Layouts](../specs/layouts.md) |
| `src/input/` | Shortcut parsing and typed actions | [Input](../specs/input.md) |
| `src/config/` | TOML schema and defaults | [Configuration](../specs/configuration.md) |
| `src/runtime/` | Config/reload, script routing, transient overview selection, analytic animation tracks and presentation planner, prepared CPU resources | [Configuration](../specs/configuration.md), [animations](../specs/animations.md), [wallpaper](../specs/wallpaper.md), [decorations](../specs/decorations.md) |
| `src/scripting/` | Bounded Rhai host and validation | [Rhai](../specs/scripting.md) |
| `src/decoration/` | Backend-independent theme descriptions | [Rendering](../specs/rendering.md), [decorations](../specs/decorations.md) |
| `src/shell/` | Toolkit-independent snapshots, commands, optional animation-target store, Unix transport | [Shell IPC](../specs/shell.md) |
| `src/platform/smithay/` | Protocols, input translation, scene, GPU resources, nested backend | [Platform](../specs/platform.md), [rendering](../specs/rendering.md) |
| `src/main.rs` | CLI parsing only | [CLI](../specs/platform.md#supported-backend-and-cli) |

## Workspace presentation

See [desktop ownership and presentation](../specs/desktop.md) for workspace/output
invariants, focus, topology changes, saved geometry, and commands. The
[virtual-output contract](../specs/platform.md#virtual-outputs) describes how the
current backend maps those concepts into a host window.

## Overview

The ID-only runtime session supplies preview selection. The adapter shares one
deterministic layout between hits and rendering, composites small GPU thumbnails
from committed buffers, and owns seat focus while open. It never reconciles
hidden workspaces into the normal scene. See [overview](../specs/overview.md).

## Window state

See [maximize and minimize](../specs/desktop.md#maximize-and-minimize) for policy and
[XDG lifecycle](../specs/platform.md#xdg-window-lifecycle-and-configures) for mapping,
pre-map requests, configures, and committed state.

## Animation foundation

Runtime owns the pure animation clock, pose tracks and presentation planner.
The planner accepts backend-independent snapshots and explicit causes; it returns
sampled poses and retention requests. The adapter's shared window-image composer
now supplies overview images, and its pointer focus wrapper supports inverse
scaling. Optional shell hints supply panel/icon geometry. The nested backend selects
frame sampling opportunities and synchronized swaps, while desktop policy remains
instant. Scene integration and effect triggers remain pending in the
[implementation plan](animations-design.md); [animation](../specs/animations.md)
and [platform timing](../specs/platform.md#nested-frame-timing) define implemented
foundations and their limits.

## Interactive resizing

See [layouts and resizing](../specs/layouts.md) for geometry and persistent sizing,
and [pointer gestures](../specs/input.md#pointer-gestures) for adapter authorization
and gesture translation.

## Wallpapers

See [wallpapers](../specs/wallpaper.md) for selection, placement, preparation,
failure behavior, and cache lifecycle. Runtime owns CPU images; the backend owns
their uploaded textures.

## Server-side decorations

See [decorations](../specs/decorations.md) for negotiation/commit timing, frame
insets, titlebar configuration, controls, resource bounds, and alpha. Its source
references distinguish protocol handling, CPU asset preparation, and GPU caching.

## Rounded window outlines

See [rounded outlines](../specs/rendering.md#rounded-outlines) for the common shape
contract shared by composition and hit-testing.

## Backdrop blur

See [rendering](../specs/rendering.md#backdrop-composition) for scene composition,
filter definitions, output isolation, and retained memory bounds. GPU captures
and validation records belong in [testing](vm-testing.md#backdrop-blur).

## Shell policy and protocol ownership

See [shell rules](../specs/configuration.md#shell-rules),
[launcher geometry](../specs/platform.md#launcher-geometry), and
[layer-shell lifecycle](../specs/platform.md#layer-shell-lifecycle).

## Extending modes

A built-in policy is a pure function over `LayoutContext`. Add its mode in core
and geometry in management; ordinary layout additions should not require Wayland
handler changes. Define the resulting contract in [layouts](../specs/layouts.md)
and document any interaction with saved state and resize sessions.

For user extensions, follow the [Rhai interface](../specs/scripting.md). Runtime
handles calls and fallback while core retains the final membership boundary.

## Optional desktop-shell boundary

The serializable shell model allows UI replacement without moving desktop policy
into a toolkit. [Shell IPC](../specs/shell.md) defines the interface; the
[integration guide](shell-integration.md) explains how to run a companion client.

## Implementation boundaries still to grow

See [current platform limitations](../specs/platform.md#current-limitations) for
unsupported functionality. Future backends must preserve the core/runtime boundary
while adapting framebuffer coordinates and resource ownership. Event-driven damage
scheduling, texture reuse for rounded bodies, extensible per-mode state, and a
richer decoration scene API remain design work rather than supported contracts.
