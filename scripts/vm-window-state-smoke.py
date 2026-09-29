#!/usr/bin/env python3
"""Bounded real-XDG maximize/minimize/restore checks on a private KWin host.

Uses the existing SHM fixture and shell IPC; no physical input injection, sudo,
Quickshell dependency, or compositor test hooks. Build Rust separately.
"""

import argparse
import importlib.util
import json
import os
import subprocess
from pathlib import Path


def load(name, filename):
    spec = importlib.util.spec_from_file_location(
        name, Path(__file__).with_name(filename)
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


LAYER = load("layer_smoke", "vm-layer-smoke.py")
SHELL = load("shell_smoke", "vm-shell-smoke.py")
CASES = ("maximized", "minimized", "restored", "initial-maximized")


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / "config.toml"
    config.write_text(LAYER.CONFIG)
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    name = f"clear-window-state-{os.getpid()}-{case}"
    clear = client = peer = None
    checks = []
    with (
        (directory / "clear.log").open("w") as log,
        (directory / "client.jsonl").open("w") as trace,
    ):
        try:
            clear = subprocess.Popen(
                [
                    str(Path(args.binary).resolve()),
                    "--config",
                    str(config),
                    "--socket",
                    name,
                    "--exit-after",
                    str(args.seconds),
                    "--capture",
                    str(frame),
                ],
                env=dict(env, WAYLAND_DISPLAY=host_name),
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            address = runtime / f"clear-{name}" / "shell.sock"
            LAYER.SMOKE.wait_socket(address, clear)
            peer = SHELL.Peer(address)
            client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
            client.command("layer panel 2 0 32 32 0 e04030")
            client.expect("panel configure", configured=("panel",))
            client.command("map panel")
            client.expect("panel mapped", sizes={"panel": (640, 32)})
            operation = "app-maximized" if case == "initial-maximized" else "app"
            client.command(f"{operation} app 640 480 0 0 30b050")
            checks.append(
                client.expect(
                    "first app",
                    sizes={"app": (640, 448)},
                    fields={"app": {"maximized": case == "initial-maximized"}},
                    focus="app",
                )
            )
            client.command("app other 640 480 0 0 3060e0")
            client.expect(
                "second app",
                sizes={"other": (640 if case == "initial-maximized" else 320, 448)},
                focus="other",
            )
            state = peer.state()
            window = next(w["id"] for w in state["windows"] if w["app_id"] == "app")
            peer.command("focus_window", window=window)
            client.command("maximize app")
            checks.append(
                client.expect(
                    "XDG maximized",
                    sizes={"app": (640, 448), "other": (640, 448)},
                    fields={"app": {"maximized": True}},
                    focus="app",
                )
            )
            state = peer.state()
            assert next(w for w in state["windows"] if w["id"] == window)["maximized"]
            # Reservations reconfigure maximized geometry without changing its saved tile state.
            client.command("resize panel 0 64 64")
            client.expect(
                "maximized respects changed reservation", sizes={"app": (640, 416)}
            )
            client.command("resize panel 0 32 32")
            client.expect("reservation restored", sizes={"app": (640, 448)})
            if case in ("minimized", "restored"):
                client.command("minimize app")
                checks.append(
                    client.expect("minimized transfers keyboard focus", focus="other")
                )

                def minimized_state():
                    assert peer is not None
                    state = peer.state()
                    return (
                        state
                        if any(
                            w["id"] == window and w["minimized"]
                            for w in state["windows"]
                        )
                        else None
                    )

                state = SHELL.wait_for(minimized_state, "minimized shell state")
                assert len(state["windows"]) == 2, (
                    "minimize must not unmap/remove the window"
                )
                assert next(w for w in state["windows"] if w["id"] == window)[
                    "maximized"
                ]
                if case == "restored":
                    peer.command("focus_window", window=window)
                    client.expect(
                        "focus restores maximized window",
                        sizes={"app": (640, 448)},
                        focus="app",
                    )
                    assert not next(
                        w for w in peer.state()["windows"] if w["id"] == window
                    )["minimized"]
                    client.command("unmaximize app")
                    checks.append(
                        client.expect(
                            "XDG unmaximize restores tiles",
                            sizes={"app": (320, 448), "other": (320, 448)},
                            fields={"app": {"maximized": False}},
                            focus="app",
                        )
                    )
                    # Exercise the same allowlisted operations used by shell window-card buttons.
                    peer.command("set_maximized", window=window, maximized=True)
                    client.expect(
                        "IPC maximize",
                        sizes={"app": (640, 448)},
                        fields={"app": {"maximized": True}},
                    )
                    peer.command("set_maximized", window=window, maximized=False)
                    peer.command("set_minimized", window=window, minimized=True)
                    client.expect("IPC minimize", focus="other")
                    peer.command("focus_window", window=window)
                    client.expect("IPC restore", sizes={"app": (320, 448)}, focus="app")
            assert not frame.exists(), (
                "scenario finished after capture; increase --seconds"
            )
            peer.close()
            peer = None
            assert clear.wait(timeout=args.seconds + 5) == 0
            width, height, pixels = LAYER.read_ppm(frame)
            assert (width, height) == (640, 480)

            def pixel(x, y, color):
                offset = (y * width + x) * 3
                actual = tuple(pixels[offset : offset + 3])
                assert actual == color, (case, x, y, actual, color)

            pixel(10, 10, LAYER.COLORS["panel"])
            pixel(
                10,
                40,
                LAYER.COLORS["bottom"] if case == "minimized" else LAYER.COLORS["app"],
            )
            pixel(
                630,
                470,
                LAYER.COLORS["bottom"]
                if case in ("minimized", "restored")
                else LAYER.COLORS["app"],
            )
            log.flush()
            text = (directory / "clear.log").read_text()
            assert "clear: stopped" in text and "panicked" not in text
        finally:
            if peer is not None:
                peer.close()
            if client is not None:
                client.stop()
            LAYER.SMOKE.stop(clear)
    (directory / "result.json").write_text(
        json.dumps({"case": case, "passed": True, "checks": checks}, indent=2) + "\n"
    )
    print(
        f"PASS: {case}: native XDG state, usable geometry, focus, shell model, GPU pixels",
        flush=True,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-window-state-smoke")
    parser.add_argument("--seconds", type=int, default=12)
    parser.add_argument("--layer-xml", type=Path)
    parser.add_argument("--case", action="append", choices=CASES)
    args = parser.parse_args()
    if not 10 <= args.seconds <= 30:
        parser.error("--seconds must be between 10 and 30")
    if not Path(args.binary).is_file():
        parser.error("build the Clear binary separately before running")
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    fixture = LAYER.build_fixture(artifacts / "fixture", args.layer_xml)
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(runtime),
        QT_SCALE_FACTOR="1",
        RUST_BACKTRACE="1",
    )
    for key in (
        "WAYLAND_SOCKET",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "CLEAR_SOCKET",
    ):
        env.pop(key, None)
    name = f"clear-window-state-host-{os.getpid()}"
    host = None
    with (artifacts / "kwin.log").open("w") as log:
        try:
            host = subprocess.Popen(
                [
                    "dbus-run-session",
                    "--",
                    "kwin_wayland",
                    "--virtual",
                    "--width",
                    "1000",
                    "--height",
                    "800",
                    "--no-lockscreen",
                    "--no-global-shortcuts",
                    "--no-kactivities",
                    "--socket",
                    name,
                ],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            LAYER.SMOKE.wait_socket(runtime / name, host)
            for case in args.case or CASES:
                run_case(args, fixture, artifacts, env, runtime, name, case)
        finally:
            LAYER.SMOKE.stop(host)
    print(f"Artifacts: {artifacts}")


if __name__ == "__main__":
    main()
