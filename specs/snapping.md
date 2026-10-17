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

The pointer can transfer the window to the visible workspace of another output.
`SnapTarget::at_pointer` recognizes twelve-logical-pixel zones inside the full
output bounds, with half-open output ownership and corner precedence. Top center
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

## Evidence

- [Core policy](../src/core/snapping.rs), [placement integration](../src/core/desktop.rs)
  and [resize invalidation](../src/core/resize.rs).
- [Deterministic tests](../tests/snapping.rs) cover exhaustive small odd partitions,
  output-local edge selection, reservations, saved geometry, maximize/fullscreen/
  minimize interleavings, role refusal, drag-out, release/cancel, output transfer,
  metadata/commit updates and changed-then-restored environment invalidation.
  These are CPU policy tests, not GPU or physical-input evidence.
