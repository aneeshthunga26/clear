# Rust source guidance

Read [the project guidance](../AGENTS.md) first. This file applies to all Rust
source; each module's guidance adds its local responsibilities and invariants.

Read the relevant [component specification](../specs/README.md) before changing
behavior, and update it in the same change as the implementation. Module guidance
describes ownership and engineering practice; specs own the behavior contract.

## Entry points

- `lib.rs` declares the public modules. Keep its exports backend-independent
  except for the explicit platform entry point.
- `main.rs` owns CLI parsing and startup, not desktop or protocol policy.
- Pass launch options through `runtime::Options`; delegate execution to the
  platform. Keep Smithay imports inside `platform/smithay/`.
- Preserve bounded-run options, explicit socket selection, and capture support.
  `--command` consumes the remaining executable/argument vector without shell
  expansion; keep help text, argument validation, and documentation consistent.

## Module guidance

- [core](core/AGENTS.md): desktop state and commands.
- [management](management/AGENTS.md): built-in geometry policies.
- [input](input/AGENTS.md): typed actions and shortcut parsing.
- [config](config/AGENTS.md): TOML schema, defaults, and shell rules.
- [runtime](runtime/AGENTS.md): orchestration, reload, and script routing.
- [scripting](scripting/AGENTS.md): bounded Rhai host and validation.
- [decoration](decoration/AGENTS.md): theme descriptions.
- [shell](shell/AGENTS.md): toolkit-independent state, commands, and local IPC.
- [platform](platform/AGENTS.md): adapter entry point; also read the nested
  [Smithay guidance](platform/smithay/AGENTS.md) for backend work.

## Verification

Follow the root Rust verification workflow. CLI changes should extend the unit
tests in `main.rs`; `cargo test --locked --bin clear` runs them specifically.
Update [the platform specification](../specs/platform.md) when flags or supported
behavior change, and adjust README usage examples or links where necessary.
