# Configuration schema and defaults

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities

- `mod.rs`: TOML deserialization, defaults, validation, configuration paths, and
  loading errors/fallback. Keep schema values backend-independent.
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
- Shell matches are exact and case-sensitive. Defaults recognize `wofi` and force
  namespace `waybar` to `top`. Reject empty/duplicate names; empty arrays disable
  their respective defaults.
- Panel rules override only layer. Anchors, margins, sizing, and exclusive zones
  remain client-owned and are interpreted in the Smithay adapter.
- Update `examples/config.toml`, `examples/vm.toml`, and user documentation for
  schema changes. Do not silently claim runtime output-topology reload support.

## Verification

Start with `cargo test --locked --test shell --test extensions --test runtime`,
then the root workflow. Test defaults, malformed input, independent array
replacement, example parsing, startup fallback, and failed-reload retention.
