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
- Built-ins: floating, scrolling columns, master/stack, columns, rows, grid,
  Fibonacci-style spiral, and monocle. Columns and rows start equally sized. The spiral also accepts
  `fibonacci` and `dwindle` as mode names.
- Saved floating rectangles, scrolling state, and per-mode tile proportions survive
  mode changes within the running session.
- Floating exceptions are available within tiled/scrolling regions. Dragging a
  tiled window with the move gesture detaches it; resizing keeps it tiled. Floating movement transfers windows between outputs
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
| leader+Up                 | Toggle maximize / restore                |
| leader+Down               | Minimize focused window                  |
| leader+j / leader+k       | Next / previous window                   |
| Alt+Tab                   | Select a window; release Alt to focus it |
| leader+q                  | Close focused window                     |
| leader+Shift+r            | Reload config and scripts                |
| leader+Escape             | Quit                                     |

Pointer gestures currently use **Super**, independently of the keyboard leader:
Super+left drag moves, Super+right drag resizes, and Super+wheel scrolls a scrolling
region. Client-side titlebar move/resize requests are also supported.

Super+right drag selects the nearest corner by pointer quadrant. In tiled modes,
only internal, supported boundaries move; outer edges are ignored.

| Mode                           | Resize behavior                                              |
| ------------------------------ | ------------------------------------------------------------ |
| Floating / floating exceptions | Resize selected edges, keeping opposite edges fixed          |
| Master/stack                   | Master/stack divider width and adjacent stack-window heights |
| Columns                        | Adjacent column widths                                       |
| Rows                           | Adjacent row heights                                         |
| Grid                           | Adjacent cell widths within a row; shared row heights        |
| Scrolling                      | Individual window width, up to the viewport width            |
| Spiral, monocle, Rhai tiles    | No tiled resizing                                            |

Floating exceptions remain resizable in every mode, including spiral. Launcher
windows remain client-sized. Tile proportions are retained per workspace, output,
and mode without overwriting saved floating geometry. Changes to layout membership,
mode, output grouping, gaps, or usable area invalidate an active resize gesture.

### Maximize and minimize

Maximize fills a window's **home output's usable area**, keeping the panel visible;
it does not span a stretched output group or change the workspace mode. Restore
returns to the saved floating rectangle or current tiled layout, preserving its
proportions. Maximized windows follow panel reservations and output size changes.
Restore before moving or resizing a maximized window.

Minimize hides a normal window without closing or unmapping the client. It stops
occupying a tile and is skipped by ordinary next/previous focus shortcuts. Select
it with **Alt+Tab** or click its dock entry/window card to restore and focus it,
including when every window is minimized. Its maximized state is retained.
Workspace switches alone do not restore minimized windows. Launcher-role windows
remain client-sized and do not accept these operations.

Client titlebar maximize, restore, and minimize requests are supported, including
maximize requested before first mapping. Quickshell's app hover cards also offer
minimize/restore and maximize/unmaximize buttons. Clear also draws these controls
for clients that negotiate [server-side titlebars](#server-side-titlebars).

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

### Wallpapers

Wallpapers are rendered by Clear itself; Quickshell or another wallpaper process
is not required. Add PNG or JPEG images to your configuration:

```toml
[wallpaper]
path = "wallpapers/landscape.png"
mode = "fill"

[wallpaper.outputs."virtual-2"]
path = "wallpapers/portrait.jpg"
mode = "fit"
```

Relative paths resolve against the config file's directory; absolute paths work
as well. `~` and environment variables are not expanded. Output overrides use the
configured output name and inherit omitted path/mode values from `[wallpaper]`.
With no image selected, Clear uses `[theme].background`.

- `fill` (default): preserve aspect ratio and crop to cover the output.
- `fit`: preserve aspect ratio and show the whole image with background-colored bars.
- `stretch`: scale to cover the output, allowing distortion.
- `center`: center at native pixel size, clipping any excess.

Each image covers its output's full rectangle, including panel-reserved space,
and sits behind all layer-shell surfaces. Stretching a workspace does not stretch
one wallpaper across multiple outputs. Reload with leader+Shift+r to change images
or reread an edited image at the same path. Failed resource reloads keep the entire
last-good configuration and wallpaper set; startup image failures warn and use
the solid theme background. GPU upload failures fall back until the next reload.
Images are limited to 8192 pixels per dimension, 16 MiB encoded, 64 MiB decoded
per image, and 128 MiB aggregate RGBA data.

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

### Window outlines

Theme options cover background, active/inactive border colors, border width, and
rounded corners:

```toml
[theme]
corner_radius = 12           # All corners (also accepts [12])
# corner_radius = [16, 4]    # Top corners, bottom corners
# corner_radius = [16, 12, 4, 0] # Top-left, top-right, bottom-right, bottom-left
```

Radii are logical pixels, including fractional values, in `0..=256`. The default
is `0` (square); `examples/vm.toml` enables `12`. Other list lengths, negative
values, and nonfinite values are rejected. Changes apply on config reload
(**Ctrl+Alt+Shift+R** in the VM example); a failed reload keeps the previous theme.

Radii describe the outer border outline. The content's inner radii subtract the
border thickness, and oversized radii scale proportionally to fit the window.
Content is clipped to the same rounded shape as pointer hit-testing, allowing
windows or wallpaper underneath to show through the cut-outs. Layer-shell panels
and popups keep their own shapes. Server-side titlebars share the window's outline;
there are no extra rounded corners between the titlebar and client content.

### Backdrop blur

```toml
[theme]
blur_method = "kawase"
blur_radius = 2
blur_passes = 3
```

These settings apply globally to transparent windows, popups, and all layer-shell
categories (background, bottom, top, and overlay). They blur **what is behind** the
surface, not its text or controls. `examples/vm.toml` uses the Dual Kawase settings
above; blur is disabled in the default theme.

- `blur_method`: `"gaussian"` (default) or `"kawase"` (Dual Kawase). Existing
  radius-only configurations retain Gaussian filtering.
- `blur_radius`: a finite scalar in `0..=32`, including fractions; default `0`
  disables either method. For Gaussian this is kernel support in logical pixels
  (try `12`). For Kawase it is the sample offset in each source pyramid level's
  texels, **not** a Gaussian-equivalent radius.
- `blur_passes`: an integer in `1..=6`, default `3`. Kawase downsamples through
  that many ceil-half levels, then upsamples through the same levels in reverse,
  stopping early if both dimensions reach `1×1`. Gaussian always uses two filtering
  passes and ignores this setting, but its range is always validated, even when
  blur is disabled.

Reload with **Ctrl+Alt+Shift+R** in the VM example; invalid reloads retain the last
good theme. Neither method adds a glass treatment (tint, noise, or saturation effects).

Clear preserves client opacity: an opaque app will not suddenly become transparent.
For Alacritty, use an opacity below `1.0` in its own config (for example,
`[window] opacity = 0.85`). The optional Quickshell panel and popup backgrounds
are approximately 90% opaque; their text and controls stay opaque.

Window blur stays inside the original rounded outline and visible workspace clips.
For client-shaped panels/popups without explicit blur regions, blur is weighted by
surface alpha to preserve fully transparent holes and shadows. Wallpapers remain
unfiltered scene content, visible directly where no translucent surface covers them.
Blur uses extra GPU passes and four framebuffer-sized scratch textures. Kawase also
retains a bounded viewport-pyramid cache; see [architecture](docs/architecture.md#backdrop-blur)
for memory bounds. Reduce Gaussian radius or Kawase depth (`blur_passes`) to reduce
filtering work, or set the radius to zero to bypass the effect. Disabling blur does
not immediately free already allocated textures.

### Server-side titlebars

Clear defaults to **server-side decorations** when an app negotiates decorations
through `xdg-decoration` or the legacy KDE protocol. An explicit client-side request
is honored: Clear then draws no titlebar or control buttons. Resetting an XDG
preference returns to server-side mode. Apps that never negotiate retain their
client-side decorations, as required by Wayland; Clear cannot safely strip an
application's own header bar from its pixels.

Server-side titlebars display the window title (falling back to app ID), plus
**minimize, maximize/restore, and close** buttons. Click to focus, or drag the title
to move; dragging a tile detaches it, but a stationary click does not. Buttons
activate only when released over the same control. **Super+left/right drag** still
moves/resizes over titlebars and takes precedence over buttons. Maximized windows
must be restored before dragging; launcher-role windows remain client-sized and
reject ordinary move/maximize/minimize operations.

Configure negotiated titlebars independently of borders and blur:

```toml
[theme.titlebar]
# Straight RGBA, exactly four finite channels in 0..=1.
active_background = [0.137, 0.157, 0.204, 1.0]
inactive_background = [0.106, 0.118, 0.149, 1.0]
active_foreground = [0.937, 0.953, 0.980, 1.0]
inactive_foreground = [0.682, 0.718, 0.780, 1.0]
height = 32                 # Integer logical pixels, 16..128; default 32.
controls_side = "right"      # "left" or "right" (default).
show_icon = false           # Default; true enables an application icon.
show_title = true           # Default; false hides title text, not controls.

# Optional local SVG replacements; create the files before uncommenting.
# [theme.titlebar.controls]
# minimize = "titlebar/minimize.svg"
# maximize = "titlebar/maximize.svg"
# restore = "titlebar/restore.svg"
# close = "titlebar/close.svg"
```

The colors above approximate the default dark backgrounds and light foregrounds.
Background alpha may be reduced (for example, to `0.9`) without changing client
opacity; title text and built-in glyphs use the selected foreground RGBA. Alpha is
preserved through premultiplied composition and the rounded shader, with no forced
opaque fill, accent stripe, or separator. Custom SVGs retain their own colors and
alpha rather than being recolored to the foreground. Square SSD borders are rings,
even with blur disabled, so border color does not fill behind transparent titlebars.
The legacy square, unblurred client-side-decoration backing is unchanged.

Controls read **minimize, maximize/restore, close** from left to right on the right
edge; the left edge mirrors that order to **close, maximize/restore, minimize**.
Narrow frames prioritize close, then maximize, then minimize. The configured height
is included in normal tiled/floating/maximized frame geometry, not drawn over
client content. Reloading height changes the content inset and configure size,
never saved floating geometry or layout proportions. Launchers remain client-sized
and are centered with the titlebar included. Layer-shell panels and XDG popups never
receive these titlebars. Unicode titles use `cosmic-text` with installed system
fonts; built-in controls render independently of fonts. No shell process is required.

SVG control paths resolve relative to the configuration file, or may be absolute;
`~` and environment variables are not expanded. Omitted controls use built-in
glyphs; an omitted `restore` first falls back to a configured `maximize` SVG.
Resources must be bounded local regular files. SVGs use minimal `resvg` (default
features disabled): no network/external resources, embedded images, SVG text/fonts,
SVGZ, or DTD/entities. The restrictive SVG subset supports simple paths and gradients,
not masks, clips, or filters: `use`, `pattern`, `marker`, `mask`, `clipPath`, and
`filter` elements are rejected before usvg conversion, even if unused or namespaced.
Limits are 256 KiB source, 1024 pixels per source dimension, 4096 XML nodes, and
32 nesting levels.

With `show_icon = true`, Clear looks up the exact app ID's desktop entry and its
`Icon` key in bounded XDG data roots, then local `hicolor`, direct icon, and `pixmaps`
locations (including legacy `~/.icons`). SVG/PNG/JPEG icons or absolute icon paths
are supported. Missing, invalid, or unsupported icons use a generic built-in icon.
This is not a full icon-theme resolver: there is no fuzzy app matching, recursive
search, theme inheritance, or desktop command execution.

Runtime prepares SVGs and app icons on the CPU at startup/reload or app
classification, never through filesystem access in the render loop. Startup control
resource failures warn and use built-in controls; invalid startup configuration
uses safe defaults. Reload rereads even same-path SVGs and swaps configuration and
prepared resources atomically: any explicit resource failure retains the entire
last-good set. The renderer caches visible titlebar strips with both a **64 MiB**
texture-byte budget and **128-entry** limit, excluding outstanding render elements
and driver overhead. See [architecture](docs/architecture.md#server-side-decorations)
for resource and cache bounds.

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
python3 scripts/vm-wallpaper-smoke.py --binary target/debug/clear
python3 scripts/vm-window-state-smoke.py --binary target/debug/clear
python3 -B scripts/vm-rounded-smoke.py --binary target/debug/clear
python3 -B scripts/vm-decoration-smoke.py --binary target/debug/clear
python3 -B scripts/test_vm_decoration_smoke.py
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --artifacts target/vm-blur-gaussian
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --method kawase --radius 2 --passes 3 --artifacts target/vm-blur-kawase
python3 -B scripts/test_vm_blur_smoke.py
python3 scripts/vm-shell-smoke.py --binary target/debug/clear
```

The smoke runner requires KDE's `kwin_wayland`, `dbus-run-session`, and `foot`.
It starts a private virtual display, runs real Wayland clients, validates a
nonblank GPU framebuffer capture, and terminates only its own processes. It can
run over SSH without sudo or a graphical login. Logs and captures stay in
`target/vm-smoke/`. It does not automate physical keyboard or mouse gestures.

See [docs/vm-testing.md](docs/vm-testing.md#backdrop-blur) for blur commands and the
recorded validation matrix. On private local virtual KWin (not a VM), Kawase passed
all ten cases at radius 2/passes 3, plus `stacking`/`output-boundary-odd` at radius
1.5/passes 1 and 6. Gaussian radius 12 passed only `xdg`, `stacking`, and
`output-boundary-odd` in this round. Full Cargo fmt/check/test/build and all 11 CPU
oracle/harness tests passed; no physical input tests were performed.

Titlebar GPU validation passed on **private LOCAL virtual KWin, not in a VM**:
the original 11 cases in `target/titlebar-geometry/` and `target/titlebar-colors/`,
both square cases in `target/titlebar-square/`, eight selected negotiation cases in
`target/titlebar-negotiation/`, and Kawase radius 2/passes 3 `rounded-ssd` in
`target/titlebar-blur-regression/`. Cargo fmt/check/test/build passed after fixes,
as did all 14 CPU harness and 10 schema/resource tests. No physical input was tested;
not all legacy negotiation cases were rerun. See
[decoration testing](docs/vm-testing.md#server-side-decorations) for exact cases and
bounded commands. CPU harness tests are not GPU validation.
Machine-specific setup and access notes belong in the git-ignored `.agents/`
directory, not in tracked documentation.

## Current limitations

This is a runnable foundation, not a production compositor. Native DRM/KMS,
physical monitor hotplug, XWayland, fractional scaling, IME, fullscreen policy,
sophisticated decorations, and animation remain future work. Popup and
layer-shell support remains intentionally small; rich client cursor rendering
still needs integration. Monitor-group editing currently
offers stretch-all and split, not arbitrary monitor subsets.

Hidden workspaces and clipped scrolling regions are excluded from hit testing.
Clients resize asynchronously; old tiled buffers are clipped while waiting for
new content rather than stalling the compositor.
