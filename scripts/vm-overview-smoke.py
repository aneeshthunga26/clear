#!/usr/bin/env python3
"""Bounded overview protocol/GPU checks on an isolated virtual KWin host.

Uses real XDG/layer clients and the optional toggle request. It does not inject
physical keys/clicks or add compositor test hooks. Build Rust separately.
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
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


LAYER = load("overview_layer", "vm-layer-smoke.py")
SHELL = load("overview_shell", "vm-shell-smoke.py")
WALLPAPER = load("overview_wallpaper", "vm-wallpaper-smoke.py")
CASES = ("cards", "minimized", "cancelled", "exclusive-deferred", "alpha-subsurface", "ssd", "odd-outputs", "many-windows", "hidden-live", "rounded-blur", "wallpaper", "scaled-content", "live-cards", "live-minimized", "live-miniatures", "live-offscreen", "live-subsurface", "sharp-preview", "rounded-previews", "rounded-asymmetric")


def check_frame(path, case):
    width, height, pixels = LAYER.read_ppm(path)
    assert (width, height) == ((1400, 900) if case in ("sharp-preview", "rounded-previews", "rounded-asymmetric") else (641, 481) if case == "odd-outputs" else (640, 480))

    def pixel(x, y):
        offset = (y * width + x) * 3
        return tuple(pixels[offset:offset + 3])

    if case in ("rounded-previews", "rounded-asymmetric"):
        # Locate complete uniformly colored SSD frames independently of layout.
        for top, bottom in ((5, 80), (100, height)):
            points = [(x, y) for y in range(top, bottom) for x in range(width) if pixel(x, y) == (48, 176, 80)]
            assert points, "rounded client/SSD preview missing"
            left = min(x for x, _ in points)
            right = max(x for x, _ in points)
            upper = min(y for _, y in points)
            lower = max(y for _, y in points)
            corners = ((left + 4, upper + 4), (right - 4, upper + 4), (right - 4, lower - 4), (left + 4, lower - 4))
            for index, (x, y) in enumerate(corners):
                # Main native-size curves cut this sample. The miniature's
                # proportionally smaller curves leave it inside the window.
                cut = top == 100 and (case == "rounded-previews" or index in (0, 2))
                if cut:
                    assert max(pixel(x, y)) < 60, ("source corner mask missing", case, (x, y), pixel(x, y))
                else:
                    assert pixel(x, y) == (48, 176, 80), ("radius did not scale with the window", case, (x, y), pixel(x, y))
            if case == "rounded-asymmetric":
                # Check original asymmetric corner order at both resolutions,
                # including the offscreen texture's vertical orientation.
                corners = ((left - 2, upper - 2), (right + 2, upper - 2), (right + 2, lower + 2), (left - 2, lower + 2))
                for index, (x, y) in enumerate(corners):
                    if index in (0, 2): assert max(pixel(x, y)) < 60, ("asymmetric corner order inverted", (x, y), pixel(x, y))
                    else: assert max(pixel(x, y)) > 80, ("square corner was rounded", (x, y), pixel(x, y))
            assert pixel((left + right) // 2, (upper + lower) // 2) == (48, 176, 80)
        return
    if case == "sharp-preview":
        def alternating_run(y):
            longest = run = 0
            previous = None
            for x in range(width):
                value = pixel(x, y)
                if value in ((0, 0, 0), (255, 255, 255)):
                    run = run + 1 if previous is not None and value != previous else 1
                else:
                    run = 0
                longest = max(longest, run)
                previous = value if run else None
            return longest
        body_top = next(y for y in range(80, height) if sum(pixel(x, y) == (48, 176, 80) for x in range(width)) > 100)
        assert max(alternating_run(y) for y in range(body_top, height)) >= 100, "client one-pixel stripes were blurred by an intermediate capture"
        assert max(alternating_run(y) for y in range(body_top - 32, body_top)) >= 18, "SSD control one-pixel stripes were blurred"
        return
    if case.startswith("live-"):
        assert max(pixel(2, 2)) < 35, "ordinary panel leaked through"
        body = pixels if case == "live-miniatures" else pixels[84 * width * 3:]
        colors = set(zip(body[0::3], body[1::3], body[2::3]))
        animated = {r for r, g, b in colors if g == 176 and b == 96 and 16 <= r <= 206 and r % 2 == 0}
        if case in ("live-cards", "live-minimized", "live-offscreen"):
            r, g, b = pixel(192, 230)
            assert g == 176 and b == 96 and r in animated, ("main window preview is not live", case, (r, g, b))
            animated = {r}
        assert animated, "no live animated pixels in GPU capture"
        return animated
    if case == "cancelled":
        assert pixel(10, 10) == (224, 64, 48), "panel not restored after close"
        assert pixel(160, 230) == (48, 176, 80)
        assert pixel(480, 230) == (48, 96, 224)
        return
    if case == "wallpaper":
        assert pixel(2, 2) != (224, 64, 48), "ordinary panel leaked through wallpaper backdrop"
        assert pixel(315, 230) == (120, 90, 60), "wallpaper canvas missing in window gutter"
        assert pixel(200, 15) == (120, 90, 60), "empty desktop miniature missing wallpaper"
    else:
        assert max(pixel(2, 2)) < 35, "overview did not cover the ordinary panel"
    if case != "odd-outputs":
        left, right = pixel(192, 230), pixel(448, 230)
        if case == "wallpaper":
            assert all(abs(a - b) <= 1 for a, b in zip(left, (84, 133, 70))), left
        elif case in ("alpha-subsurface", "hidden-live"):
            assert 30 <= left[0] <= 45 and 95 <= left[1] <= 110 and 50 <= left[2] <= 70, left
        else:
            assert left == (48, 176, 80), (case, left)
        assert right == (48, 96, 224), (case, right)
    else:
        assert pixel(160, 230) == (48, 176, 80), "thumbnail missing on odd initiating output"
        assert max(pixel(500, 230)) < 50, "cards escaped onto the dimmed output"
    if case != "odd-outputs":
        assert max(pixel(10, 230)) < (60 if case == "wallpaper" else 35), "client overflow escaped the card"
        if case != "wallpaper":
            assert max(pixel(315, 230)) < 35, "content escaped into the card gutter"
        # Narrow paginated cards put their captions at the old sample point.
        assert max(pixel(166, 475 if case == "many-windows" else 420)) < 35, "content escaped beneath the card"
    if case == "rounded-blur":
        corner = pixel(87, 110)
        assert max(corner) < 60, ("thumbnail outline lost its transparent corner", corner)
        assert pixel(110, 130) == (48, 176, 80), "thumbnail corner clipped foreground content"
    colors = list(zip(pixels[0::3], pixels[1::3], pixels[2::3]))
    assert not any(r > 150 and b > 140 and g < 100 for r, g, b in colors), "popup was included in a thumbnail"
    if case == "alpha-subsurface":
        assert sum(r < 60 and g > 160 and b > 180 for r, g, b in colors) > 20, "subsurface missing"
    if case == "ssd":
        assert sum(r > 180 and g > 180 and b < 80 for r, g, b in colors) > 100, "committed SSD titlebar missing"
    if case == "scaled-content":
        assert pixel(275, 375) == (32, 192, 208), "thumbnail cropped the texture instead of scaling its full source"


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config_text = LAYER.CONFIG
    if case in ("sharp-preview", "rounded-previews", "rounded-asymmetric"):
        config_text = config_text.replace('width = 640\nheight = 480', 'width = 1400\nheight = 900')
    if case == "sharp-preview":
        stripes = ''.join(f'<rect x="{x}" y="0" width="1" height="20" fill="{"white" if x % 2 else "black"}"/>' for x in range(20))
        (directory / "stripes.svg").write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20">{stripes}</svg>')
        config_text += '\n[theme.titlebar.controls]\nminimize = "stripes.svg"\nmaximize = "stripes.svg"\nclose = "stripes.svg"\n'
    if case in ("rounded-previews", "rounded-asymmetric"):
        config_text = config_text.replace("border_width = 0", "border_width = 0\ncorner_radius = 24")
        if case == "rounded-asymmetric": config_text = config_text.replace("corner_radius = 24", "corner_radius = [48, 0, 32, 0]")
        color = [48 / 255, 176 / 255, 80 / 255, 1.0]
        config_text += '\n[theme.titlebar]\nshow_title = false\n' + ''.join(f'{name} = {color}\n' for name in ("active_background", "inactive_background", "active_foreground", "inactive_foreground"))
    if case == "wallpaper":
        WALLPAPER.png(directory / "wallpaper.png", 640, 480, lambda x, y: (120, 90, 60))
        config_text += '\n[wallpaper]\npath = "wallpaper.png"\nmode = "fill"\n'
    if case == "odd-outputs":
        config_text = config_text.replace('width = 640\nheight = 480', 'width = 319\nheight = 481\n[[outputs]]\nname = "right"\nwidth = 322\nheight = 481')
    if case == "rounded-blur":
        config_text = config_text.replace("border_width = 0", "border_width = 0\ncorner_radius = 24\nblur_method = 'kawase'\nblur_radius = 2.0\nblur_passes = 3")
    if case == "ssd":
        config_text += '\n[theme.titlebar]\nactive_background = [0.9, 0.9, 0.1, 1.0]\ninactive_background = [0.9, 0.9, 0.1, 1.0]\n'
    config = directory / "config.toml"
    config.write_text(config_text)
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    name = f"clear-overview-{os.getpid()}-{case}"
    clear = client = peer = None
    extra_clients = []
    extra_logs = []
    with (directory / "clear.log").open("w") as log, (directory / "client.jsonl").open("w") as trace:
        try:
            clear = subprocess.Popen([str(Path(args.binary).resolve()), "--config", str(config), "--socket", name,
                "--exit-after", str(args.seconds), "--capture", str(frame)],
                env=dict(env, WAYLAND_DISPLAY=host_name), stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            address = runtime / f"clear-{name}" / "shell.sock"
            LAYER.SMOKE.wait_socket(address, clear)
            peer = SHELL.Peer(address)
            client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
            if case != "odd-outputs":
                client.command("layer panel 2 0 32 32 0 e04030")
                client.expect("panel configure", configured=("panel",))
                client.command("map panel")
                client.expect("panel mapped", sizes={"panel": (1400 if case in ("sharp-preview", "rounded-previews", "rounded-asymmetric") else 640, 32)})
            operation = "app-xdg" if case in ("ssd", "sharp-preview", "rounded-previews", "rounded-asymmetric") else "app"
            client.command(f"{operation} app 640 480 {1 if case in ('sharp-preview', 'rounded-previews', 'rounded-asymmetric') else 0} 7 30b050" + (" server" if operation == "app-xdg" else ""))
            client.expect("first app mapped", focus="app")
            if case not in ("odd-outputs", "sharp-preview", "rounded-previews", "rounded-asymmetric"):
                client.command("app other 640 480 0 0 3060e0")
                client.expect("second app mapped", focus="other")
            if case == "many-windows":
                for i in range(2, 14):
                    client.command(f"app extra{i} 640 480 0 0 " + ("30b050" if i == 12 else "3060e0"))
                client.expect("first clients mapped", focus="extra13")
                for group in range(2):
                    extra_log = (directory / f"extra{group}.jsonl").open("w")
                    extra_logs.append(extra_log)
                    extra = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), extra_log)
                    extra_clients.append(extra)
                    for i in range(16):
                        color = "30b050" if group == 1 and i == 14 else "3060e0"
                        extra.command(f"app batch{group}-{i} 640 480 0 0 {color}")
                    extra.expect("batch mapped", focus=f"batch{group}-15")
                client.expect("all clients settled")
            subject = "child" if case == "live-subsurface" else "mini0" if case == "live-miniatures" else "app"
            if case == "live-miniatures":
                peer.command("switch_workspace", output="1", workspace="3")
                for i in range(5): client.command(f"app mini{i} 640 480 0 0 30b050")
                client.expect("miniature clients mapped", focus="mini4")
                peer.command("switch_workspace", output="1", workspace="1")
                client.expect("miniature client hidden", focus="other")
            if case == "live-offscreen":
                for i in range(3): client.command(f"app off{i} 640 480 0 0 3060e0")
                peer.command("set_mode", output="1", mode="scrolling")
                client.expect("scrolling windows settled", focus="off2")
                app_id = next(w["id"] for w in peer.state()["windows"] if w["app_id"] == "app")
                off_id = next(w["id"] for w in peer.state()["windows"] if w["app_id"] == "off2")
                peer.command("focus_window", window=app_id)
                client.expect("first scrolling page selected", focus="app")
            if case == "live-subsurface":
                client.command("subsurface child app 80 80 50 80 30b050")
                client.expect("animated child mapped", sizes={"child": (80, 80)})
            if case.startswith("live-"):
                if case == "live-miniatures":
                    for i in range(5): client.command(f"animate mini{i}")
                else: client.command(f"animate {subject}")
            if case in ("minimized", "live-minimized"):

                client.command("minimize app")
                client.expect("minimized window retained", focus="other")
            if case == "alpha-subsurface":
                client.command("subsurface child app 60 60 20 20 20c0d0")
                client.expect("subsurface committed", sizes={"child": (60, 60)})
            if case == "scaled-content":
                size = client.command("state")["views"]["app"]
                client.command(f"subsurface child app 60 60 {size['width'] - 60} {size['height'] - 60} 20c0d0")
                client.expect("bottom corner committed", sizes={"child": (60, 60)})
            if case == "sharp-preview":
                client.command("pattern app")
                client.expect("sharp content committed", focus="app")
            if case not in ("odd-outputs", "many-windows"):
                client.command("popup popup app 40 60 100 60 d040c0")
                client.expect("transient popup mapped", sizes={"popup": (100, 60)})
            if case == "exclusive-deferred":
                client.command("layer exclusive 3 100 60 -1 1 f0c020")
                client.expect("exclusive configured", configured=("exclusive",))
                client.command("map exclusive")
                client.expect("exclusive owns keyboard", focus="exclusive")
            before = peer.state()
            before_views = client.command("state")["views"]
            def memory_kib():
                status = Path(f"/proc/{clear.pid}/status").read_text()
                return {key: int(next(line.split()[1] for line in status.splitlines() if line.startswith(key + ":"))) for key in ("VmRSS", "VmHWM")}
            baseline_memory = memory_kib() if case == "many-windows" else {}
            started = time.monotonic()
            peer.command("toggle_overview")
            if case == "exclusive-deferred":
                assert peer.state()["overview_open"] is False
                client.expect("opening deferred", focus="exclusive")
                client.command("unmap exclusive")
            SHELL.wait_for(lambda: peer.state()["overview_open"], "overview entry")
            entry_seconds = time.monotonic() - started
            client.expect("overview removes protocol keyboard focus", focus="none")
            for extra in extra_clients:
                extra.expect("overview removes other clients' focus", focus="none")
            if case in ("alpha-subsurface", "wallpaper"):
                # Commit new content after entry to exercise live cache invalidation.
                client.command("alpha app 128")
                client.expect("new frame remains overview-owned", focus="none")
            opened = peer.state()
            assert opened["focused_window"] == before["focused_window"]
            assert opened["groups"] == before["groups"]
            if case == "hidden-live":
                # Keep the preview identity while an independent desktop command
                # hides its workspace, then damage a now-thumbnail-only window.
                peer.command("switch_workspace", output="1", workspace="3")
                client.command("alpha app 128")
                client.expect("hidden preview keeps input ownership", focus="none")
                hidden = peer.state()
                assert hidden["overview_open"] and hidden["outputs"][0]["workspace"] == "3"
                opened = hidden
            if case == "cancelled":
                peer.command("toggle_overview")
                SHELL.wait_for(lambda: not peer.state()["overview_open"], "overview exit")
                client.expect("cancel restores keyboard focus", focus="other")
            elif case == "minimized":
                assert any(w["app_id"] == "app" and w["minimized"] for w in opened["windows"])
            # Buffer/configure sizes are distinct from activation-only configures.
            after_views = client.command("state")["views"]
            for window in ("app", "other"):
                if window in before_views:
                    for field in ("width", "height", "configure_width", "configure_height"):
                        assert after_views[window][field] == before_views[window][field], (case, window, field)
            if case == "live-offscreen":
                # Keep the first overview page while ordinary layout scrolls away.
                peer.command("focus_window", window=off_id)
                client.expect("offscreen cards retain overview input", focus="none")
                opened = peer.state()
            metrics = {}
            if case.startswith("live-"):
                first = client.command("state")["views"][subject]
                assert first["outputs"] > 0 and not first["suspended"], first
                time.sleep(1)
                second = client.command("state")["views"][subject]
                assert second["frame_count"] - first["frame_count"] >= 10, (case, first, second)
                if case in ("live-minimized", "live-miniatures", "live-offscreen"):
                    peer.command("toggle_overview")
                    SHELL.wait_for(lambda: not peer.state()["overview_open"], "live overview exit")
                    client.expect("output visibility restored")
                    closed = client.command("state")["views"][subject]
                    assert closed["outputs"] == 0, (case, closed)
                    assert closed["suspended"] == (case == "live-minimized"), (case, closed)
                    time.sleep(0.2)
                    assert client.command("state")["views"][subject]["frame_count"] == closed["frame_count"]
                    if case == "live-offscreen": peer.command("focus_window", window=app_id)
                    peer.command("toggle_overview")
                    SHELL.wait_for(lambda: peer.state()["overview_open"], "live overview reopen")
                    if case == "live-offscreen": peer.command("focus_window", window=off_id)
                    client.expect("live preview resumed", focus="none")
                metrics["animation_first"] = first
                metrics["animation_second"] = second
                if case == "live-miniatures":
                    excluded = client.command("state")["views"]["mini4"]
                    assert excluded["outputs"] == 0 and excluded["frame_count"] == 0, excluded
                    metrics["excluded_fifth_miniature"] = excluded

            if case == "many-windows":
                def cpu_seconds():
                    fields = Path(f"/proc/{clear.pid}/stat").read_text().rsplit(")", 1)[1].split()
                    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
                cpu_start = cpu_seconds()
                sample_start = time.monotonic()
                time.sleep(1)
                metrics["cpu_seconds_per_wall_second"] = (cpu_seconds() - cpu_start) / (time.monotonic() - sample_start)
                metrics["baseline_memory_kib"] = baseline_memory
                metrics["overview_memory_kib"] = memory_kib()
            assert not frame.exists(), "scenario finished after capture; increase --seconds"
            if case.startswith("live-"):
                deadline = time.monotonic() + args.seconds + 3
                latest = second
                while not frame.exists():
                    assert time.monotonic() < deadline and clear.poll() is None, "animation capture timed out"
                    latest = client.command("state")["views"][subject]
                    time.sleep(0.06)
                metrics["animation_at_capture"] = latest
            assert clear.wait(timeout=args.seconds + 5) == 0
            captured = check_frame(frame, case)
            if case.startswith("live-"):
                expected = {16 + (n // 4 % 96) * 2 for n in range(max(0, latest["frame_count"] - 12), latest["frame_count"] + 13)}
                assert captured & expected, ("GPU preview lagged behind live callbacks", case, captured, latest)

            (directory / "result.json").write_text(json.dumps({"case": case, "passed": True,
                "entry_seconds": entry_seconds, "windows": len(opened["windows"]), "metrics": metrics,
                "before": before, "opened": opened}, indent=2) + "\n")
        finally:
            if peer is not None: peer.close()
            if client is not None: client.stop()
            for extra in extra_clients: extra.stop()
            for extra_log in extra_logs: extra_log.close()
            LAYER.SMOKE.stop(clear)
    print(f"PASS: {case}: protocol focus, unchanged configure sizes, GPU overview", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-overview-smoke")
    parser.add_argument("--seconds", type=int, default=12)
    parser.add_argument("--layer-xml", type=Path)
    parser.add_argument("--case", action="append", choices=CASES)
    args = parser.parse_args()
    if not 10 <= args.seconds <= 30: parser.error("--seconds must be between 10 and 30")
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    fixture = LAYER.build_fixture(artifacts / "fixture", args.layer_xml)
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), QT_SCALE_FACTOR="1", RUST_BACKTRACE="1")
    for key in ("WAYLAND_SOCKET", "WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "CLEAR_SOCKET"):
        env.pop(key, None)
    name = f"clear-overview-host-{os.getpid()}"
    host = None
    with (artifacts / "kwin.log").open("w") as log:
        try:
            host = subprocess.Popen(["dbus-run-session", "--", "kwin_wayland", "--virtual", "--width", "1600",
                "--height", "1000", "--no-lockscreen", "--no-global-shortcuts", "--no-kactivities", "--socket", name],
                env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            LAYER.SMOKE.wait_socket(runtime / name, host)
            for case in args.case or CASES:
                run_case(args, fixture, artifacts, env, runtime, name, case)
        finally:
            LAYER.SMOKE.stop(host)
    print(f"Artifacts: {artifacts}")


if __name__ == "__main__":
    main()
