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
Radius zero MUST bypass blur filtering for either method; the shared backdrop
renderer still runs when liquid glass is enabled. Gaussian radius is
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
Square borders are rings when backdrop blur/glass is enabled or SSD is in use; only square
unblurred CSD retains legacy backing. The SSD radius-zero ring preserves titlebar
alpha independently of blur.

## Gaussian filter

Gaussian uses two separable passes, sigma `max(radius / 3, 0.5)`, and at most 65
samples per pass. Filtering covers the tree and required vertical blur halo;
enabled liquid glass expands that work to the owning viewport for its two sampling
stages. Sample positions clamp to the output/workspace viewport's texel centers;
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

`[theme.liquid_glass]` translates the resting SVG filter of kube.io's
[Magnifying Glass](https://kube.io/blog/liquid-glass-css-svg/#magnifying-glass).
It replaces the previous lens/Snell/mirror implementation entirely. The three
reference controls are joined by zoom and refraction-width controls, plus the
existing enable switch:

| Field | Default | Accepted value |
| --- | --- | --- |
| `enabled` | false | Boolean |
| `specular_opacity` | 0.5 | Finite scalar 0–1 |
| `specular_saturation` | 9 | Finite scalar 0–50 |
| `refraction_level` | 1 | Finite scalar 0–10; values above 1 extend the reference's displacement strength |
| `refraction_width` | 1 | Finite scalar 0–10; multiplier of the refraction rim's inward reach, zero disables refraction |
| `zoom_level` | 1 | Finite scalar 0–2; multiplier of the reference magnification displacement |

Unknown/removed fields, invalid types, and nonfinite/out-of-range values MUST fail
even when disabled. Reload retains the previous complete theme on failure.
The previous optical controls are unsupported, not silently ignored.

### Reference filter translation

Use the reference's three original RGBA8 maps, embedded locally with provenance
in [glass maps](../src/platform/smithay/glass-maps/README.md). The reference map
canvas is 210×150; refraction/specular maps contain 420×300 pixels and magnification
contains 210×150. Magnification scales over the full original tree rectangle.
Fit the refraction/specular maps with a nine-slice coordinate mapping: their
75-pixel circular corners map to the corresponding fitted native corner radii,
while their straight middle strips extend over the remaining frame. For unequal
radii, interpolate each side's radius smoothly between its corner regions; floor
zero radii at half a pixel to keep coordinates finite. The vertical middle strip
maps to source Y=75; the horizontal strip maps to source X=75..135. Thus the source
capsule's rim follows the actual window perimeter rather than appearing as a
capsule inside it. Corner-size changes scale the map's local bezel/rim reach.
At 210×150 with radius 75 this reduces to the original reference coordinates.
`refraction_width = 1` preserves that mapping exactly. Other positive widths
redistribute only the refraction map's radial coordinates within its existing
corner/edge slices; specular coordinates, magnification, and native corners do
not change. For source capsule radius 75, let `c = (clamp(x, 75, 135), 75)`
and `t = length(p - c) / 75`. Sample at
`c + (p - c) * width / (1 + (width - 1) * t)`.
This keeps the outer rim and central line fixed, continuously expands the rim
inward for widths above one, and narrows it below one. Near the rim the width
multiplier is literal; farther inward the mapping compresses smoothly to keep
the center continuous. Its reach remains bounded by the fitted corner/edge
slices, rather than being an absolute pixel width or changing the window shape.
Zero bypasses the second displacement, including its neutral-byte bias.
For adjusted widths, interpolate the refraction map explicitly in high precision
from its four neighboring texel centers; hardware interpolation-weight rounding
would otherwise be amplified by high refraction strength. Width one retains
the original sampling path.
Existing coverage and input shapes remain authoritative and unchanged. No shadow,
drag animation, or CSS transform is added. Output crops use original frame coordinates.

The two displacement stages MUST remain separate, including the RGBA8
magnified intermediate and its bilinear sampling. In sRGB channel space:

1. Magnify the filtered lower scene using the magnification map, SVG displacement
   `24 * zoom_level * (RG - 0.5)`. The default scale 24 is the demo's resting
   value. Zero disables magnification only; one preserves the existing result;
   two doubles the displacement (the reference's active scale 48). This controls
   displacement strength, not a literal image magnification ratio, which depends
   on the window dimensions. Refraction and specular controls remain independent.
2. Displace the magnified intermediate using the refraction map with scale
   `122.80891678834695 * 0.8 * refraction_level`. Values above one scale the
   displacement linearly, up to ten times the reference strength. Sampling
   remains clamped to the owning output viewport. Refraction map sampling uses
   high precision so increased strength does not amplify low-precision rounding.
   The fixed 0.8 is the demo's
   resting multiplier. Each axis uses SVG's `channel - 0.5` convention, including
   the slight bias of neutral byte 128; do not substitute a custom lens formula.
3. Apply the SVG saturation matrix (luma coefficients 0.213, 0.715, 0.072) with
   `specular_saturation`, clipped to valid channels. Mask that result by the
   specular map alpha and source-over it onto the displaced image.
4. Multiply the specular map alpha by `specular_opacity`, then source-over that
   layer onto the prior result. Preserve premultiplied composition, including
   premultiplied filtering of the specular map.

The reference's internal Gaussian stage has standard deviation zero. Clear's
existing optional Gaussian/Dual Kawase backdrop filter remains independent and
precedes these stages. `blur_radius = 0` MUST allow glass without blur; both
zero blur and disabled glass retain the original fast path. Refraction level
zero disables the second displacement only: configured magnification and specular
stages remain, just as in the reference. Specular opacity zero does not disable
the separate saturation stage. Disabling glass bypasses all optical stages.

Foreground text, controls, opacity, stacking, transparent holes, and rounded
coverage MUST retain the shared composition contract. Windows use original
frames including SSD; layers/popups use original tree bounds. Output cropping
MUST NOT rescale maps or introduce another optical edge. All source reads clamp
to the owning viewport's texel centers. Enabled glass initializes/filter-copies
the entire owning viewport before magnification, preventing stale scratch reads.
Square client overflow outside its frame retains ordinary backdrop pixels.
Client-alpha layer trees may opt into geometric glass outlines with the
`clear-glass-pill-*` and `clear-glass-rounded-*` layer-shell namespaces. Clear
fits those opt-in radii to the original tree bounds before applying the optical
maps. For these explicit glass layers, the fitted outline supplies independent
backdrop coverage across the whole panel, including transparent interior pixels;
client alpha still composites foreground text and controls sharply. Other
layer-shell surfaces continue using alpha-derived backdrop coverage and square
optical bounds because their protocol does not expose client corner radii.

### Resources

Glass adds one lazy shader and three immutable map textures (1,134,000 RGBA8
bytes total), shared across every tree; no per-window cache or file/network IO
occurs during rendering. Maps are embedded and decoded/uploaded once per renderer
when glass is first enabled. The existing four framebuffer textures are reused
for the magnified intermediate and final composite; read/write attachments MUST
remain distinct. Auxiliary sampler bindings and the window framebuffer MUST be
restored. Maps remain allocated until renderer destruction after disabling glass.

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

## Overview pass

[Overview rendering](overview.md#rendering-and-resource-bounds) is a final
compositor-owned pass after desktop filtering, with independently bounded GPU
thumbnails and labels. Cards reuse committed buffers without client resizes.

## Implementation and evidence

- [Theme](../src/decoration/mod.rs), [rounded renderer/hit shapes](../src/platform/smithay/rounded.rs),
  [blur renderer and inline tests](../src/platform/smithay/blur.rs),
  [scene assembly](../src/platform/smithay/scene.rs).
- [Rounded schema/reload](../tests/rounded.rs) and [blur schema/reload](../tests/blur.rs)
  tests cover CPU policy, not rendered pixels.
- [Glass schema/reload tests](../tests/liquid_glass.rs) cover the optical controls,
  removed-field rejection, blur independence, and atomic reload.
  The blur fixture's `--liquid-glass` option compares GPU captures against a
  scalar translation of the reference SVG maps and two displacement passes.
  CPU oracle tests check map provenance/dimensions, SVG displacement conventions,
  independent controls, quantized magnification, and premultiplied specular blending.
- [Rounded GPU fixture](../scripts/vm-rounded-smoke.py),
  [blur GPU fixture](../scripts/vm-blur-smoke.py), and
  [blur oracle self-tests](../scripts/test_vm_blur_smoke.py) separate independent
  pixel oracles from renderer implementation. CPU self-tests are not GPU passes.
  Historical results and physical-input limitations remain in the
  [testing guide](../docs/vm-testing.md#backdrop-blur).
