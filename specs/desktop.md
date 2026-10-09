# Desktop state

## Ownership and identity

Desktop policy MUST use backend-independent IDs and integer logical rectangles.
A managed window belongs to exactly one workspace and appears once in that
workspace's stable window order. Workspace identity is independent of its label.
An output group contains connected outputs and presents exactly one workspace.
Groups are disjoint; each connected output belongs to one group, and a workspace
MUST NOT be presented by more than one group at a time.

Each window has a home output. For a visible workspace, that home MUST belong to
its presenting group. Hidden windows retain workspace ownership. With no outputs,
windows and workspace state remain retained, but no placements or focused window
are produced. A home may be absent or refer to a disconnected output in that state.

Adding a new window assigns it to the active workspace/output, appends it to
workspace order, and focuses it. Its initial saved floating rectangle is centered
in the usable area, capped at 800×600; without an output it starts at `(0,0,800,600)`.
Adding an existing window ID updates metadata only. Metadata updates MUST NOT
reset ownership, order, focus, or saved geometry. Removal cleans up remembered
focus and sizing entries, including entries retained in other workspaces/modes.

## Workspace presentation and topology

- A new output gets an unused workspace; core can create additional workspaces
  when all existing ones are visible. A returning output prefers its previously
  detached workspace only if that workspace is still available.
- Switching to a hidden workspace changes the active group's presentation.
  Switching to one already visible swaps the two groups' workspace presentations,
  including when either group spans multiple outputs. Windows are never duplicated.
- Stretch joins all connected outputs into one group showing the active workspace.
  Other workspaces remain intact and hidden.
- Unstretch splits the active group. The focused output keeps its workspace;
  the other outputs get unused workspaces. It does not partition window ownership
  into newly created copies.
- Removing an output repairs affected homes using surviving outputs. Removing a
  separate group can hide its workspace; removing a member of a stretched group
  keeps that workspace visible. Disconnecting everything MUST retain policy state
  for reconnection.

Outputs retain separate full `bounds` and usable `area` rectangles. Normal layouts,
launchers, and maximized windows use `area`; fullscreen uses `bounds`.
`add_output_with_bounds` and `set_output_bounds` normalize full bounds and MUST
ignore empty results atomically, including metadata and topology changes. Updating
usable area MUST NOT overwrite full bounds or saved floating geometry. Usable area
MAY be empty when entirely reserved. The legacy `add_output` initializes both
rectangles from its area; an empty update retains existing full bounds. A newly
added empty legacy output has no fullscreen placement until valid bounds arrive.
Repeated valid output IDs update metadata/geometry without changing presentation.

The core accepts arbitrary logical output origins. Current runtime topology
and backend limits are defined in [configuration](configuration.md#reload) and
[platform](platform.md#virtual-outputs).

## Focus and commands

Focus is global desktop policy, with remembered focus per workspace/output region.
`FocusOutput` restores suitable focus in that region; `CycleOutput` visits output
IDs in order. Ordinary next/previous window focus cycles workspace order across
the active group, excluding minimized windows. [Alt-Tab](input.md#alt-tab) has a
separate candidate policy.

Explicit `Focus(window)` reveals a hidden workspace when outputs exist, repairs
the window's home, restores it if minimized, and focuses that window/output.
Workspace switching alone MUST NOT restore minimized windows.

`MoveToWorkspace` moves the focused window without following it to the destination.
`MoveWindowToWorkspace(window, workspace)` targets a specific window without
revealing or restoring it, following the destination, or changing unrelated focus.
If it moves the focused window, ordinary focus repair selects a remaining source
window. Invalid IDs and same-workspace targets are complete no-ops.
`MoveToOutput` moves it to the destination output's visible workspace and follows
it. Transfers preserve saved window state; crossing workspace ownership appends
the window to the destination's order. Invalid target IDs MUST be harmless no-ops,
including no incidental scroll or focus change.

Commands that spawn, close, or quit return explicit effects. Desktop policy MUST
NOT execute processes, destroy a client in response to a close request, or stop
the backend itself. The adapter performs these effects and observes later client
lifecycle changes.

## Overview preview

The transient [overview](overview.md#preview-and-activation) reads desktop records
and commits through existing commands. It does not own workspaces, compute hidden
placements, or mutate saved desktop state during navigation.

## Modes and saved geometry

Effective layout mode is the workspace-specific output override, if set, otherwise
the workspace default. Changing the default preserves explicit overrides. Cycling
a region's mode stores an override; clearing it exposes the current default.
Scroll offsets, remembered focus, and layout sizing belong to workspace/output
pairs, with sizing further scoped by mode.

Saved floating rectangles are independent of tile placements and client-committed
sizes. They MUST survive mode changes, temporary output shrinkage, and panel
reservations. Floating rectangles can extend offscreen or across outputs; they
MUST NOT be fitted back into the usable area. `SetFloatingRect` normalizes geometry
without changing the floating flag. See [surface clipping](platform.md#scene-and-hit-testing)
for the limits on their visible/input regions.

Floating exceptions do not consume tiles or scrolling content in tiled modes.
Launcher roles and maximized/minimized/fullscreen windows are also excluded from
normal layout inputs. Placement order is back-to-front: tiles, normal floats,
maximized windows, fullscreen windows, then launchers. A focused normal window
may rise above a maximized neighbor on its output; fullscreen remains above that
window. Launchers remain above all normal windows, including fullscreen.

## Maximize and minimize

Maximized and minimized are independent flags, not layout modes or mapping state.
Both operations MUST preserve workspace order, floating flag, saved floating
rectangle, and persistent layout proportions.

Maximize places a normal window in its home output's usable rectangle with an
explicit clip. It follows reservation and output-size changes and MUST NOT span
the whole stretched group. Restoring uses the current layout or unchanged saved
floating rectangle. A normal window must be restored before an interactive move
or resize.

Minimize removes placements, layout participation, and ordinary focus-cycle
eligibility, but retains the managed window and its maximized flag. Focus is
repaired if necessary. `SetMinimized(window, false)` restores without explicitly
revealing its workspace or focusing it; ordinary focus repair can select it if no
other suitable focus exists. Explicit focus or accepting Alt-Tab restores and
focuses the selected window.

Launchers reject maximize/minimize/fullscreen. Late classification as a launcher
clears all three flags while preserving ownership, order, and saved floating state. Launcher
classification and sizing are specified in [configuration](configuration.md#shell-rules)
and [platform](platform.md#launcher-geometry).

## Fullscreen

Fullscreen is an independent normal-window policy flag exposed by
`ToggleFullscreen` for the focused window and `SetFullscreen(window, bool)` for
an explicit window. Setters MUST NOT reveal a hidden workspace, transfer ownership,
restore minimized state, or explicitly focus the window. Invalid IDs and launcher
roles are harmless no-ops. A headless or hidden window MAY retain the flag before
it receives a visible output placement.

A visible fullscreen window MUST occupy and clip to its home output's full,
nonempty `bounds`, without panel reservations or layout gaps. It MUST NOT span a
stretched output group. Bounds updates and ordinary topology/home repair determine
its current rectangle; disconnecting all outputs retains its flags and saved
state while producing no placements. Empty full bounds produce no fullscreen
placement, rather than an invalid size.

Fullscreen MUST preserve workspace order, floating state, saved floating geometry,
persistent layout proportions, and the underlying maximized flag. Setting or
toggling maximize during fullscreen updates the restore state without changing
fullscreen placement. Minimization hides fullscreen and preserves both fullscreen
and maximized flags; explicit focus restores it to fullscreen. Exiting fullscreen
while minimized leaves it minimized. Exiting while visible restores to maximized
usable area or the current ordinary layout/saved floating rectangle as appropriate.
Newly remapped windows start with fullscreen disabled.

Fullscreen MUST bypass normal built-in and custom layout inputs, including
scrolling content. Entering or leaving fullscreen invalidates resize sessions in
its region, including a session that outlives a fullscreen/restore round trip.
A fullscreen window cannot start an interactive resize. Layout proportions remain
retained for restoration.

Multiple fullscreen windows use deterministic stable placement order, with the
globally focused fullscreen window placed above other fullscreen windows. Focusing
an ordinary or maximized window does not lift it above fullscreen. Launchers still
stack above fullscreen at the core boundary. Protocol layer ordering and hit
authorization belong to the adapter.

This section specifies desktop policy. XDG request/state handling, protocol
capability advertisement, requested versus committed geometry, layer stacking,
and decoration behavior are separately specified in [platform](platform.md) and
[decorations](decorations.md); a core command alone does not establish those
capabilities or animated presentation.

## Implementation and evidence

- [Desktop](../src/core/desktop.rs), [types and commands](../src/core/types.rs),
  [geometry](../src/core/geometry.rs).
- [Core tests](../tests/core.rs): workspace swaps, stretched groups, disconnected
  outputs, hidden windows, focus restoration, saved geometry, clipping inputs,
  and deterministic random-command invariants. Fullscreen cases cover every mode,
  saved geometry/order, full versus reserved bounds, stretched home output repair,
  headless/hidden setters, maximize/minimize interleavings, deterministic stacking,
  launcher rejection, positive bounds validation, and persistent resize proportions.
- [Window-state tests](../tests/window_state.rs): preservation across every mode,
  minimized focus behavior, maximize reservations, role changes, shell setters,
  and resize-session cancellation/restoration across fullscreen.
- [Window-state fixture](../scripts/vm-window-state-smoke.py): protocol and GPU
  assertions; it does not synthesize physical keyboard or pointer input.
