# Clear

A modular, early-stage Wayland window manager built with Smithay and Rust 2024.
Desktop policy is independent of Wayland: workspaces, monitor groups, focus, and
layout modes operate on plain Rust IDs and logical rectangles. Rhai provides
custom layouts and actions without exposing compositor objects.

**Current backend:** nested winit, running inside an existing Wayland/X11 desktop.
Two virtual monitors are advertised inside one host window by default. This is
not yet a native DRM/KMS desktop session or a replacement login-session compositor.

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

Clear sets `WAYLAND_DISPLAY` explicitly for children; it does not change the
process-wide environment. `--command` takes an executable and arguments, not a
shell command string, and must be the last CLI option. Without it, Clear starts
with an empty desktop. Closing the outer window exits Clear.

For bounded tests:

```sh
cargo run -- --socket clear-test --exit-after 15 --capture /tmp/clear-frame.ppm --command foot
```

`--capture` saves Clear's rendered framebuffer as PPM near the test deadline, or
after three seconds without `--exit-after`. It overwrites the requested file.

## Desktop behavior

- Nine initial workspaces, with persistent focus and window state.
- Each output initially presents a different workspace independently.
- **Stretch** joins all outputs into the focused workspace. Other workspaces
  remain intact but hidden. Switching workspaces then switches the whole group.
- **Unstretch** keeps the active workspace on the focused output and assigns
  unused workspaces to the other outputs.
- Selecting a workspace already displayed elsewhere swaps group presentations;
  windows are never duplicated.
- Each workspace has a default mode. Each output can override that mode **within
  that workspace**. Changing the workspace default retains explicit overrides.
- Built-ins: floating, scrolling columns, master/stack, equal columns, equal
  rows, grid, Fibonacci-style spiral, and monocle. The spiral also accepts
  `fibonacci` and `dwindle` as mode names.
- Saved floating rectangles and scrolling state survive mode changes.
- Floating exceptions are available within tiled/scrolling regions. Dragging a
  tiled window detaches it. Floating movement transfers windows between outputs
  when the pointer crosses their boundary. Float positions are unbounded: windows
  can straddle outputs or remain offscreen; resizing outputs or changing panel
  reservations does not clamp or overwrite their saved rectangles. Rendering and
  input are clipped to the workspace's output group, so floats cannot leak into
  an independently displayed workspace.
- Launcher-role XDG windows are centered in their home output's usable area at
  their client-committed size, above normal windows and outside tiled layouts.
  They retain workspace ownership and saved floating geometry.
- Layer-shell panels reserve usable space per output. Reservation changes and
  panel removal recompute tiles and launcher centers without moving saved floats.

### Default shortcuts

`leader` is Super by default; set `[keys].leader = "Ctrl+Alt"` to change it,
including the default bindings. Host desktops may intercept Super shortcuts
before a nested compositor sees them.

| Binding                   | Action                                   |
| ------------------------- | ---------------------------------------- |
| leader+Return             | Open foot                                |
| leader+1…9                | Switch workspace                         |
| leader+Shift+1…9          | Move focused window to a workspace       |
| leader+s / leader+Shift+s | Stretch / unstretch                      |
| leader+o                  | Focus next output                        |
| leader+m                  | Cycle the focused output's mode override |
| leader+f                  | Toggle a window's floating exception     |
| leader+j / leader+k       | Next / previous window                   |
| Alt+Tab                   | Select a window; release Alt to focus it |
| leader+q                  | Close focused window                     |
| leader+Shift+r            | Reload config and scripts                |
| leader+Escape             | Quit                                     |

Pointer gestures currently use **Super**, independently of the keyboard leader:
Super+left drag moves, Super+right drag resizes, and Super+wheel scrolls a scrolling
region. Client-side titlebar move/resize requests are also supported.

### VM demo

`examples/vm.toml` uses Ctrl+Alt as leader and adds:

- leader+Shift+Left/Right: move a window to virtual monitor 1/2.
- leader+t/g/h: set the workspace default to master/stack, floating, or scrolling.
- leader+BackSpace: remove the focused monitor's override.
- leader+Left/Right: scroll the focused region.

Open several terminals, press leader+s, then leader+Shift+Right to move one into
the second region. Workspace 1 uses master/stack on the left and scrolling on the
right. Use leader+o and leader+m to change one region independently.

## Configuration and Rhai

Clear reads `$XDG_CONFIG_HOME/clear/config.toml`, falling back to
`~/.config/clear/config.toml`. `--config PATH` selects an explicit file.
Missing/invalid startup config uses defaults. A failed reload retains the last
good configuration and script host. Output topology changes require restarting.

See [examples/config.toml](examples/config.toml) and
[examples/columns.rhai](examples/columns.rhai). Supplied arrays replace the default
configuration declarations; workspace declarations update persistent IDs rather
than deleting existing workspaces or their windows. Relative script paths resolve
against the configuration file's directory.

### Shell rules

The defaults recognize the exact XDG app ID `wofi` as a launcher and override the
exact layer-shell namespace `waybar` to the `top` layer:

```toml
[shell]
launcher_app_ids = ["wofi"]

[[shell.panels]]
namespace = "waybar"
layer = "top"
```

Matching is case-sensitive. Each supplied array replaces its own defaults; use
`launcher_app_ids = []` or `panels = []` inside `[shell]` to disable that policy.
IDs and namespaces must be nonempty and unique within their respective arrays.
Each explicit panel rule requires `namespace` and `layer`; valid layers are
`background`, `bottom`, `top`, and `overlay`. A panel rule overrides **only the
layer**, not client anchors, margins, sizing, or exclusive zones. Unmatched
layer-shell clients retain their requested layer; `top`/`overlay` panels stay
above normal windows.

Launcher placement follows the client's committed size, even if larger than the
usable area; it is not stretched into a tile or clamped to fit. Rules are applied
on mapping, late app-ID updates, and successful config reloads, including windows
on hidden workspaces. Layer-shell launchers remain protocol-managed surfaces,
not XDG windows classified by app ID. Mapped exclusive top/overlay layers take
keyboard focus, with overlay taking precedence; unmapping restores application
focus. On-demand layers take focus when clicked. Panel clicks do not focus or
start dragging the application underneath, and panel popups render above their
parent layer.

These rules describe placement, **not process management**: there is no shell
config autostart or launcher/panel toggle support. Start clients explicitly, for
example via an existing `spawn` binding or from a terminal inside Clear.

### Rhai layouts and actions

Select a script function with `mode = "script:columns"`. Rhai receives maps of
plain metadata and returns placements keyed by window ID. Layouts are currently
bounded to their output region. Script actions return the same typed action maps
used in shortcut configuration.

Scripts are compiled once, cannot import modules or access native IO, and have
operation/depth/collection limits. Failed functions are disabled until reload,
with master/stack fallback for layouts. These are in-process safeguards, **not a
hostile-code security boundary**. Top-level script statements are not executed;
configuration and state should be passed explicitly rather than kept in globals.

Theme options currently cover background, active/inactive border colors, and
border width. Application titlebars in screenshots are client-side decorations,
not a custom server-side widget toolkit.

## Optional shell / Quickshell

Clear runs without a shell and has no Qt or Quickshell build dependency. Panels
use standard layer-shell; a versioned local IPC interface exposes Clear's
workspaces, output groups, window metadata, focus, and management modes to any
shell implementation.

An optional example panel is provided in `examples/quickshell/`. With Quickshell
installed separately, run:

```sh
cargo run --locked -- --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

It provides workspace buttons, grouped applications with window cards and pins,
per-output mode icons and controls, a system tray, notifications, and a searchable
desktop-entry launcher. Alt+Tab selection is managed by Clear and shown by the
shell until Alt is released. Clear exports `WAYLAND_DISPLAY` and `CLEAR_SOCKET` to its
children. `--no-shell-ipc` disables the bridge; failure to bind it never prevents
Clear starting. The example is not autostarted and can be replaced by a future
native shell without changing desktop policy.

Window cards contain titles and workspaces; live thumbnails need a toplevel
capture protocol that Clear does not yet expose. See [docs/shell-integration.md](docs/shell-integration.md)
for the protocol, security/limits, and optional example/test instructions.

## Source map

```text
src/core/                Desktop model, IDs, geometry, commands and effects
src/management/          Built-in layout policies
src/input/               Platform-independent bindings and typed actions
src/config/              TOML parsing and defaults
src/scripting/           Bounded Rhai host and data validation
src/decoration/          Static theme descriptions
src/runtime/             Config/reload, script routing and policy orchestration
src/shell/               Toolkit-independent shell model, commands and local IPC
src/platform/smithay/    Protocols, input translation, scene and nested backend
src/main.rs              CLI only
```

`core`, `management`, `runtime`, configuration, and scripts do not import Smithay.
The adapter owns all surfaces, seats, serials, rendering and asynchronous configure
handling. See [docs/architecture.md](docs/architecture.md).

## Validation and VM testing

```sh
cargo fmt --check
cargo check --locked
cargo test --locked
cargo build --locked
python3 scripts/vm-smoke.py
python3 scripts/vm-smoke.py --script examples/columns.rhai --artifacts target/vm-rhai
python3 scripts/vm-layer-smoke.py --binary target/debug/clear
python3 scripts/vm-shell-smoke.py --binary target/debug/clear
```

The smoke runner requires KDE's `kwin_wayland`, `dbus-run-session`, and `foot`.
It starts a private virtual display, runs real Wayland clients, validates a
nonblank GPU framebuffer capture, and terminates only its own processes. It can
run over SSH without sudo or a graphical login. Logs and captures stay in
`target/vm-smoke/`. It does not automate physical keyboard or mouse gestures.

See [docs/vm-testing.md](docs/vm-testing.md) for portable isolated-test commands.
Machine-specific setup and access notes belong in the git-ignored `.agents/`
directory, not in tracked documentation.

## Current limitations

This is a runnable foundation, not a production compositor. Native DRM/KMS,
physical monitor hotplug, XWayland, fractional scaling, IME, fullscreen/maximize
policy, sophisticated decorations, and animation remain future work. Popup and
layer-shell support remains intentionally small; rich client cursor rendering
still needs integration. Monitor-group editing currently
offers stretch-all and split, not arbitrary monitor subsets.

Hidden workspaces and clipped scrolling regions are excluded from hit testing.
Clients resize asynchronously; old tiled buffers are clipped while waiting for
new content rather than stalling the compositor.
