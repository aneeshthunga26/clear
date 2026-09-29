# Configuration schema and defaults

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities

- `mod.rs`: TOML deserialization, defaults, validation, configuration paths, and
  loading errors/fallback. Keep schema values backend-independent.
- `wallpaper.rs`: optional global `[wallpaper]` path/mode and partial
  `[wallpaper.outputs."name"]` overrides; keep these separate from output topology.
  Paths resolve against the config directory, without tilde/environment expansion.
  Omitted override values inherit globally; an omitted global path means solid
  theme background. Reject empty paths, unknown modes/outputs/fields.
- `shell.rs`: exact app-ID launcher matching and namespace-to-panel-layer rules.
  These declarations describe placement, not autostart or process toggles.

## Invariants

- The XDG location is `clear/config.toml`. Missing or invalid startup config uses
  safe defaults; atomic last-good reload retention belongs to runtime.
- Resolve relative script paths against the configuration file's directory when
  loading files. Keep parsing from a string distinct from path resolution.
- Supplied arrays replace their respective defaults. Workspace declarations
  configure persistent IDs; runtime/core must not delete existing windows merely
  because a workspace declaration disappears.
- Validate keys/actions, modes, output/workspace references, and theme values
  before applying configuration. Preserve rejection of unknown schema fields.
- `theme.corner_radius` defaults to zero. Accept a scalar or one/two/four-element
  list, expanding two as top/bottom and four clockwise from top-left. The decoration
  deserializer rejects other lengths and nonfinite/out-of-range values (`0..=256`
  logical pixels). Failed reloads must retain the entire last good theme.
- Global `theme.blur_method` accepts `gaussian` (default) or `kawase` (Dual Kawase);
  reject unknown methods. Existing radius-only configurations retain Gaussian.
- `theme.blur_radius` is a finite scalar in `0..=32`, default zero (disables either
  method). Gaussian interprets it as logical-pixel kernel support; Kawase as a sample
  offset in source pyramid texels, not a Gaussian-equivalent radius. Reject lists,
  negative and nonfinite/out-of-range values.
- `theme.blur_passes` is an integer in `1..=6`, default 3. Kawase ceil-halves through
  that many downsample levels, then upsamples through the same levels, stopping
  early at `1×1`. Gaussian ignores it but validation always applies, even with blur
  disabled. `examples/vm.toml` selects Kawase radius 2/passes 3.
- Failed blur reloads preserve the full previous theme. Neither method changes
  client opacity. The independent `[theme.liquid_glass]` table is always validated,
  including when disabled; see [rendering](../../specs/rendering.md#liquid-glass).
- `[theme.titlebar]` is strict/defaulted: finite straight RGBA color arrays in
  `0..=1`, height `16..=128` (default 32), controls_side `left|right` (default
  right), show_icon false, show_title true. Its strict `[theme.titlebar.controls]`
  table accepts optional minimize/maximize/restore/close SVG paths. Reject empty,
  whitespace-only, and NUL paths. Resolve relative paths against the config
  directory without environment/tilde expansion, alongside wallpaper/script paths.
  String parsing does no IO; SVG validation/rasterization belongs to runtime.
  The bounded SVG subset supports paths/gradients, not `use`, `pattern`, `marker`,
  `mask`, `clipPath`, or `filter`: runtime rejects definitions by local name before
  usvg conversion, independent of namespace, usage, CSS, or href references.
  Unsupported explicit resources use built-ins at startup and reject reloads;
  unsupported app icons use the generic fallback.
- Shell matches are exact and case-sensitive. Defaults recognize `wofi` and force
  namespace `waybar` to `top`. Reject empty/duplicate names; empty arrays disable
  their respective defaults.
- Panel rules override only layer. Anchors, margins, sizing, and exclusive zones
  remain client-owned and are interpreted in the Smithay adapter.
- Update `examples/config.toml`, `examples/vm.toml`, and user documentation for
  schema changes. Do not silently claim runtime output-topology reload support.

## Verification

Start with `cargo test --locked --test shell --test extensions --test runtime`,
plus `cargo test --locked --test rounded --test blur --test titlebar_theme` for
theme schema/reload and titlebar path/resource coverage,
then the root workflow. Test defaults, malformed input, independent array
replacement, example parsing, startup fallback, and failed-reload retention.
