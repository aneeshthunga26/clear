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
and separate from saved floating geometry. Tiled resize gestures change persistent
layout proportions, not floating flags or saved floating rectangles. Spiral tiles
cannot resize; floating exceptions can. Wallpapers are optional and compositor-
rendered behind all layer-shell surfaces, using full output rectangles.
Rounded window bodies share the same original outline for rendering and hit tests;
output crops must not create new rounded corners. Panels and popups keep their
own shapes. Composite body subsurfaces before applying rounded coverage once.
Server-side titlebars default on negotiated XDG/KDE decorations; honor explicit
client-side requests and leave non-negotiating clients alone. Use pending mode for
configure sizes, committed mode for rendering/input. Core placements are total
frames; titlebar insets belong to the adapter, never saved-geometry mutations.
`[theme.titlebar]` has four straight RGBA colors (`active_background`,
`inactive_background`, `active_foreground`, `inactive_foreground`), exactly four
finite channels in `0..=1`; integer height `16..=128` (default 32), controls_side
left/right (default right), show_icon false, and show_title true. Controls mirror
minimize/maximize-or-restore/close on the right to close/maximize-or-restore/minimize
on the left. `[theme.titlebar.controls]` optionally supplies local SVG paths for
minimize/maximize/restore/close relative to the config; omitted restore falls back
to maximize SVG, then built-ins. Keep nonexistent SVG examples commented out.
Runtime prepares bounded SVG/app-icon CPU resources outside rendering; minimal
resvg disables external/network/embedded images, SVG text/fonts, and SVGZ. Reject
DTD/entities and use/pattern/marker/mask/clipPath/filter elements before usvg
conversion, including unused/namespaced definitions; only the restrictive path/
gradient subset is supported, not masks/clips/filters. App icons use exact desktop IDs and
bounded hicolor/direct-icon/pixmaps lookup, not a full theme resolver. Missing app
icons use a generic glyph; startup control resource failures use built-ins; failed
reloads retain the whole last-good config/resource set. Reload rereads same-path
SVGs. Preserve RGBA alpha through source-over and rounded shaders, with no forced
opaque fill or accent stripe. Square SSD borders are rings even without blur; keep
legacy square unblurred CSD backing unchanged. Dynamic height changes frame insets/configures, never
saved geometry. Titlebar texture cache limits are both 64 MiB and 128 entries,
excluding outstanding elements/driver overhead; see architecture for resource bounds.
Backdrop blur is global: `blur_method` is `gaussian` (default) or `kawase` (Dual
Kawase); finite `blur_radius` in `0..=32` defaults to zero, disabling either method.
Gaussian radius is logical-pixel support; Kawase radius is a source-pyramid-texel
offset. Integer `blur_passes` in `1..=6` defaults to 3, ignored by Gaussian but always
validated. Kawase uses ceil-half downsample levels and the same upsample levels,
stopping early at `1×1`; the VM example uses radius 2/passes 3. Filter the lower
scene, not foreground content, once per composed window/layer/popup tree. Keep
rounded coverage distinct from client alpha; preserve holes, stacking, and
output/workspace sample boundaries. Neither method adds a glass treatment.

Maximize uses a window's home output usable area, not the whole output group.
Maximize/minimize preserve saved geometry, floating flags, and layout proportions.
Minimized windows remain owned/mapped but have no placements; explicit focus or
Alt-Tab restores them, while workspace switching does not. Launchers stay client-sized.

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
