#!/usr/bin/env python3
"""Check wallpaper GPU output and layer ordering on a private, bounded KWin host.

Uses only stdlib PNG generation and the existing layer-shell C fixture. Does not
build Rust, inject input, or require Quickshell or a logged-in graphical session.
"""

import argparse
import importlib.util
import json
import os
import struct
import subprocess
import zlib
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "vm_layer_smoke", Path(__file__).with_name("vm-layer-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
LAYER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LAYER)

RED = (224, 32, 48)
GREEN = (32, 208, 64)
BLUE = (48, 64, 224)
CYAN = (32, 192, 208)
BLACK = (0, 0, 0)
PANEL = (224, 64, 48)
WHITE = (240, 240, 240)


def png(path, width, height, pixel):
    def chunk(kind, data):
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data))
        )

    rows = b"".join(
        b"\0" + b"".join(bytes(pixel(x, y)) for x in range(width))
        for y in range(height)
    )
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )


def check_frame(path, mode):
    width, height, pixels = LAYER.read_ppm(path)
    LAYER.require(
        (width, height) == (640, 240), f"unexpected capture: {width}x{height}"
    )

    def pixel(x, y, expected):
        offset = (y * width + x) * 3
        actual = tuple(pixels[offset : offset + 3])
        LAYER.require(
            all(abs(a - b) <= 2 for a, b in zip(actual, expected)),
            f"{mode} pixel ({x},{y}): expected {expected}, got {actual}",
        )

    # Output override selects another image AND stretch mode, independently of
    # the left output. Fixed background/top layers must both cover wallpapers.
    for x, y in [(321, 1), (480, 120), (638, 238)]:
        pixel(x, y, CYAN)
    pixel(160, 16, PANEL)
    pixel(160, 120, WHITE)
    if mode == "fill":
        pixel(20, 60, RED)
        pixel(80, 60, GREEN)
        pixel(300, 60, BLUE)
        pixel(20, 220, BLUE)
    elif mode == "stretch":
        pixel(80, 60, RED)
        pixel(240, 60, BLUE)
        pixel(80, 220, BLUE)
    elif mode == "fit":
        pixel(80, 50, BLACK)
        pixel(80, 70, RED)
        pixel(240, 70, BLUE)
        pixel(80, 165, BLUE)
        pixel(80, 185, BLACK)
    elif mode == "center":
        pixel(20, 90, BLACK)
        pixel(80, 70, BLACK)
        pixel(80, 82, RED)
        pixel(240, 82, BLUE)
        pixel(80, 155, BLUE)
        pixel(80, 175, BLACK)


def run_case(args, fixture, artifacts, env, runtime, host_name, mode):
    directory = artifacts / mode
    directory.mkdir(parents=True, exist_ok=True)
    png(
        directory / "bands.png",
        240,
        80,
        lambda x, y: (RED, GREEN, BLUE)[x // 80 if y < 40 else 2 - x // 80],
    )
    png(directory / "cyan.png", 64, 64, lambda x, y: CYAN)
    config = directory / "config.toml"
    config.write_text(f'''gaps = 0
[theme]
background = [0.0, 0.0, 0.0, 1.0]
border_width = 0
[wallpaper]
path = "bands.png"
mode = "{mode}"
[wallpaper.outputs.right]
path = "cyan.png"
mode = "stretch"
[[outputs]]
name = "left"
width = 320
height = 240
[[outputs]]
name = "right"
width = 320
height = 240
''')
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    clear_name = f"clear-wallpaper-{os.getpid()}-{mode}"
    clear = client = None
    with (
        (directory / "clear.log").open("w") as log,
        (directory / "client.jsonl").open("w") as client_log,
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
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            LAYER.SMOKE.wait_socket(runtime / clear_name, clear)
            client = LAYER.Client(
                fixture, dict(env, WAYLAND_DISPLAY=clear_name), client_log
            )
            client.command("layer panel 2 0 32 32 0 e04030")
            client.expect("panel configured", configured=("panel",))
            client.command("map panel")
            client.expect("panel mapped", sizes={"panel": (320, 32)})
            client.command("layer background 0 48 48 -1 0 f0f0f0")
            client.expect("background configured", configured=("background",))
            client.command("map background")
            client.expect("background mapped", sizes={"background": (48, 48)})
            LAYER.require(
                not frame.exists(),
                "scenario finished after capture; increase --seconds",
            )
            result = clear.wait(timeout=args.seconds + 10)
            LAYER.require(result == 0, f"Clear exited {result}")
            log.flush()
            text = (directory / "clear.log").read_text()
            LAYER.require(
                "clear: stopped" in text and "panicked" not in text, "unclean shutdown"
            )
            LAYER.require(
                "clear: wallpaper" not in text.lower(),
                f"wallpaper resource error: {text}",
            )
            check_frame(frame, mode)
        finally:
            if client is not None:
                client.stop()
            LAYER.SMOKE.stop(clear)
    print(
        f"PASS: {mode}, independent output image, full-output placement, background/top layers",
        flush=True,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-wallpaper-smoke")
    parser.add_argument("--seconds", type=int, default=8)
    parser.add_argument("--layer-xml", type=Path)
    args = parser.parse_args()
    if not 8 <= args.seconds <= 30:
        parser.error("--seconds must be between 8 and 30")
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
    ):
        env.pop(key, None)
    host_name = f"clear-wallpaper-host-{os.getpid()}"
    host = None
    reports = []
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
                    host_name,
                ],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            LAYER.SMOKE.wait_socket(runtime / host_name, host)
            for mode in ("fill", "fit", "stretch", "center"):
                run_case(args, fixture, artifacts, env, runtime, host_name, mode)
                reports.append({"mode": mode, "passed": True})
        finally:
            LAYER.SMOKE.stop(host)
            (artifacts / "results.json").write_text(
                json.dumps(reports, indent=2) + "\n"
            )
    print(f"Artifacts: {artifacts}")


if __name__ == "__main__":
    main()
