# Optional desktop shells

This guide covers running a companion shell. The [shell IPC specification](../specs/shell.md)
is the canonical contract for messages, snapshots, validation, discovery,
permissions, limits, and failures. Graphical integration is specified by
[layer-shell lifecycle](../specs/platform.md#layer-shell-lifecycle); app-ID and
namespace configuration is specified by [shell rules](../specs/configuration.md#shell-rules).

## Run the example

Install Quickshell separately with matching Qt/Qt Wayland libraries. The optional
example uses Quickshell's Socket, PanelWindow, Variants, and WlrLayershell APIs.
See the [example guide](../examples/quickshell/README.md) for panel controls,
launcher, notifications, tray, pins, and UI implementation notes.

From the checkout root, inside an existing graphical session:

```sh
cargo build --locked
./target/debug/clear --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

A terminal launched inside Clear inherits the client connection environment, so
the example can also be started there. If Qt selects the wrong platform plugin,
use:

```sh
./target/debug/clear --config examples/vm.toml --command env QT_QPA_PLATFORM=wayland quickshell -p examples/quickshell/shell.qml
```

Use the endpoint exported by the running instance; do not assume a socket name
or point the shell at an unrelated host display.

If the host already owns notification or tray D-Bus names, start the entire nested
session on one private bus so the shell and its child apps share it:

```sh
dbus-run-session -- ./target/debug/clear --config examples/vm.toml --command quickshell -p examples/quickshell/shell.qml
```

## Implement another client

Use layer-shell for graphical surfaces and the optional Clear IPC for desktop
information and commands. Start with the contract sections below rather than
assuming compatibility with a shell written for another compositor.

| Task | Reference |
| --- | --- |
| Find and connect to the instance | [Boundary and discovery](../specs/shell.md#boundary-and-discovery) |
| Send a command or subscribe | [Wire format and requests](../specs/shell.md#wire-format-and-requests) |
| Handle replies, changes, and reconnects | [Responses and ordering](../specs/shell.md#responses-and-ordering) |
| Interpret workspaces, groups, focus, and windows | [Snapshot fields](../specs/shell.md#snapshot-fields) |
| Bound buffering and request sizes | [Transport bounds](../specs/shell.md#transport-bounds) |
| Present the switcher | [Alt-Tab](../specs/input.md#alt-tab) |
| Check unavailable protocols | [Platform limitations](../specs/platform.md#current-limitations) |

After an unclean exit, inspect a leftover endpoint directory and confirm its
owning instance has stopped before manually removing it. Keep machine-specific
setup and recovery notes in `.agents/`.

## Verification

```sh
cargo test --locked --test shell_ipc
cargo test --locked --lib shell::server
node --test examples/quickshell/Protocol.test.mjs
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear
python3 -B scripts/vm-shell-smoke.py --binary target/debug/clear --quickshell quickshell --artifacts target/vm-quickshell
```

The Node tests require no Quickshell installation and cover only example
message/model helpers. The smoke runner uses a private virtual KWin host, real
foot, a bounded Clear instance, and optionally Quickshell on a private D-Bus
session. See [testing](vm-testing.md#optional-shell-integration) for dependencies,
overlay coverage, artifact handling, and physical-input limitations.
