# Core desktop policy

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities

- `types.rs`: stable IDs, desktop records, modes, commands/effects, layout inputs,
  and placements. No Smithay, Wayland, renderer, or client-resource types.
- `geometry.rs`: backend-neutral logical rectangle operations.
- `desktop.rs`: window/workspace ownership, output groups, focus, persistent
  geometry, mode overrides, and ordered placement computation.
- `mod.rs`: module exports.

## Invariants

- Workspaces own windows; disjoint output groups present workspaces. A workspace
  is visible in at most one group. Never duplicate a window across workspaces.
- Output mode overrides, scroll positions, and remembered focus belong to a
  workspace/output pair. Preserve them through mode and topology changes.
- Commands mutate policy and return explicit effects; never spawn processes,
  access protocol objects, or perform rendering here. Invalid IDs are safe no-ops.
- Saved floating rectangles are independent of tiles and committed client sizes.
  Allow offscreen positions and oversized floats; panel reservations and output
  shrinkage must not clamp or overwrite saved geometry.
- Launcher roles bypass layout inputs and scrolling content. Center them using
  committed size in their home output's usable area without shrinking them or
  changing saved floating state. Role classification belongs to runtime.
- Placements are back-to-front: tiles, floats, then launchers. The adapter owns
  protocol layer stacking and clips to the visible output-group union.
- Keep custom-layout result validation and built-in fallback at this boundary,
  even though the scripting host also validates its output.

## Verification

Start with `cargo test --locked --test core`, then the root workflow. Cover
workspace/group invariants, topology changes, focus, persistent geometry,
reservations, and launcher behavior. Extend deterministic command-sequence tests
when changing ownership or focus semantics.
