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
Client frame callbacks use an independent monotonic delivery timestamp.
[Animation sampling](animations.md)
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

## Demand-driven scheduling

Clear MUST request host redraws only for visual invalidation, active compositor
motion including an unsubmitted terminal pose, or host exposure. Visual generations
are independent of policy dirty/reconciliation state. A successful submission
acknowledges only the generation actually composed; later invalidation MUST survive.
At most one Clear-requested host redraw is pending. Active motion continues using
host opportunities and the sampling rules above. Completing motion MUST stop new
requests once the final pose and any overview-to-desktop handoff are submitted.
An unchanged scene with no eligible callbacks or deadlines sleeps indefinitely.

Eligible committed frame callbacks MUST continue advancing without rebuilding or
submitting identical pixels. Callback-only delivery uses absolute deadlines based
on advertised host refresh, skipping missed opportunities without catch-up bursts.
It does not count as a rendered frame, acknowledge animation poses, retire GPU
images or claim presentation feedback. Visual demand delivers callbacks after a
successful submission instead; an overdue callback timer MUST NOT spin while a
host draw is pending. Overview retains its existing bounded/throttled eligibility.

Protocol commits/lifecycle changes, compositor input/policy changes, reload and
host exposure wake demand through a coalesced Calloop ping. Callback-only commits
with unchanged effective visual state wake callback scheduling without repainting.
The explicit [rounded-popup glass opt-in](rendering.md#liquid-glass) makes an
empty/nonempty input-region transition part of that popup's visual state while
effects are enabled; it therefore repaints. Ordinary input-only changes retain
the nonpainting behavior below.
When the compositor owns pointer routing, committed input-region-only changes
that leave effective visual state unchanged MUST refresh stationary pointer
ownership from last-submitted presentation geometry without requesting pixels. This refresh sends
motion/frame only if surface identity, origin or inverse scale changed; repeated
unchanged callbacks MUST NOT inject identical pointer motion. Submitted animation
frames retain their unconditional pointer-coordinate refresh.
New buffer assignment or an unknown change is conservatively visual. Animation
startup uses current monotonic time, including after a long idle period.

The event loop has one cancellable earliest-deadline timer for callbacks,
[diagnostic idle indication](rendering.md#fps-overlay), capture, bounded shutdown
and maintenance. Shell transport uses readiness as specified by [shell](shell.md).
Children are reaped at one-second maintenance deadlines only while children exist;
advisory targets schedule their nearest expiry. A blocked Wayland output flush
uses a bounded 16 ms retry only while a per-client backend flush reports
`WouldBlock`; the pin's bulk flush discards those errors.
There is no blanket 16 ms maintenance timeout.

Capture forces a full current frame at its own deadline and retains demand until
a drawable host opportunity. Bounded shutdown is independent of successful
rendering. Host suspension/occlusion is not exposed by the pin, so capture can still
be delayed by missing drawable opportunities; host focus loss does not imply
occlusion. Ordinary unfiltered client frames use actual current-window EGL buffer
age and the damage tracker's returned backbuffer repair rectangles for submission.
Unknown age, new buffer content without valid reported damage, initial draw,
exposure, resize, capture, policy/reload changes, active
compositor motion and enabled blur/glass force a complete redraw. The adapter MUST
resize before making the window current and querying age; an offscreen target or
stale EGL surface MUST NOT supply buffer age. Resize replaces damage history.
Damage-free scenes MAY acknowledge their visual generation without swapping,
counting FPS or acknowledging an animation pose. Active motion and terminal poses
are forced complete frames so their submission acknowledgement cannot be skipped.
[Rendering](rendering.md) owns generated element identities and content versions.

When `CLEAR_RENDER_STATS=1` is present, shutdown emits one bounded JSON summary
with redraw requests/events, successful submissions, callback-only deliveries,
diagnostic frames, serviced loop turns, scene-work submissions, the last scene-work
submission time, full/partial frames and damage-free skipped draws. Scene-work
counters include compositor motion and Commit/UncertainCommit/Policy/Input/Metadata/Lifecycle
reasons, excluding forced exposure, capture and diagnostic-only draws. The counters are test diagnostics,
not physical presentation measurements; no per-frame log or publication is emitted.

[Rendering](rendering.md#incremental-nested-damage) owns the bounded
`CLEAR_PARTIAL_FRAME_CAPTURE=PATH` diagnostic, including uncertain-buffer recovery
readbacks. Shutdown statistics include `partial_captures`, `uncertain_frames` and
the last uncertain submission time. Readback/fence costs perturb these frames;
they MUST NOT be treated as an active-frame performance benchmark.

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

When [animations](animations.md) are enabled, configures MUST still request the
final placement once. Rendering fits the latest committed body/SSD image to the
sampled frame, including while the client holds a resize/fullscreen commit.
The final fitted image MUST remain until committed dimensions and fullscreen
appearance match; reaching the animation endpoint MUST NOT snap back to old
content bounds. SSD/fullscreen appearance still changes only at a buffered root
commit. Popup placement and inverse pointer coordinates MUST follow the same
submitted transform. With motion disabled the ordinary requested/committed clips
remain in effect. Launchers and layer-shell clients are outside window animation
ownership.

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
full cursor-surface handling, and activation policy
are not implemented. Group editing exposes stretch-all and split, not arbitrary
output subsets. Public Wayland toplevel listing/capture protocols are not exposed;
the optional [private shell preview capability](shell.md#private-window-previews)
provides bounded committed-window images to same-process panel clients.
Layer and popup support is intentionally limited to the implemented
adapter paths; this is not a promise of every desktop protocol.

The backend uses demand and damage tracking with conservative complete-frame
fallbacks and the timing limitations described above. Rounded/blur coordinates assume one scale-1
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
  `redraw_requests_coalesce_until_the_host_opportunity`,
  `callback_deadlines_skip_late_ticks_without_immediate_reentry`),
  [visual generation bookkeeping](../src/platform/smithay/render_demand.rs)
  (`later_invalidation_survives_submission_of_earlier_scene`,
  `diagnostic_work_does_not_restart_visual_activity`),
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
