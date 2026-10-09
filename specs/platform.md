# Platform and surface lifecycle

## Supported backend and CLI

Clear currently runs through nested winit in an existing Wayland/X11 desktop.
Smithay and Wayland objects belong exclusively to `src/platform/smithay/`.
The adapter owns protocol state, input serials, client mapping, configure timing,
renderer resources, and execution of core effects.

| Option | Behavior |
| --- | --- |
| `--config PATH` | Select the [configuration](configuration.md) file |
| `--socket NAME` | Select a nonempty Wayland socket filename; paths, `.` and `..` are rejected |
| `--no-shell-ipc` | Disable the optional [shell bridge](shell.md) |
| `--exit-after SECONDS` | Exit after an integer duration in 1–86400 seconds |
| `--capture PATH` | Write a PPM framebuffer near the exit deadline, or after three seconds without a deadline; overwrites the destination |
| `--command PROGRAM [ARG...]`, `-c` | Spawn an executable/argument vector; consumes all remaining arguments and must be last |
| `--help`, `-h` | Print usage without starting the compositor |

Unknown options and missing required values fail argument parsing. Without a
command the compositor starts with an empty desktop. Closing the outer window or
executing the Quit effect ends the session.

Spawned clients receive this instance's `WAYLAND_DISPLAY` explicitly, without
mutating the process-wide environment. They receive its `CLEAR_SOCKET` when the
bridge exists; an inherited endpoint MUST be removed when this instance has none.
Spawning is not shell-string evaluation.

## Virtual outputs

Configured outputs are virtual `wl_output`s within one host framebuffer, arranged
horizontally at scale 1. Widths are relative weights of the host width; heights
scale relative to the maximum configured output height. Host resizing recomputes
those rectangles, output modes, and usable areas. All virtual outputs share the
host framebuffer's advertised refresh, as described below. These outputs are not
physical DRM/KMS connectors.

## Nested frame timing

The backend MUST request synchronized EGL swaps at interval 1, compose into the
back buffer, and submit serially through Smithay. It MUST NOT select a larger
swap interval in response to late frames. Clear coalesces its own redraw requests;
it does not maintain a queue of obsolete animation frames. EGL and the host
compositor may allocate additional internal buffers, so this does not guarantee
an exact two-buffer physical swapchain or a particular delivered frame rate.

Each redraw captures one monotonic timestamp relative to compositor startup.
Client frame callbacks share that timestamp. [Animation sampling](animations.md)
has a separate captured timestamp: fixed caps select host opportunities nearest
absolute 30/60/120 Hz deadlines and hold that timestamp between samples, while
refresh-rate sampling selects every redraw. Missed deadlines MUST be skipped,
not replayed; caps MUST NOT change normal client callback or repaint cadence.
The effective cap is bounded by the advertised host refresh, without rounding to
an integer divisor or lowering the configured target after a slow frame.

The timing source is the host window's current monitor/video-mode metadata when
the pinned winit API supplies a positive representable rate. The backend checks
it on every redraw, updates virtual output modes on change, and resets sampling
phase on monitor, refresh/source, or configured cap changes. When metadata is
unavailable, it MUST report and advertise a nominal 60000 mHz estimate. It MUST
NOT infer a display rate from measured compositor throughput. Animation caps do
not change advertised `wl_output` refresh.

The pinned Smithay backend requests host frame callbacks before swaps and exposes
host redraw events, but does not expose presentation feedback, an actual presented
timestamp, explicit readiness, or occlusion/monitor-move notifications. Captured
times are redraw times, not predicted or confirmed presentation times. Monitor
metadata is not a guarantee about host presentation. Interval-1 initialization
and CPU scheduling tests alone do not establish physical synchronization.

Continuous host redraw requests remain in place to preserve client/layer callback
opportunities, overview preview throttling, captures, and content updates. The
scheduler adds no animation timer or separate wakeup loop. It does not implement
damage-driven idle scheduling or suspension handling. The event loop's 16 ms
timeout remains for maintenance and bounded shutdown; it is not an animation
sampling timer or a requested frame-rate ceiling. A scheduled capture still
depends on a drawable host redraw opportunity; an occluded/suspended host can
delay it. Explicit invalidation and capture timers are future work.

## XDG window lifecycle and configures

A stable window ID is allocated on toplevel creation. The first buffer map adds
the window to desktop policy; a null-buffer unmap removes it from policy. The
adapter retains the unmapped toplevel handle so the same object can map again.
Actual destruction removes that handle. Metadata changes update the existing
window and apply current classification rather than recreating it.

Initial XDG configuration belongs to the bufferless-commit path. Reconciliation
configures changed desired geometry/state and maps visible placements into the
scene. It MUST distinguish compositor-requested frame/content size from the
client's committed buffer and window geometry. Resizing is asynchronous: delayed
clients do not block all other clients in a global transaction. Old tiled content
remains clipped while waiting for new content.

XDG maximize/unmaximize/minimize/fullscreen/unfullscreen requests target core
window state. Pre-map maximize and fullscreen are retained for the first map and
reflected in initial size/state. A connected requested fullscreen output is used
for initial placement; a mapped normal window transfers through ordinary
focus/move commands before setting fullscreen. An absent or unavailable output
uses the window's home/focused output. Fullscreen requests MUST NOT affect
launcher policy. Rejected requests still receive a configure.

Reconciliation owns Maximized, Fullscreen, and Suspended state; the
[overview contract](overview.md) provides temporary protocol visibility for live
previews without restoring core minimized state. Minimizing removes ordinary
rendering without marking the client unmapped. State changes cancel incompatible
drags. Capabilities advertise maximize/minimize/fullscreen, but not window-menu
policy. See [desktop state](desktop.md#fullscreen) for saved-state and placement
semantics, including full bounds distinct from usable area.

Fullscreen configure sizes use full output bounds without SSD insets. Displayed
SSD/rounding suppression follows the fullscreen bit in the last ACKed configure
at a buffered root commit; an ACK alone MUST NOT change it. Unmap clears committed
fullscreen and pre-map intent. Delayed client commits do not block other clients;
policy placement and scene priority may change before client content catches up.

XDG popups are tracked separately from workspace-owned toplevels, configured and
repositioned using their parent origin and output constraints. Layer popups can
extend outside their panel's body. Decoration mode, geometry offsets, and popup
origins MUST agree as specified in [decorations](decorations.md).

## Launcher geometry

Classified XDG launchers bypass built-in and scripted layouts. They remain in
their workspace and are centered in their home output's usable area, above all
normal windows. Their last positive committed dimensions determine size; before
valid dimensions exist, saved floating dimensions are the fallback. Invalid size
updates retain the last valid size.

A subsequent client commit updates size and center without replacing saved
floating geometry. Oversized launchers MUST retain their client size rather than
be fitted to the output. Negotiated titlebar height is added to committed content
height before centering the frame. Reservation changes recenter launchers without
changing their saved float rectangles. Layer-shell launchers follow the layer
protocol instead of this XDG classification policy.

## Layer-shell lifecycle

Layer objects belong to the adapter, not to core window/workspace ownership.
The client's supplied output is used when resolvable, otherwise the focused
output; without an available output the surface is closed.

Initial bufferless configuration may temporarily arrange a layer, but MUST NOT
reserve desktop space or take keyboard ownership. Reservations begin with buffer
mapping. A null-buffer unmap or destruction releases reservations and focus.
After unmap, a fresh bufferless commit starts a new configure/ack mapping handshake.
Existence of a protocol object alone never implies a mapped reservation.

Arrangement is per output; the resulting usable rectangle is sent to core.
Panel changes retile normal windows and recenter launchers without clamping saved
floating rectangles. [Namespace rules](configuration.md#shell-rules) change only
the effective current layer. Pending client state MUST remain untouched until its
own commit; another client's commit cannot apply or lose a pending `set_layer`.

Mapped exclusive overlay/top surfaces preempt application keyboard focus, with
overlay above top. Within a layer, mapping order determines priority, including
after unmap/remap. Background/bottom exclusive layers do not automatically preempt
application focus. On-demand keyboard ownership follows clicks on eligible layers.
Unmapping the owner restores the next eligible layer or application. Desktop
policy focus remains distinct from protocol keyboard ownership.

## Overview

[Desktop overview](overview.md) defines compositor input ownership, layer/grab
precedence, focus reconciliation and committed-buffer previews.

## Scene and hit-testing

Front-to-back priority is overlay, top, application placements, bottom, background,
then wallpaper on outputs without visible fullscreen. On a fullscreen output,
priority is overlay, launchers homed there, fullscreen windows, top layers, other
application placements, bottom, background, then wallpaper. Top-layer input is
partitioned by the output under the pointer to preserve the render priority when
an oversized launcher crosses output boundaries. Exclusive layer keyboard
ownership remains independent of this visual ordering. Layer popup trees participate above their parent layer body and
are not cropped to that body. Within application content, rendering and input use
the same placement order, origins, and visible workspace/output-group clips.

Floats and their popups may cover the union of full output rectangles presenting
their workspace, excluding gaps and independently presented groups. Tiled bodies
additionally respect requested rectangles; tiled/scrolling viewports respect
panel-reduced usable areas. Hidden workspaces MUST NOT render or receive hits.
Output crops MUST NOT introduce extra rounded corners or reposition titlebar
controls. [Rendering](rendering.md) owns outline and alpha rules.

An input-region miss continues to the next eligible surface. Rounded cut-outs
pass hits underneath. Decoration hits are compositor-owned, not Wayland surface
pointer focus. A layer hit MUST NOT focus or start dragging the application under
it. Popup/layer geometry is not subjected to application body rounding.

## Current limitations

Native DRM/KMS, physical monitor hotplug, XWayland, fractional scaling, IME,
visual animation effects, full cursor-surface handling, and activation policy
are not implemented. Group editing exposes stretch-all and split, not arbitrary
output subsets. Toplevel listing/capture protocols for live shell thumbnails are
not exposed. Layer and popup support is intentionally limited to the implemented
adapter paths; this is not a promise of every desktop protocol.

The backend currently repaints continuously with buffer age zero, with the timing
limitations described above. Rounded/blur coordinates assume one scale-1
framebuffer; a future native/scaled backend cannot
reuse those assumptions without adaptation. Physical keyboard/pointer behavior
is not established merely by CPU tests or protocol/capture fixtures.

## Implementation and evidence

- [CLI and parser tests](../src/main.rs), [backend](../src/platform/smithay/backend.rs),
  [frame scheduler and tests](../src/platform/smithay/frame_scheduler.rs)
  (`fixed_caps_keep_absolute_phase_on_144_hz`,
  `missed_frames_do_not_replay_or_lower_the_target`,
  `callbacks_can_redraw_between_animation_samples`,
  `source_and_config_changes_reset_phase_without_stale_samples`,
  `redraw_requests_coalesce_until_the_host_opportunity`),
  [protocols](../src/platform/smithay/protocols.rs),
  [layers](../src/platform/smithay/layers.rs),
  [scene and clip tests](../src/platform/smithay/scene.rs).
- [Core tests](../tests/core.rs): launcher sizing and usable-area boundary.
- [Layer fixture](../scripts/vm-layer-smoke.py) and
  [fixture assertion self-tests](../scripts/test_vm_layer_smoke.py): mapping,
  reservations, pending layer commits, popup pixels, keyboard ownership, and
  client-sized launchers. Assertion self-tests alone are not integration runs.
- [Fullscreen fixture](../scripts/vm-fullscreen-smoke.py) and
  [action/IPC tests](../tests/fullscreen.rs) cover fullscreen protocol states,
  full bounds, output requests, held commits, and normal/launcher policy.
- [Window-state fixture](../scripts/vm-window-state-smoke.py) and
  [decoration fixture](../scripts/vm-decoration-smoke.py): native state/configure
  transitions and framebuffer assertions. See [testing](../docs/vm-testing.md)
  for commands and the scope of historical runs.
