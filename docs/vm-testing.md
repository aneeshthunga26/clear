# Isolated compositor testing

The [component specifications](../specs/README.md) define expected behavior.
This guide owns test setup, commands, fixture coverage, troubleshooting, and
historical validation records. Scenario assertions below describe what each
fixture checks; they are not a second behavior specification or a claim of a new run.

These instructions are portable across Linux development environments. Prefer
an isolated VM for compositor integration tests. Install Rust, the native build
requirements listed in the README, Python 3, KWin, D-Bus tools, and foot in that
environment. Neither passwordless sudo nor a logged-in graphical session is
needed for the automated runners.

Personal SSH endpoints, usernames, machine paths, VM configuration, and recovery
notes belong in the git-ignored `.agents/` directory. If it exists, consult
`.agents/AGENTS.md` for local access instructions. Do not assume a particular VM,
distribution, SSH agent, user ID, or display socket.

## Build and automated smoke tests

Run from the checkout root inside the test environment:

```sh
cargo build --locked
cargo test --locked
python3 -B scripts/vm-smoke.py --binary target/debug/clear --artifacts target/vm-smoke
python3 -B scripts/vm-smoke.py --binary target/debug/clear --script examples/columns.rhai --artifacts target/vm-rhai
```

When syncing a checkout to a test machine, exclude `.git/`, `.agents/`, `target/`,
`build/`, `.cache/`, and `__pycache__/`. Build artifacts and compilation databases
are machine-specific; private environment notes should stay on the machine that
owns them.

The runner uses a private D-Bus session and virtual KWin host. It does not stop the
login manager or interact with another desktop. It checks real client mapping and
rendering, not physical input. Unit/runtime tests cover workspace and mode commands.
PPM captures can be viewed directly or converted with ImageMagick.

## Maximize, minimize, and restore

Contract: [window state](../specs/desktop.md#maximize-and-minimize) and
[XDG lifecycle](../specs/platform.md#xdg-window-lifecycle-and-configures).

```sh
python3 -B scripts/vm-window-state-smoke.py --binary target/debug/clear
```

Four bounded runs use the existing real SHM/XDG fixture and shell IPC to check
native maximize/unmaximize/minimize, a maximize request before first mapping,
restoration through explicit focus, retained window identity/maximize state,
keyboard focus transfer, tile restoration, and live panel reservation changes.
GPU captures distinguish a maximized window, a minimized window (only its neighbor
visible), and restored tiles, with the panel always above them. IPC setters used
by shell window cards are exercised too. No physical clicks/keys are injected.

The fixture dependencies are the same as the layer-shell suite below. Each Clear
instance runs for 12 seconds; allow 90 seconds overall. Use `--seconds 20` for slow
guests or repeat `--case maximized`, `--case minimized`, `--case restored`, and
`--case initial-maximized` to select checks. Logs, client traces, results, and PPM
captures remain under `target/vm-window-state-smoke/` by default.

## Wallpapers

Contract: [wallpaper selection, placement, and resources](../specs/wallpaper.md).

```sh
python3 -B scripts/vm-wallpaper-smoke.py --binary target/debug/clear --artifacts target/vm-wallpaper-smoke
```

This bounded test generates small PNG fixtures with Python's standard library and
checks real GPU captures for `fill`, `fit`, `stretch`, and `center`. Each run uses
two virtual outputs with different images, a reserved top panel, and a background
layer-shell surface. Samples distinguish cropping from stretching, verify image
orientation, fit/center margins and placement in the full output rather than its
usable area, and ensure even background-layer surfaces render above wallpapers.
Quickshell is not required. The existing layer C fixture is built with the same
dependencies and optional `--layer-xml` override described below.

Four sequential Clear instances are bounded by `--exit-after 8`; allow 90 seconds
for the suite. Use `--seconds 15` on slower guests. Logs, generated images/configs,
client protocol traces, and captures stay under the selected artifacts directory.
This checks rendered wallpaper behavior, not keyboard reloads or pointer drags;
resource reload/failure semantics and resize geometry are covered by Rust tests.

## Rounded window outlines

Contract: [rounded outlines](../specs/rendering.md#rounded-outlines).

```sh
python3 -B scripts/vm-rounded-smoke.py --binary target/debug/clear
```

Eleven bounded cases check scalar, top/bottom, four-corner, oversized, opposing
corner, borderless, translucent border, translucent body, opaque/translucent
subsurface, and square outlines. Real SHM clients place a resized XDG launcher
with a nonzero geometry offset over another window, beside a reserved top panel
and its popup. GPU pixel assertions check fitted radii, corner ordering, cut-outs,
content/border antialiasing and alpha, subsurface composition/orientation, and
unchanged panel/popup shapes. The oracle compares partially covered edge pixels,
not just solid interiors. No Quickshell or compositor test hooks are needed.

Dependencies and `--layer-xml` are the same as the layer-shell fixture below.
Each Clear instance runs for eight seconds; allow two minutes for all cases.
Use `--seconds 15` for slower environments, or repeat `--case all`, `--case four`,
`--case translucent-body`, etc. to select cases. Logs, client traces, configs,
results, and PPM captures stay under `target/vm-rounded-smoke/` by default.

This suite does not synthesize physical clicks, keys, or drags. Rust tests cover
shape hit regions, accepted/rejected configuration forms, and atomic reload;
interactive pointer routing still needs manual verification.

## Server-side decorations

Contract: [decoration negotiation, titlebars, and resources](../specs/decorations.md).

```sh
python3 -B scripts/vm-decoration-smoke.py --binary target/debug/clear --artifacts target/vm-decoration-smoke
python3 -B scripts/test_vm_decoration_smoke.py
```

**Recorded titlebar GPU passes — private LOCAL virtual KWin, not a VM:**

| Passed cases                                                                                                  | Artifacts                   |
| ------------------------------------------------------------------------------------------------------------- | --------------------------- |
| `titlebar-height`, `titlebar-launcher`, `titlebar-popup`, `titlebar-left`, `titlebar-hidden`, `titlebar-icon` | `target/titlebar-geometry/` |
| `titlebar-translucent`, `titlebar-inactive`, `titlebar-svg`, `titlebar-svg-small`, `titlebar-svg-restore`     | `target/titlebar-colors/`   |

| `titlebar-translucent-square`, `titlebar-transparent-square` | `target/titlebar-square/` |
| `xdg-default`, `xdg-client`, `to-ssd-held`, `to-ssd-committed`, `to-csd-held`, `to-csd-committed`, `kde-default`, `kde-client` | `target/titlebar-negotiation/` |
| Kawase radius 2/passes 3 `rounded-ssd` | `target/titlebar-blur-regression/` |

These runs cover all 13 configurable-titlebar cases, eight selected negotiation
cases, and the focused blur regression—not all legacy protocol cases. No VM run
or physical input was tested. Cargo fmt/check/test/build passed after fixes, as did
all 14 CPU harness/oracle tests (including two square regressions) and all 10
schema/resource tests.

Real SHM clients negotiate XDG and legacy KDE decoration modes on a private virtual
KWin display. Cases cover default/explicit server-side selection, explicit client-side
selection, no negotiation, unset preferences, object destruction/replacement, remap,
and held commits. Captures distinguish ACKed-but-uncommitted mode changes from those
applied with the root buffer, including decoration destruction. Trace assertions
reject configures before the initial bufferless handshake and transient CSD on remap.

GPU checks cover title text and updates, app-ID fallback, all three control icons,
combined rounded frame/content, geometry offsets, launcher sizing, maximization,
panel reservations, and popup content origins. Installed fonts are required for text
assertions. Dependencies otherwise match the layer fixture below; the build also
generates XDG/KDE decoration bindings from installed/cargo-registry protocol XML.

The 11 passed configurable-titlebar cases check:

| Case                   | Assertions                                                                             |
| ---------------------- | -------------------------------------------------------------------------------------- |
| `titlebar-height`      | 48-pixel titlebar, frame/content inset and changing panel reservations                 |
| `titlebar-launcher`    | Client-sized launcher centering with the custom height and geometry offset             |
| `titlebar-popup`       | Popup origin relative to custom-height launcher content                                |
| `titlebar-translucent` | Background alpha over two differently colored real lower surfaces and rounded cut-outs |
| `titlebar-inactive`    | Active/inactive background and foreground RGBA on adjacent windows                     |
| `titlebar-left`        | Mirrored controls and title placement on the left                                      |
| `titlebar-hidden`      | Hidden title text without hiding controls                                              |
| `titlebar-icon`        | Generic app-icon fallback for an intentionally unknown desktop ID                      |
| `titlebar-svg`         | Generated config-relative SVGs retain literal colors at height 48                      |
| `titlebar-svg-small`   | SVG size/placement at height 24                                                        |
| `titlebar-svg-restore` | Maximization selects a distinct restore SVG                                            |

Rerun just those 11 cases with separate artifacts, preserving the recorded runs:

```sh
python3 -B scripts/vm-decoration-smoke.py --binary target/debug/clear --seconds 12 --artifacts target/vm-decoration-titlebars \
  --case titlebar-height --case titlebar-launcher --case titlebar-popup \
  --case titlebar-translucent --case titlebar-inactive --case titlebar-left \
  --case titlebar-hidden --case titlebar-icon --case titlebar-svg \
  --case titlebar-svg-small --case titlebar-svg-restore
```

The two added square regressions use a two-pixel red border, zero corner radius,
and zero blur. They check 50% and fully transparent titlebar backgrounds against two
real lower surfaces, with the border visible only as a ring rather than a backing
fill. Both passed; rerun them with separate artifacts:

```sh
python3 -B scripts/vm-decoration-smoke.py --binary target/debug/clear --seconds 12 --artifacts target/vm-decoration-titlebar-square \
  --case titlebar-translucent-square --case titlebar-transparent-square
```

`--seconds` accepts integers `10..30` (default `12`) and bounds each Clear instance,
not the whole suite. The original 11 cases spend at least 132 seconds in captures;
the two square cases add 24 seconds (156 seconds for all 13 titlebar cases). Allow
additional time for fixture build, KWin startup and checks. Without `--case`, all
38 protocol/titlebar cases run (at least 456 seconds of captures at the default).
Use repeatable `--case` selections for focused checks, `--seconds 20` for slower
systems, or `--build-only` to compile only the C fixture. The runner does not build
Rust. Logs, traces, generated configs/SVGs, JSON results and PPM captures stay in
the selected `target/` artifacts directory.

`scripts/test_vm_decoration_smoke.py` checks the harness/oracle with synthetic CPU
images and mocked protocol traces. Its 14 tests include two regressions rejecting
square titlebar backing fills and missing/incorrect border rings. They do not
exercise a compositor, GPU, actual font rendering, or physical input. The GPU
`titlebar-icon` case checks the
generic fallback, not successful desktop icon-theme resolution. Runtime resource
limits, exact desktop-ID lookup, SVG restrictions, restore fallback and atomic
same-path resource reload are covered by the 10 schema/resource Rust tests. SVG
review regressions include rejection of `mask`, `clipPath`, and `filter` alongside
`use`, `pattern`, and `marker` before usvg conversion, including unused/namespaced
definitions; masks/clips/filters are not part of the supported path/gradient subset.
Adapter tests separately cover square SSD rings without blur while retaining
legacy square unblurred CSD backing:

```sh
cargo test --locked --test titlebar_theme
cargo test --locked --lib platform::smithay
```

The GPU fixture does not exercise reload (the private shell IPC allowlist excludes
it), pointer or keyboard events. Button clicks, dragging and keyboard shortcuts
still need interactive verification. Unit tests separately cover configurable hit
boxes, tiny/wide/clipped frames, mirrored order, alpha composition, cache budgets,
title normalization, negotiation state, drag thresholds and modifier precedence.

## Backdrop blur

Contract: [backdrop composition and filters](../specs/rendering.md#backdrop-composition).

```sh
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --artifacts target/vm-blur-gaussian
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --method kawase --radius 2 --passes 3 --artifacts target/vm-blur-kawase
python3 -B scripts/test_vm_blur_smoke.py
```

Recorded GPU validation passed on **private local virtual KWin, not in a VM**:

| Filter   | Radius / passes | Passed cases                                                 | Artifacts                          |
| -------- | --------------- | ------------------------------------------------------------ | ---------------------------------- |
| Kawase   | 2 / 3           | `xdg`, `rounded-ssd`, `output-boundary-odd`                  | `target/kawase-smoke/`             |
| Kawase   | 2 / 3           | `rounded-ssd` after titlebar review fixes                    | `target/titlebar-blur-regression/` |
| Kawase   | 2 / 3           | All four layer kinds, `popup`, `stacking`, `output-boundary` | `target/kawase-layers/`            |
| Kawase   | 1.5 / 1         | `stacking`, `output-boundary-odd`                            | `target/kawase-p1/`                |
| Kawase   | 1.5 / 6         | `stacking`, `output-boundary-odd`                            | `target/kawase-p6/`                |
| Gaussian | 12 / ignored    | `xdg`, `stacking`, `output-boundary-odd`                     | `target/gaussian-regression/`      |

Together the first two runs cover all ten Kawase cases at radius 2/passes 3.
Full Cargo fmt/check/test/build and all 11 CPU oracle/harness tests also passed.
No physical input tests were performed. No other Gaussian cases or VM runs are
claimed by this validation record; the commands here describe the broader workflow.

The smoke runner accepts:

- `--method gaussian|kawase`: default `gaussian`.
- `--radius`: finite `0..=32`; defaults to `12` for Gaussian or `2` for Kawase.
  These are smoke-runner defaults; compositor defaults and filter units are in
  the [rendering specification](../specs/rendering.md).
- `--passes`: integer `1..=6`, default `3`; forwarded to theme `blur_passes`.

Ten cases compare radius zero and the selected radius against independent Gaussian
or Dual Kawase pixel oracles: XDG, rounded SSD, all four layer-shell categories,
popups, stacking, `output-boundary`, and `output-boundary-odd`. The XDG case also
compares omitted `blur_radius` with explicit zero (21 bounded captures per full run
with nonzero radius). Patterned PNG backgrounds and real SHM clients exercise
transparent content, opaque glyph proxies, composed subsurfaces, holes, rounded
cut-outs, and lower-versus-upper scene order. The Kawase oracle models ceil-half
levels, center-aligned bilinear sampling, source-level offsets, edge clamping, and
RGBA8 quantization after every step, without importing renderer code or deriving
expected blur from captured pixels.

`output-boundary-odd` filters a `321×241` viewport at `(319, 0)` beside a solid
magenta output, exercising odd pyramid dimensions and a nonzero viewport origin.
It selects the second output using private shell IPC, not physical input. For a
focused fractional-radius/depth regression:

```sh
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --method kawase --radius 1.5 --passes 1 --case stacking --case output-boundary-odd --artifacts target/vm-blur-kawase-p1
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --method kawase --radius 1.5 --passes 3 --case stacking --case output-boundary-odd --artifacts target/vm-blur-kawase-p3
python3 -B scripts/vm-blur-smoke.py --binary target/debug/clear --method kawase --radius 1.5 --passes 6 --case stacking --case output-boundary-odd --artifacts target/vm-blur-kawase-p6
```

The existing C fixture is reused; no extra Python packages or shell UI are required.
Use repeated `--case` options for focused runs. Each compositor instance defaults
to eight seconds, configurable with `--seconds 8..30`; this bounds each capture,
not the whole suite. A full nonzero-radius run spends at least 168 seconds in
captures, plus fixture build, startup, and CPU oracle time. `--build-only` checks
fixture compilation without a compositor. Artifacts default to `target/vm-blur-smoke/`;
use distinct `--artifacts` directories under `target/` to preserve each parameter
set's captures and results.

`scripts/test_vm_blur_smoke.py` runs deterministic, compositor-free oracle and
harness regressions, including a rational-coordinate reference for tiny/odd
images, fractional radii, depths 1/3/6, clamping, CLI/config propagation, and assertion
sampling. These self-tests do not validate GPU pixels. GPU smokes check final captures
and protocol state, not physical pointer/keyboard gestures; config validation/reload
is covered by `cargo test --locked --test blur`.

## Optional shell integration

Contract: [shell IPC](../specs/shell.md). Setup: [shell integration](shell-integration.md).

The shell IPC smoke needs only the base test dependencies. Quickshell is an
optional separately installed client, not a build or runtime requirement:

```sh
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --artifacts target/vm-shell-smoke
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell --artifacts target/vm-quickshell
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell --exercise-overlays --artifacts target/vm-overlays
```

Each run uses a bounded compositor (25 seconds), a real foot client, a private
virtual KWin host, and actual Unix sockets. It checks exported socket discovery,
subscriptions, workspace/group/mode commands, invalid requests, reconnects,
disabled IPC, and cleanup. Allow 75 seconds for startup, assertions, and cleanup.
With `--quickshell`, it also runs the example on a private D-Bus session using Qt
Quick's software renderer, checks both output reservations, stops/restarts the
shell without stopping Clear, confirms fresh subscription, and samples panel
pixels in the capture. It does not automate physical clicks or keypresses.
The optional `--exercise-overlays` fixture requires Alacritty. It invokes
Quickshell's launcher and other view methods in a copied QML config, launches
Alacritty through its desktop entry, simulates switcher snapshot state, and
verifies that closing the launcher, notification view, app preview, and switcher
does not disconnect the panels. It still
does not synthesize pointer or keyboard input.

The compositor's CLI child is used to verify environment propagation. Quickshell
is launched separately by the runner using those exported values so the test
can stop/restart only its own shell process group. Logs and captures stay under
the selected artifacts directory. The optional Quickshell process starts its
notification and tray services on the private test bus; the smoke does not send
notifications or tray items.

For the protocol contract, see [shell IPC](../specs/shell.md); for the example,
see [shell integration](shell-integration.md).
Test its pure message/model helpers without a GUI using
`node --test examples/quickshell/Protocol.test.mjs`.

## Layer-shell and launcher integration fixture

Contract: [layer lifecycle](../specs/platform.md#layer-shell-lifecycle),
[launcher geometry](../specs/platform.md#launcher-geometry), and
[scene clips and priority](../specs/platform.md#scene-and-hit-testing).

`scripts/vm-layer-smoke.py` drives real SHM-backed layer-shell and XDG clients
from `scripts/layer-smoke-client.c`. It compiles only this small C fixture, using
installed Wayland headers/libraries and generated protocol bindings. It never
builds Rust or syncs the repository. No sudo, input injection, host desktop
changes, extra Cargo dependencies, or logged-in graphical session are needed.

Compile only the fixture from the checkout root (no compositor run):

```sh
python3 -B scripts/vm-layer-smoke.py --build-only --artifacts target/vm-layer-smoke
```

Against a separately built compositor binary, run:

```sh
python3 -B scripts/vm-layer-smoke.py --binary target/debug/clear --artifacts target/vm-layer-smoke
```

The full suite has sixteen checkpoints and takes approximately three minutes:
one private D-Bus/KWin virtual host, with sixteen sequential Clear instances,
each bounded by `--exit-after 10`.
The C fixture has its own 45-second watchdog. Subprocess waits, builds, and socket
readiness checks are bounded; process groups are cleaned up on normal completion
and Python exceptions. Use a terminal-tool timeout of at least 300 seconds for
the full suite. For a quick targeted run, repeat `--case` as needed:

```sh
python3 -B scripts/vm-layer-smoke.py --binary target/debug/clear --artifacts target/vm-layer-quick --case launcher-resized --case panel-reconfigured --case panel-remapped --case pending-layer-uncommitted --case pending-layer-committed --case layer-popup
```

### What is asserted

| Checkpoints                                            | Protocol and framebuffer assertions                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `panel-configured`                                     | A configured layer with no buffer reserves nothing and does not steal app focus.                                                                                                                                                                                                                                                                |
| `panel-mapped`, `panel-resized`                        | A full-width top panel maps at 32px, then changes height and exclusive zone to 64px. The real XDG tile receives 640x448 then 640x416 configures. Captures check panel edges and tile placement.                                                                                                                                                 |
| `panel-unmapped`, `panel-destroyed`                    | After map and resize, a null-buffer commit or direct destruction releases the reservation; the tile returns to 640x480 and fills the capture.                                                                                                                                                                                                   |
| `panel-reconfigured`, `panel-remapped`                 | After null-buffer unmap, the same layer requests bottom/left/right anchors, 48px height and zone, and performs a fresh bufferless configure/ack before attaching a new buffer. No reservation or panel pixels are allowed before remap; afterward the tile is 640x432 and the panel occupies the bottom 48px.                                   |
| `pending-layer-uncommitted`, `pending-layer-committed` | A mapped bottom layer sends `set_layer(Overlay)` without a surface commit. A separate Wayland client commits its own layer. Keyboard ownership and capture must still show the top layer until the pending layer's own commit, which must then reveal the overlay and transfer focus. Commit counters guard against accidental fixture commits. |
| `layer-popup`                                          | A panel owns a separate XDG popup via `get_popup`, not a subsurface or toplevel. Its 160x100 buffer renders at (120,32), entirely outside the 32px panel body. Captures check popup boundaries, the panel, and surrounding tile pixels; popup configure coordinates and unchanged tile size/focus are also checked.                             |
| `priority-bottom`                                      | A centered bottom layer is hidden beneath the ordinary XDG tile; bottom exclusive keyboard interactivity does not preempt app focus.                                                                                                                                                                                                            |
| `priority-top`, `priority-overlay`                     | Top paints over the tile, overlay paints over top, and exclusive keyboard focus follows the same priority. A newer top surface cannot steal focus from an existing overlay.                                                                                                                                                                     |
| `priority-restored`                                    | Unmapping overlay restores top keyboard focus; destroying top restores app focus. The remaining bottom layer stays hidden.                                                                                                                                                                                                                      |
| `launcher`, `launcher-resized`                         | A launcher commits 180x100, then 260x140, independently of the last compositor configure. It has a 7px nonzero XDG window-geometry offset and contrasting buffer margin. Captures assert exact content bounds centered within the panel-reduced usable area; the ordinary tile retains its size.                                                |

Every checkpoint starts from a new Clear instance and replays the preceding
transitions. This is deliberate: Clear currently captures only once, near exit.
The fixture stays connected and holds the final scene until capture. State
`width`/`height` report the latest committed content geometry (excluding the XDG
buffer margin), or zero while unmapped; `configure_width`/`configure_height`
retain the last compositor configure separately. Configure and buffer-commit
counters distinguish fresh handshakes from stale state. Committed sizes, configure
sizes, and keyboard focus are checked over repeated event-loop turns, rather than
assuming a Wayland roundtrip is a compositor render barrier. Cross-client tests
use explicit roundtrips to order requests before those stability checks. A failure still
waits for the scheduled capture and does not stop later checkpoints.

Artifacts under the selected directory:

- `kwin.log`: isolated host startup/errors.
- `fixture/build.log`, `fixture/build.json`, `fixture/compile_commands.json`:
  compiler output, exact commands, protocol XML paths, and a clangd compilation
  database; generated C bindings and the fixture executable stay here.
- `<case>/config.toml`, `clear.log`, `client.jsonl`, `frame.ppm`, `result.json`:
  exact config, compositor log, client commands/configures/commits/keyboard
  enter-leave events, GPU capture, and assertion results. `peer.jsonl` records the
  second connection in pending-layer cases (empty for other cases).
- `results.json`: suite summary; any failed checkpoint makes the runner exit 1.

`cc`, `pkg-config`, `wayland-scanner`, `wayland-client`, and `wayland-protocols`
are required. The wlr layer-shell XML is discovered in `CARGO_HOME/registry/src`
(or `~/.cargo/registry/src`); use `--layer-xml /absolute/path/to/wlr-layer-shell-unstable-v1.xml`
if necessary. No download occurs. The fixture compiles with
`-Wall -Wextra -Werror`. `set_layer` cases require layer-shell version 2; unsupported
protocol versions fail explicitly rather than silently skipping the regression.
A successful fixture build or assertion self-test is not an integration pass.

State and capture assertion self-tests run locally without Wayland or a compositor.
They reject stale launcher sizes, stale panel placement/reservations, prematurely
applied or lost pending layer state, and missing/clipped/displaced popup pixels:

```sh
python3 -B scripts/test_vm_layer_smoke.py
```

### C editor diagnostics

Protocol headers are generated build artifacts, not checked-in source. The
fixture build always writes `fixture/compile_commands.json`. For local clangd
header discovery, optionally create a project-local database in `build/`:

```sh
python3 -B scripts/vm-layer-smoke.py --build-only --artifacts target/vm-layer-smoke-fixture-check --compile-commands build/compile_commands.json
clangd --check=scripts/layer-smoke-client.c --compile-commands-dir=build
```

`--compile-commands` refuses to overwrite an existing database. It contains
absolute paths for this checkout; do not copy it between host and VM or commit it
(or generated headers). If already present, omit the option on subsequent builds
using the same artifact directory. Restart Zed's clangd language server if it
cached missing-header diagnostics before the database was generated. No global
editor configuration is needed.

### Scope and troubleshooting

This fixture checks protocol-driven keyboard **ownership** using actual
`wl_keyboard.enter/leave` events; it does not synthesize key presses or clicks.
On-demand click focus, pointer hit-test priority, physical key delivery,
offscreen floating-window drag/resize, nested popup chains or popup grabs,
and multi-output reservations are not covered. The existing foot smoke still
provides separate real-terminal coverage. Normal floating rectangles cannot be
positioned arbitrarily through XDG-shell, so offscreen float coverage needs a
separate safe input-injection path rather than compositor test-only hooks.

A baseline focus failure means the nested Clear window did not receive keyboard
focus from the isolated host, or the compositor failed to focus the mapped app;
inspect both logs. A framebuffer size other than 640x480 fails explicitly instead
of silently sampling incorrect coordinates. A slow guest can use `--seconds 20`
(allowed range 8..30). If a scenario finishes after capture, the runner fails and
asks for a longer bound; it never accepts a stale frame. Assertion failures are
real failures, not expected failures or automatic skips.

## Interactive run

Expected gesture and state behavior is defined by [input](../specs/input.md),
[resizing](../specs/layouts.md#resize-sessions), and
[window state](../specs/desktop.md#maximize-and-minimize).

Log into a graphical desktop in the test environment, then run from the checkout
root in a terminal belonging to that session:

```sh
./target/debug/clear --config examples/vm.toml --exit-after 120 --command foot
```

For remote interactive runs, use the actual session's runtime directory and
display socket. Those values depend on the login environment; a greeter alone
does not establish a user graphical session. Keep machine-specific commands in
`.agents/`, not in this guide.

Demo sequence (leader is Ctrl+Alt):

1. Open three terminals with leader+Return.
2. Stretch with leader+s.
3. Move the focused terminal right with leader+Shift+Right.
4. Focus either region with leader+o and cycle its mode with leader+m.
5. Switch the whole group using leader+3; return with leader+1.
6. Split with leader+Shift+s and verify independent switching.
7. Super+right-drag near an internal tile boundary. In master/stack, change the
   master width and the split between stack windows; cycle modes to check columns,
   rows, grid, and scrolling widths. Tiles must remain tiled, and returning to a
   previous mode should restore its proportions.
8. In spiral, verify a tiled resize does nothing. Toggle a floating exception
   with leader+f and verify all edges resize, including offscreen geometry. Moving
   a tile with Super+left still detaches it. Launchers remain client-sized.
9. Toggle maximize with leader+Up: only the current output's usable area should
   fill, keeping its panel visible. Toggle again to restore the tile/float.
10. Minimize with leader+Down. Select the window in Alt+Tab or the dock to restore;
    a previously maximized window should return maximized. Also try client titlebar
    controls and the Quickshell hover-card buttons.
11. Quit with leader+Escape or close the outer window.

Host KDE shortcuts may still take precedence; use bindings that your host leaves
unused. Super+mouse gestures are independent of the configured keyboard leader.

## Environment safety

Use a disposable or backed-up test environment and record its recovery procedure
privately under `.agents/`. Do not change host desktop settings or grant extra VM
privileges just to run these nested tests. Never replace a VM disk image or UEFI
store while that VM is running.
