#!/usr/bin/env python3
"""Bounded real-XDG fullscreen protocol/GPU checks on a private virtual KWin host.

Builds only the SHM fixture; prepare the Rust binary separately. Cases cover
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
import time
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
    "output-target-stretched",
    "ssd-held",
    "launcher-rejected",
    "desktop-slot",
    "premap-supersession",
    "transient-dialog",
    "low-latency-steady",
    "low-latency-mixed",
    "low-latency-recovery",
    "low-latency-overlay",
    "low-latency-held",
    "low-latency-tree",
    "low-latency-initial",
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

    if case.startswith("low-latency-"):
        if case == "low-latency-initial":
            client.command("app-fullscreen app 640 480 0 0 30b050")
            expect("initial fullscreen uses current app rule", sizes={"app": (640, 480)}, fields={"app": {"committed_fullscreen": True}}, focus="app")
            identity = window(peer, "app")["id"]
        elif case == "low-latency-tree":
            client.command("app app 640 480 0 7 30b050")
            expect("offset app", sizes={"app": (640, 448)}, focus="app")
            identity = window(peer, "app")["id"]
        else:
            identity = app()
        def path(expected, reason=None):
            def matches():
                slot = peer.state()["fullscreen_desktop"]
                return slot if slot and slot["mode"] == "low-latency" and slot["presentation"]["path"] == expected and (reason is None or slot["presentation"]["reason"] == reason) else None
            result = SHELL.wait_for(matches, f"{expected} {reason}", seconds=3)
            checks.append({"check": "effective path", "slot": result})
        if case == "low-latency-held":
            client.command("hold app")
            client.command("fullscreen app")
            expect("held content cannot enter minimal presenter", configure_sizes={"app": (640, 480)}, fields={"app": {"committed_fullscreen": False, "hold_commit": True}})
            path("normal-overlay", "pending-commit")
            client.command("commit app")
            expect("committed content enables presenter", sizes={"app": (640, 480)}, fields={"app": {"committed_fullscreen": True}})
        elif case == "low-latency-mixed":
            client.command("app other 640 480 0 0 3060e0")
            expect("second ordinary app", sizes={"app": (320, 448), "other": (320, 448)})
            peer.command("focus_window", window=identity)
            client.command("fullscreen app right")
            expect("minimal right output with normal left", sizes={"app": (640, 480), "other": (640, 448)}, fields={"app": {"committed_fullscreen": True}}, focus="app")
        elif case != "low-latency-initial":
            full()
        path("full-frame-composed", "nested")
        client.command("color-undamaged app 30b050")
        if case == "low-latency-tree":
            client.command("subsurface child app 60 40 80 100 e04030")
            client.command("subsurface-sync syncchild app 70 50 200 210 20c0d0")
            client.command("color syncchild 3060e0")
            client.command("commit app")
            client.command("callbacks app")
            time.sleep(0.4)
            client.command("stop-frames app")
            return (640, 480), [(0, 0, APP), (85 - 7, 105 - 7, PANEL), (205 - 7, 215 - 7, OTHER), (639, 479, APP)]
        if case == "low-latency-steady":
            client.command("animate app")
            time.sleep(0.7)
            client.command("stop-frames app")
            before = client.command("state")["views"]["app"]["frame_count"]
            client.command("callbacks app")
            time.sleep(0.7)
            after = client.command("state")["views"]["app"]["frame_count"]
            assert after - before >= 15, (before, after)
            client.command("stop-frames app")
            client.command("color-undamaged app 30b050")
            # Transparent content must reveal black, with no wallpaper/panel underneath.
            client.command("alpha app 0")
            return (640, 480), [(0, 0, (0, 0, 0)), (320, 240, (0, 0, 0)), (639, 479, (0, 0, 0))]
        if case == "low-latency-recovery":
            peer.command("toggle_overview")
            SHELL.wait_for(lambda: peer.state()["overview_open"], "overview entry")
            path("normal-overlay", "overview")
            peer.command("toggle_overview")
            SHELL.wait_for(lambda: not peer.state()["overview_open"], "overview exit")
            path("full-frame-composed", "nested")
            peer.command("set_fullscreen", window=identity, fullscreen=True, mode="desktop")
            assert peer.state()["fullscreen_desktop"]["mode"] == "desktop"
            client.command("unfullscreen app")
            expect("normal damage history restored", sizes={"app": (640, 448)}, fields={"app": {"committed_fullscreen": False}})
            client.command("color app 3060e0")
            return (640, 480), [(10, 10, PANEL), (10, 40, OTHER), (630, 470, OTHER)]
        if case == "low-latency-overlay":
            client.command("layer overlay 3 100 50 0 0 20c0d0")
            expect("overlay configured", configured=("overlay",))
            client.command("map overlay")
            expect("overlay mapped", sizes={"overlay": (100, 50)})
            path("normal-overlay", "overlay")
            client.command("unmap overlay")
            path("full-frame-composed", "nested")
            client.command("app dialog 200 100 1 0 e04030")
            client.command("parent dialog app")
            expect("related dialog mapped", sizes={"dialog": (200, 100)})
            path("normal-overlay", "dialog")
            client.command("unmap dialog")
            path("full-frame-composed", "nested")
            client.command("popup popup app 20 30 90 60 20c0d0")
            expect("app popup in minimal scene", sizes={"popup": (90, 60)})
            return (640, 480), [(25, 35, (32, 192, 208)), (320, 240, APP)]
        if case == "low-latency-mixed":
            return (1280, 480), [(10, 10, PANEL), (10, 40, OTHER), (630, 470, OTHER), (640, 0, APP), (1279, 479, APP)]
        return (640, 480), [(0, 0, APP), (639, 479, APP)]

    if case == "desktop-slot":
        identity = app()
        full()
        client.command("hold app")
        client.command("app-fullscreen other 640 480 0 0 3060e0")
        expect("new occupant replaces held old fullscreen", fields={"app": {"fullscreen": False, "hold_commit": True}, "other": {"fullscreen": True, "committed_fullscreen": True}}, focus="other")
        state = peer.state()
        other = next(w["id"] for w in state["windows"] if w["app_id"] == "other")
        assert sum(w["fullscreen"] for w in state["windows"]) == 1, state
        assert state["fullscreen_desktop"]["window"] == other and state["fullscreen_desktop"]["active"], state
        home = next(w["workspace"] for w in state["windows"] if w["id"] == other)
        client.command("unfullscreen app")
        assert peer.state()["fullscreen_desktop"]["window"] == other
        peer.command("switch_workspace", output=state["fullscreen_desktop"]["output"], workspace=home)
        expect("normal desktop suspends dormant fullscreen", fields={"other": {"fullscreen": True, "suspended": True}})
        assert not peer.state()["fullscreen_desktop"]["active"]
        peer.command("activate_fullscreen")
        expect("special desktop reactivates", fields={"other": {"fullscreen": True, "suspended": False}}, focus="other")
        assert next(w for w in peer.state()["windows"] if w["id"] == other)["workspace"] == home
        client.command("alpha other 0")
        checks.append({"check": "single occupied desktop and home ownership", "state": peer.state()})
        return (640, 480), [(10, 10, (0, 0, 0)), (320, 240, (0, 0, 0)), (630, 470, (0, 0, 0))]

    if case == "premap-supersession":
        identity = app()
        full()
        for name in ("older", "newer"):
            client.command(f"app-xdg-deferred {name} 640 480 0 0 3060e0 client")
            client.command(f"fullscreen {name}")
        client.command("app-xdg-deferred launcher 300 160 1 0 20c0d0 client")
        client.command("fullscreen launcher")
        assert peer.state()["fullscreen_desktop"]["window"] == identity
        client.command("start older")
        expect("older premap request was revoked", fields={"older": {"fullscreen": False}})
        assert peer.state()["fullscreen_desktop"]["window"] == identity
        client.command("start newer")
        expect("latest premap request enters", fields={"newer": {"fullscreen": True}}, focus="newer")
        client.command("app-xdg-deferred late 640 480 0 0 e0c030 client")
        client.command("fullscreen late")
        peer.command("focus_window", window=identity)
        client.command("fullscreen app")
        expect("mapped request supersedes pending intent", fields={"app": {"fullscreen": True}}, focus="app")
        client.command("start late")
        expect("late first buffer cannot reclaim fullscreen", fields={"late": {"fullscreen": False}}, focus="app")
        assert peer.state()["fullscreen_desktop"]["window"] == identity
        checks.append({"check": "pending intent revisions", "state": peer.state()})
        return (640, 480), [(10, 10, APP), (630, 470, APP)]

    if case == "transient-dialog":
        app()
        full()
        client.command("app-xdg-deferred dialog 250 180 1 0 e0c030 client")
        client.command("parent dialog app")
        client.command("start dialog")
        expect("related dialog owns focus above fullscreen", fields={"dialog": {"fullscreen": False}}, focus="dialog")
        state = peer.state()
        assert len([w for w in state["windows"] if w["fullscreen"]]) == 1, state
        assert next(w for w in state["windows"] if w["app_id"] == "dialog")["fullscreen_transient"], state
        checks.append({"check": "related transient presentation", "state": state})
        peer.command("switch_workspace", output=state["fullscreen_desktop"]["output"], workspace=state["outputs"][0]["workspace"])
        expect("dormant slot suspends the whole related presentation", fields={"app": {"suspended": True}, "dialog": {"suspended": True}})
        peer.command("activate_fullscreen")
        expect("reactivated slot resumes related presentation", fields={"app": {"suspended": False}, "dialog": {"suspended": False}}, focus="app")
        return (640, 480), [(10, 10, APP), (320, 240, (224, 192, 48)), (630, 470, APP)]

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
            "IPC fullscreen hides the ordinary desktop",
            sizes={"app": (640, 480)},
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
                "check": "minimize exits fullscreen and preserves maximize",
                "window": policy(peer, "app", fullscreen=False, maximized=True, minimized=True),
            },
        )
        assert len(peer.state()["windows"]) == 2, "minimize must retain managed window"
        peer.command("focus_window", window=identity)
        expect(
            "focus restores to ordinary maximized presentation",
            sizes={"app": (640, 448)},
            fields={"app": {"fullscreen": False, "maximized": True}},
            focus="app",
        )
        policy(peer, "app", fullscreen=False, maximized=True, minimized=False)
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

    if case in ("output-target", "output-target-stretched"):
        app()
        client.command("app other 640 480 0 0 3060e0")
        expect("two left-output columns", sizes={"app": (320, 448), "other": (320, 448)}, focus="other")
        if case == "output-target-stretched":
            peer.command("stretch", output=peer.state()["outputs"][0]["id"])
        peer.command("focus_window", window=policy(peer, "app")["id"])
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
        assert app_state["output"] != right, state
        assert state["fullscreen_desktop"]["output"] == right, state
        assert other_state["output"] != right, state
        assert app_state["workspace"] == other_state["workspace"], state
        checks.append(
            {"check": "output target changes presentation without transferring home ownership", "state": state},
        )
        client.command("unfullscreen app")
        expect("cross-output exit restores the invoking desktop", sizes={"app": (320, 448), "other": (320, 448)}, fields={"app": {"fullscreen": False}}, focus="app")
        assert peer.state()["focused_output"] == app_state["output"]
        client.command("fullscreen app right")
        expect("cross-output reentry preserves return context", sizes={"app": (640, 480), "other": (640, 448)}, fields={"app": {"fullscreen": True}}, focus="app")
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
        focus="app",
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
    return (640, 480), [(10, 10, APP), (320, 240, APP), (630, 470, APP)]


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
    extra = ""
    if case.startswith("low-latency-"):
        extra = '\n[fullscreen]\ndefault_mode = "desktop"\n[[fullscreen.rules]]\napp_id = "app"\nmode = "low-latency"\n[animations]\nenabled = true\n'
    base_config = CONFIG
    if case == "low-latency-mixed":
        base_config = base_config.replace('background = [0.0, 0.0, 0.0, 1.0]', 'background = [0.0, 0.0, 0.0, 1.0]\nblur_radius = 2\nblur_method = "kawase"\nblur_passes = 3\n[theme.liquid_glass]\nenabled = true')
    config.write_text(base_config + (RIGHT if case in ("output-target", "output-target-stretched", "low-latency-mixed") else "") + ("\n[animations]\nenabled = true\n" if case == "output-target-stretched" else "") + extra)
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
                env=dict(env, WAYLAND_DISPLAY=host_name, CLEAR_RENDER_STATS="1"),
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
            if case.startswith("low-latency-"):
                stats = json.loads(next(line.removeprefix("clear: render stats ") for line in text.splitlines() if line.startswith("clear: render stats ")))
                assert stats["low_latency_submissions"] > 0, stats
                assert stats["low_latency_commits"] > 0, stats
                assert stats["repair_calls"] < stats["submissions"], stats
                if case == "low-latency-steady":
                    assert stats["callback_only"] >= 15, stats
                    assert stats["policy_reconciliations"] < stats["low_latency_commits"], stats
                    assert stats["last_scene_submission_ms"] < 6000, stats
                    assert stats["submissions"] < 180 and stats["loop_turns"] < 800, stats
                checks.append({"check": "presenter instrumentation", "stats": stats})
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
