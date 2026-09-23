# Built-in window management

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` implements pure layout geometry over core `Mode` and `LayoutContext`.
Built-ins currently include floating, scrolling, master-stack, columns, and
monocle. Do not introduce protocol objects, config loading, script execution,
process effects, or persistent desktop ownership here.

- The supplied area already excludes panel reservations. Do not subtract them
  again or query the platform.
- Preserve stable window IDs and return back-to-front `Placement`s.
- Floating mode preserves normalized saved rectangles without fitting them to
  the output. Normalization is not permission to clamp positions or sizes.
- Scrolling may return offscreen rectangles with a viewport clip. Preserve this
  distinction rather than forcing every window into the visible area.
- Handle empty layouts, small regions, gaps, and arithmetic limits safely.
- Core excludes launchers and tiled-mode floating exceptions from layout inputs;
  do not reproduce role classification or global stacking policy here.
- For new modes, coordinate `core::Mode`, parsing/cycling, runtime routing,
  examples, and tests. Rhai execution stays in `scripting` via `runtime`.

## Verification

Start with `cargo test --locked --test core`, then the root workflow. Test exact
geometry, empty/small regions, scrolling clips, and preservation of floats.
