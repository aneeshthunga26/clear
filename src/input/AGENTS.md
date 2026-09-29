# Typed actions and bindings

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` defines serializable `Action`s, binding declarations, modifiers,
shortcut parsing/normalization, leader substitution, and default bindings.
This is not the physical input adapter: seats, keycodes, protocol serials,
keyboard ownership, grabs, and pointer events belong in `platform/smithay/`.

- Keep actions declarative and validate their arguments before execution.
  Runtime routes actions; platform code executes returned effects.
- Preserve rejection of malformed chords, invalid actions, and duplicate
  normalized shortcuts.
- The configured leader applies to default bindings too. Keep default commands
  and documented shortcuts aligned with the examples and README.
- Binding lookup expects normalized unshifted key names. Smithay must translate
  actual keysyms with `xkb::keysym_get_name`, not debug labels such as `XK_Return`.
- Spawn actions carry executable/argument vectors, not shell command strings.
- `alt_tab` is a declarative gesture advance. The Smithay adapter detects the
  physical Alt release and Escape cancellation; runtime retains pending selection.
- `toggle_maximized` (leader+Up) and `minimize` (leader+Down) operate on the
  focused normal window. Restore minimized windows via explicit focus/Alt-Tab.
- When extending `Action`, update validation, runtime dispatch, Rhai action
  conversion as needed, and config examples together.

## Verification

Use `cargo test --locked --test extensions` and
`cargo test --locked --test runtime`, then the root workflow. Physical-key
translation regressions live in `platform/smithay/input.rs`; run them with
`cargo test --locked --lib platform::smithay::input`.
