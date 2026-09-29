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
| Behavior specifications                                  | [specs/AGENTS.md](specs/AGENTS.md)                              |
| Architecture, integration, and testing guides              | [docs/AGENTS.md](docs/AGENTS.md)                                |

See [specs/README.md](specs/README.md) for implemented behavior and limitations,
[docs/architecture.md](docs/architecture.md) for code organization, and
[README.md](README.md) for setup.

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

## Specifications and behavior changes

[specs/README.md](specs/README.md) is the canonical reference for implemented
behavior. Read the relevant component specification alongside its module guidance
before changing code. Every new implementation, behavior change, or bug fix that
changes a contract MUST add or update the relevant specification in the same
change. This includes defaults, validation, state transitions, failure handling,
resource limits, and supported/unsupported behavior. Work is not complete while
implemented behavior is missing from or contradicts its specification.

For a new component, add a focused specification and link it from the spec index.
Ground requirements in code and tests, and link to that evidence. For internal
refactors that preserve behavior, confirm that the existing spec still describes
the result; do not invent a behavior change. Keep proposals clearly separate from
implemented contracts, and surface discrepancies rather than silently broadening
support claims.

Keep each contract in one specification. README and `docs/` should point to it
instead of restating defaults, lifecycle rules, protocol schemas, or invariants.
Guides retain setup, examples, architecture rationale, testing procedures, and
historical validation records. Module `AGENTS.md` files retain ownership and
engineering guidance and should link to specs for detailed behavior.

Key cross-component contracts:

- [Desktop state](specs/desktop.md): ownership, groups, focus, preserved floating
  geometry, maximize/minimize, and explicit effects.
- [Layouts](specs/layouts.md) and [input](specs/input.md): saved proportions,
  resize invalidation, gesture authorization, and physical release suppression.
- [Configuration](specs/configuration.md) and [Rhai](specs/scripting.md): defaults,
  atomic reload, shell classification, declarative scripts, and bounded execution.
- [Platform](specs/platform.md): requested versus committed geometry, common
  render/input clips, launcher sizing, and layer mapping/focus lifecycle.
- [Decorations](specs/decorations.md), [rendering](specs/rendering.md), and
  [wallpapers](specs/wallpaper.md): negotiated insets, prepared resources, alpha,
  outlines, output-local filtering, and cache bounds.
- [Shell IPC](specs/shell.md): optional toolkit-independent model, strict allowlist,
  private endpoints, and bounded transport.

## Style

Use idiomatic Rust and `cargo fmt`. Favor small concrete modules over abstract
frameworks. Public types and methods documenting local behavior should have short
doc comments. Inline comments should explain non-obvious protocol timing, state
ownership, or safety—not restate assignments.

## Verification

For Markdown-only changes, read the relevant code and tests and review the text
and links. Do not run builds, tests, formatters, or compositor sessions unless
the user requests them; reading a test is not evidence of a passing run.

After Rust changes run `cargo fmt`, `cargo check --locked`, and `cargo test --locked`.
Check editor diagnostics. Use a bounded runtime for compositor tests, such as
`--exit-after 15`; don't start unbounded servers through agent terminal tools.

Prefer an isolated VM for compositor integration tests:

- `python3 scripts/vm-smoke.py`: isolated virtual KWin host and real foot clients;
  also run with `--script examples/columns.rhai` for the Rhai path.
- `python3 scripts/vm-layer-smoke.py --binary target/debug/clear`: real layer-shell
  and XDG clients checking lifecycle, reservations, stacking, keyboard ownership,
  panel popups, and client-sized launcher centering.
- `python3 -B scripts/vm-wallpaper-smoke.py --binary target/debug/clear`: generated
  PNGs and GPU captures checking per-output selection, all four scaling modes,
  full-output placement despite panel reservations, and layer priority.
- `python3 -B scripts/vm-window-state-smoke.py --binary target/debug/clear`: real
  XDG maximize/minimize/restore, pre-map maximize, reservations, focus, IPC state,
  and GPU visibility checks.
- `python3 -B scripts/vm-rounded-smoke.py --binary target/debug/clear`: GPU checks
  for configurable rounded outlines, transparency, subsurfaces, geometry offsets,
  and unchanged layer/popup shapes.
- `python3 -B scripts/vm-decoration-smoke.py --binary target/debug/clear`: XDG/KDE
  decoration negotiation, commit timing/lifecycle, title/control pixels, content
  insets, launcher centering, maximize, reservations, and popup origins. All 11
  original configurable `titlebar-*` GPU cases passed on private LOCAL virtual KWin,
  not a VM: `target/titlebar-geometry/` and `target/titlebar-colors/`. Both square
  GPU regressions passed in `target/titlebar-square/`; eight selected negotiation
  cases passed in `target/titlebar-negotiation/`. Kawase radius 2/passes 3
  `rounded-ssd` passed in `target/titlebar-blur-regression/`. No physical input was
  tested, and not all legacy cases were rerun; see `docs/vm-testing.md` for exact
  cases and bounded commands.
- `python3 -B scripts/test_vm_decoration_smoke.py`: 14 compositor-free harness/oracle
  tests, including two square regressions, passed. All 10 titlebar schema/resource
  tests and full Cargo fmt/check/test/build passed after review fixes. CPU results
  do not establish GPU or physical-input behavior.
- `python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear`: independent
  Gaussian/Dual Kawase GPU oracles for windows, all layer kinds, popups, stacking,
  and output boundaries, including sharp foreground content and transparent/rounded
  holes. Select `--method gaussian|kawase`, `--radius` (defaults 12/2 respectively),
  and `--passes 1..6` (default 3). Include `--case output-boundary-odd` for odd-sized,
  nonzero-origin viewport sampling; use separate `--artifacts` directories per run.
  Recorded GPU passes used private local virtual KWin, not a VM: Kawase radius
  2/passes 3 covered all ten cases; radius 1.5/passes 1 and 6 covered `stacking` and
  `output-boundary-odd`. Gaussian radius 12 covered only `xdg`, `stacking`, and
  `output-boundary-odd` in this round. No physical input tests were performed.
  See `docs/vm-testing.md` for the validation record and artifact paths.
- `python3 -B scripts/test_vm_blur_smoke.py`: compositor-free Dual Kawase oracle
  and harness regressions, not GPU validation.
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
