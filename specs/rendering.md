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
| `liquid_glass` | Disabled | Optional [optical treatment](#liquid-glass) after either filter |
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
protocol state. The optional liquid-glass treatment below refracts and lights only
the filtered lower scene; ordinary blur remains unchanged.

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
samples per pass. Filtering is bounded to the tree plus any liquid-glass sampling halo and the
required vertical blur halo. Sample positions clamp to the output/workspace viewport's texel centers;
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

## Liquid glass

`[theme.liquid_glass]` applies an optical treatment after either Gaussian or Dual
Kawase filtering. It is independent of `blur_method`, disabled by default, and
radius zero MUST still bypass the complete backdrop renderer, including glass.
Unknown fields, wrong types, and nonfinite/out-of-range values MUST fail validation
even when disabled. Reload uses the same atomic theme transaction.

| Field | Default | Accepted value |
| --- | --- | --- |
| `enabled` | false | Boolean |
| `refraction_strength` | 12 | Finite scalar 0–64; maximum per-axis displacement in logical pixels |
| `edge_width` | 24 | Finite scalar 1–128 logical pixels; fitted to the smaller half-dimension |
| `liquidity` | 0.5 | Finite scalar 0–1; interior dome curvature |
| `dispersion` | 0.15 | Finite scalar 0–1; chromatic separation |
| `highlight` | 0.25 | Finite scalar 0–1; directional edge reflection |

The fitted outline's distance gradient supplies an outward normal. A smooth curved
edge and an interior dome produce a dielectric normal; Snell refraction samples the
filtered backdrop with index 1.5. Dispersion offsets the red/blue indices by
`-/+ 0.15 * dispersion`. A top-left light and grazing-angle reflection brighten
the meniscus. This is a static optical approximation integrated with Clear's
existing blur and premultiplied composition. No animation, noise or opacity
override is added.

Glass replaces only `blurred_lower_scene` in the composition equation above.
Foreground text, controls and opaque pixels MUST remain sharp; coverage and input
shapes MUST remain unchanged. Refraction strength zero disables displacement;
`highlight` remains independently effective. Setting both to zero is neutral.
Dispersed channels are unpremultiplied individually and premultiplied using the
green sample's alpha, preserving valid premultiplied RGBA for translucent backdrops.

Windows use their original fitted outline, including SSD. Layers/popups use the
original uncropped surface-tree bounding rectangle with square optical edges,
while their existing alpha mask preserves transparent holes. Cropping MUST NOT
create new optical edges. Square-window overflow outside its frame retains ordinary
blur. Rounded cut-outs retain the original scene.

All displaced samples MUST clamp to the owning output/workspace viewport's texel
centers. Either filter initializes an expanded tree region of
`ceil(refraction_strength) + 1` pixels per side when displacement is active,
intersected with the viewport, so bilinear samples cannot read stale scratch data.
Gaussian additionally initializes its vertical support halo; Kawase still filters
the whole isolated viewport before copying this expanded region. Glass reuses the
existing composite pass, samplers, and scratch textures; it adds no texture cache.

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
- [Glass schema/reload tests](../tests/liquid_glass.rs) cover independent filter
  selection, strict disabled validation, and last-good reload retention.
  The blur fixture’s `--liquid-glass` option compares GPU captures with a scalar
  Snell/dispersion/lighting oracle and checks foreground/holes and output bounds.
- [Rounded GPU fixture](../scripts/vm-rounded-smoke.py),
  [blur GPU fixture](../scripts/vm-blur-smoke.py), and
  [blur oracle self-tests](../scripts/test_vm_blur_smoke.py) separate independent
  pixel oracles from renderer implementation. CPU self-tests are not GPU passes.
  Historical results and physical-input limitations remain in the
  [testing guide](../docs/vm-testing.md#backdrop-blur).
