# Optional desktop shells

Clear does not require Quickshell, Qt, a panel, or a notification service. Run it
alone, use any compatible layer-shell clients, or build a separate shell. The
optional example in `examples/quickshell/` demonstrates a companion panel, not a
new required part of the compositor.

## Two independent interfaces

- **Wayland/layer-shell** provides panel surfaces, placement, exclusive zones,
  popup rendering, and input. It remains the normal integration for graphical
  shell clients, including Waybar and Quickshell.
- **Clear shell IPC v1** exposes desktop state and a small allowlist of commands.
  It is toolkit-independent, with no Quickshell-specific protocol or model.

`src/shell/mod.rs` owns the backend-independent model and validation;
`src/shell/server.rs` owns bounded Unix socket transport. The Smithay adapter
reconciles successful commands with the real scene before publishing state.
A future native shell can consume the same model without retaining the QML UI.
Rhai remains the extension language for window-management policy; QML/JavaScript
is confined to the optional presentation example.

## Run the example

Install Quickshell separately with a matching Qt/Qt Wayland environment. The
example uses the documented Quickshell Socket, PanelWindow, Variants, and
WlrLayershell APIs; it does not use Hyprland- or Sway-specific modules.

From the checkout root, inside an existing graphical session:

```sh
cargo build --locked
./target/debug/clear --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

Clear sets `WAYLAND_DISPLAY` and `CLEAR_SOCKET` for its child. A terminal launched
inside Clear inherits those values, so the example can also be started from
there. If Qt selects the wrong platform plugin, run it through
`--command env QT_QPA_PLATFORM=wayland quickshell -p examples/quickshell/shell.qml`.
Do not point the shell at an unrelated host display or assume a socket name.

The example creates a 36px top panel on each output with workspace buttons,
grouped apps, persisted pins, mode icons/controls, a StatusNotifier tray, and a
notification button. The launcher searches installed desktop entries. The
notification center receives messages on session D-Bus and supports actions and
dismissal. Hover app cards list all its windows and their workspaces; live pixel
thumbnails require a toplevel capture protocol that Clear does not expose.
Output matching uses exact Wayland screen names. Mode changes do not steal
output focus. Client text uses plain-text rendering. Missing IPC disables
desktop controls while the bridge retries without replaying stale commands.
Clear does not silently launch/restart Quickshell. Closing it leaves Clear
running, and panel reservations follow the normal surface lifecycle.
Larger views use Quickshell `LazyLoader` on first use. After they load, overlay
layer surfaces remain mapped at 1×1 transparent size while closed, releasing
keyboard/pointer input. This avoids a disconnect when a visible layer role is
destroyed in the current nested backend.
If the host session already owns notification or tray D-Bus names, launch
the entire nested Clear session through `dbus-run-session --` so the shell and
its child apps share one private bus.

## Discovery, isolation, and optional startup

When available, the endpoint is:

```text
XDG_RUNTIME_DIR/clear-<WAYLAND_DISPLAY>/shell.sock
```

The actual absolute path is printed on startup and exported as `CLEAR_SOCKET`
to spawned clients. Each Clear instance uses its own endpoint. Inherited
`CLEAR_SOCKET` is cleared before spawning if this instance has no IPC server.
Use `--no-shell-ipc` to disable it entirely. A bind failure produces a warning
and Clear continues without the optional bridge.

The runtime directory must be absolute, nonsymlinked, owner-private (0700).
The server creates a private 0700 subdirectory and a 0600 socket. It refuses
existing directories rather than unlinking another instance's socket. Normal
shutdown removes only entries it created; after an unclean exit, inspect a
leftover directory and confirm the owning instance has stopped before removing
it manually. There is no TCP listener or token/config file.

This is a same-user desktop control interface, not a sandbox against malicious
programs running as that user. Subscribers can see titles/app IDs on hidden
workspaces and can request focus/workspace/mode changes. Do not expose or proxy
this endpoint to untrusted users or a network.

## Wire format

UTF-8 JSON, one object followed by LF per message over a persistent Unix stream.
Partial writes/reads are supported. Every request carries version 1 and a `u32`
request ID. **Desktop IDs are canonical decimal strings** because a Rust `u64`
need not fit exactly in a JavaScript number. Unknown fields, unknown operations,
wrong types, duplicate fields, and unsupported versions are rejected.

```json
{"version":1,"id":1,"request":{"type":"subscribe"}}
{"version":1,"id":2,"request":{"type":"switch_workspace","output":"1","workspace":"3"}}
{"version":1,"id":3,"request":{"type":"set_mode","output":"1","mode":"columns"}}
```

Each line above is a separate request. Available requests:

| `type`             | Additional fields               | Behavior                                                                                             |
| ------------------ | ------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `snapshot`         | None                            | Return a complete current state, without subscribing.                                                |
| `subscribe`        | None                            | Return a complete state and subscribe to subsequent changes. Idempotent.                             |
| `focus_output`     | `output`                        | Focus a connected output.                                                                            |
| `switch_workspace` | `output`, `workspace`           | Select the output, then switch its group using core workspace semantics.                             |
| `focus_window`     | `window`                        | Restore a minimized window and focus it; may reveal its hidden workspace.                            |
| `set_maximized`    | `window`, `maximized` (boolean) | Set maximize state on a normal window without explicitly changing focus or its saved floating state. |
| `set_minimized`    | `window`, `minimized` (boolean) | Hide/restore a normal window without changing workspace; focus is repaired if necessary.             |
| `set_mode`         | `output`, `mode`                | Set that output's override for its currently visible workspace, without stealing focus.              |
| `clear_mode`       | `output`                        | Clear that override, retaining workspace policy.                                                     |
| `stretch`          | `output`                        | Focus the output and stretch its workspace across all outputs.                                       |
| `split`            | `output`                        | Focus the output and split its group.                                                                |

Modes use core parsing, with canonical names in responses: `floating`,
`scrolling`, `master_stack`, `columns`, `rows`, `grid`, `spiral`, `monocle`, or
`script:NAME`. `fibonacci` and `dwindle` are accepted aliases for `spiral`. The mode
string is limited to 256 UTF-8 bytes. Named layouts use existing Rhai validation
and fallback; this does not grant arbitrary script/action execution.

All targets and values are validated before a command mutates state. There are
no arbitrary spawn, quit, reload, script-action, or raw core-command requests.
Focus shown in snapshots is desktop policy focus, not keyboard ownership held
by a layer-shell surface.

### Responses

- Initial/query state: `{"version":1,"type":"snapshot","id":1,"state":{...}}`.
- Unsolicited state: `{"version":1,"type":"state","state":{...}}`.
- Command accepted: `{"version":1,"type":"ok","id":2}`.
- Error: `{"version":1,"type":"error","id":2,"message":"..."}`.

Malformed envelopes, unsupported versions, and parse errors use `id: null`.
Errors from validated envelopes retain their request ID. Invalid JSON does not
kill a connection; transport limit violations do. Commands on one connection
are ordered, and a subsequent snapshot reflects their settled policy state.
Subscribers must handle interleaved replies/events. Updates replace the entire
snapshot; they are coalesced, not a lossless event history. An initial snapshot
may be followed by an identical state event. Reconnect and subscribe again to
recover; there is no resume cursor and clients should not replay stale commands.

### Snapshot fields

- `outputs`: `{id, name, area, workspace, effective_mode, mode_override}`.
  `area` is `{x,y,width,height}` in usable logical coordinates **after panel
  reservations**, not full monitor bounds. `mode_override` may be null.
- `workspaces`: `{id, name, mode, windows}`; `windows` contains stable window IDs
  in workspace order, including windows on hidden workspaces.
- `groups`: `{outputs, workspace}` describing which outputs jointly present a
  workspace. Do not infer presentation from a window's home output.
- `windows`: `{id,title,app_id,workspace,output,role,floating,maximized,minimized,focused}`.
  `output` is its saved home or null; `role` is `normal` or `launcher`.
  `floating` is its saved exception flag, not its effective layout mode.
  `maximized` fills the home output's usable area without changing that flag.
  `minimized` windows remain in snapshots/docks but have no rendered placement;
  explicit focus restores them and preserves their maximized state. These boolean
  fields and the corresponding commands are additive extensions of v1. Launcher
  targets reject maximize/minimize commands without mutating state.
- `focused_output` and `focused_window`: IDs or null.
- `switcher`: null or `{output,windows,selected}` while Alt+Tab is held.
  The candidate IDs belong to the active workspace; focus remains unchanged
  until the physical Alt release. Minimized normal windows remain candidates;
  accepting one restores it. Escape cancels without changing focus or minimization.

### Resource limits

The server is nonblocking and serviced from the existing compositor event loop.
It accepts at most 32 clients and 16 KiB per request (excluding LF), with bounded
read/write/request work per turn. Each outgoing queue is limited to 1 MiB;
slow clients or snapshots exceeding that limit cause disconnection rather than
unbounded buffering. The client count includes idle connections. There is no
background thread or shell work inside the renderer.

## Notifications, tray, and switching

The example uses Quickshell's notification and tray D-Bus services; these are
not compositor dependencies. Clear owns Alt+Tab candidate selection, modifier
release acceptance, Escape cancellation, and focus policy. The shell overlay
reads the transient `switcher` snapshot field and cannot commit focus itself.

Third-party Quickshell configurations targeting another compositor are not
necessarily drop-in compatible. Clear does not currently expose every protocol
such configurations may expect, including toplevel-list/capture integrations.
Use the Clear state interface for Clear-specific desktop information.

For nested tests, keep notification/tray services on a private session bus so
they cannot compete with the host's services. Keep local VM/package setup notes
in `.agents/`.

## Verification

```sh
cargo test --locked --test shell_ipc
cargo test --locked --lib shell::server
node --test examples/quickshell/Protocol.test.mjs
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell --artifacts target/vm-quickshell
```

The Node tests need no Quickshell installation and check only the example's
message/model helpers. The VM runner uses a private virtual KWin host, real foot,
a bounded Clear run, and optionally the real Quickshell example on a private
D-Bus session. It checks IPC commands/subscriptions, failure handling, disabled
IPC environment cleanup, and optional panel reservations/captured pixels—not
physical clicks or keypresses. See `docs/vm-testing.md` for shared dependencies.
