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
its own endpoint. `--no-shell-ipc` disables it; binding or readiness-registration
failure warns and leaves the compositor running with the bridge disabled.
An unavailable bridge MUST NOT leak an inherited
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
| `activate_fullscreen` | None | Activate the occupied fullscreen desktop on its target output; empty/headless slots fail |
| `focus_output` | `output` | Focus a connected output |
| `switch_workspace` | `output`, `workspace` | Focus output, then switch its group |
| `focus_window` | `window` | Explicit core focus, including restore/reveal |
| `close_window` | `window` | Send a close request to a mapped normal client without focusing it; the application may refuse |
| `move_window_to_workspace` | `window`, `workspace` | Move an explicit normal window through core transfer policy without switching workspaces or focusing it |
| `window_previews` | `windows` | Request a bounded batch of committed window images; requires a mapped panel owned by the requesting process |
| `set_maximized` | `window`, `maximized` boolean | Set normal-window maximize state without explicitly changing focus |
| `set_fullscreen` | `window`, `fullscreen` boolean | Replace/clear the single fullscreen occupant; a focused target activates, background targets remain dormant; minimized enters fail |
| `set_minimized` | `window`, `minimized` boolean | Hide/restore a normal window; repair focus when needed |
| `set_mode` | `output`, `mode` | Set current workspace's override on that output without stealing focus |
| `clear_mode` | `output` | Clear that workspace/output override |
| `stretch` | `output` | Focus output and stretch across all outputs |
| `split` | `output` | Focus output and split its group |
| `animation_panels` | None | Discover eligible mapped panels owned by this peer's Wayland process |
| `set_animation_targets` | `panel`, `output`, `targets` | Atomically replace this connection's advisory icon rectangles for one live panel |

Modes use [core mode parsing](layouts.md#layout-inputs-and-names) with canonical
names in responses, including `script:NAME`. Mode strings are limited to 256
UTF-8 bytes. Script selection uses existing Rhai validation/fallback; it does not
permit arbitrary script or action execution.

All targets and values MUST be validated before any mutation, including requests
with multiple targets. Invalid requests cannot partly change focus. Launcher
targets reject maximize/minimize/fullscreen without mutation. There are no spawn, quit,
reload, script-action, or raw core-command requests. Close and workspace-transfer
requests validate exact normal-window IDs; launchers and unknown/noncanonical
targets fail without mutation. Transfers validate the destination before mutation,
and sending an ordinary window to its current workspace succeeds as a no-op; for the fullscreen occupant it exits fullscreen without changing ownership.

## Responses and ordering

| Meaning | Envelope |
| --- | --- |
| Query/initial state | `{"version":1,"type":"snapshot","id":1,"state":{...}}` |
| Unsolicited change | `{"version":1,"type":"state","state":{...}}` |
| Accepted command | `{"version":1,"type":"ok","id":2}` |
| Error | `{"version":1,"type":"error","id":2,"message":"..."}` |
| Panel discovery | `{"version":1,"type":"animation_panels","id":3,"panels":[...]}` |
| Window images | `{"version":1,"type":"window_previews","id":4,"frames":[{"window":"7","image":"data:image/png;base64,..."}]}` |

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
| `windows` | `{id,title,app_id,workspace,output,role,floating,maximized,minimized,fullscreen,fullscreen_transient,focused}` |
| `focused_output`, `focused_window` | ID or null |
| `overview_open` | Boolean; active compositor-owned overview, excluding a deferred request |
| `fullscreen_desktop` | null or `{window,mode:"desktop",output,active}`; output can be null when headless; separate from numbered workspaces |
| `switcher` | null or `{output,windows,selected}` during the compositor's Alt-held gesture |
| `capabilities` | String array; includes `animation_targets_v1`, `window_previews_v1` and `fullscreen_desktop_v1` for the implemented adapter protocols |

Window `output` is its saved home or null, not a visibility test. `role` is
`normal` or `launcher`. `floating` is the saved exception flag, not the effective
mode. Minimized windows remain in snapshots with no placement. Maximized/minimized/fullscreen
fields and setters are additive v1 extensions; state transitions follow
[desktop policy](desktop.md#maximize-and-minimize).

Snapshots report desktop focus, not keyboard ownership held by a layer. Group
presentation describes underlying normal workspaces; `fullscreen_desktop`
overrides only its target output while active. `fullscreen_transient` identifies
related dialog windows sharing that presentation without another fullscreen flag.
Clients MUST NOT infer visibility from home alone. The optional Quickshell example
appends an `F` entry only on the slot output, activates it with the typed request,
and keeps task activation routed through normal window focus. Switcher candidates include minimized normal windows; focus
changes only upon acceptance, as specified in [input](input.md#alt-tab).

## Advisory panel icon targets

The additive `animation_targets_v1` capability supports optional shell-provided
minimize destinations. The animation adapter consumes accepted live reports
for enabled minimize motion and uses a panel/output fallback otherwise. Their absence,
rejection or expiry MUST NOT affect minimize policy, focus, client configuration,
or compositor operation. These requests do not run desktop reconciliation or
publish changed policy snapshots. Backend-independent `execute` refuses them;
only the adapter can validate live Wayland ownership and handle them.

`animation_panels` returns `{panel,output,namespace,width,height}` entries only
for mapped panels whose Wayland client PID and UID match the requesting Unix
stream's kernel credentials. Another process receives an empty list even if
it runs as the same user. Credentials are not accepted from clients. If kernel
credentials cannot be obtained, the request fails. This narrower association is
an identity check for this advisory capability, not same-user security isolation.

Eligible panels are on a connected output, in the effective top or bottom layer,
and either match a configured panel namespace or reserve a positive exclusive
zone at an output edge. Overlay/background surfaces and whole-output bodies are
excluded. The committed root surface view MUST begin at local `(0,0)`; panels
with a shifted root body are omitted because this narrow discovery format has
no body-local offset field. At most 32 mappings are discovered, in layer
registration order.
`panel` is a canonical decimal opaque mapping identity, not a client-supplied
namespace or output rectangle. It rotates after unmap/remap, output migration,
committed geometry/origin changes, or ownership changes. Old registrations and
identities are discarded immediately on unmap/destruction and during lifecycle
refresh. ID exhaustion omits new mappings rather than reusing an identity.

The `output` field in `set_animation_targets` MUST exactly match the discovered
live output ID. `targets` is an array whose entries have exactly one selector:

```json
{"window":"7","rect":{"x":120,"y":8,"width":20,"height":20}}
{"app_id":"foot","rect":{"x":120,"y":8,"width":20,"height":20}}
```

Rectangles use **panel-surface-local integer logical coordinates**. The adapter
derives their global origin from the actual committed surface and arranged
output, never from shell-supplied global coordinates. Extents MUST be positive
and at most 16384 each. Local origins MUST be nonnegative, arithmetic MUST not
overflow, and the entire translated rectangle MUST fit both the committed panel
dimensions and visible body clipped to full output bounds. Partially clipped
icons are refused rather than truncated. Unknown fields, mixed selectors,
duplicate selectors and noninteger geometry fail.

Window selectors MUST name live normal windows whose saved home is the panel's
output. App selectors use exact nonempty app IDs of at most 256 UTF-8 bytes,
without NUL; a matching live normal window on that home output MUST exist.
Hidden and minimized normal windows remain eligible. Launchers and foreign
home-output windows are refused. Resolution prefers exact-window selectors,
then exact app IDs, using the lowest live mapping identity for deterministic
selection within each selector type.

One live IPC connection owns a panel registration; another connection cannot
replace it until release, expiry or disconnect, including another connection
from the same process. An accepted complete replacement renews a five-second
monotonic lease; an empty replacement removes the registration. Validation is
atomic: a failed replacement retains the previous records and lease. Records
are discarded on lease expiry, disconnect, panel lifecycle/geometry changes,
output removal, target destruction/reclassification, or home-output migration.
Limits are 128 entries per panel, 512 entries total, and eight connections with
nonempty registrations, in addition to the transport limits below.

The prepared fallback helper uses the window's home full-output bounds. It
chooses an eligible panel body's center, preferring reserved panels and then
mapping identity, or the output's bottom center if no eligible panel exists.
The target is at most 24×24 logical pixels and fits the selected body/output.
Overlay launchers do not become fallback panels. Visible motion and retained
images follow the [animation presentation](animations.md) contract.

The Quickshell example discovers its own panels after subscription and once per
second. It matches output, namespace and committed dimensions unambiguously,
then reports the actual 22×22 app-icon item rectangles after layout and dock
scrolling and renews them once per second. Entirely visible icons with matching
live normal home-output windows are included; pinned-only and clipped icons are
omitted. It sends empty replacements when its dock becomes hidden and never
replays reports across disconnection. No shell process is required for fallback.

## Private window previews

`window_previews_v1` is an optional toolkit-independent capability on the existing
private socket, not a public Wayland capture protocol. Requests require a live
mapped panel whose Wayland PID/UID match the socket peer, using the same panel
eligibility as advisory discovery. Ownership is checked when queued and again
before GPU capture. A different process receives an error even for the same user.
Pure model execution rejects capture requests; only the adapter can fulfill them.

A request MUST name one to four distinct, canonical, mapped normal-window IDs.
All IDs and ownership MUST be validated before queueing; launchers, unknown IDs,
duplicates, empty batches and oversized batches fail atomically. Hidden-workspace
and minimized windows are eligible without revealing, restoring, moving, resizing
or focusing them. There is one pending batch globally and at least 200ms between
accepted batches globally; excess requests receive an error rather than queueing.
Disconnected peers, unmapped panels and disappeared clients cannot receive stale
captures. A window disappearing before capture yields `image: null`.

Capture uses the latest committed window body, subsurfaces, negotiated decoration
and rounded outline through the shared window-image composer. It excludes popups,
other windows, wallpaper and backdrop filtering. Images retain their aspect ratio,
fit within 256×144 pixels, and do not upscale small sources. Premultiplied pixels
are converted to straight-alpha PNG data URIs. GPU/readback/encoding failures yield
`image: null` for the affected window and MUST NOT terminate the compositor.
Capture readback MUST restore the host EGL target before ordinary composition.
No frame files or filesystem IO are introduced in the capture/render path.

Images are separate solicited replies, never state snapshots or subscriptions.
At most four transient small images/readbacks and the shared composer's existing
bounded scratch are used; no persistent per-window GPU image cache is added.
A batch's maximum uncompressed PNG/base64 payload remains below the transport's
1 MiB outgoing cap; slow consumers retain the existing bounded disconnect policy.
While polled, captured windows receive a 450ms renewable callback/suspension lease.
Nonvisible roots use the existing overview callback throttle. Lease expiry,
disconnect or lost panel ownership removes that eligibility without changing
minimized state or ordinary placements. Once polling stops, capture work stops;
the compositor retains only bounded ID/timestamp records until lease cleanup.
These previews are rate-limited committed frames, not presentation feedback or a
full-rate screencasting service.

## Transport bounds

The nonblocking transport exposes live listener/stream descriptors and opaque
connection identities to the adapter. The adapter registers listener/read
readiness and writable readiness only while that peer has queued output. It
removes disconnected registrations and read interest after a peer write-half
closes. Descriptor-number reuse MUST NOT apply an old readiness event to a new
connection. Transport IO, framing and budgets remain backend-independent; shell
work MUST NOT run in the renderer or create an unbounded background queue.

A signalled direction is drained within the budgets below. Reaching a work budget
retains actionable demand; complete buffered requests and newly queued replies
or publications schedule one coalesced next-turn continuation, even if no further
socket read event occurs. An EAGAIN clears that direction's actionable demand and
waits for kernel readiness; appending output to an already blocked queue MUST NOT
create a retry loop. Peer rotation still applies to readiness-driven dispatch,
including competing busy peers. Read-only requests and advisory icon reports wake
transport processing without invalidating unchanged pixels. Lease expiry uses the
nearest live registration deadline; an empty advisory store needs no expiry timer.

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
controls, and overlay lifecycle: `examples/quickshell/shell.qml` selects the
standard appearance, while `examples/quickshell/liquid-glass.qml` selects the
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
Below 600 pixels the overview button is also hidden, panel spacing tightens,
and workspace controls have a smaller viewport. Workspace buttons MUST scroll
horizontally when they exceed their viewport; changing the selected workspace
MUST bring its button into view. The application dock scrolls independently.

The standard appearance uses a 44-pixel panel inset 12 pixels horizontally and
8 pixels from the top, reserving 52 pixels total. Its painted and input silhouettes
have radius 16. Panel and overlay fills are `#171c29` at 237/255 opacity and
`#1c2232` at 245/255 opacity respectively. Overlays have radius 20, a subtle
outline, and opaque text/icons. Workspace selection uses a pale blue accent;
application selection uses a quieter tinted surface. Shared controls use
32-pixel hit targets (workspace buttons are 30×28), rounded hover/press fills,
and 140ms color transitions. Panel launcher, overview, notification and window
controls use bundled SVGs rather than font-dependent symbols. Application icons
in the dock, preview, launcher and switcher MUST fall back to the bundled generic
icon when the desktop entry, theme icon or loaded image is unavailable.

Application previews MUST wait for a one-second hover dwell. Leaving an application
before that dwell completes MUST cancel opening; moving to another application
starts a new dwell. Metadata-only snapshots MUST retain dock buttons by app ID
so updates do not interrupt the dwell. Leaving an open preview or its application
starts a 300ms close delay, allowing the pointer to cross the gap; hovering anywhere inside the
preview keeps it open, including its header, whitespace and window controls.
Clicking an app, opening the launcher/notification center, scrolling the dock,
or resizing the panel dismisses the preview. These popups MUST NOT grab keyboard
focus. Their anchor is centered on the actual hovered item, clamped within the
panel width, 8 pixels below the panel's bottom. They MUST expand downward and
MUST NOT vertically flip over the hovered control. Preview width is bounded by
350 pixels and the panel width minus 16; height is bounded by 560 pixels and the
space below the panel, with a scrolling window list. Each card includes a live
committed window image, title, workspace/state labels and window controls. Missing
or unavailable initial images fall back to the application's icon. Once a frame
has loaded, the card MUST retain it while the next frame decodes or a capture is
temporarily unavailable/invalid. A decoded replacement swaps in without an icon
flash. Metadata snapshots update cards by window ID without recreating them or
resetting scroll position. Each card retains at most two uncached decoded image
slots, one displayed and one loading; closing/removing its card releases them.
The shell requests images for up to four windows starting at the first visible
card, every 220ms, with at most one request in flight. Closing the hover popup stops requests.

Right-clicking an application MUST open a context menu, rather than immediately
toggling its pin. The menu includes the matched desktop entry's published actions
(up to 32), launch, pin/unpin, per-window minimize/restore, maximize/unmaximize,
fullscreen, move-to-workspace, and close. Grouped apps expose a window selector
and close-all action. Close-all sends individual close requests. Desktop actions
execute through Quickshell's desktop-action API without shell interpolation;
apps that publish no actions have no invented app-specific actions. This is not
an application's in-window File/Edit menu or a recent-documents service.
Opening the menu dismisses hover previews. A bounded output-local menu layer owns
keyboard input while open; Escape closes it. A separate fully transparent
output-sized dismissal layer catches outside clicks, with the menu's rounded
region subtracted from its input mask so it cannot block menu controls regardless
of layer registration order. Only the bounded painted menu opts into geometric
glass treatment, keeping refraction and blur inside its actual silhouette.
Closing the menu retains both roles as transparent 1×1 layers with empty input
regions and restores ordinary keyboard ownership through the existing lifecycle.
The shell asynchronously prepares the closed menu at startup. Its content keeps
its full layout while transparent, and opening requests its final bounded size
from the screen dimensions, independently of the dismissal layer's asynchronous
configure. Right-click has no hover dwell or opening animation. Tall menus scroll
within the output.

Shared tooltips MUST wait one second and have an empty input region. In the glass
appearance they receive the same independent backdrop treatment within their
10-pixel rounded silhouette; noninteractive does not mean optically untreated. Panel tooltips
use the same 8-pixel gap and downward placement, preventing them from intercepting
the hovered control. Applications with open windows use their preview instead
of a simultaneous tooltip. Pinned apps without windows retain a text tooltip.

Launcher and notification layers are placed relative to the panel-reduced usable
area: the launcher starts 16 pixels below it, the notification center 8 pixels.
Their heights are bounded by the remaining output height. The launcher has a
themed search field, selected result, keyboard hints and an empty-search message;
changing the query resets the selection and scroll position. Up/Down navigate
the results and keep the selection visible, Enter launches, and Escape closes.
Launching still uses the desktop entry without shell interpolation. Notifications
use shared card styling and a centered empty state. Opening either of these
overlays through the panel closes the other.

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
loading fails. The switcher does not capture window pixels. Launcher, notification
and switcher layers retain lazy creation and their transparent 1×1 closed state,
while the app-preview popup is hidden. Glass
styles share the interaction behavior above: shell commands, keyboard ownership, per-output
placement, and compositor-side filtering retain their existing contracts.
The QML draws its own rounded alpha and input masks; Clear's window corner-radius
setting does not shape layer-shell panels. Its `clear-glass-pill-*` and
`clear-glass-rounded-*` layer namespaces also tell Clear the opt-in optical outline
to fit to the painted layer bounds. The glass panel uses the `-rounded-popups`
opt-in for its hover cards; details are owned by
[rendering](rendering.md#liquid-glass). Other layer clients keep their own shape behavior.
No client-side blur shader is added. Style values are centralized in
`ShellStyle.qml`; the glass entry point does not copy the shell logic or autostart
another shell.

## Implementation and evidence

- [Model, validation, serialization](../src/shell/mod.rs),
  [transport and inline tests](../src/shell/server.rs),
  [adapter dispatch/reconciliation](../src/platform/smithay/shell.rs),
  [readiness registrations](../src/platform/smithay/shell_readiness.rs). Transport
  inline tests cover already-buffered request continuations, global-budget
  fairness, real socket write backpressure without immediate retry, queued replies,
  half-close interest changes and stale connection identities.
- [Hint store and inline tests](../src/shell/animation_targets.rs): strict
  selectors, connection/process ownership, atomic rejection, local/global clips,
  exact-window precedence, expiry, lifecycle pruning and provider limits.
- [Adapter inline tests](../src/platform/smithay/shell.rs): panel fallback
  priority, full bounds with nonzero origins, short-body/tiny-output fitting,
  and exclusion of shifted-body/whole-output panel geometry.
- [Private capture adapter](../src/platform/smithay/shell_previews.rs): panel
  authorization, global batch/rate bounds, shared committed-image composition,
  straight-alpha PNG encoding and lease cleanup. Its inline test covers base64
  padding; the UI fixture checks actual GPU frames.
- [Real Quickshell hint fixture](../scripts/vm-shell-hints-smoke.py): accepted
  nonempty icon reports, same-process discovery, foreign-process refusal,
  unchanged desktop policy, and fresh identities after panel restart. It does
  not establish scroll gesture, GPU pixel, physical input or animation behavior.
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
  [UI fixture](../scripts/vm-shell-ui-smoke.py) captures panel, preview, tooltip,
  launcher/search-empty, notification and context-menu scenes on a single output. Its QML timers
  exercise dwell cancellation and opening through the real example's hover
  handlers; real popup configure traces and GPU pixels check below-panel
  placement. Preview cases check the one-second dwell and require distinct real
  frames after minimizing a changing client. Changing window titles must retain
  one preview card; injected null and undecodable replies must not restore the
  fallback icon after a successful frame, and subsequent good frames must swap in. Context-menu cases execute a fixture desktop action and move a window
  without revealing its destination. Context-menu traces require preloading,
  dispatch under 150ms and a first configure at the final requested size, with no
  intervening tiny sizes on opening or reopening. This bounds dispatch/configure
  behavior, not actual input-to-presentation latency. A foreign socket process must be denied
  capture. `--width 480` exercises compact layouts; `--glass` checks the shared
  appearance. `--glass --verify-backdrops` enables actual compositor effects and
  compares patterned plain, blur-only, glass-only and combined GPU captures of
  hover cards, shared noninteractive tooltips and context menus, requiring
  distinct filter results and untouched rounded corners and exterior pixels. Merely selecting `--glass` styles the QML; it does not enable
  the compositor's filters in the ordinary UI fixture. This does not inject physical input or
  establish real pointer travel, keyboard delivery, or animation cadence.
  [Example model tests](../examples/quickshell/Protocol.test.mjs) cover its client
  helpers, not compositor protocol/GPU or physical input behavior.
