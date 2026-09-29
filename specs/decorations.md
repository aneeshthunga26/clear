# Server-side decorations

## Negotiation and commit timing

Clear defaults to server-side decorations (SSD) for new/unset XDG decoration
preferences and legacy KDE decoration objects. Explicit client-side (CSD)
requests are honored. XDG takes precedence when both protocols exist. A client
that never negotiates MUST remain undecorated by the compositor; its own header
bar cannot be removed from its pixels.

Pending negotiated mode determines configure sizes. Committed root-buffer mode
determines rendering and input coordinates. A mode request or configure ACK alone
MUST NOT change the displayed mode. XDG decoration destruction retains the current
render decision until the next root commit, after which the remaining protocol
state determines it. Object replacement and unmap/remap MUST NOT reuse a stale
object's committed SSD configure. KDE mode changes also take effect on root commit;
the lifetime of a released KDE object is not retained as a decoration entitlement.

The bufferless-commit path owns initial configures; decoration creation/request
and reconciliation of hidden windows MUST NOT send a premature initial configure.
The pinned Smithay handler has no XDG decoration destruction callback, so the
adapter's dispatch bridge forwards destruction after normal protocol handling.

## Titlebar configuration

`[theme.titlebar]` is independent of borders and blur. Unknown fields and invalid
types/ranges MUST fail configuration validation. Colors are straight RGBA with
exactly four finite channels in 0–1. Default RGB values below are divided by 255;
all default alpha values are 1.

| Field | Default | Accepted value |
| --- | --- | --- |
| `active_background` | RGB 35,40,52 | RGBA |
| `inactive_background` | RGB 27,30,38 | RGBA |
| `active_foreground` | RGB 239,243,250 | RGBA |
| `inactive_foreground` | RGB 174,183,199 | RGBA |
| `height` | 32 | Integer logical pixels, 16–128 |
| `controls_side` | `right` | `left` or `right` |
| `show_icon` | false | Boolean |
| `show_title` | true | Boolean; hiding title does not hide controls |

`[theme.titlebar.controls]` accepts optional `minimize`, `maximize`, `restore`,
and `close` local SVG paths. Missing entries use built-ins; omitted restore first
falls back to a supplied maximize SVG. SVGs retain their own colors and alpha.
Paths follow [configuration path rules](configuration.md#loading-and-defaults).
Examples referring to files that do not exist must keep those paths commented out.

## Frame and content geometry

Core placements are total frames. The adapter insets SSD client content by the
configured titlebar height, retaining at least one content row in a tiny positive
frame. Normal configure sizes subtract that inset. Launcher committed dimensions
add the titlebar before core centering, retaining client-owned content size.

Client rendering, input, and popup origins MUST use the same inset and XDG
window-geometry offset. Decoration changes and height reloads MUST NOT rewrite
saved floating rectangles or tile proportions. Layer-shell surfaces and XDG
popups never receive these titlebars. The whole SSD body and titlebar share one
[rounded outline](rendering.md#rounded-outlines); the content/titlebar join does
not create new corners.

## Controls and input

Right-edge controls read minimize, maximize/restore, close from left to right.
Left-edge controls mirror to close, maximize/restore, minimize. Narrow frames
prioritize close, then maximize, then minimize. Hit boxes and pixels derive from
the original frame, never a crop-dependent layout.

Titles use Unicode shaping/rasterization with installed system fonts and fall
back to app ID when no title exists. Built-in controls render without fonts.
Text is kept out of the icon/control boxes. Missing app icons use a generic glyph.

SSD hits belong to the compositor, not Wayland pointer focus. A control activates
only when released over the same window/control where it was pressed; matching
button releases are suppressed. A titlebar click focuses and arms movement, but
a tile detaches only after crossing the movement threshold. Super+mouse gestures
take precedence over controls. Minimize/maximize target existing core state;
close sends a graceful client close. Maximized windows must be restored before
dragging; launchers reject ordinary move/maximize/minimize operations.

## Prepared SVG and app-icon resources

Runtime prepares immutable premultiplied CPU images outside rendering, at
startup/reload and app classification. Render accessors MUST NOT read files or
decode SVGs. Explicit controls are one candidate resource set: startup failure
uses built-ins; reload failure retains the entire last-good configuration/resource
set. Same-path reload rereads files. App-icon failure uses the generic glyph and
does not reject otherwise valid configuration.

Control SVGs MUST be local regular files, at most 256 KiB, with source dimensions
at most 1024 per axis, at most 4096 XML nodes, and at most 32 nesting levels.
DTD/entity expansion is rejected. Minimal resvg disables default features,
SVGZ, SVG text/fonts, and embedded raster decoders; both image resolvers are
disabled, preventing external/network/embedded image loads.

The supported subset is simple paths and gradients. `use`, `pattern`, `marker`,
`mask`, `clipPath`, and `filter` elements MUST be rejected by local name before
usvg conversion, including unused or namespaced definitions. CSS/href references
must not bypass this preflight. Raster boxes are transparent squares of
`clamp(height - 12, 1, 20)` pixels with aspect-fitted content, independent of width.

App icons use exact desktop-file IDs under bounded XDG data roots and the entry's
`Icon` key. Supported sources are absolute SVG/PNG/JPEG paths or named icons in
fixed hicolor size/app locations, direct icon directories, and pixmaps, including
legacy `~/.icons`. There is no recursive/fuzzy lookup, theme inheritance, or
execution of `Exec`/`TryExec`. A higher-priority desktop entry shadows lower ones
even when hidden or without a usable icon.

| Resource | Bound |
| --- | --- |
| Positive and negative app-icon cache | 256 entries, reset on reload |
| XDG data roots | 8 |
| Probes per lookup / per asset set | 256 / 4096 |
| Read budget per asset set | 16 MiB |
| Desktop entry | 64 KiB |
| Raster app icon | 2 MiB encoded; 1024 pixels per axis; 8 MiB decoded |

Unsupported resources and exhausted lookup budgets fall back to the generic icon.

## Rasterization, alpha, and cache

System font discovery/reads occur at cache initialization. Title strings are
bounded to 1024 bytes; only visible strips are rasterized, each at most 4096×128.
Cache keys include title, local clip, size, activation, maximization, complete
style, app ID, and prepared-image identity, including same-path replacements.
Absolute window position alone MUST NOT invalidate a matching local strip.

The titlebar texture cache is bounded by both 64 MiB and 128 entries, excluding
outstanding render elements and driver overhead. This is a retained-cache bound,
not a total GPU-process memory guarantee.

Theme colors are premultiplied once. Glyphs and prepared icon pixels compose with
source-over in all four channels, without double premultiplication, forced opaque
fill, accent stripe, or separator. Translucent titlebars reveal the lower scene
(filtered when blur is enabled) without fading client content. Rounded composition
preserves sampled alpha and reports no opaque region for the combined element.
Square SSD borders MUST remain rings even when blur is off, preventing border
color from filling transparent titlebars. Legacy square unblurred CSD backing
is unchanged; see [rendering](rendering.md).

## Implementation and evidence

- [Theme schema](../src/decoration/titlebar.rs),
  [CPU resources](../src/runtime/titlebar.rs),
  [negotiation and inline tests](../src/platform/smithay/decorations.rs),
  [titlebar rendering and inline tests](../src/platform/smithay/titlebar.rs),
  [input](../src/platform/smithay/input.rs), [scene](../src/platform/smithay/scene.rs).
- [Titlebar schema/resource tests](../tests/titlebar_theme.rs): strict colors and
  paths, atomic reload, SVG preflight, asset alpha, bounded icon lookup.
- [Decoration fixture](../scripts/vm-decoration-smoke.py) and
  [harness tests](../scripts/test_vm_decoration_smoke.py): protocol lifetimes,
  insets, control pixels, transparency, and square-border regressions. Harness
  self-tests do not validate GPU behavior; fixtures do not automate physical clicks.
