#!/usr/bin/env python3
"""Bounded rounded-window GPU tests using real SHM clients on a private KWin host.

Checks scalar/top-bottom/per-corner radii, borders, transparent cut-outs exposing
another window, translucent content, overlapping subsurfaces, XDG geometry offsets,
and unchanged layer/popup shapes. No input
injection, Quickshell dependency, or compositor test hooks. Build Rust separately.
"""

import argparse
import importlib.util
import json
import math
import os
import subprocess
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "layer_smoke", Path(__file__).with_name("vm-layer-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
LAYER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LAYER)

# Config spelling, expanded clockwise radii, border width, border opacity.
CASES = {
    "all": ("24", [24, 24, 24, 24], 4, 1),
    "top-bottom": ("[32, 8]", [32, 32, 8, 8], 4, 1),
    "four": ("[36, 0, 16, 8]", [36, 0, 16, 8], 4, 1),
    "large": ("256", [256, 256, 256, 256], 4, 1),
    "opposite": ("[256, 0, 256, 0]", [256, 0, 256, 0], 0, 1),
    "borderless": ("24", [24, 24, 24, 24], 0, 1),
    "translucent": ("24", [24, 24, 24, 24], 4, 0.5),
    "translucent-body": ("24", [24, 24, 24, 24], 4, 1),
    "subsurface": ("24", [24, 24, 24, 24], 4, 1),
    "translucent-subsurface": ("24", [24, 24, 24, 24], 4, 1),
    "square": ("0", [0, 0, 0, 0], 4, 1),
}


def fitted(radii, width, height):
    ratios = [1.0]
    for extent, total in (
        (width, radii[0] + radii[1]),
        (width, radii[3] + radii[2]),
        (height, radii[0] + radii[3]),
        (height, radii[1] + radii[2]),
    ):
        if total:
            ratios.append(extent / total)
    return [r * min(ratios) for r in radii]


def distance(x, y, box, radii):
    bx, by, width, height = box
    x, y = x - bx, y - by
    result = min(x, width - x, y, height - y)
    for cx, cy, r, corner in (
        (radii[0], radii[0], radii[0], x < radii[0] and y < radii[0]),
        (width - radii[1], radii[1], radii[1], x > width - radii[1] and y < radii[1]),
        (
            width - radii[2],
            height - radii[2],
            radii[2],
            x > width - radii[2] and y > height - radii[2],
        ),
        (radii[3], height - radii[3], radii[3], x < radii[3] and y > height - radii[3]),
    ):
        if corner:
            result = min(result, r - math.hypot(x - cx, y - cy))
    return result


def coverage(distance):
    t = max(0, min(1, distance + 0.5))
    return t * t * (3 - 2 * t)


def check_frame(path, radii, border, opacity, case):
    width, height, pixels = LAYER.read_ppm(path)
    assert (width, height) == (640, 480), (width, height)

    def pixel(x, y):
        offset = (y * width + x) * 3
        return tuple(pixels[offset : offset + 3])

    def expect(x, y, color):
        actual = pixel(x, y)
        assert all(abs(a - b) <= 3 for a, b in zip(actual, color)), (
            x,
            y,
            actual,
            color,
        )

    body = (250, 186, 140, 140) if case == "opposite" else (190, 186, 260, 140)
    bx, by, bw, bh = body
    outer = (bx - border, by - border, bw + 2 * border, bh + 2 * border)
    radii = fitted(radii, outer[2], outer[3])
    inner_radii = fitted([max(r - border, 0) for r in radii], body[2], body[3])
    background = LAYER.COLORS["app"]
    content = LAYER.COLORS["launcher"]
    checked = antialiased = 0
    for y in range(by - border - 6, by + bh + border + 6):
        for x in range(bx - border - 6, bx + bw + border + 6):
            outside = coverage(distance(x + 0.5, y + 0.5, outer, radii))
            inside = coverage(distance(x + 0.5, y + 0.5, body, inner_radii))
            body_alpha = 128 / 255 if case == "translucent-body" else 1
            # SHM fixture premultiplies with integer division before GPU import.
            body_color = tuple(math.floor(c * body_alpha) for c in content)
            if case in ("subsurface", "translucent-subsurface") and by <= y < by + 70:
                child_alpha = 128 / 255 if case == "translucent-subsurface" else 1
                body_color = tuple(
                    math.floor(c * child_alpha) + b * (1 - child_alpha)
                    for c, b in zip((240, 176, 32), body_color)
                )
            ring = max(0, outside - inside)
            remaining = 1 - body_alpha * inside - opacity * ring
            expected = tuple(
                round(c * inside + b * opacity * ring + bg * remaining)
                for c, b, bg in zip(body_color, (255, 0, 0), background)
            )
            expect(x, y, expected)
            checked += 1
            if 0 < inside < 1 or 0 < outside < 1:
                antialiased += 1
    assert checked > 20000
    if any(radii):
        assert antialiased > 10, "rounded edges must be antialiased"
    # Layer-shell panel and its independent popup retain square corners.
    for x, y in [(0, 0), (639, 31)]:
        expect(x, y, LAYER.COLORS["panel"])
    for x, y in [(450, 32), (609, 32), (450, 131), (609, 131)]:
        expect(x, y, LAYER.COLORS["popup"])
    return {"checked_pixels": checked, "antialiased_pixels": antialiased}


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    value, radii, border, opacity = CASES[case]
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / "config.toml"
    config.write_text(
        LAYER.CONFIG.replace(
            "border_width = 0",
            f"border_width = {border}\ncorner_radius = {value}\nactive_border = [1.0, 0.0, 0.0, {float(opacity)}]",
        )
    )
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    name = f"clear-rounded-{os.getpid()}-{case}"
    clear = client = None
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
            LAYER.SMOKE.wait_socket(runtime / name, clear)
            client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
            client.command("app app 640 480 0 0 30b050")
            client.expect(
                "normal window mapped", sizes={"app": (640, 480)}, focus="app"
            )
            client.command("layer panel 2 0 32 32 0 e04030")
            client.expect("panel configured", configured=("panel",))
            client.command("map panel")
            client.expect("panel reserves space", sizes={"app": (640, 448)})
            offset = 7 if any(radii) else 0
            client.command(f"app launcher 180 100 1 {offset} 20c0d0")
            client.expect(
                "launcher mapped", sizes={"launcher": (180, 100)}, focus="launcher"
            )
            body_width = 140 if case == "opposite" else 260
            client.command(f"resize launcher {body_width} 140 0")
            client.expect(
                "client-sized launcher resized", sizes={"launcher": (body_width, 140)}
            )
            if case == "translucent-body":
                client.command("alpha launcher 128")
            if case in ("subsurface", "translucent-subsurface"):
                client.command(
                    f"subsurface child launcher 260 70 {offset} {offset} f0b020"
                )
                if case == "translucent-subsurface":
                    client.command("alpha child 128")
            client.command("popup popup panel 450 32 160 100 d040c0")
            client.expect("independent layer popup", sizes={"popup": (160, 100)})
            assert not frame.exists(), (
                "scenario finished after capture; increase --seconds"
            )
            assert clear.wait(timeout=args.seconds + 5) == 0
            result = check_frame(frame, radii, border, opacity, case)
            log.flush()
            text = (directory / "clear.log").read_text()
            assert "clear: stopped" in text and "panicked" not in text
        finally:
            if client is not None:
                client.stop()
            LAYER.SMOKE.stop(clear)
    (directory / "result.json").write_text(
        json.dumps({"case": case, "passed": True, **result}, indent=2) + "\n"
    )
    print(
        f"PASS: {case}: rounded body/border pixels, underlying window, geometry offsets, panel/popup",
        flush=True,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-rounded-smoke")
    parser.add_argument("--seconds", type=int, default=8)
    parser.add_argument("--layer-xml", type=Path)
    parser.add_argument("--case", action="append", choices=CASES)
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
        "CLEAR_SOCKET",
    ):
        env.pop(key, None)
    name = f"clear-rounded-host-{os.getpid()}"
    host = None
    print(f"Artifacts: {artifacts}", flush=True)
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


if __name__ == "__main__":
    main()
