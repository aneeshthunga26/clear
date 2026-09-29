# Decoration descriptions

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` defines backend-independent `Theme` data and its defaults. Current
styling covers background, active/inactive border colors, border width, and
`CornerRadii` for the outer window outline, and global backdrop-blur method, radius,
and pyramid depth (`BlurMethod`, `blur_radius`, `blur_passes`).
`titlebar.rs` defines `TitlebarTheme`, `TitlebarControls`, and `ControlsSide`,
re-exported by this module. Rendering, surface lifetimes, and GPU buffers belong
in `platform/smithay/`.

- Theme colors are non-premultiplied RGBA; the adapter performs renderer-specific
  conversion. Keep logical border thickness distinct from physical scaling.
- `CornerRadii` deserializes a scalar or lists of one, two (top/bottom), or four
  (top-left, top-right, bottom-right, bottom-left) finite values in `0..=256` logical
  pixels. Reject other lengths/ranges. Default zero preserves square outlines.
  Inner radii, proportional fitting, and pixel/input masks belong in the adapter.
- `blur_method` accepts `gaussian` (default) or `kawase` (Dual Kawase); existing
  radius-only configurations retain Gaussian behavior.
- `blur_radius` is a finite scalar in `0..=32`, default zero (off for either method).
  Gaussian uses logical-pixel kernel support; Kawase uses sample offsets in source
  pyramid texels, not a Gaussian-equivalent radius.
- `blur_passes` is an integer in `1..=6`, default 3. Kawase uses that many ceil-half
  downsample levels and the same upsample levels, stopping early at `1×1`. Gaussian
  always uses two filtering passes and ignores this field, but it is always validated,
  even with radius zero. The VM example selects Kawase radius 2/passes 3.
- Blur describes neither client opacity nor foreground filtering.
  `liquid_glass.rs` owns strict, defaulted optical parameters independent of blur
  method; see [rendering](../../specs/rendering.md#liquid-glass). Shader resources,
  grouping, optical sampling, and alpha/coverage handling stay in the adapter.
- `Theme.titlebar` is strict/defaulted declarative data. Its four straight RGBA
  colors must be finite in `0..=1`; defaults are background 35/40/52 (active),
  27/30/38 (inactive), foreground 239/243/250 and 174/183/199, all alpha 255.
  Height defaults to 32 and validates in `16..=128`; controls default right,
  `show_icon` false, and `show_title` true. `icon_size()` is `(height-12)` clamped
  to `1..=20`, the CPU raster box, not the full titlebar or physical output scale.
- Optional minimize/maximize/restore/close control paths reject empty/whitespace
  and NUL values. Config loading resolves relative paths; runtime reads and
  rasterizes SVGs, preserving their original colors. Missing overrides use the
  adapter's built-in glyphs. Runtime's bounded SVG subset supports paths and
  gradients but rejects `use`, `pattern`, `marker`, `mask`, `clipPath`, and `filter`
  definitions by local name before usvg conversion, including unused/namespaced
  definitions referenced through CSS or href. This module performs no resource IO.
- Keep schema validation and defaults consistent with configuration examples.
- Do not place Smithay types, protocol decorations, or a rendering loop here.
- SSD negotiation and titlebar widgets live in `platform/smithay/decorations.rs`
  and `titlebar.rs`. Apps may instead negotiate CSD; do not confuse that with the
  backend-independent theme data here. A decoration scripting API is not exposed.
- Future styling descriptions should remain declarative rather than giving
  extensions ownership of compositor objects.

## Verification

Use `cargo test --locked --test titlebar_theme` for titlebar schema, bounded CPU
resources, and reload coverage. These tests do not establish a renderer/GPU pass.
Use `cargo test --locked --test extensions --test rounded --test blur` for configuration and
reload validation, then the root workflow. For renderer-visible changes, use
bounded rounded/decoration GPU smokes and inspect color/geometry output. For blur,
use `scripts/vm-blur-smoke.py` with `--method`, `--radius`, and `--passes`, including
`output-boundary-odd`; see `docs/vm-testing.md`. The compositor-free
`scripts/test_vm_blur_smoke.py` checks the oracle/harness, not GPU pixels; all 11
tests passed. Separate GPU runs on private local virtual KWin (not a VM) passed
all ten Kawase cases at radius 2/passes 3, plus `stacking`/`output-boundary-odd` at
radius 1.5/passes 1 and 6. See `docs/vm-testing.md` for exact coverage and artifacts.
No physical input tests were performed; parsing and oracle self-tests alone do not
establish a GPU pass.
