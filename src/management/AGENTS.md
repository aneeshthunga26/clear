# Built-in window management

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` implements pure layout geometry over core `Mode` and `LayoutContext`.
Built-ins currently include floating, scrolling, master-stack, columns, rows,
grid, spiral, and monocle. Do not introduce protocol objects, config loading,
script execution, process effects, or persistent desktop ownership here.
`arrange_with_sizing` consumes optional core-owned `LayoutSizing`; `arrange` keeps
exact unsized defaults and existing `LayoutContext` callers unchanged.

- The supplied area already excludes panel reservations. Do not subtract them
  again or query the platform.
- Preserve stable window IDs and return back-to-front `Placement`s.
- Floating mode preserves normalized saved rectangles without fitting them to
  the output. Normalization is not permission to clamp positions or sizes.
- Scrolling may return offscreen rectangles with a viewport clip. Preserve this
  distinction rather than forcing every window into the visible area.
- Handle empty layouts, small regions, gaps, and arithmetic limits safely.
- Sized splits use integer weights with feasible 64-pixel width / 48-pixel height
  floors and exact remainder distribution. Reduce floors when space is scarce;
  zero extents are valid, negative extents and overlapping adjacent tiles are not.
- Grid height weights belong to row anchors and width weights to individual cells.
  Scrolling widths are viewport-relative; placement, reveal, and clamping must all
  use the same prefix sums. Scrolling resizing retains the viewport except when
  shorter content requires clamping.
- Core excludes launchers and tiled-mode floating exceptions from layout inputs;
  do not reproduce role classification or global stacking policy here.
- For new modes, coordinate `core::Mode`, parsing/cycling, runtime routing,
  examples, and tests. Rhai execution stays in `scripting` via `runtime`.

## Verification

Start with `cargo test --locked --test core --test resize`, then the root workflow. Test exact
geometry, empty/small regions, scrolling clips, and preservation of floats.
