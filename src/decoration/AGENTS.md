# Decoration descriptions

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` defines backend-independent `Theme` data and its defaults. Current
styling covers background, active/inactive border colors, and border width.
Rendering, surface lifetimes, and GPU buffers belong in `platform/smithay/`.

- Theme colors are non-premultiplied RGBA; the adapter performs renderer-specific
  conversion. Keep logical border thickness distinct from physical scaling.
- Keep schema validation in configuration and defaults consistent with examples.
- Do not place Smithay types, protocol decorations, or a rendering loop here.
- Client titlebars are not Clear-provided decoration widgets. Do not document a
  rich decoration scripting API until it is implemented across the boundaries.
- Future styling descriptions should remain declarative rather than giving
  extensions ownership of compositor objects.

## Verification

Use `cargo test --locked --test extensions` for configuration validation and the
root workflow. For renderer-visible changes, use bounded VM captures and inspect
color/geometry output; parsing tests alone do not validate pixels.
