#!/usr/bin/env python3
"""Bounded real-XDG fullscreen protocol/GPU checks on a private virtual KWin host.

Builds only the SHM fixture; prepare the Rust binary separately. Six cases cover
reserved/full output geometry, layer priority, IPC/maximize/minimize restore,
pre-map requests, an explicit right-output target, ACKed but uncommitted SSD
entry, and launcher rejection. Artifacts include protocol commands/state and
final pixels. No physical input, delayed ACKs, animations, or native DRM are tested.
"""

import argparse
import importlib.util
import json
import os
import subprocess
from pathlib import Path


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


LAYER = load("fullscreen_layer", "vm-layer-smoke.py")
SHELL = load("fullscreen_shell", "vm-shell-smoke.py")
CASES = (
    "xdg-entry",
    "state-interleavings",
    "initial-fullscreen",
    "output-target",
    "ssd-held",
    "launcher-rejected",
)
CONFIG = """gaps = 0
[theme]
border_width = 4
corner_radius = 16
background = [0.0, 0.0, 0.0, 1.0]
[theme.titlebar]
show_title = false
[shell]
launcher_app_ids = ["launcher"]
panels = []
[[outputs]]
name = "left"
width = 640
height = 480
"""
RIGHT = """[[outputs]]
name = "right"
width = 640
height = 480
"""
APP = LAYER.COLORS["app"]
PANEL = LAYER.COLORS["panel"]
OTHER = LAYER.COLORS["bottom"]
BAR_COLORS = ((35, 40, 52), (27, 30, 38))


def window(peer, app_id):
    return next(w for w in peer.state()["windows"] if w["app_id"] == app_id)


def policy(peer, app_id, **expected):
    def matches():
        state = window(peer, app_id)
        return state if all(state.get(key) == value for key, value in expected.items()) else None

    return SHELL.wait_for(matches, f"{app_id} policy {expected}", seconds=3)


def drive(client, peer, case, checks):
    def expect(label, **expected):
        result = client.expect(label, **expected)
        checks.append(result)
        return result["state"]

    client.command("layer panel 2 0 32 32 0 e04030")
    expect("panel initially configured", configured=("panel",))
    client.command("map panel")
    expect("panel reserves top 32 pixels", sizes={"panel": (640, 32)})

    def app(operation="app"):
        client.command(f"{operation} app 640 480 0 0 30b050")
        expect(
            "ordinary reserved app",
            sizes={"app": (640, 448)},
            fields={"app": {"fullscreen": False, "wm_fullscreen_capability": True}},
            focus="app",
        )
        return window(peer, "app")["id"]

    def full(label="XDG fullscreen"):
        client.command("fullscreen app")
        expect(
            label,
            sizes={"app": (640, 480)},
            fields={"app": {"fullscreen": True, "committed_fullscreen": True}},
            focus="app",
        )
        checks.append({"check": label + " policy", "window": policy(peer, "app", fullscreen=True)})

    if case == "xdg-entry":
        app()
        full()
        client.command("layer overlay 3 100 60 0 0 f0c020")
        expect("overlay configured", configured=("overlay",))
        client.command("map overlay")
        expect(
            "overlay mapped without displacing fullscreen",
            sizes={"overlay": (100, 60), "app": (640, 480)},
        )
        return (640, 480), [(10, 10, APP), (630, 470, APP), (320, 240, LAYER.COLORS["overlay"])]

    if case == "initial-fullscreen":
        client.command("app-fullscreen app 640 480 0 0 30b050")
        expect(
            "pre-map fullscreen opens in full output",
            sizes={"app": (640, 480)},
            fields={"app": {
                "fullscreen": True,
                "committed_fullscreen": True,
                "wm_fullscreen_capability": True,
            }},
            focus="app",
        )
        policy(peer, "app", fullscreen=True)
        client.command("unfullscreen app")
        expect(
            "pre-map fullscreen restores reservation",
            sizes={"app": (640, 448)},
            fields={"app": {"fullscreen": False}},
        )
        full("re-enter after initial fullscreen")
        return (640, 480), [(0, 0, APP), (10, 10, APP), (639, 479, APP)]

    if case == "state-interleavings":
        identity = app()
        client.command("app other 640 480 0 0 3060e0")
        expect("two normal columns", sizes={"app": (320, 448), "other": (320, 448)}, focus="other")
        peer.command("focus_window", window=identity)
        peer.command("set_fullscreen", window=identity, fullscreen=True)
        expect(
            "IPC fullscreen reflows neighbor",
            sizes={"app": (640, 480), "other": (640, 448)},
            fields={"app": {"fullscreen": True}},
            focus="app",
        )
        client.command("maximize app")
        expect(
            "maximize updates underlying fullscreen restore state",
            sizes={"app": (640, 480)},
            fields={"app": {"fullscreen": True, "maximized": True}},
        )
        client.command("minimize app")
        expect("minimize transfers focus", focus="other")
        checks.append(
            {
                "check": "minimize retains both state flags",
                "window": policy(peer, "app", fullscreen=True, maximized=True, minimized=True),
            },
        )
        assert len(peer.state()["windows"]) == 2, "minimize must retain managed window"
        peer.command("focus_window", window=identity)
        expect(
            "focus restores to fullscreen",
            sizes={"app": (640, 480)},
            fields={"app": {"fullscreen": True, "maximized": True}},
            focus="app",
        )
        policy(peer, "app", fullscreen=True, maximized=True, minimized=False)
        client.command("unfullscreen app")
        expect(
            "XDG exit restores underlying maximize",
            sizes={"app": (640, 448)},
            fields={"app": {"fullscreen": False, "maximized": True}},
        )
        peer.command("set_fullscreen", window=identity, fullscreen=True)
        expect("IPC re-entry", sizes={"app": (640, 480)}, fields={"app": {"fullscreen": True}})
        peer.command("set_fullscreen", window=identity, fullscreen=False)
        expect(
            "IPC exit restores maximize",
            sizes={"app": (640, 448)},
            fields={"app": {"fullscreen": False, "maximized": True}},
        )
        policy(peer, "app", fullscreen=False, maximized=True, minimized=False)
        return (640, 480), [(10, 10, PANEL), (10, 40, APP), (630, 470, APP)]

    if case == "output-target":
        app()
        client.command("app other 640 480 0 0 3060e0")
        expect("two left-output columns", sizes={"app": (320, 448), "other": (320, 448)}, focus="other")
        client.command("fullscreen app right")
        expect(
            "explicit right-output fullscreen",
            sizes={"app": (640, 480), "other": (640, 448)},
            fields={"app": {"fullscreen": True}},
            focus="app",
        )
        state = peer.state()
        right = next(output["id"] for output in state["outputs"] if output["name"] == "right")
        app_state = next(w for w in state["windows"] if w["app_id"] == "app")
        other_state = next(w for w in state["windows"] if w["app_id"] == "other")
        assert app_state["output"] == right, state
        assert other_state["output"] != right, state
        assert app_state["workspace"] != other_state["workspace"], state
        checks.append(
            {"check": "output target transfers one window without copying workspace ownership", "state": state},
        )
        return (1280, 480), [
            (10, 10, PANEL), (10, 40, OTHER), (630, 470, OTHER),
            (640, 0, APP), (1279, 479, APP),
        ]

    if case == "ssd-held":
        client.command("app-xdg app 640 480 0 0 30b050 server")
        expect(
            "negotiated SSD in reserved area",
            sizes={"app": (640, 416)},
            fields={"app": {"xdg_mode": 2, "fullscreen": False}},
            focus="app",
        )
        full("committed fullscreen suppresses SSD inset")
        client.command("unfullscreen app")
        expect(
            "exit restores negotiated SSD inset",
            sizes={"app": (640, 416)},
            fields={"app": {"xdg_mode": 2, "fullscreen": False, "committed_fullscreen": False}},
        )
        before = client.command("state")["views"]["app"]["commit_count"]
        client.command("hold app")
        client.command("fullscreen app")
        state = expect(
            "ACKed fullscreen configure does not commit old SSD content",
            sizes={"app": (640, 416)},
            configure_sizes={"app": (640, 480)},
            fields={"app": {
                "fullscreen": True,
                "committed_fullscreen": False,
                "hold_commit": True,
                "xdg_mode": 2,
                "commit_count": before,
            }},
        )
        assert state["views"]["app"]["ack_serial"] > 0
        policy(peer, "app", fullscreen=True)
        # The last committed source still has its negotiated titlebar/content
        # inset. Its old height ends at y448, leaving the final 32 pixels empty.
        return (640, 480), [(200, 16, BAR_COLORS), (200, 40, APP), (200, 470, (0, 0, 0))]

    assert case == "launcher-rejected"
    app()
    full()
    client.command("app-fullscreen launcher 300 160 1 0 20c0d0")
    expect(
        "pre-map launcher rejects fullscreen",
        sizes={"launcher": (300, 160)},
        fields={"launcher": {"fullscreen": False, "committed_fullscreen": False}},
        focus="launcher",
    )
    identity = window(peer, "launcher")["id"]
    result = peer.request("set_fullscreen", window=identity, fullscreen=True)
    assert result["type"] == "error", result
    checks.append({"check": "IPC launcher fullscreen rejected", "response": result})
    client.command("fullscreen launcher")
    expect(
        "mapped launcher rejects XDG fullscreen",
        sizes={"launcher": (300, 160)},
        fields={"launcher": {"fullscreen": False, "committed_fullscreen": False}},
    )
    policy(peer, "launcher", fullscreen=False, role="launcher")
    return (640, 480), [(10, 10, APP), (320, 240, LAYER.COLORS["launcher"]), (630, 470, APP)]


def verify_pixels(frame, size, probes):
    width, height, pixels = LAYER.read_ppm(frame)
    assert (width, height) == size, ((width, height), size)
    results = []
    for x, y, expected in probes:
        actual = tuple(pixels[(y * width + x) * 3 : (y * width + x) * 3 + 3])
        choices = expected if isinstance(expected[0], tuple) else (expected,)
        assert actual in choices, (x, y, actual, choices)
        results.append({"x": x, "y": y, "actual": actual, "expected": choices})
    return results


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / "config.toml"
    config.write_text(CONFIG + (RIGHT if case == "output-target" else ""))
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    name = f"clear-fullscreen-{os.getpid()}-{case}"
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
                    "--config", str(config),
                    "--socket", name,
                    "--exit-after", str(args.seconds),
                    "--capture", str(frame),
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
            size, probes = drive(client, peer, case, checks)
            assert not frame.exists(), "scenario finished after capture; increase --seconds"
            peer.close()
            peer = None
            assert clear.wait(timeout=args.seconds + 5) == 0
            pixels = verify_pixels(frame, size, probes)
            trace.flush()
            if case == "initial-fullscreen":
                # The shared fixture log also receives stderr, including the
                # expected display-disconnected message after bounded shutdown.
                events = [
                    json.loads(line)
                    for line in (directory / "client.jsonl").read_text().splitlines()
                    if line.startswith("{")
                ]
                for kind in ("configure", "commit"):
                    first = next(
                        event for event in events
                        if event.get("event") == kind and event.get("name") == "app"
                    )
                    assert (first["width"], first["height"]) == (640, 480), first
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
        json.dumps({"case": case, "passed": True, "checks": checks, "pixels": pixels}, indent=2)
        + "\n"
    )
    print(f"PASS: {case}: real XDG/IPC assertions and final GPU pixels", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-fullscreen-smoke")
    parser.add_argument("--seconds", type=int, default=16)
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
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), QT_SCALE_FACTOR="1", RUST_BACKTRACE="1")
    for key in ("WAYLAND_SOCKET", "WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "CLEAR_SOCKET"):
        env.pop(key, None)
    name = f"clear-fullscreen-host-{os.getpid()}"
    host = None
    with (artifacts / "kwin.log").open("w") as log:
        try:
            host = subprocess.Popen(
                [
                    "dbus-run-session", "--", "kwin_wayland", "--virtual",
                    "--width", "1400", "--height", "800",
                    "--no-lockscreen", "--no-global-shortcuts", "--no-kactivities",
                    "--socket", name,
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
