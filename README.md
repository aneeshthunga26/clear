# Clear

A modular, early-stage Wayland window manager built with Smithay and Rust 2024.
Desktop policy operates on plain Rust IDs and logical rectangles; Rhai provides
custom layouts and actions without exposing compositor objects.

**Current backend:** nested winit inside an existing Wayland/X11 desktop. See the
[platform specification](specs/platform.md) for supported behavior and current
limitations.

## Run

Requirements: Rust/Cargo, a graphical session, Mesa/EGL, Wayland, xkbcommon, and a
Wayland client such as `foot`. On Arch Linux, the usual build prerequisites are
`base-devel rust pkgconf wayland libxkbcommon mesa libglvnd`.

```sh
cargo run -- --command foot
```

With VM-friendly shortcuts and a mixed-mode workspace:

```sh
cargo run -- --config examples/vm.toml --command foot
```

For a bounded run and framebuffer capture:

```sh
cargo run -- --socket clear-test --exit-after 15 --capture /tmp/clear-frame.ppm --command foot
```

See the [CLI contract](specs/platform.md#supported-backend-and-cli) for option
validation, command arguments, child environment, capture timing, and shutdown.

## Desktop behavior

The [specification index](specs/README.md) is the canonical behavior reference.
Use these component specs for workspace groups, layouts, floating geometry,
focus, panel interactions, and window lifecycle:

- [Desktop state and commands](specs/desktop.md)
- [Layouts, scrolling, and resizing](specs/layouts.md)
- [Surface lifecycle, launchers, and panels](specs/platform.md)

### Default shortcuts

See [input and default shortcuts](specs/input.md#default-shortcuts) for the binding
table, leader configuration, pointer gestures, and Alt-Tab. Host desktops may
intercept shortcuts before a nested compositor receives them.

### Maximize and minimize

See [window-state behavior](specs/desktop.md#maximize-and-minimize) for maximize,
restore, minimize, saved-state preservation, and focus semantics.

### VM demo

`examples/vm.toml` provides Ctrl+Alt bindings and a mixed-mode workspace. Follow
the [interactive demo](docs/vm-testing.md#interactive-run) for the stretch/split,
per-output mode, resize, and restore sequence.

## Configuration and Rhai

Start from [examples/config.toml](examples/config.toml) or
[examples/vm.toml](examples/vm.toml). The
[configuration specification](specs/configuration.md) defines paths, schema,
defaults, validation, startup fallback, and reload behavior.

### Wallpapers

For example, after creating the image files:

```toml
[wallpaper]
path = "wallpapers/landscape.png"
mode = "fill"

[wallpaper.outputs."virtual-2"]
path = "wallpapers/portrait.jpg"
mode = "fit"
```

See [wallpapers](specs/wallpaper.md) for inheritance, scaling modes, placement,
resource limits, and failure handling.

### Shell rules

See [shell rules](specs/configuration.md#shell-rules) for launcher app-ID and panel
namespace matching. The shipped TOML examples show their syntax.

### Rhai layouts and actions

Use [examples/columns.rhai](examples/columns.rhai) as a starting point. The
[Rhai specification](specs/scripting.md) defines context/result maps, action
decoding, execution limits, validation, and fallback.

### Window outlines

```toml
[theme]
corner_radius = 12
```

See [theme and rounded outlines](specs/rendering.md) for accepted forms, geometry,
alpha, and shared input/render coverage. Merge snippets into an existing theme
table rather than declaring the table twice.

### Backdrop blur

```toml
[theme]
blur_method = "kawase"
blur_radius = 2
blur_passes = 3
```

The [rendering specification](specs/rendering.md#backdrop-composition) defines
filter units, composition, validation, and resource bounds. To see blur through
an application, configure transparency in that application; for example, set
Alacritty's `[window] opacity = 0.85` in its own config.

### Server-side titlebars

See [decorations](specs/decorations.md) for negotiation, titlebar configuration,
controls, SVGs, app icons, alpha, insets, and cache limits. The
[configuration example](examples/config.toml) includes titlebar fields and
commented optional control paths.

## Optional shell / Quickshell

With Quickshell installed separately:

```sh
cargo run --locked -- --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

See the [shell integration guide](docs/shell-integration.md) for setup and private
D-Bus sessions, the [example guide](examples/quickshell/README.md) for its UI, and
[shell IPC](specs/shell.md) for the compositor interface.

## Source map

See [architecture](docs/architecture.md) for module ownership, data flow, and
extension points. Component contracts live in [specs/](specs/README.md).

## Validation and VM testing

See [isolated compositor testing](docs/vm-testing.md) for build/check commands,
bounded smoke runs, dependencies, artifact locations, historical validation
records, and coverage limitations. Machine-specific setup belongs in the ignored
`.agents/` directory.

## Current limitations

See [platform limitations](specs/platform.md#current-limitations) and component
specs for the current supported scope. This remains an early-stage compositor.
