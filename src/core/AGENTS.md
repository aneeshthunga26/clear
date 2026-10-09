# Core desktop policy

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities

- `types.rs`: stable IDs, desktop records, modes, commands/effects, layout inputs,
  and placements. No Smithay, Wayland, renderer, or client-resource types.
- `geometry.rs`: backend-neutral logical rectangle operations.
- `desktop.rs`: window/workspace ownership, output groups, focus, persistent
  geometry, mode overrides, and ordered placement computation.
- `resize.rs`: opaque baseline resize sessions, region lifecycle validation, and
  persistent per-workspace/output/mode sizing. Floating gestures edit only saved
  rectangles; tiled gestures edit only layout proportions.
- `mod.rs`: module exports.

## Invariants

- Workspaces own windows; disjoint output groups present workspaces. A workspace
  is visible in at most one group. Never duplicate a window across workspaces.
- Output mode overrides, scroll positions, and remembered focus belong to a
  workspace/output pair. Preserve them through mode and topology changes.
- Layout sizing is additionally mode-scoped. Keep it across mode switches and
  remove all window-keyed sizing entries when a window is unmapped, including
  entries retained in former workspaces. LayoutContext remains sizing-independent.
- Resize displacement is TOTAL from the captured session baseline. Region mode,
  usable area, gaps, group membership, and window membership/role/floating/state changes
  invalidate sessions, including changes subsequently reversed. Focus and client
  commits do not invalidate sessions. Drop the session to end a gesture; a false
  update result means the adapter must cancel it.
- Tiled resizing changes internal boundaries only: adjacent columns/rows, the
  master divider and adjacent stack heights, grid rows and cells within a row.
  Scrolling widths are independent viewport-relative proportions; prefix widths
  drive positions, focus reveal, and content clamps. Floats retain opposite edges
  with 64x48 minima and may extend offscreen. Spiral, monocle, script tiles, and
  launchers cannot be resized; normal floating exceptions remain resizable.
- Commands mutate policy and return explicit effects; never spawn processes,
  access protocol objects, or perform rendering here. Invalid IDs are safe no-ops.
  Explicit window-to-workspace transfers must not focus/restore the source first
  or follow its destination; preserve unrelated focus and use ordinary repair
  when the moved window was focused. See [desktop](../../specs/desktop.md#focus-and-commands).
- Saved floating rectangles are independent of tiles and committed client sizes.
  Allow offscreen positions and oversized floats; panel reservations and output
  shrinkage must not clamp or overwrite saved geometry.
- Launcher roles bypass layout inputs and scrolling content. Center them using
  committed size in their home output's usable area without shrinking them or
  changing saved floating state. Role classification belongs to runtime.
- Maximize/minimize are independent window flags, not modes or client unmaps.
  Fullscreen is an independent third flag preserving underlying maximize state.
  All exclude normal layout inputs without erasing saved geometry, order, or
  proportions. Maximized rectangles use only the home output's usable area;
  fullscreen uses only its full bounds, never a stretched group's bounds.
  Minimized windows have no placements, focus-cycle entries, or remembered-focus
  eligibility. Explicit focus restores them; workspace switching does not.
  Launchers reject these states, and launcher reclassification clears all three.
  Full bounds and usable output area are independent. Full-bounds setters reject
  empty normalized rectangles; retain the legacy empty-output behavior without
  emitting empty fullscreen placements. See [desktop](../../specs/desktop.md#fullscreen).
- Placements are back-to-front: tiles, floats, maximized windows, fullscreen,
  then launchers. A focused normal window can rise above a maximized neighbor on
  its visible output, but remains below fullscreen. Focused fullscreen rises above
  other fullscreen windows; stable policy order resolves the rest.
  The adapter owns protocol layer stacking and visible output-group clips.
- Keep custom-layout result validation and built-in fallback at this boundary,
  even though the scripting host also validates its output.

## Verification

Start with `cargo test --locked --test core --test resize --test window_state`,
then the root workflow.
Core and resize tests can also be compiled directly using `rustc --edition=2024 --test` when
platform dependencies are unavailable. Resize tests cover boundaries, baseline
updates, lifecycle cancellation, state isolation, and randomized tiny layouts. Cover
workspace/group invariants, topology changes, focus, persistent geometry,
reservations, and launcher behavior. Extend deterministic command-sequence tests
when changing ownership or focus semantics.
