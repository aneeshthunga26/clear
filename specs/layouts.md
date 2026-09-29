# Layouts and interactive resizing

## Layout inputs and names

Built-in layouts are pure geometry functions over a region's usable rectangle,
stable ordered participants, focus, gaps, scroll offset, and optional saved sizing.
They MUST NOT mutate desktop state or invoke scripts. The desktop selects eligible
participants according to [desktop state](desktop.md#modes-and-saved-geometry).
Placements are ordered back-to-front and use total frames, including any
[server-side titlebar inset](decorations.md#frame-and-content-geometry).

Canonical mode names are `floating`, `scrolling`, `master_stack`, `columns`, `rows`,
`grid`, `spiral`, `monocle`, and `script:NAME`. Core parsing trims whitespace and
matches built-in names case-insensitively. Aliases are `float`, `scroll`,
`master-stack`, `masterstack`, `tiling`, `tile`, `fibonacci`, and `dwindle`.
The last two select spiral. A script name retains its case.

Mode cycling visits floating → scrolling → master_stack → columns → rows → grid
→ spiral → monocle → floating. Cycling from a script mode also selects floating.
Configuration/action validation is defined in [configuration](configuration.md)
and [input](input.md); core parsing does not by itself validate a Rhai function.

## Geometry

Layouts normalize the available rectangle and safely reduce requested gaps when
space is too small. Integer allocation assigns remaining pixels deterministically
without arithmetic overflow. Tiny regions can yield zero-sized tiles; ordinary
geometry guarantees MUST NOT assume every tile is positive when space is exhausted.

| Mode | Initial geometry and order |
| --- | --- |
| Floating | Saved normalized floating rectangles; focused window raised |
| Columns | Equal vertical columns, left to right |
| Rows | Equal horizontal rows, top to bottom |
| Master/stack | First window takes 3/5 of width remaining after the split gap; others divide the right stack vertically; a single window fills the inner area |
| Grid | `ceil(sqrt(count))` columns; sequential rows; a partial last row divides its full width among its actual cells |
| Spiral | Repeated 3/5 splits of the remaining area after each gap, rotating left, top, right, bottom; final window takes the remainder |
| Monocle | Every participant fills the inner area; focused tile raised |
| Scrolling | Horizontal columns initially 2/3 of the inner viewport width, with offscreen positions and a viewport clip |
| Script | Validated custom placements; master/stack fallback when unavailable or rejected |

Columns, rows, grid, and master/stack consume saved proportions where available.
Missing per-window weights use the mean of known weights. Weighted allocation
uses feasible minimum widths/heights and distributes leftover pixels by largest
remainder with stable index tie-breaking. This preserves total extents and avoids
overlap even when the usual minimum cannot fit.

## Scrolling

Each workspace/output retains its own offset through workspace and mode changes.
Manual scroll is clamped to content extent and only affects a scrolling region.
Explicit focus changes reveal the selected column; merely recomputing placements
MUST NOT continually overwrite a manually scrolled position. Widths, content
prefixes, reveal positions, and scroll limits MUST use the same sizing data.

Resized scrolling widths are stored relative to viewport width. They scale with
the viewport and are clamped between the feasible 64-pixel minimum and full
viewport width. Floats, launchers, minimized windows, and maximized windows do
not add scrolling content.

## Resize sessions

`begin_resize` captures a baseline and selected edges. Updates apply total
displacement from that baseline, not incremental deltas: repeating the same
displacement MUST produce the same geometry. An axis is selected only when exactly
one of its two edges is requested. A request with no selected axis, an unknown or
hidden window, a launcher, a maximized/minimized window, or no supported tile
boundary cannot start a session.

| Window/mode | Supported resize |
| --- | --- |
| Floating or floating exception | Selected edges; opposite edges stay fixed; nominal minimum 64×48 with representable-coordinate constraints |
| Columns | Adjacent column pair |
| Rows | Adjacent row pair |
| Master/stack | Master/stack width divider and adjacent stack-window heights |
| Grid | Adjacent cell widths within one row; shared heights of adjacent rows |
| Scrolling | Individual column width |
| Spiral, monocle, script tiles | No tile resize |

An unsupported axis of a corner resize is ignored if the other axis is supported.
In pair/divider layouts, outer edges do not create a resize boundary. Pair resizing
clamps both neighbors to feasible minimum sizes. Floating exceptions remain resizable in
every mode and may grow offscreen.

Tile resizing MUST change persistent proportions, never detach a tile, alter its
floating flag, or overwrite saved floating geometry. Sizing is scoped to
workspace/output/mode and survives switching away and back. Window removal cleans
up all of that window's retained sizing entries.

A session becomes invalid after changes to mode, membership, role/floating/state,
gaps, usable area, visibility, or output grouping. Changing those values back
MUST NOT revive an old session. Metadata, client size commits, focus, and unrelated
regions do not alone invalidate it. The adapter ends invalid gestures on motion
and brackets accepted native resizing with XDG Resizing state. Authorization and
pointer edge selection are specified in [input](input.md#pointer-gestures).

## Implementation and evidence

- [Built-in policies](../src/management/mod.rs), [resize sessions](../src/core/resize.rs),
  [desktop orchestration](../src/core/desktop.rs).
- [Core tests](../tests/core.rs): exact odd-sized geometry, stable grid/spiral order,
  focus reveal, retained manual scrolling, gaps, and extreme geometry.
- [Resize tests](../tests/resize.rs): baseline reuse, neighbor floors, per-mode
  persistence, invalidation even after reversed changes, and random resize invariants.
