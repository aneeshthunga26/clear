# Optional Quickshell desktop shell

This is an example client, not a Clear dependency or required startup service.
Install Quickshell 0.3.1 and matching Qt/Qt Wayland libraries separately.

From the repository root:

```sh
cargo build --locked
./target/debug/clear --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

The 36px top panel on each output offers workspace buttons, grouped applications,
mode controls with SVG icons, a StatusNotifier tray, and a notification button.
Hover an application to see cards for all of its open windows, including windows
on other workspaces; click a card to reveal and focus it. Left click an app icon
to focus its first window or launch its desktop entry. Right click to pin or unpin
it. Window cards have minimize/restore and maximize/unmaximize buttons, and show
minimized/maximized status, following Clear's
[window-state contract](../../specs/desktop.md#maximize-and-minimize).
Pins are saved in Quickshell's per-shell state directory. The mode button
cycles built-in modes and the reset button clears the current output override.
Rows, grid, and the Fibonacci-style spiral have their own mode icons.

The launcher button opens a searchable desktop-entry launcher. Type to filter,
press Enter or click an entry to launch, use its star to pin, and press Escape
to close. Quickshell executes the desktop entry without shell interpolation. The notification tray
runs a freedesktop notification server, shows tracked messages, invokes their
actions, and supports individual or bulk dismissal. The system tray uses
Quickshell's StatusNotifier service. Both need a working session D-Bus; run the
nested compositor with a private bus when testing to avoid competing with host
services.

For notifications from all nested apps to reach this shell while a host
notification daemon is running, start the entire nested session on one private
bus:

```sh
dbus-run-session -- ./target/debug/clear --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

The Alt+Tab overlay displays each window's desktop-entry icon, app name, and
title from Clear's [switcher state](../../specs/shell.md#snapshot-fields). It
uses its bundled generic application icon when the app ID has no desktop-entry
match or the icon theme cannot load the entry's icon.
Gesture, acceptance, and cancellation behavior is defined by the
[input specification](../../specs/input.md#alt-tab).

The hover cards show window titles, app IDs, and workspaces. They are not live
pixel thumbnails: Clear does not yet offer a toplevel capture protocol. The
launcher uses Quickshell's desktop-entry index, so only installed desktop entries
appear. Notification history exists for the life of the Quickshell process.

## Translucent panels

### Liquid-glass variant

Run the glass appearance with your existing liquid-glass compositor settings:

```sh
dbus-run-session -- ./target/debug/clear --config examples/liquid-glass.toml --command quickshell -p examples/quickshell/liquid-glass.qml
```

This interactive command has no exit timer. Ctrl+Alt+Return opens the configured
translucent terminal, Ctrl+Alt+Shift+R reloads Clear's TOML, and Ctrl+Alt+Escape
quits. The private bus keeps this example's notifications separate from the host.
The compositor configuration uses the included cracked-earth image at
`examples/liquid-glass-wallpaper.jpg` as its wallpaper.

The floating top bars have low-opacity backgrounds and semicircular left/right
ends, including matching input masks. Popups share the tint and rounded styling.
Its explicit layer namespace also gives Clear the pill radius needed to fit the
refraction and specular maps to the bar, so the glass follows the curved ends.
The existing controls and services are shared with the standard example. Adjust
glass colors, opacity, spacing, and sizing in [ShellStyle.qml](ShellStyle.qml);
restart Quickshell after editing QML. Clear's TOML continues to control the
compositor's glass/blur settings. See the
[appearance contract](../../specs/shell.md#optional-quickshell-appearances) for
exact geometry and opacity. Rounded input masks use Quickshell's
[Region radius](https://quickshell.org/docs/v0.3.1/types/Quickshell/Region/).

### Standard appearance

The top bar uses `#e60f172a`; launcher, notification, app-preview, and Alt+Tab
panel backgrounds use `#e6111827` (QML `#AARRGGBB`). Alpha `0xe6` is 230/255,
about 90.2% opacity. Window backing is transparent, and only the background
rectangles have alpha: text, icons, controls, and window/notification cards stay
opaque. Closed overlay layers remain fully transparent.

Transparency is useful on its own and does not require blur. For optional
compositor-side backdrop blur in Clear, set this in Clear's TOML configuration
(merge into an existing `[theme]` table if present):

```toml
[theme]
blur_method = "kawase"
blur_radius = 2
blur_passes = 3
```

These are the settings in `examples/vm.toml`; blur is not required to use this shell.
The QML adds no blur shaders or effects. See the
[rendering specification](../../specs/rendering.md) for accepted values, defaults,
filter units, composition, and resource limits, and
[the blur test workflow](../../docs/vm-testing.md#backdrop-blur) for oracle coverage
and recorded compositor fixture results.

Modules:

- `ClearBridge.qml` and `Protocol.js`: Clear IPC and derived desktop models.
- `PinnedApps.qml`: persistent pinned app IDs.
- `AppDock.qml`, `ModeIcon.qml`, `Tray.qml`: panel features.
- `AppLauncher.qml`, `NotificationStore.qml`, `NotificationCenter.qml`: app and notification UI.
- `Switcher.qml`: read-only Alt+Tab overlay.
- `DesktopShell.qml`: shared assembly and per-output panel.
- `ShellStyle.qml`: standard and glass palettes/geometry.
- `shell.qml` / `liquid-glass.qml`: appearance entry points.

The bridge, notification service, tray, and small panel controls stay loaded so
they can receive state and messages. The launcher, notification view, Alt-Tab
view, and app preview are created with Quickshell `LazyLoader` on first use.
After loading, overlay layers stay mapped as transparent 1×1 surfaces when
closed. This avoids a Quickshell disconnect on layer destruction in Clear's
current nested backend while releasing keyboard and pointer input.

Missing IPC leaves workspace, mode, and window controls disabled; launcher,
notifications, and tray remain usable. Commands are not queued for replay on
reconnect. Clear does not autostart or respawn Quickshell and supports
`--no-shell-ipc`.

```sh
node --test examples/quickshell/Protocol.test.mjs
qmllint examples/quickshell/*.qml
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell --exercise-overlays
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell --quickshell-style glass --exercise-overlays --artifacts target/vm-glass-shell
```

See [the integration guide](../../docs/shell-integration.md) for setup and
[the shell specification](../../specs/shell.md) for the IPC contract.
