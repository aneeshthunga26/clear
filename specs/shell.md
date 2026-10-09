# Optional shell IPC v1

## Boundary and discovery

Clear MUST work without Quickshell, Qt, panels, notification services, or any
other companion UI. Graphical shell clients use standard
[Wayland/layer-shell](platform.md#layer-shell-lifecycle). This independent,
toolkit-neutral IPC exposes desktop state and a validated command allowlist.
Notifications and tray services remain outside the compositor. The optional
[Quickshell example](../examples/quickshell/README.md) does not own desktop policy.

When enabled and successfully bound, the endpoint is
`XDG_RUNTIME_DIR/clear-<WAYLAND_DISPLAY>/shell.sock`. The actual absolute path is
reported at startup and exported as `CLEAR_SOCKET` to children. Each instance uses
its own endpoint. `--no-shell-ipc` disables it; binding failure warns and leaves
the compositor running. An unavailable bridge MUST NOT leak an inherited
`CLEAR_SOCKET` into spawned clients.

The runtime directory MUST be absolute, nonsymlinked, owned by the effective user,
and mode 0700. The server creates a private 0700 directory and 0600 socket. It
refuses existing directories rather than unlinking a different instance's socket.
Normal cleanup removes only entries whose identities still match those it created.
There is no TCP listener, token file, or configuration file for authentication.

This is same-user desktop control, not isolation from malicious programs running
as that user. Snapshots include hidden-workspace titles/app IDs. The endpoint
must not be exposed to untrusted users or proxied onto a network.

## Wire format and requests

Messages are UTF-8 JSON objects followed by LF on a persistent Unix stream.
Partial reads and writes are supported. Requests carry numeric `version: 1` and
a `u32` `id`. Desktop IDs MUST be canonical decimal strings, preserving the full
`u64` range without JavaScript number rounding. Unknown fields/operations,
duplicate fields, missing fields, wrong types, and unsupported versions fail.

```json
{"version":1,"id":1,"request":{"type":"subscribe"}}
{"version":1,"id":2,"request":{"type":"switch_workspace","output":"1","workspace":"3"}}
{"version":1,"id":3,"request":{"type":"set_mode","output":"1","mode":"columns"}}
```

Each line is a separate request. The complete allowlist is:

| `type` | Additional fields | Effect |
| --- | --- | --- |
| `snapshot` | None | Complete current state without subscription |
| `subscribe` | None | Complete state and subsequent changes; idempotent |
| `toggle_overview` | None | Request compositor-owned [overview](overview.md) entry/exit, deferred while input is busy |
| `focus_output` | `output` | Focus a connected output |
| `switch_workspace` | `output`, `workspace` | Focus output, then switch its group |
| `focus_window` | `window` | Explicit core focus, including restore/reveal |
| `set_maximized` | `window`, `maximized` boolean | Set normal-window maximize state without explicitly changing focus |
| `set_fullscreen` | `window`, `fullscreen` boolean | Set normal-window fullscreen state without revealing, restoring or explicitly focusing it |
| `set_minimized` | `window`, `minimized` boolean | Hide/restore a normal window; repair focus when needed |
| `set_mode` | `output`, `mode` | Set current workspace's override on that output without stealing focus |
| `clear_mode` | `output` | Clear that workspace/output override |
| `stretch` | `output` | Focus output and stretch across all outputs |
| `split` | `output` | Focus output and split its group |

Modes use [core mode parsing](layouts.md#layout-inputs-and-names) with canonical
names in responses, including `script:NAME`. Mode strings are limited to 256
UTF-8 bytes. Script selection uses existing Rhai validation/fallback; it does not
permit arbitrary script or action execution.

All targets and values MUST be validated before any mutation, including requests
with multiple targets. Invalid requests cannot partly change focus. Launcher
targets reject maximize/minimize/fullscreen without mutation. There are no spawn, quit,
reload, close-window, script-action, or raw core-command requests.

## Responses and ordering

| Meaning | Envelope |
| --- | --- |
| Query/initial state | `{"version":1,"type":"snapshot","id":1,"state":{...}}` |
| Unsolicited change | `{"version":1,"type":"state","state":{...}}` |
| Accepted command | `{"version":1,"type":"ok","id":2}` |
| Error | `{"version":1,"type":"error","id":2,"message":"..."}` |

Malformed envelopes, unsupported versions, and parse errors use `id: null`.
Errors after envelope validation retain its ID. Invalid JSON does not itself
disconnect the connection; transport violations do.

Commands on a connection execute in order. The adapter reconciles successful
commands before responses/publication; a following snapshot reflects settled
policy, not a guarantee that clients have committed new buffers or the GPU has
presented a frame. Subscribers MUST handle interleaved events and replies.
Changes replace the whole snapshot and are coalesced, not a lossless event log.
The first snapshot may be followed by an identical state event. There is no
resume cursor; recovery means reconnecting and subscribing again. Clients should
not replay stale commands.

## Snapshot fields

All identity fields below are decimal strings, including identities inside arrays.

| Field | Shape and meaning |
| --- | --- |
| `outputs` | `{id,name,area,workspace,effective_mode,mode_override}`; `area` is `{x,y,width,height}` in usable logical coordinates after reservations; override may be null |
| `workspaces` | `{id,name,mode,windows}`; window IDs in stable workspace order, including hidden workspaces |
| `groups` | `{outputs,workspace}`; outputs jointly presenting a workspace |
| `windows` | `{id,title,app_id,workspace,output,role,floating,maximized,minimized,fullscreen,focused}` |
| `focused_output`, `focused_window` | ID or null |
| `overview_open` | Boolean; active compositor-owned overview, excluding a deferred request |
| `switcher` | null or `{output,windows,selected}` during the compositor's Alt-held gesture |

Window `output` is its saved home or null, not a visibility test. `role` is
`normal` or `launcher`. `floating` is the saved exception flag, not the effective
mode. Minimized windows remain in snapshots with no placement. Maximized/minimized/fullscreen
fields and setters are additive v1 extensions; state transitions follow
[desktop policy](desktop.md#maximize-and-minimize).

Snapshots report desktop focus, not keyboard ownership held by a layer. Group
presentation determines workspace visibility; clients MUST NOT infer it from a
window's home alone. Switcher candidates include minimized normal windows; focus
changes only upon acceptance, as specified in [input](input.md#alt-tab).

## Transport bounds

The nonblocking server is polled from the compositor event loop. It MUST NOT
perform shell work in the renderer or create an unbounded background queue.

| Resource/work | Limit |
| --- | --- |
| Connected clients, including idle peers | 32 |
| Request frame, excluding LF | 16 KiB |
| Outgoing queue per client | 1 MiB |
| Accept attempts per poll | 8 |
| IO attempts per client per direction | 8 |
| Requests per client per poll | 8 |
| Per-client IO byte budget per direction | 32 KiB |
| Requests per poll | 64 |
| Total read bytes per poll | 128 KiB |
| Total write bytes per poll | 256 KiB |

Oversized input, IO failure, and overflowing output queues disconnect the peer.
A snapshot exceeding the outgoing limit also disconnects rather than growing
storage indefinitely. Unterminated input at EOF is discarded; a write-half-closed
peer can still receive replies to complete requests before disconnection.

## Optional Quickshell appearances

The optional example provides two entry points sharing its bridge, services,
controls, and overlay lifecycle: `examples/quickshell/shell.qml` retains the
existing appearance, while `examples/quickshell/liquid-glass.qml` selects the
glass appearance intended for `examples/liquid-glass.toml`.

Both panels show registered StatusNotifierItem icons and a local two-line
time/date display at the right. A volume control appears when PipeWire has a
default audio sink: clicking it toggles mute, and scrolling changes volume
within 0–100%. A Bluetooth control appears when an adapter exists and toggles
that adapter's enabled state. A battery indicator appears only for a present
laptop battery. Unavailable services MUST NOT produce placeholder controls.
The show-desktop button minimizes the visible normal windows in its output's
workspace and restores the same windows on a second click; it does not change
other workspaces. On outputs narrower than 900 pixels, Bluetooth, battery,
show-desktop, and mode controls are hidden to leave room for the launcher,
workspace controls, dock, volume, and clock.

The glass variant uses a 44-pixel top panel on every output, inset 12 pixels
from the left/right edges and 10 pixels from the top, reserving 54 pixels total.
Its background and input
region have radius 22, producing semicircular ends with transparent, noninteractive
corners. Controls are inset beyond the end caps. Background tint is `#232834`
at 18% opacity; text and icons retain their own opacity. Hover/selection fills
are translucent, with a subtle light outline. No shadows are added.

Launcher, notification, and app-preview surfaces share a 24%-opaque tint and
rounded corners; the horizontal switcher uses capsule ends. Each switcher card
shows the window title, app name, and a desktop-entry icon resolved from its app
ID, falling back to a bundled generic application icon when lookup or icon
loading fails. The switcher does not capture window pixels. These overlays
retain their existing lazy creation and transparent 1×1 closed state. Glass
styles change appearance only: shell commands, keyboard ownership, per-output
placement, and compositor-side filtering retain their existing contracts.
The QML draws its own rounded alpha and input masks; Clear's window corner-radius
setting does not shape layer-shell panels. Its `clear-glass-pill-*` and
`clear-glass-rounded-*` layer namespaces also tell Clear the opt-in optical outline
to fit to the panel bounds. Other layer clients keep their own shape behavior.
No client-side blur shader is added. Style values are centralized in
`ShellStyle.qml`; the glass entry point does not copy the shell logic or autostart
another shell.

## Implementation and evidence

- [Model, validation, serialization](../src/shell/mod.rs),
  [transport and inline tests](../src/shell/server.rs),
  [adapter polling/reconciliation](../src/platform/smithay/shell.rs).
- [Optional panel status controls](../examples/quickshell/SystemStatus.qml) and
  [StatusNotifierItem tray](../examples/quickshell/Tray.qml).
- [IPC tests](../tests/shell_ipc.rs): strict shapes, large IDs, hidden windows,
  ordered snapshots, atomic target validation, groups/modes, and switcher state.
- [Window-state tests](../tests/window_state.rs): validated state setters.
- [Shell fixture](../scripts/vm-shell-smoke.py): actual socket discovery,
  subscriptions, reconnects, disabled environment cleanup, optional real panels.
  `--quickshell-style glass` checks capsule silhouettes, low-opacity pixels, and
  the glass variant's output reservations; `--exercise-overlays` checks both
  appearances' shared lifecycle without physical input injection.
  [Example model tests](../examples/quickshell/Protocol.test.mjs) cover its client
  helpers, not compositor protocol/GPU or physical input behavior.
