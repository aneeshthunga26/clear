# Wallpapers

## Selection

Wallpapers are optional compositor-owned PNG/JPEG images, independent of the
shell and workspace layout. `[wallpaper]` accepts an optional `path`, a `mode`
(default `fill`), and `outputs` overrides keyed by configured output name.
`[wallpaper.outputs."NAME"]` can replace either path or mode, inheriting each
omitted field from the global selection. Unknown output names/fields and invalid
modes are rejected. Omission of an override path inherits; it is not a way to
disable a globally selected image.

Relative paths resolve against the configuration file's directory; absolute paths
are accepted. Empty/whitespace-only and NUL-containing paths fail validation.
There is no shell, `~`, or environment-variable expansion. With no selected image,
the solid theme background is used.

## Placement and stacking

| Mode | Behavior within each output |
| --- | --- |
| `fill` | Preserve aspect ratio; crop to cover the full output |
| `fit` | Preserve aspect ratio; display the whole image with background margins |
| `stretch` | Independently scale each axis to cover the output |
| `center` | Center at native pixel size and clip excess |

Placement MUST use the full output rectangle, including panel-reserved space,
not its usable rectangle. Each output is independent; stretching a workspace
MUST NOT stretch one wallpaper across the group. Wallpaper renders below every
layer-shell surface, including background layers, and has no input region.
The theme background remains visible through transparent pixels and fit/center
margins.

## Resources and failure behavior

Runtime prepares a complete bounded set of immutable premultiplied RGBA CPU images
at startup/reload, deduplicating identical configured paths. Explicit paths are
validated even when a global image is overridden on every output. Rendering MUST
NOT perform image reads or decoding. Limits are 8192 pixels per dimension,
16 MiB encoded per image, 64 MiB
decoded per image, and 128 MiB aggregate decoded RGBA data. Header/size checks
reject oversized resources before unrestricted pixel allocation.

Startup preparation failure warns and uses the solid theme background without
discarding other valid settings. Reload prepares all images before publication
and rereads same-path edits; any failure retains the whole last-good configuration
and wallpaper set. See [configuration](configuration.md#reload).

The renderer imports shared images once, retains per-output element identities,
and evicts stale resources after resource changes. A GPU import failure falls
back to the solid background and is remembered until resources change; it does
not trigger repeated import attempts every frame.

## Implementation and evidence

- [Selection schema](../src/config/wallpaper.rs),
  [CPU preparation](../src/runtime/wallpaper.rs),
  [GPU cache and geometry](../src/platform/smithay/wallpaper.rs).
- [Wallpaper tests](../tests/wallpaper.rs): inheritance, strict schema, paths,
  premultiplication/deduplication, JPEG, size limits, same-path rereads, startup
  fallback, and atomic reload.
- [Wallpaper fixture](../scripts/vm-wallpaper-smoke.py): generated images and GPU
  assertions for all four modes, output independence, full-output placement,
  image orientation, and layer priority; no physical input or reload-key testing.
