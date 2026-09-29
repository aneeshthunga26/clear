# Theme and rendering

## Theme

The backend-independent theme describes appearance; GPU objects belong to the
adapter. Configuration validation rejects unknown fields, invalid types, and
nonfinite/out-of-range values. Reload follows the
[atomic configuration contract](configuration.md#reload).

| `[theme]` field | Default | Value |
| --- | --- | --- |
| `background` | `[0.07, 0.08, 0.10, 1.0]` | Straight RGBA, finite channels 0–1 |
| `active_border` | `[0.40, 0.65, 0.95, 1.0]` | Straight RGBA, finite channels 0–1 |
| `inactive_border` | `[0.22, 0.24, 0.28, 1.0]` | Straight RGBA, finite channels 0–1 |
| `border_width` | 2 | Integer logical pixels, 0–64 |
| `corner_radius` | 0 | Scalar or one/two/four finite values in 0–256 |
| `blur_method` | `gaussian` | `gaussian` or `kawase` |
| `blur_radius` | 0 | Finite scalar 0–32, fractions allowed |
| `blur_passes` | 3 | Integer 1–6; always validated |
| `titlebar` | See [decorations](decorations.md#titlebar-configuration) | Independent titlebar style/resources |

Wallpapers use full outputs behind all surfaces, as specified in
[wallpaper](wallpaper.md). Scene stacking, clipping, and hit-testing are specified
in [platform](platform.md#scene-and-hit-testing).

## Rounded outlines

A scalar or single radius applies to every corner. Two values specify top and
bottom pairs. Four values are clockwise top-left, top-right, bottom-right,
bottom-left. Zero preserves the square path. Radii MUST NOT change core
placements, saved geometry, layout sizing, or client configure dimensions.

An original unclipped outline is derived from the frame. Adjacent radius sums
are proportionally fitted to its dimensions; inner radii subtract border
thickness. Coverage and body hit-testing intersect every applicable corner arc,
including overlapping opposite-corner regions. Output/workspace crops MUST NOT
introduce new rounded corners. Cut-outs reveal lower content and pass input
underneath. Layer-shell trees and popups retain their own shapes.

For each visible clip, the adapter composites the unmasked body surface tree
(including SSD titlebar when present) into RGBA before applying coverage once.
Overlapping subsurfaces MUST NOT accumulate rounded coverage or leak obscured
parent colors. Body and disjoint border-ring coverage contribute premultiplied
color while preserving client and border alpha, without a colored seam.

The combined rounded element reports no opaque regions. Fresh element identities
damage the complete clip, including theme-only changes. Offscreen allocation is
bounded to the visible clip but currently occurs per frame. Shader ownership
stays with the backend and reload updates theme data/uniforms. Coordinates assume
the current scale-1 single framebuffer, including its height.

## Backdrop composition

Blur is global for application windows, popup trees, and every layer-shell kind.
Radius zero MUST bypass the blur renderer for either method. Gaussian radius is
logical-pixel kernel support; Kawase radius is a source-pyramid-texel sample
offset. Those units are not equivalent. Gaussian ignores `blur_passes`, but the
field is validated even for Gaussian or disabled blur.

Filtering MUST affect the already composed lower scene, not foreground text or
controls. Logical surface-tree groups are processed bottom-to-top, compositing
subsurfaces before one backdrop replacement per tree. Popups sample their parent
and lower groups, never surfaces stacked above them. Wallpapers are ordinary
unfiltered lower-scene content. Neither filter changes client opacity, geometry,
protocol state, or adds tint/noise/saturation treatment.

For premultiplied foreground `F`, composed alpha `A`, and independent rounded frame
coverage `C`, the result is:

```text
F + (C - A) * blurred_lower_scene + (1 - C) * original_lower_scene
```

Coverage MUST remain distinct from client alpha so rounded cut-outs stay holes
and opaque foreground stays sharp. Client-shaped layers/popups have no separate
frame shape; their blur contribution is `A * (1 - A)`, preserving fully transparent
holes. Square client overflow outside the frame also uses its alpha-derived shape.
Effective coverage is clamped to at least stored alpha to tolerate RGBA8 rounding.
Square borders are rings when blur is enabled or SSD is in use; only square
unblurred CSD retains legacy backing. The SSD radius-zero ring preserves titlebar
alpha independently of blur.

## Gaussian filter

Gaussian uses two separable passes, sigma `max(radius / 3, 0.5)`, and at most 65
samples per pass. Filtering is bounded to the tree plus the required vertical
halo. Sample positions clamp to the output/workspace viewport's texel centers;
one independent output MUST NOT contribute samples to another.

## Dual Kawase filter

Kawase copies the entire output/workspace viewport to an isolated level-zero
texture, not merely the tree bounds. Each downsample level ceil-halves both
dimensions, for up to `blur_passes` levels, stopping when both dimensions reach
1×1. Upsampling follows those same levels in reverse to the original viewport.

- Downsample: center tap weight 4 and four diagonal taps weight 1, normalized by 8.
- Upsample: four axial taps at twice the offset with weight 1, and four diagonal
  taps weight 2, normalized by 12.

Offsets are in source-level texels with no implicit half-texel offset. Linear
sampling is center-aligned and uses actual source/destination ratios for odd
dimensions. Samples clamp to each level's texel centers; intermediates are RGBA8.
Nonzero viewport origins MUST be handled without sampling neighboring outputs.
The VM example's radius 2/passes 3 is not equivalent to Gaussian radius 2.

## Blur resource bounds and lifecycle

Declared-opaque groups MAY skip filtering. Both methods reuse four
framebuffer-sized RGBA8 scratch textures, approximately 16 bytes per host pixel.
Kawase also retains at most four viewport pyramids, keyed by size/effective depth,
not origin or radius. Their aggregate budget is three framebuffer areas of texels
including level zero, up to another 12 bytes per host pixel.

Retained blur texture storage is therefore bounded by seven framebuffer areas,
approximately 28 bytes per host pixel, excluding driver overhead and other
renderer resources. This is not a peak-allocation guarantee during replacement.
Upsampling overwrites downsample levels. Every filtered tree refreshes level zero;
the cache retains allocations, never stale scene pixels.

Framebuffer resizing replaces scratch storage and clears pyramids on the next
blur render. Changing methods or disabling blur does not immediately release old
allocations; disabled blur retains the last blur-rendered size. Method shaders
initialize lazily. Read/write attachments MUST be separate, auxiliary sampler
state restored, and the window framebuffer restored before presentation/capture.

## Implementation and evidence

- [Theme](../src/decoration/mod.rs), [rounded renderer/hit shapes](../src/platform/smithay/rounded.rs),
  [blur renderer and inline tests](../src/platform/smithay/blur.rs),
  [scene assembly](../src/platform/smithay/scene.rs).
- [Rounded schema/reload](../tests/rounded.rs) and [blur schema/reload](../tests/blur.rs)
  tests cover CPU policy, not rendered pixels.
- [Rounded GPU fixture](../scripts/vm-rounded-smoke.py),
  [blur GPU fixture](../scripts/vm-blur-smoke.py), and
  [blur oracle self-tests](../scripts/test_vm_blur_smoke.py) separate independent
  pixel oracles from renderer implementation. CPU self-tests are not GPU passes.
  Historical results and physical-input limitations remain in the
  [testing guide](../docs/vm-testing.md#backdrop-blur).
