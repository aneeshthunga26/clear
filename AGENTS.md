# Agent Guidance

## Project

Clear is a Rust 2024 Wayland compositor built on pinned Smithay. It currently
runs nested through winit with virtual outputs; native DRM/KMS is not implemented.

## Local environment notes

If `.agents/AGENTS.md` exists, read it for this checkout's local development and
test environment. The entire `.agents/` directory is git-ignored. Keep personal
names, machine paths, SSH endpoints, VM identities, and backup/recovery notes
there, never in tracked documentation, examples, or scripts. Tracked guidance
must work without that directory; never invent local connection details.

## Module guidance

Read this file first, then the `AGENTS.md` for the directory being changed.
Parent guidance also applies to nested modules. Keep these files updated when
responsibilities, invariants, or verification workflows change; use the standard
`AGENTS.md` filename for new module directories.

| Scope                                                   | Guidance                                                         |
| ------------------------------------------------------- | ---------------------------------------------------------------- |
| Crate exports and CLI (`src/lib.rs`, `src/main.rs`)     | [src/AGENTS.md](src/AGENTS.md)                                   |
| Desktop state, geometry, and commands                   | [src/core/AGENTS.md](src/core/AGENTS.md)                         |
| Built-in layout policies                                | [src/management/AGENTS.md](src/management/AGENTS.md)             |
| Typed actions and shortcut parsing                      | [src/input/AGENTS.md](src/input/AGENTS.md)                       |
| TOML schema, defaults, and shell rules                  | [src/config/AGENTS.md](src/config/AGENTS.md)                     |
| Orchestration, reload, and script routing               | [src/runtime/AGENTS.md](src/runtime/AGENTS.md)                   |
| Rhai extensions and validation                          | [src/scripting/AGENTS.md](src/scripting/AGENTS.md)               |
| Optional shell state, commands, and local IPC           | [src/shell/AGENTS.md](src/shell/AGENTS.md)                       |
| Backend-independent theme descriptions                  | [src/decoration/AGENTS.md](src/decoration/AGENTS.md)             |
| Platform entry point and adapter boundary               | [src/platform/AGENTS.md](src/platform/AGENTS.md)                 |
| Smithay protocols, rendering, input, and nested backend | [src/platform/smithay/AGENTS.md](src/platform/smithay/AGENTS.md) |

See [docs/architecture.md](docs/architecture.md) for the full design and
[README.md](README.md) for supported behavior and current limitations.

## Shared boundaries

- Keep all Smithay and Wayland types in `src/platform/smithay/`.
- Desktop state, workspaces, output groups, focus, IDs, geometry, and commands live
  in `src/core/`; use explicit, backend-independent data structures.
- Built-in layout geometry lives in `src/management/`.
- Key parsing and typed actions live in `src/input/`; physical event translation
  and protocol serials belong in the platform adapter.
- Config parsing lives in `src/config/`; orchestration, reload and script routing
  live in `src/runtime/`.
- User extensions use Rhai, not Lua or JavaScript. Keep script values declarative;
  scripts never own compositor objects or mutate desktop state directly.
- Theme descriptions live in `src/decoration/`; rendering stays in the adapter.
- Shell state and validated commands live in `src/shell/`, independent of any
  UI toolkit. Quickshell is an optional example client, never a compositor
  dependency or mandatory startup process. Preserve bounded private IPC.

## Configuration and behavior

See `examples/config.toml` and `examples/vm.toml` for the current schema. The XDG
path is `clear/config.toml`. Missing or invalid startup config falls back to safe
defaults. Failed reloads retain the last good configuration. Output topology
changes currently require a restart.

Workspaces own windows; output groups present workspaces. A workspace can be
visible in only one group. Per-output modes are workspace-specific overrides.
Preserve window state when changing modes or output topology. Do not conflate
client-committed geometry with compositor-requested geometry.

Floating rectangles may extend offscreen; never clamp saved geometry because of
panel reservations or temporary output sizes. Rendering and hit-testing must use
the same visible workspace/output-group clips. Launcher geometry is client-sized
and separate from saved floating geometry.

Layer-shell panels stay above normal windows on top/overlay layers. Reservations
and keyboard ownership follow mapping lifecycle, not merely object existence.
Track suppressed key releases by physical code.

## Style

Use idiomatic Rust and `cargo fmt`. Favor small concrete modules over abstract
frameworks. Public types and methods documenting local behavior should have short
doc comments. Inline comments should explain non-obvious protocol timing, state
ownership, or safety—not restate assignments.

## Verification

After Rust changes run `cargo fmt`, `cargo check --locked`, and `cargo test --locked`.
Check editor diagnostics. Use a bounded runtime for compositor tests, such as
`--exit-after 15`; don't start unbounded servers through agent terminal tools.

Prefer an isolated VM for compositor integration tests:

- `python3 scripts/vm-smoke.py`: isolated virtual KWin host and real foot clients;
  also run with `--script examples/columns.rhai` for the Rhai path.
- `python3 scripts/vm-layer-smoke.py --binary target/debug/clear`: real layer-shell
  and XDG clients checking lifecycle, reservations, stacking, keyboard ownership,
  panel popups, and client-sized launcher centering.
- `python3 -B scripts/test_vm_layer_smoke.py`: fixture assertion self-tests, with
  no compositor needed.
- `python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear`: local shell
  IPC integration; add `--quickshell quickshell` for the optional real panel.
- Add `--exercise-overlays` to the Quickshell smoke to test launcher, notification
  center, and app preview lifecycles with a real Alacritty desktop entry.
- `node --test examples/quickshell/Protocol.test.mjs`: optional example's pure
  message/model tests, without launching a shell.

Logs and PPM captures stay under `target/`. These tests do not automate physical
clicks, key presses, or drags; state explicitly what was verified. See
[docs/vm-testing.md](docs/vm-testing.md) for portable bounded commands and test
dependencies. Consult `.agents/` for any locally configured VM access and recovery
notes. Do not modify the host desktop or VM privileges unnecessarily. Never store
passwords or private keys in this repository, including ignored directories.
