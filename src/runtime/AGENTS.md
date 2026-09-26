# Runtime orchestration

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` connects configuration, compiled bindings, desktop commands, and Rhai.
It also defines backend-neutral launch `Options`. Keep Smithay and protocol
objects out of this module; return effects for the platform to execute.

- Startup may fall back to safe defaults. Reload must prepare and validate the
  candidate config, bindings, and script host before replacing live state.
  Rejected reloads retain the last good configuration and behavior.
- Output topology changes currently require restart. Do not partially apply an
  unsupported topology reload.
- Apply workspace declarations to persistent desktop IDs without discarding
  existing windows or unrelated saved state.
- Classify XDG app IDs into core window roles on mapping/metadata changes and
  reclassify all managed windows after successful reload, including hidden ones.
  Classification must preserve ownership, focus, and saved floating geometry.
- Script actions return validated declarative actions; never let scripts mutate
  the desktop or own compositor objects directly.
- Keep per-function failure handling: disable failing script functions until
  reload and use the built-in layout fallback rather than failing the compositor.
- Spawning, closing clients, and shutdown are explicit effects, not OS/protocol
  operations performed by policy code.
- Pending Alt-Tab selection contains normal windows on the focused workspace.
  Advancing it must not change focus; finish commits the selected window and
  cancel preserves focus. The adapter owns physical release/cancel detection.

## Verification

Start with `cargo test --locked --test runtime`, then the root workflow. Cover
command routing, reload atomicity, script failure/fallback, hidden-window
reclassification, and preserved state. Use the VM Rhai smoke for adapter-level
integration; a pure runtime test does not verify physical shortcuts.
