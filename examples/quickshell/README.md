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
minimized/maximized status. Minimized windows remain in the dock; selecting one
restores it. Maximizing respects panel reservations and preserves saved geometry
and tiling state. Pins are saved in Quickshell's per-shell state directory. The mode button
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

Alt+Tab opens a selection overlay for normal windows on the active workspace.
Repeated Tab presses advance the selection, releasing Alt commits it, and Escape
cancels. The VM example binds Alt+Tab explicitly; the default bindings include
it as well. Clear owns the gesture and focus change. The Quickshell view only
renders its state.

The hover cards show window titles, app IDs, and workspaces. They are not live
pixel thumbnails: Clear does not yet offer a toplevel capture protocol. The
launcher uses Quickshell's desktop-entry index, so only installed desktop entries
appear. Notification history exists for the life of the Quickshell process.

## Translucent panels

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
`blur_method` is global and accepts `"gaussian"` (default) or `"kawase"` (Dual Kawase).
`blur_radius` accepts finite values from 0 through 32, including fractions, and
defaults to 0 (disables either method). Gaussian measures kernel support in logical
pixels (try radius 12); Kawase measures sample offsets in source pyramid texels,
not Gaussian-equivalent pixels. `blur_passes` is an integer from 1 through 6,
default 3: Kawase downsamples through ceil-half levels and upsamples through the
same levels in reverse, stopping early at `1×1`. Gaussian ignores this setting but
it is always validated, even when blur is disabled.

Clear owns the blur; the QML adds no blur shaders or effects. Neither method adds
a glass treatment or changes client opacity: the panel colors and alpha described
above remain client-owned, and foreground text/controls stay sharp. See
[the blur test workflow](../../docs/vm-testing.md#backdrop-blur) for method/radius/depth
options, oracle self-tests, and recorded results. Kawase passed all ten GPU cases
at radius 2/passes 3 on private local virtual KWin (not a VM), plus
`stacking`/`output-boundary-odd` at radius 1.5/passes 1 and 6. These were compositor
fixture checks, not Quickshell or physical input tests.

Modules:

- `ClearBridge.qml` and `Protocol.js`: Clear IPC and derived desktop models.
- `PinnedApps.qml`: persistent pinned app IDs.
- `AppDock.qml`, `ModeIcon.qml`, `Tray.qml`: panel features.
- `AppLauncher.qml`, `NotificationStore.qml`, `NotificationCenter.qml`: app and notification UI.
- `Switcher.qml`: read-only Alt+Tab overlay.
- `shell.qml`: assembly and per-output panel.

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
```

See [the integration guide](../../docs/shell-integration.md) for the IPC contract.
