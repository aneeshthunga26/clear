# Isolated compositor testing

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

## Optional shell integration

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

For the contract and example, see [shell-integration.md](shell-integration.md).
Test its pure message/model helpers without a GUI using
`node --test examples/quickshell/Protocol.test.mjs`.

## Layer-shell and launcher integration fixture

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
7. Quit with leader+Escape or close the outer window.

Host KDE shortcuts may still take precedence; use bindings that your host leaves
unused. Super+mouse gestures are independent of the configured keyboard leader.

## Environment safety

Use a disposable or backed-up test environment and record its recovery procedure
privately under `.agents/`. Do not change host desktop settings or grant extra VM
privileges just to run these nested tests. Never replace a VM disk image or UEFI
store while that VM is running.
