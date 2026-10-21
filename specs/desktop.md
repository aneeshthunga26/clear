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
without changing the floating flag. Explicit rectangle edits also clear any
[half/quadrant snap override](snapping.md); that override otherwise preserves free
geometry independently of its current output placement. See [surface clipping](platform.md#scene-and-hit-testing)
for the limits on their visible/input regions.

Floating exceptions do not consume tiles or scrolling content in tiled modes.
Launcher roles, snapped windows and maximized/minimized/fullscreen windows are
also excluded from normal layout inputs. Ordinary placement order is back-to-front:
tiles, normal floats, maximized windows, then launchers. A focused normal window
may rise above a maximized neighbor on its output. The dedicated
[fullscreen presentation](#fullscreen) overrides ordinary placement on its target
output and excludes unrelated launchers.

## Maximize and minimize

Maximized and minimized are independent flags, not layout modes or mapping state.
Both operations MUST preserve workspace order, floating flag, saved floating
rectangle, and persistent layout proportions.

Maximize places a normal window in its home output's usable rectangle with an
explicit clip. It follows reservation and output-size changes and MUST NOT span
the whole stretched group. Restoring uses the current layout or unchanged saved
floating rectangle. Starting an interactive move restores maximized size under
the pointer through a [move session](snapping.md#core-move-sessions). A maximized
window must be restored before an interactive resize.

Minimize removes placements, layout participation, and ordinary focus-cycle
eligibility, but retains the managed window and its maximized flag. Focus is
repaired if necessary. `SetMinimized(window, false)` restores without explicitly
revealing its workspace or focusing it; ordinary focus repair can select it if no
other suitable focus exists. Explicit focus or accepting Alt-Tab restores and
focuses the selected window.

Launchers reject maximize/minimize/fullscreen. Late classification as a launcher
clears all three flags and any [snap override](snapping.md) while preserving
ownership, order, and saved floating state. Launcher
classification and sizing are specified in [configuration](configuration.md#shell-rules)
and [platform](platform.md#launcher-geometry).

## Fullscreen

Clear has one fixed, occupied-only `Fullscreen` presentation destination across
all outputs. `Desktop::fullscreen_slot` is its authoritative owner; a window's
read-only `fullscreen` flag mirrors slot membership. There MUST NOT be two
fullscreen occupants. This is desktop fullscreen; a low-latency/game presenter is
not implemented.

`ToggleFullscreen` enters/exits for the focused normal window. An explicit
`SetFullscreen(window, true)` activates an already focused window, while a hidden
or background target installs a dormant occupant without revealing its workspace
or changing focus. `request_fullscreen` also accepts a presentation-output
preference and explicit activation. Invalid IDs, launchers and minimized targets
MUST be rejected before displacing the existing occupant. Restore/focus a minimized
window before entering. Repeated enter for the same window/output is idempotent.
An exit for an older displaced window MUST NOT clear the current slot.

Entering another window replaces the occupant in one core mutation: the previous
window exits fullscreen and resumes ordinary layout participation; only the new
window retains the flag. Preserve both windows' home workspace/output, stable
workspace order, saved floating rectangles, snap/maximize state and retained
layout proportions. A presentation-output preference MUST NOT transfer home
ownership. A disconnected preference falls back to the existing target, home,
focused or first surviving output. Headless policy retains the slot without
placements; ordinary topology repair may migrate disconnected home outputs.

`PresentationTarget::Fullscreen` is distinct from `Workspace(WorkspaceId)`.
`workspace_for_output` and groups continue reporting the underlying normal
workspace. `presentation_for_output` reports what is actually selected. The
special desktop MUST NOT be inserted into the workspace map, numbered shortcut
IDs, layout configuration or workspace-transfer destinations. Navigation appends
it after ordinary desktops only on its target output and only while occupied.
`ActivateFullscreen` reveals it there; `CycleDesktop` includes that target.

An active fullscreen desktop presents the occupant at that output's full,
nonempty bounds, without reservations/gaps, never spanning a stretched group.
Other outputs keep their normal presentations. Ordinary windows and launchers
MUST NOT be placed on the overridden output. The occupant MUST NOT participate
in home layouts or home overview cards even when dormant. Selecting a normal
workspace hides the special presentation without unfullscreening its occupant;
explicitly focusing the occupant activates it without switching its home workspace.
Focusing an ordinary window on its output returns to ordinary presentation.

Exiting an active slot restores the recorded invoking output, ordinary
presentation and focus when the fullscreen target still owns global focus, with missing IDs repaired by normal policy.
Exiting a dormant/background slot MUST NOT reveal or focus another desktop.
Recompute restored geometry from current layouts, reservations and topology;
never restore a stale saved placement. Minimize, explicit workspace/output
transfer (including a transfer back to the same home workspace), removal and
launcher reclassification release the slot first. Minimize retains the underlying
maximize/snap/free state and then hides the window. Remapping starts without old
fullscreen state. Changing maximize while fullscreen only updates restoration.

Normal transient descendants of the occupant share its presentation and appear
above it, without occupying another slot or changing ownership. Parent links
MUST reject cycles and unknown parents; removing a parent clears direct child
links. Related dialogs hide with a dormant slot and focusing one reactivates it.
The adapter centers newly attached dialogs using committed size; their saved
frames translate between home/target output origins without changing home.
Independent move/resize/snap and overview transfers of these attached dialogs are
refused while their parent occupies the slot. Popups/subsurfaces remain owned by
the application tree. Unrelated windows and launchers stay in ordinary desktops.

Fullscreen and its descendants bypass normal custom/built-in layout inputs and
retain layout sizing for restoration. State/parent changes invalidate incompatible
move/resize baselines. Bounds updates and output loss repair the target without
creating invalid placements. The implementation and deterministic evidence are
[fullscreen policy](../src/core/fullscreen.rs),
[desktop tests](../tests/fullscreen_desktop.rs), and [core tests](../tests/core.rs).

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
