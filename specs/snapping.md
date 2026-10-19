# Floating-window snapping

## Placement policy

`SnapTarget` describes maximize, left/right halves, and the four named quadrants
of one output's usable area. Its geometry MUST exclude panel reservations and
MUST NOT span a stretched output group. Integer splits assign the remainder to
the right/bottom so odd-sized halves/quadrants have neither overlap nor gaps.
Empty targets are rejected. Halves require width at least two; quadrants also
require height at least two. No target may create a zero-sized placement.

`Desktop::snap_window` is a nonrevealing, nonfocusing operation on a normal,
nonminimized, nonfullscreen window with a valid home output and target geometry.
Invalid requests MUST leave policy unchanged. Successful snapping makes the
window a floating exception. Top-edge maximize uses the existing maximized flag;
halves/quadrants use `Window::snapped` without setting that flag. Snapped windows
MUST appear exactly once and MUST NOT consume normal layout slots.

The saved floating rectangle MUST remain independent of the snap override.
Reservations, output resizing, layout-mode changes and transfers recompute snap
placement without replacing saved free geometry. An unrepresentable target after
an output shrink has no placement until usable geometry recovers. Maximize and
fullscreen temporarily cover a half/quadrant override; exiting them restores it.
Minimize retains the override. Launcher classification clears it. Explicit
`SetFloatingRect` edits and `ToggleFloating` clear it; a floating-rectangle edit
MUST NOT implicitly clear the existing maximized/fullscreen flags.

## Core move sessions

`MoveSession` is an opaque baseline for one visible normal-window move. Fullscreen
and minimized windows and launchers reject moves. The caller supplies the actual
visible policy frame, including a custom layout's geometry. Beginning a move
focuses the window and promotes it to floating. For a snapped/maximized source,
restore its saved free size with the normalized pointer anchor over that frame;
otherwise keep the supplied frame size. Movement MUST use total displacement
from that baseline, without easing pointer translation.

Core sessions enable snapping by default. The caller MAY disable snapping with
`MoveSession::set_snapping_enabled`; motion and the cached target still update,
but preview queries and release MUST NOT snap until reenabled. Reenabling exposes
the latest valid target without requiring another pointer update. Core owns no clock.

The pointer can transfer the window to the visible workspace of another output.
`SnapTarget::at_pointer` recognizes twelve-logical-pixel edge strips and
64-by-64-logical-pixel corner zones inside the full output bounds. A corner can
be selected without touching either edge. Corner zones take precedence over edge
strips; each corner extent clamps to half the output dimension so opposing zones
never overlap. Output ownership remains half-open. Top center
selects maximize, side centers select halves, and bottom center has no target.
Preview geometry uses the target output's usable area, independently of those
full-output trigger zones. Unsupported tiny targets, output gaps, out-of-bounds
and nonfinite pointer positions MUST NOT select a snap target.

Updating a move MUST NOT resize into the proposed snap target. Successful finish
applies the last valid preview, preserving the pre-drag free rectangle for later
restore. Finishing without a target keeps the moved floating rectangle. Cancel
restores original workspace/output, stable window-order position, floating flag,
maximized/snap state and saved rectangle. It MUST NOT roll back client metadata,
committed size, or unrelated windows.

Policy/focus, output bounds and resize-environment revision changes invalidate a
move, including changed-then-restored environment state. Metadata and committed
client-size updates do not. Invalidated sessions MUST NOT snap or roll back newer
policy. Their preview queries return no target. Dropping a session owns no engine,
protocol or GPU resource.

## Drag input and presentation

The adapter uses the same move session for Super+left drag, SSD titlebar dragging
(after the existing four-pixel threshold), and authorized XDG move requests.
The existing client pointer-grab serial and same-client origin checks remain
required. A plain titlebar click MUST NOT restore or move a snapped/maximized
window. During a move, Escape and host focus loss cancel; only the initiating
button's release commits. Consumed compositor presses/releases remain paired,
including an Escape press/release. Overview takes no snap gesture ownership.

Snapping MUST stay disabled for the first 500 ms of each move gesture, measured
from successful drag initiation using monotonic elapsed time, independently of
animation speed or frame sampling. For SSD titlebars this starts after the
movement threshold. Pointer-following remains immediate, and release before the
deadline keeps the moved free rectangle. Once the deadline is reached, snapping
stays enabled until that gesture ends: leaving/reentering zones, switching targets
or crossing outputs MUST NOT restart the delay. The next move starts a new delay;
resize gestures have none. Release MUST check elapsed time even if the timer has
not yet dispatched.

The existing earliest-deadline event source MUST wake once to unlock a held move,
including a stationary pointer in a target zone. It MUST request pixels only if
a valid preview becomes visible, with no repeated delay timer after unlocking.
Cancellation, release and invalidation remove the pending deadline.

A valid pending target MUST show a noninteractive rectangle over normal
application windows, below shell top/overlay trees and elevated fullscreen trees.
Its geometry is the prospective frame; it has no input target and MUST NOT
resize/configure the client to the target before release. It uses the active
border RGB with a 0.16-alpha fill and a 0.8-alpha, at-most-two-pixel outline,
clipped to that target. Preview corners MUST use the configured window corner
radii, including asymmetric/zero corners and proportional fitting for tiny
rectangles; the preview outline stays inside its target independently of window
border thickness. Rounded previews use one pixel-shader element with the shared
window coverage function; square previews use at most five solid elements. Both
paths use one unfiltered scene group and no thumbnail, offscreen target, filter
invocation or retained GPU image. Leaving the zone, cancellation, invalidation
and release remove it.
A stationary held gesture MUST NOT by itself require continuous rendering.

Drop uses the existing `window_movement` effect for halves/quadrants and
`maximize` for the top target, with ordinary animation disable/reduced-motion,
speed, pacing and asynchronous-buffer behavior. Pointer-following and drag-out
remain immediate. No new animation law or configuration option is introduced.
Snapped bodies MUST use their requested frame for both rendering and input clips
while old committed client buffers remain larger; popups retain the normal
output viewport clip. Halves/quadrants remain floating XDG windows, not tiled
layout members; top-edge maximize advertises ordinary XDG maximization.

An explicit resize of a half/quadrant leaves the override at its current shown
rectangle, then uses an ordinary floating resize session. Maximized windows
reject resize until restored. Fullscreen and launcher windows reject both
ordinary move and resize. Invalid resize edges MUST NOT detach a snap override.
Core `begin_resize` itself rejects a retained override; the adapter's explicit
floating-rectangle edit performs the transition first. Ordinary resize cancellation
ends at the current geometry, without applying move-session rollback.

## Evidence

- [Core policy](../src/core/snapping.rs), [placement integration](../src/core/desktop.rs)
  and [resize invalidation](../src/core/resize.rs).
- [Deterministic tests](../tests/snapping.rs) cover exhaustive small odd partitions,
  output-local edge selection, reservations, saved geometry, maximize/fullscreen/
  minimize interleavings, role refusal, drag-out, release/cancel, output transfer,
  metadata/commit updates and changed-then-restored environment invalidation.
  These are CPU policy tests, not GPU or physical-input evidence.
- [Adapter input/seat regression](../src/platform/smithay/input.rs),
  `snapping_release_escape_drag_out_and_stale_target_use_the_real_seat_paths`,
  covers all seven target releases, bounded preview-element geometry/removal,
  resize detachment, maximized drag-out, paired Escape suppression and stale
  target invalidation. Injected monotonic times cover the exact delay boundary,
  stationary activation, instant target changes, per-gesture reset, early drop,
  release-before-timer dispatch, and absence of repeated timer/redraw demand.
  It uses synthetic seat input without GPU or physical
  device injection. [Scene assembly](../src/platform/smithay/scene.rs) owns the
  unfiltered preview and requested-body clips.
- `snap_preview_reuses_window_radii_and_fits_inside_the_target` in
  [rounded renderer tests](../src/platform/smithay/rounded.rs) checks matching
  window radii, asymmetric/square corners, bounded outlines and tiny-target fitting.
- `generous_corner_zones_work_without_touching_edges_and_leave_edge_centers_available`
  in [policy tests](../tests/snapping.rs) checks near-corner selection, preserved
  edge centers, output offsets and clamping on tiny outputs.
