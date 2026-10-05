#!/usr/bin/env python3
"""Bounded shell IPC smoke; optionally render the real Quickshell example.

Run in an isolated test environment. Uses a private virtual KWin host and never
injects physical input or changes the desktop. Quickshell is an optional argument.
"""

import argparse
import importlib.util
import json
import os
import shutil
import socket
import subprocess
import time
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "vm_smoke", Path(__file__).with_name("vm-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


class Peer:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(4)
        self.socket.connect(str(path))
        self.file = self.socket.makefile("rb")
        self.next_id = 0

    def read(self):
        data = self.file.readline(1024 * 1024 + 1)
        assert data.endswith(b"\n"), "incomplete/oversize response"
        message = json.loads(data)
        assert message["version"] == 1, message
        return message

    def request(self, kind, **fields):
        self.next_id += 1
        data = (
            json.dumps(
                {"version": 1, "id": self.next_id, "request": {"type": kind, **fields}}
            ).encode()
            + b"\n"
        )
        # Exercise real fragmented writes, including the separate framing byte.
        self.socket.sendall(data[:7])
        self.socket.sendall(data[7:-1])
        self.socket.sendall(data[-1:])
        while True:
            result = self.read()
            if result.get("id") == self.next_id:
                return result
            assert result["type"] == "state", result

    def state(self):
        result = self.request("snapshot")
        assert result["type"] == "snapshot", result
        return result["state"]

    def command(self, kind, **fields):
        result = self.request(kind, **fields)
        assert result["type"] == "ok", result

    def close(self):
        self.file.close()
        self.socket.close()


def wait_for(predicate, label, seconds=8):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError("timed out: " + label)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-shell-smoke")
    parser.add_argument(
        "--quickshell", help="Optional Quickshell executable (or wrapper) to test"
    )
    parser.add_argument(
        "--client", choices=("foot", "alacritty"), default="foot",
        help="Real terminal client used for mapping and shell dock checks",
    )
    parser.add_argument(
        "--quickshell-config",
        help="QML entry point for optional isolated Quickshell runs",
    )
    parser.add_argument(
        "--quickshell-style", choices=("standard", "glass"), default="standard",
        help="Example appearance and matching panel geometry/pixel oracle",
    )
    parser.add_argument(
        "--allow-focus-change", action="store_true",
        help="Allow a custom Quickshell fixture to launch and focus another client",
    )
    parser.add_argument(
        "--quickshell-wayland-debug", action="store_true",
        help="Record Quickshell's Wayland client protocol trace",
    )
    parser.add_argument(
        "--exercise-overlays", action="store_true",
        help="Exercise launcher, Alacritty launch, notification panel, and app preview lifecycle",
    )
    args = parser.parse_args()
    if args.exercise_overlays and not args.quickshell:
        parser.error("--exercise-overlays requires --quickshell")
    glass = args.quickshell_style == "glass"
    reserved = 54 if glass else 36
    entry = "liquid-glass.qml" if glass else "shell.qml"
    if args.quickshell_config is None:
        args.quickshell_config = str(Path(__file__).resolve().parents[1] / "examples/quickshell" / entry)
    binary = str(Path(args.binary).resolve())
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    if args.exercise_overlays:
        source = Path(__file__).resolve().parents[1] / "examples/quickshell"
        fixture = artifacts / f"overlay-fixture-{os.getpid()}"
        shutil.copytree(source, fixture)
        qml = fixture / entry
        assembly = fixture / "DesktopShell.qml"
        text = assembly.read_text()
        marker = "            screen: modelData\n"
        assert marker in text
        timers = """
            Timer {
                interval: 900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    launcherLoader.active = true;
                    launcherLoader.item.open();
                    console.info("overlay fixture: launcher opened");
                }
            }
            Timer {
                interval: 1900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    launcherLoader.item.launch(DesktopEntries.byId("Alacritty"));
                    console.info("overlay fixture: app launched");
                }
            }
            Timer {
                interval: 2900
                running: panel.screen && panel.screen.name === "right"
                onTriggered: {
                    launcherLoader.active = true;
                    launcherLoader.item.open();
                    console.info("overlay fixture: second launcher opened");
                }
            }
            Timer {
                interval: 3900
                running: panel.screen && panel.screen.name === "right"
                onTriggered: {
                    launcherLoader.item.dismiss();
                    console.info("overlay fixture: second launcher dismissed");
                }
            }
            Timer {
                interval: 4900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    notificationsLoader.active = true;
                    notificationsLoader.item.toggle();
                    console.info("overlay fixture: notifications opened");
                }
            }
            Timer {
                interval: 5900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    notificationsLoader.item.toggle();
                    console.info("overlay fixture: notifications dismissed");
                }
            }
            Timer {
                interval: 6900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    if (dock.groups.length) dock.showPreview(dock.groups[0].id, dock);
                    console.info("overlay fixture: preview opened");
                }
            }
            Timer {
                interval: 7900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    dock.hoveredApp = "";
                    console.info("overlay fixture: preview dismissed");
                }
            }
            Timer {
                interval: 8900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    if (bridge.state && bridge.state.windows.length) {
                        var state = Object.assign({}, bridge.state);
                        state.switcher = {
                            output: panel.output.id,
                            windows: [bridge.state.windows[0].id],
                            selected: bridge.state.windows[0].id
                        };
                        bridge.state = state;
                    }
                    console.info("overlay fixture: switcher shown");
                }
            }
            Timer {
                interval: 9900
                running: panel.screen && panel.screen.name === "left"
                onTriggered: {
                    if (bridge.state) {
                        var state = Object.assign({}, bridge.state);
                        state.switcher = null;
                        bridge.state = state;
                    }
                    console.info("overlay fixture: switcher dismissed");
                }
            }
"""
        assembly.write_text(text.replace(marker, timers + marker, 1))
        args.quickshell_config = str(qml)
        args.allow_focus_change = True
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(runtime),
        CLEAR_SOCKET="must-not-leak-from-parent",
    )
    env.pop("WAYLAND_SOCKET", None)
    env.pop("DISPLAY", None)
    host_name = f"clear-shell-host-{os.getpid()}"
    clear_name = f"clear-shell-test-{os.getpid()}"
    address = runtime / f"clear-{clear_name}" / "shell.sock"
    disabled_name = clear_name + "-disabled"
    config = artifacts / "config.toml"
    config.write_text(
        '[[outputs]]\nname="left"\nwidth=640\nheight=480\n[[outputs]]\nname="right"\nwidth=640\nheight=480\n'
    )
    capture = artifacts / "frame.ppm"
    capture.unlink(missing_ok=True)
    exported = artifacts / "child-environment.json"
    disabled_exported = artifacts / "disabled-child-environment.json"
    exported.unlink(missing_ok=True)
    disabled_exported.unlink(missing_ok=True)
    probe = 'import json,os,sys; from pathlib import Path; Path(sys.argv[1]).write_text(json.dumps({k:os.environ.get(k) for k in ("WAYLAND_DISPLAY","CLEAR_SOCKET")}))'
    host = clear = foot = shell = disabled = peer = subscriber = None
    with (
        (artifacts / "kwin.log").open("w") as host_log,
        (artifacts / "clear.log").open("w") as clear_log,
        (artifacts / "quickshell.log").open("w") as shell_log,
        (artifacts / "foot.log").open("w") as foot_log,
        (artifacts / "disabled.log").open("w") as disabled_log,
    ):
        try:
            host = subprocess.Popen(
                [
                    "dbus-run-session",
                    "--",
                    "kwin_wayland",
                    "--virtual",
                    "--width",
                    "1400",
                    "--height",
                    "900",
                    "--no-lockscreen",
                    "--no-global-shortcuts",
                    "--no-kactivities",
                    "--socket",
                    host_name,
                ],
                env=env,
                stdout=host_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            SMOKE.wait_socket(runtime / host_name, host)
            clear = subprocess.Popen(
                [
                    binary,
                    "--config",
                    str(config),
                    "--socket",
                    clear_name,
                    "--exit-after",
                    "25",
                    "--capture",
                    str(capture),
                    "--command",
                    "python3",
                    "-c",
                    probe,
                    str(exported),
                ],
                env=dict(env, WAYLAND_DISPLAY=host_name),
                stdout=clear_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            SMOKE.wait_socket(address, clear)
            wait_for(exported.is_file, "spawn environment")
            child = json.loads(exported.read_text())
            assert child == {
                "WAYLAND_DISPLAY": clear_name,
                "CLEAR_SOCKET": str(address),
            }, child
            assert address.stat().st_mode & 0o777 == 0o600
            peer = Peer(address)
            state = peer.state()
            assert [o["name"] for o in state["outputs"]] == ["left", "right"]
            assert len(state["groups"]) == 2
            subscriber = Peer(address)
            assert subscriber.request("subscribe")["type"] == "snapshot"
            client_command = (
                ["foot", "--title", "Shell IPC smoke", "sh", "-c", "sleep 40"]
                if args.client == "foot"
                else ["alacritty", "--title", "Shell IPC smoke", "-e", "sh", "-c", "sleep 40"]
            )
            foot = subprocess.Popen(
                client_command,
                env=dict(env, **child),
                stdout=foot_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            read_state = peer.state
            state = wait_for(
                lambda: s if (s := read_state())["windows"] else None, "window snapshot"
            )
            window = state["windows"][0]["id"]
            assert state["windows"][0]["title"] == "Shell IPC smoke"
            wait_for(
                lambda: subscriber.read().get("state", {}).get("windows"),
                "subscribed window event",
            )
            before = peer.state()
            assert (
                peer.request("switch_workspace", output="2", workspace="99999")["type"]
                == "error"
            )
            assert peer.state() == before, "invalid target mutated focus/state"
            peer.command("set_mode", output="2", mode="columns")
            assert peer.state()["outputs"][1]["effective_mode"] == "columns"
            peer.command("clear_mode", output="2")
            assert peer.state()["outputs"][1]["mode_override"] is None
            peer.command("stretch", output="1")
            assert peer.state()["groups"] == [{"outputs": ["1", "2"], "workspace": "1"}]
            peer.command("switch_workspace", output="2", workspace="3")
            assert peer.state()["groups"][0]["workspace"] == "3"
            peer.command("focus_window", window=window)
            assert peer.state()["focused_window"] == window
            peer.command("split", output="1")
            assert len(peer.state()["groups"]) == 2
            peer.socket.sendall(b"{not-json}\n")
            assert peer.read()["type"] == "error"
            assert peer.state()["windows"], "malformed request killed connection"
            peer.close()
            peer = Peer(address)
            assert peer.state()["focused_window"] == window, (
                "reconnect lost compositor state"
            )
            if args.quickshell:
                qml = Path(args.quickshell_config).resolve()
                shell = subprocess.Popen(
                    ["dbus-run-session", "--", args.quickshell, "-p", str(qml)],
                    env=dict(
                        env,
                        **child,
                        QT_QPA_PLATFORM="wayland",
                        QT_QUICK_BACKEND="software",
                        **({"WAYLAND_DEBUG": "client"} if args.quickshell_wayland_debug else {}),
                    ),
                    stdout=shell_log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
                wait_for(
                    lambda: all(
                        o["area"]["y"] == reserved and o["area"]["height"] == 480 - reserved
                        for o in peer.state()["outputs"]
                    ),
                    "Quickshell reservations on both outputs",
                )
                wait_for(
                    lambda: (
                        "Clear shell: subscribed to protocol v1"
                        in (artifacts / "quickshell.log").read_text()
                    ),
                    "Quickshell consumed the IPC snapshot",
                )
                assert shell.poll() is None, "Quickshell exited"
                SMOKE.stop(shell)
                wait_for(
                    lambda: all(
                        o["area"]["y"] == 0 and o["area"]["height"] == 480
                        for o in peer.state()["outputs"]
                    ),
                    "shell exit releases reservations without stopping Clear",
                )
                if not args.allow_focus_change:
                    assert peer.state()["focused_window"] == window
                shell = subprocess.Popen(
                    ["dbus-run-session", "--", args.quickshell, "-p", str(qml)],
                    env=dict(
                        env,
                        **child,
                        QT_QPA_PLATFORM="wayland",
                        QT_QUICK_BACKEND="software",
                        **({"WAYLAND_DEBUG": "client"} if args.quickshell_wayland_debug else {}),
                    ),
                    stdout=shell_log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
                wait_for(
                    lambda: all(
                        o["area"]["y"] == reserved and o["area"]["height"] == 480 - reserved
                        for o in peer.state()["outputs"]
                    ),
                    "restarted shell restores reservations",
                )
                wait_for(
                    lambda: (
                        (artifacts / "quickshell.log")
                        .read_text()
                        .count("Clear shell: subscribed to protocol v1")
                        == 2
                    ),
                    "restarted shell consumed a fresh snapshot",
                )
            # A second compositor must not inherit the first/parent shell endpoint.
            if args.exercise_overlays:
                wait_for(
                    lambda: "overlay fixture: switcher dismissed"
                    in (artifacts / "quickshell.log").read_text(),
                    "overlay lifecycle completed",
                    seconds=12,
                )
                assert len(peer.state()["windows"]) >= 2, (
                    "launcher did not map Alacritty"
                )
                assert all(
                    o["area"]["y"] == reserved for o in peer.state()["outputs"]
                ), "overlay dismissal disconnected the panel"
            disabled = subprocess.Popen(
                [
                    binary,
                    "--config",
                    str(config),
                    "--socket",
                    disabled_name,
                    "--no-shell-ipc",
                    "--exit-after",
                    "3",
                    "--command",
                    "python3",
                    "-c",
                    probe,
                    str(disabled_exported),
                ],
                env=dict(env, WAYLAND_DISPLAY=host_name),
                stdout=disabled_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            wait_for(disabled_exported.is_file, "disabled spawn environment")
            assert json.loads(disabled_exported.read_text())["CLEAR_SOCKET"] is None
            assert not (runtime / f"clear-{disabled_name}" / "shell.sock").exists()
            assert disabled.wait(timeout=8) == 0
            assert not capture.exists(), "checks ran after capture deadline"
            assert clear.wait(timeout=30) == 0
            assert not address.exists(), "socket was not cleaned up"
            raw = capture.read_bytes()
            magic, dimensions, maximum, pixels = raw.split(b"\n", 3)
            assert (magic, dimensions, maximum) == (b"P6", b"1280 480", b"255")
            assert len(pixels) == 1280 * 480 * 3
            if args.quickshell:
                # QML #e60f172a over Clear's default background; the reserved
                # bar area has no client underneath. Allow GPU byte rounding.
                alpha = 46 / 255 if glass else 230 / 255
                panel_pixel = tuple(
                    round(alpha * foreground + (1 - alpha) * background * 255)
                    for foreground, background in zip((35, 40, 52) if glass else (15, 23, 42), (0.07, 0.08, 0.10))
                )
                for x in ((320, 960) if glass else (2, 642)):
                    offset = ((12 if glass else 1) * 1280 + x) * 3
                    assert all(
                        abs(actual - expected) <= 1
                        for actual, expected in zip(
                            pixels[offset : offset + 3], panel_pixel
                        )
                    ), "missing alpha-composited panel pixels"
                if glass:
                    background = tuple(round(c * 255) for c in (0.07, 0.08, 0.10))
                    for origin in (0, 640):
                        # Outside the capsule corners, but within its rectangular surface.
                        for x, y in ((13, 11), (626, 11), (13, 52), (626, 52)):
                            offset = (y * 1280 + origin + x) * 3
                            assert all(abs(a - b) <= 1 for a, b in zip(pixels[offset:offset + 3], background)), "opaque capsule corner"
                        # Midpoints of both circular ends must still contain the panel.
                        for x in (15, 624):
                            offset = (32 * 1280 + origin + x) * 3
                            assert all(abs(a - b) <= 1 for a, b in zip(pixels[offset:offset + 3], panel_pixel)), "missing capsule end"
                # Controls must remain visible instead of inheriting background alpha.
                for x in ((75, 715) if glass else (330, 970)):
                    offset = ((32 if glass else 10) * 1280 + x) * 3
                    assert any(
                        abs(actual - expected) > 1
                        for actual, expected in zip(
                            pixels[offset : offset + 3], panel_pixel
                        )
                    ), "missing application button"
                log = (artifacts / "quickshell.log").read_text()
                for failure in (
                    "WARN scene:",
                    "ReferenceError",
                    "TypeError",
                    "Failed to load configuration",
                    "Cannot assign",
                    "is not a type",
                    "No Clear output named",
                ):
                    assert failure not in log, log
            log = (artifacts / "clear.log").read_text()
            assert "clear: stopped" in log and "panicked" not in log
            print(
                "PASS: socket discovery, snapshots, subscriptions, commands, validation, reconnect, disable, cleanup"
            )
            if args.quickshell:
                print(
                    "PASS: real Quickshell panels, per-output reservations and captured pixels; no physical click test"
                )
            print(f"Artifacts: {artifacts}")
        finally:
            for connection in (peer, subscriber):
                if connection is not None:
                    connection.close()
            for process in (shell, foot, disabled, clear, host):
                SMOKE.stop(process)


if __name__ == "__main__":
    main()
