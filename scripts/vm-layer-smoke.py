#!/usr/bin/env python3
"""Bounded layer-shell/XDG integration tests on an isolated virtual KWin host.

Only compiles the C fixture; never syncs or builds Rust. Run inside the VM against
an explicitly prepared Clear binary. Uses no input injection or desktop session.
"""

import argparse
import importlib.util
import json
import os
import queue
import shlex
import subprocess
import threading
import time
from pathlib import Path

# Reuse the existing harness's process-group cleanup and socket readiness checks.
SPEC = importlib.util.spec_from_file_location(
    "vm_smoke", Path(__file__).with_name("vm-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)

CASES = (
    "panel-configured",
    "panel-mapped",
    "panel-resized",
    "panel-unmapped",
    "panel-destroyed",
    "panel-reconfigured",
    "panel-remapped",
    "pending-layer-uncommitted",
    "pending-layer-committed",
    "layer-popup",
    "priority-bottom",
    "priority-top",
    "priority-overlay",
    "priority-restored",
    "launcher",
    "launcher-resized",
)
COLORS = {
    "app": (48, 176, 80),
    "panel": (224, 64, 48),
    "bottom": (48, 96, 224),
    "overlay": (240, 192, 32),
    "launcher": (32, 192, 208),
    "popup": (208, 64, 192),
}
CONFIG = """gaps = 0
[theme]
border_width = 0
[shell]
launcher_app_ids = ["launcher"]
panels = []
[[outputs]]
name = "test"
width = 640
height = 480
"""


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def build_fixture(directory, layer_xml=None, compile_commands=None, kde_xml=None):
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    protocols = Path(
        subprocess.check_output(
            ["pkg-config", "--variable=pkgdatadir", "wayland-protocols"],
            text=True,
            timeout=10,
        ).strip()
    )
    if layer_xml is None:
        cargo = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
        matches = sorted(
            (cargo / "registry/src").glob(
                "*/wayland-protocols-wlr-*/wlr-protocols/unstable/wlr-layer-shell-unstable-v1.xml"
            )
        )
        if not matches:
            raise RuntimeError("wlr layer XML not found; supply --layer-xml PATH")
        layer_xml = matches[-1]
    if kde_xml is None:
        cargo = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
        matches = sorted(
            (cargo / "registry/src").glob(
                "*/wayland-protocols-misc-*/protocols/server-decoration.xml"
            )
        )
        installed = protocols.parent / "plasma-wayland-protocols/server-decoration.xml"
        if installed.is_file():
            kde_xml = installed
        elif matches:
            kde_xml = matches[-1]
        else:
            raise RuntimeError(
                "KDE server-decoration.xml not found; fetch Cargo dependencies or "
                "pass kde_xml to build_fixture (--kde-xml in vm-decoration-smoke.py)"
            )
    sources = {
        "xdg-shell": protocols / "stable/xdg-shell/xdg-shell.xml",
        "wlr-layer-shell": Path(layer_xml).resolve(),
        "xdg-decoration": protocols
        / "unstable/xdg-decoration/xdg-decoration-unstable-v1.xml",
        "server-decoration": Path(kde_xml).resolve(),
    }
    commands = []
    with (directory / "build.log").open("w") as log:
        for name, xml in sources.items():
            for mode, suffix in (
                ("client-header", "client-protocol.h"),
                ("private-code", "protocol.c"),
            ):
                command = [
                    "wayland-scanner",
                    mode,
                    str(xml),
                    str(directory / f"{name}-{suffix}"),
                ]
                commands.append(command)
                subprocess.run(command, check=True, stdout=log, stderr=log, timeout=15)
        flags = shlex.split(
            subprocess.check_output(
                ["pkg-config", "--cflags", "--libs", "wayland-client"],
                text=True,
                timeout=10,
            )
        )
        binary = directory / "layer-smoke-client"
        command = [
            "cc",
            "-std=c11",
            "-O2",
            "-g",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
            str(directory),
            str(Path(__file__).resolve().with_name("layer-smoke-client.c")),
            *(str(directory / f"{name}-protocol.c") for name in sources),
            "-o",
            str(binary),
            *flags,
        ]
        commands.append(command)
        subprocess.run(command, check=True, stdout=log, stderr=log, timeout=30)
    (directory / "build.json").write_text(json.dumps(commands, indent=2) + "\n")
    # Keep generated bindings and database out of source control. clangd can use
    # this directly, or discover an explicitly requested build/ database.
    source = Path(__file__).resolve().with_name("layer-smoke-client.c")
    database = [
        {
            "directory": str(directory),
            "file": str(source),
            "arguments": [
                *command[: command.index(str(source))],
                "-c",
                str(source),
                *flags,
            ],
        }
    ]
    (directory / "compile_commands.json").write_text(
        json.dumps(database, indent=2) + "\n"
    )
    if compile_commands is not None:
        compile_commands = compile_commands.resolve()
        compile_commands.parent.mkdir(parents=True, exist_ok=True)
        require(
            not compile_commands.exists(), f"refusing to overwrite {compile_commands}"
        )
        compile_commands.write_text(json.dumps(database, indent=2) + "\n")
    return binary


def state_matches(
    state, *, sizes=None, configure_sizes=None, focus=None, configured=(), fields=None
):
    views = state["views"]
    for expected, prefix in ((sizes, ""), (configure_sizes, "configure_")):
        for name, size in (expected or {}).items():
            view = views.get(name, {})
            if (view.get(prefix + "width"), view.get(prefix + "height")) != size:
                return False
    return (
        all(views.get(name, {}).get("configured") for name in configured)
        and (focus is None or state["focus"] == focus)
        and all(
            views.get(name, {}).get(key) == value
            for name, values in (fields or {}).items()
            for key, value in values.items()
        )
    )


class Client:
    def __init__(self, binary, env, log):
        self.events = queue.Queue()
        self.log = log
        self.process = subprocess.Popen(
            [str(binary)],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=log,
            text=True,
            bufsize=1,
            start_new_session=True,
        )
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        try:
            self.until("ready")
        except Exception:
            self.stop()
            raise

    def read(self):
        assert self.process.stdout is not None
        try:
            for line in self.process.stdout:
                self.log.write(line)
                self.log.flush()
                self.events.put(json.loads(line))
        except (OSError, ValueError) as error:
            self.events.put({"event": "reader-error", "message": str(error)})
        finally:
            self.events.put({"event": "eof"})

    def until(self, kind):
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            event = self.events.get(timeout=max(0.01, deadline - time.monotonic()))
            if event["event"] == kind:
                return event
            if event["event"] in ("eof", "reader-error"):
                raise RuntimeError(f"fixture stopped waiting for {kind}: {event}")
        raise TimeoutError(f"fixture did not send {kind}")

    def command(self, command):
        assert self.process.stdin is not None
        self.log.write(json.dumps({"command": command}) + "\n")
        self.log.flush()
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()
        state = self.until("state") if command == "state" else None
        self.until("done")
        return state

    def expect(self, label, **expected):
        # A protocol roundtrip is not a render/reconcile barrier. Wait for the
        # expected state, then require it to survive another event-loop turn.
        deadline = time.monotonic() + 2.5
        stable = 0
        state = None
        while time.monotonic() < deadline:
            time.sleep(0.12)
            state = self.command("state")
            assert state is not None
            okay = state_matches(state, **expected)
            stable = stable + 1 if okay else 0
            if stable == 2:
                return {"check": label, "state": state}
        raise AssertionError(f"{label}: expected={expected}; got {state}")

    def stop(self):
        SMOKE.stop(self.process)
        self.reader.join(timeout=2)
        assert self.process.stdin is not None and self.process.stdout is not None
        self.process.stdin.close()
        self.process.stdout.close()


def drive(client, case, checks, peer: Client | None = None):
    def expect(label, **kwargs):
        checks.append(client.expect(label, **kwargs))

    def layer(name, kind, w, h, zone, keyboard, color):
        client.command(f"layer {name} {kind} {w} {h} {zone} {keyboard} {color}")
        expect(f"{name} configured without buffer", configured=(name,))
        client.command(f"map {name}")

    client.command("app app 640 480 0 0 30b050")
    expect("baseline tile and keyboard focus", sizes={"app": (640, 480)}, focus="app")
    if case.startswith("panel-"):
        client.command("layer panel 2 0 32 32 0 e04030")
        expect(
            "unmapped configured panel reserves nothing",
            configured=("panel",),
            sizes={"app": (640, 480), "panel": (0, 0)},
            configure_sizes={"panel": (640, 32)},
            fields={"panel": {"mapped": False}},
            focus="app",
        )
        if case == "panel-configured":
            return
        client.command("map panel")
        expect("map reserves 32 pixels", sizes={"app": (640, 448)}, focus="app")
        if case == "panel-mapped":
            return
        client.command("resize panel 0 64 64")
        expect(
            "resize reserves 64 pixels",
            sizes={"app": (640, 416), "panel": (640, 64)},
            focus="app",
        )
        if case == "panel-resized":
            return
        before = client.command("state")["views"]["panel"]["configure_count"]
        client.command("destroy panel" if case == "panel-destroyed" else "unmap panel")
        expect(
            "reservation released",
            sizes={"app": (640, 480), "panel": (0, 0)},
            fields={"panel": {"mapped": False, "configured": False}},
            focus="app",
        )
        if case in ("panel-reconfigured", "panel-remapped"):
            # Bottom + left + right: change placement, size and reservation after
            # unmap, using the same role/surface and a fresh bufferless handshake.
            client.command("reconfigure panel 0 48 14 48 0")
            expect(
                "fresh bufferless configure reserves nothing",
                configured=("panel",),
                configure_sizes={"panel": (640, 48)},
                sizes={"app": (640, 480), "panel": (0, 0)},
                fields={"panel": {"mapped": False}},
                focus="app",
            )
            after = client.command("state")["views"]["panel"]["configure_count"]
            require(after > before, "remap reused a stale configure")
            if case == "panel-remapped":
                client.command("map panel")
                expect(
                    "remap reserves 48 pixels at bottom",
                    sizes={"app": (640, 432), "panel": (640, 48)},
                    focus="app",
                )
    elif case.startswith("pending-layer-"):
        assert peer is not None, "pending-layer regression needs a separate client"
        layer("pending", 1, 160, 100, -1, 1, "f0c020")
        layer("top", 2, 240, 140, -1, 1, "e04030")
        expect("top owns keyboard above bottom", focus="top")
        peer.command("layer witness 2 0 8 -1 0 3060e0")
        checks.append(peer.expect("other client configured", configured=("witness",)))
        peer.command("map witness")
        checks.append(peer.expect("other client mapped", sizes={"witness": (640, 8)}))
        before = client.command("state")["views"]["pending"]["commit_count"]
        client.command("set-layer pending 3")
        client.command("sync")
        expect(
            "set_layer alone does not apply",
            focus="top",
            fields={"pending": {"commit_count": before}},
        )
        peer.command("commit witness")
        peer.command("sync")
        expect(
            "another client's commit must not apply pending layer",
            focus="top",
            fields={"pending": {"commit_count": before}},
        )
        if case == "pending-layer-committed":
            client.command("commit pending")
            expect(
                "own commit applies surviving Overlay state",
                focus="pending",
                sizes={"pending": (160, 100)},
            )
            after = client.command("state")["views"]["pending"]["commit_count"]
            require(after > before, "pending layer was never committed")
    elif case == "layer-popup":
        layer("panel", 2, 0, 32, 32, 0, "e04030")
        expect("panel reservation before popup", sizes={"app": (640, 448)})
        client.command("popup menu panel 120 32 160 100 d040c0")
        expect(
            "layer-owned XDG popup outside panel body",
            sizes={"app": (640, 448), "panel": (640, 32), "menu": (160, 100)},
            fields={"menu": {"popup_x": 120, "popup_y": 32}},
            focus="app",
        )
    elif case.startswith("priority-"):
        layer("bottom", 1, 300, 180, -1, 1, "3060e0")
        expect(
            "bottom exclusive cannot preempt app",
            sizes={"app": (640, 480)},
            focus="app",
        )
        if case == "priority-bottom":
            return
        layer("top", 2, 240, 140, -1, 1, "e04030")
        expect("top exclusive takes keyboard", focus="top")
        if case == "priority-top":
            return
        layer("overlay", 3, 160, 100, -1, 1, "f0c020")
        expect("overlay preempts top", focus="overlay")
        # A newer top surface must not win over an older overlay.
        layer("late-top", 2, 240, 140, -1, 1, "e04030")
        expect("overlay wins over newer top", focus="overlay")
        client.command("destroy late-top")
        expect("destroying obscured top preserves overlay focus", focus="overlay")
        if case == "priority-overlay":
            return
        client.command("unmap overlay")
        expect("overlay null commit restores top focus", focus="top")
        client.command("destroy top")
        expect("top destroy restores app focus", focus="app", sizes={"app": (640, 480)})
    else:
        layer("panel", 2, 0, 32, 32, 0, "e04030")
        expect("panel reservation before launcher", sizes={"app": (640, 448)})
        client.command("app launcher 180 100 1 7 20c0d0")
        expect(
            "launcher commits 180x100, excluded from tiling",
            sizes={"app": (640, 448), "launcher": (180, 100)},
            focus="launcher",
        )
        if case == "launcher-resized":
            client.command("resize launcher 260 140 0")
            expect(
                "launcher changes committed geometry independently",
                sizes={"app": (640, 448), "launcher": (260, 140)},
                focus="launcher",
            )


def read_ppm(path):
    magic, dimensions, maximum, pixels = path.read_bytes().split(b"\n", 3)
    width, height = map(int, dimensions.split())
    require(magic == b"P6" and maximum == b"255", "invalid PPM header")
    require(len(pixels) == width * height * 3, "truncated capture")
    return width, height, pixels


def check_frame(path, case):
    width, height, pixels = read_ppm(path)
    require(
        (width, height) == (640, 480),
        f"unexpected framebuffer {width}x{height}; check host scale",
    )

    def pixel(x, y, color):
        start = (y * width + x) * 3
        actual = tuple(pixels[start : start + 3])
        require(
            all(abs(a - b) <= 3 for a, b in zip(actual, COLORS[color])),
            f"{case}: pixel ({x},{y}) expected {color}={COLORS[color]}, got {actual}",
        )

    def rectangle(x, y, w, h, color, *, center=True):
        # Sample every row and column, including boundaries, without depending on
        # antialiasing or font rasterization. All fixture surfaces are opaque.
        for px in range(x, x + w):
            pixel(px, y, color)
            pixel(px, y + h - 1, color)
        for py in range(y, y + h):
            pixel(x, py, color)
            pixel(x + w - 1, py, color)
        if center:
            pixel(x + w // 2, y + h // 2, color)

    if case.startswith("panel-"):
        reserved = {"panel-mapped": 32, "panel-resized": 64}.get(case, 0)
        if reserved:
            rectangle(0, 0, 640, reserved, "panel")
        if case == "panel-remapped":
            rectangle(0, 0, 640, 432, "app")
            rectangle(0, 432, 640, 48, "panel")
        else:
            rectangle(0, reserved, 640, 480 - reserved, "app")
    elif case.startswith("pending-layer-"):
        rectangle(0, 0, 640, 8, "bottom")
        rectangle(0, 8, 640, 140, "app")
        rectangle(200, 170, 240, 140, "panel", center=False)
        if case == "pending-layer-committed":
            rectangle(240, 190, 160, 100, "overlay")
        else:
            rectangle(240, 190, 160, 100, "panel")
        pixel(199, 240, "app")
        pixel(440, 240, "app")
    elif case == "layer-popup":
        rectangle(0, 0, 640, 32, "panel")
        rectangle(120, 32, 160, 100, "popup")
        rectangle(0, 32, 120, 100, "app")
        rectangle(280, 32, 360, 100, "app")
        rectangle(0, 132, 640, 348, "app")
    elif case.startswith("priority-"):
        if case in ("priority-bottom", "priority-restored"):
            rectangle(0, 0, 640, 480, "app")
        # The bottom layer extends beyond top, but must still be hidden by the app.
        pixel(175, 240, "app")
        pixel(320, 155, "app")
        if case in ("priority-top", "priority-overlay"):
            pixel(205, 240, "panel")
            pixel(320, 175, "panel")
            pixel(320, 240, "overlay" if case == "priority-overlay" else "panel")
        else:
            pixel(320, 240, "app")
    else:
        rectangle(0, 0, 640, 32, "panel")
        w, h = (260, 140) if case == "launcher-resized" else (180, 100)
        x, y = (640 - w) // 2, 32 + (448 - h) // 2
        rectangle(x, y, w, h, "launcher")
        # Verify the exact colored geometry bounds, not merely its center pixel.
        matched = []
        color = COLORS["launcher"]
        for py in range(height):
            for px in range(width):
                start = (py * width + px) * 3
                if all(
                    abs(a - b) <= 3 for a, b in zip(pixels[start : start + 3], color)
                ):
                    matched.append((px, py))
        require(
            len(matched) == w * h, f"launcher colored area {len(matched)} != {w * h}"
        )
        require(
            (
                min(p[0] for p in matched),
                min(p[1] for p in matched),
                max(p[0] for p in matched),
                max(p[1] for p in matched),
            )
            == (x, y, x + w - 1, y + h - 1),
            "launcher committed geometry is not centered",
        )
        pixel(x - 8, y + h // 2, "app")
        pixel(x + w + 7, y + h // 2, "app")
    return {"check": "GPU capture pixels", "width": width, "height": height}


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / "config.toml"
    config.write_text(CONFIG)
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    clear_name = f"clear-layer-{os.getpid()}-{case}"
    clear = client = peer = None
    checks = []
    error = None
    with (
        (directory / "clear.log").open("w") as clear_log,
        (directory / "client.jsonl").open("w") as client_log,
        (directory / "peer.jsonl").open("w") as peer_log,
    ):
        try:
            clear = subprocess.Popen(
                [
                    str(Path(args.binary).resolve()),
                    "--config",
                    str(config),
                    "--socket",
                    clear_name,
                    "--exit-after",
                    str(args.seconds),
                    "--capture",
                    str(frame),
                ],
                env=dict(env, WAYLAND_DISPLAY=host_name),
                stdout=clear_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            SMOKE.wait_socket(runtime / clear_name, clear)
            client = Client(fixture, dict(env, WAYLAND_DISPLAY=clear_name), client_log)
            if case.startswith("pending-layer-"):
                peer = Client(fixture, dict(env, WAYLAND_DISPLAY=clear_name), peer_log)
            drive(client, case, checks, peer)
            require(
                not frame.exists(),
                "scenario completed after capture; increase --seconds",
            )
        except (
            AssertionError,
            RuntimeError,
            OSError,
            ValueError,
            queue.Empty,
            subprocess.SubprocessError,
        ) as caught:
            error = f"{type(caught).__name__}: {caught}"
        finally:
            # Even failed assertions keep the scene alive for the scheduled capture.
            if clear is not None:
                try:
                    deadline = time.monotonic() + args.seconds + 5
                    while clear.poll() is None and not frame.exists():
                        if time.monotonic() >= deadline:
                            raise TimeoutError(
                                "Clear did not reach its capture/exit deadline"
                            )
                        for fixture_client in (client, peer):
                            if (
                                fixture_client is not None
                                and fixture_client.process.poll() is not None
                            ):
                                error = error or "fixture exited before capture"
                        time.sleep(0.05)
                    result = clear.wait(timeout=5)
                    require(result == 0, f"Clear exited {result}")
                    clear_log.flush()
                    log = (directory / "clear.log").read_text()
                    require(
                        "clear: stopped" in log and "panicked" not in log,
                        "unclean Clear shutdown",
                    )
                    require(frame.is_file(), "Clear did not produce a capture")
                    if error is None:
                        checks.append(check_frame(frame, case))
                except (
                    AssertionError,
                    RuntimeError,
                    OSError,
                    ValueError,
                    subprocess.SubprocessError,
                ) as caught:
                    error = error or f"{type(caught).__name__}: {caught}"
            if peer is not None:
                peer.stop()
            if client is not None:
                client.stop()
            SMOKE.stop(clear)
    report = {"case": case, "passed": error is None, "checks": checks, "error": error}
    (directory / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        f"{'PASS' if error is None else 'FAIL'}: {case}"
        + (f": {error}" if error else ""),
        flush=True,
    )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-layer-smoke")
    parser.add_argument(
        "--layer-xml",
        type=Path,
        help="Override cargo-registry wlr protocol XML discovery",
    )
    parser.add_argument(
        "--compile-commands",
        type=Path,
        help="Also write a clangd database here (e.g. build/compile_commands.json); never overwrite",
    )
    parser.add_argument(
        "--build-only",
        action="store_true",
        help="Compile C only; do not start any compositor",
    )
    parser.add_argument(
        "--case",
        action="append",
        choices=CASES,
        help="Repeat to select checkpoints; default: all",
    )
    parser.add_argument(
        "--seconds",
        type=int,
        default=10,
        help="Per-Clear-run exit bound (8..30, default 10)",
    )
    args = parser.parse_args()
    if not 8 <= args.seconds <= 30:
        parser.error("--seconds must be between 8 and 30")
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    fixture = build_fixture(
        artifacts / "fixture", args.layer_xml, args.compile_commands
    )
    if args.build_only:
        print(f"Built C fixture: {fixture}")
        return
    if not Path(args.binary).is_file():
        parser.error(f"Clear binary does not exist: {args.binary}; build it separately")
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(runtime),
        RUST_BACKTRACE="1",
        QT_SCALE_FACTOR="1",
    )
    for key in (
        "WAYLAND_SOCKET",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
    ):
        env.pop(key, None)
    host_name = f"clear-layer-host-{os.getpid()}"
    host = None
    reports = []
    with (artifacts / "kwin.log").open("w") as host_log:
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
                    host_name,
                ],
                env=env,
                stdout=host_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            SMOKE.wait_socket(runtime / host_name, host)
            for case in args.case or CASES:
                require(host.poll() is None, "isolated KWin host exited")
                reports.append(
                    run_case(args, fixture, artifacts, env, runtime, host_name, case)
                )
        finally:
            SMOKE.stop(host)
            (artifacts / "results.json").write_text(
                json.dumps(reports, indent=2) + "\n"
            )
    print(f"Artifacts: {artifacts}")
    if any(not report["passed"] for report in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
