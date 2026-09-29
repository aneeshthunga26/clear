#!/usr/bin/env python3
"""Bounded SSD protocol/GPU smoke on a private virtual KWin host.

Builds only the real SHM fixture; never syncs a VM or builds Rust. No physical
input injection. Requires installed fonts for title assertions. Each case keeps
its final protocol state alive until Clear's bounded --capture, including ACKed
but uncommitted mode switches. Artifacts include commands/events and pixel counts.
The titlebar-* cases exercise configurable SSDs, including literal SVG colors.
Reload is not exercised: the private shell IPC capability allowlist excludes it.
"""

import argparse
import importlib.util
import json
import os
import subprocess
import time
from pathlib import Path


def load(name, filename):
    spec = importlib.util.spec_from_file_location(
        name, Path(__file__).with_name(filename)
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


LAYER = load("decoration_layer", "vm-layer-smoke.py")
ROUNDED = load("decoration_rounded", "vm-rounded-smoke.py")
SHELL = load("decoration_shell", "vm-shell-smoke.py")
TITLEBAR_CASES = (
    "titlebar-height",
    "titlebar-launcher",
    "titlebar-popup",
    "titlebar-translucent",
    "titlebar-translucent-square",
    "titlebar-transparent-square",
    "titlebar-inactive",
    "titlebar-left",
    "titlebar-hidden",
    "titlebar-icon",
    "titlebar-svg",
    "titlebar-svg-small",
    "titlebar-svg-restore",
)
CASES = (
    "xdg-default",
    "xdg-initial-deferred",
    "xdg-server",
    "xdg-client",
    "unnegotiated",
    "to-ssd-held",
    "to-ssd-committed",
    "to-csd-held",
    "to-csd-committed",
    "xdg-unset",
    "xdg-destroy",
    "xdg-destroy-held",
    "xdg-destroy-committed",
    "xdg-recreate",
    "xdg-remap",
    "launcher",
    "title-updated",
    "title-cleared",
    "maximized",
    "popup",
    "kde-default",
    "kde-server",
    "kde-client",
    "kde-none",
    "kde-switch",
) + TITLEBAR_CASES
OVERLAYS = {
    "launcher",
    "title-updated",
    "title-cleared",
    "popup",
    "titlebar-launcher",
    "titlebar-popup",
    "titlebar-translucent",
    "titlebar-translucent-square",
    "titlebar-transparent-square",
}
SQUARE_CASES = {"titlebar-translucent-square", "titlebar-transparent-square"}
SQUARE_BORDER = (255, 0, 0)
BAR_HEIGHT = 32
BAR = (35, 40, 52)
SVG_COLORS = {
    "minimize": (232, 48, 80),
    "maximize": (48, 96, 232),
    "restore": (32, 216, 200),
    "close": (216, 72, 224),
}
UNKNOWN_APP = "clear-ssd-unknown-8f20a7"


def view_name(case):
    if case == "titlebar-icon":
        return UNKNOWN_APP
    return "launcher" if case in OVERLAYS else "app"


def titlebar_theme(case):
    """Explicit test inputs, independent of the compositor's parsed theme."""
    assert case in TITLEBAR_CASES, case
    theme = {
        "height": 48,
        "active_background": [80 / 255, 32 / 255, 96 / 255, 1.0],
        "inactive_background": [20 / 255, 64 / 255, 96 / 255, 1.0],
        "active_foreground": [240 / 255, 176 / 255, 48 / 255, 0.8],
        "inactive_foreground": [64 / 255, 224 / 255, 160 / 255, 0.8],
        "controls_side": "left" if case == "titlebar-left" else "right",
        "show_title": case not in ("titlebar-hidden", "titlebar-icon")
        and not case.startswith("titlebar-svg"),
        "show_icon": case == "titlebar-icon",
    }
    if case in ("titlebar-hidden", "titlebar-icon"):
        theme["height"] = 32
    if case == "titlebar-svg-small":
        theme["height"] = 24
    if case == "titlebar-translucent" or case in SQUARE_CASES:
        theme["active_background"][3] = (
            0.0 if case == "titlebar-transparent-square" else 0.5
        )
    return theme


def config_text(case):
    text = LAYER.CONFIG.replace(
        "border_width = 0", "border_width = 0\ncorner_radius = 24"
    )
    if case in SQUARE_CASES:
        text = text.replace(
            "border_width = 0\ncorner_radius = 24",
            "border_width = 2\ncorner_radius = 0\nblur_radius = 0\n"
            "active_border = [1.0, 0.0, 0.0, 1.0]",
        )
    if case not in TITLEBAR_CASES:
        return text
    text += "\n[theme.titlebar]\n"
    for key, value in titlebar_theme(case).items():
        text += f"{key} = {json.dumps(value)}\n"
    if case.startswith("titlebar-svg"):
        text += "\n[theme.titlebar.controls]\n"
        for control in SVG_COLORS:
            # Deliberately relative to config, not the harness's working directory.
            text += f'{control} = "{control}.svg"\n'
    return text


def write_svg_controls(directory):
    for name, color in SVG_COLORS.items():
        color = "#" + "".join(f"{channel:02x}" for channel in color)
        (directory / f"{name}.svg").write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" '
            f'viewBox="0 0 20 20"><path fill="{color}" d="M0 0H20V20H0Z"/></svg>\n'
        )


def drive_titlebar(client, peer, case, checks):
    def expect(label, **kwargs):
        result = client.expect(label, **kwargs)
        checks.append(result)
        return result["state"]

    theme = titlebar_theme(case)
    height = theme["height"]
    name = view_name(case)
    overlay = case in OVERLAYS
    client.command("layer panel 2 0 32 32 0 e04030")
    expect("panel configured", configured=("panel",))
    client.command("map panel")
    expect("panel reservation", sizes={"panel": (640, 32)})
    if overlay:
        client.command("app background 640 480 0 0 30b050")
        expect("undecorated underlying scene", sizes={"background": (640, 448)})
        if case == "titlebar-translucent":
            client.command("app background-right 640 480 0 0 3060e0")
            expect(
                "two actual, differently colored surfaces under the titlebar",
                sizes={"background": (320, 448), "background-right": (320, 448)},
            )
        if case in SQUARE_CASES:
            # Two real underlying surfaces, without an unrelated tiled-window
            # border at their color boundary. The launcher is the floating SSD.
            client.command(
                "subsurface background-right background 320 448 320 0 3060e0"
            )
            expect(
                "contrasting right-half subsurface under the square SSD",
                sizes={"background": (640, 448), "background-right": (320, 448)},
            )
        # Square windows retain client overflow; don't let the fixture's SHM
        # geometry margin cover the very border ring being tested.
        offset = 0 if case in SQUARE_CASES else 7
        client.command(f"app-xdg {name} 300 160 1 {offset} 20c0d0 default")
        size = (300, 160)
    else:
        client.command(f"app-xdg {name} 640 480 0 0 30b050 default")
        size = (640, 448 - height)
    expect(
        "configured titlebar height preserves total frame and content origin",
        sizes={name: size},
        configure_sizes={name: (0, 0) if overlay else size},
        fields={name: {"xdg_mode": 2, "xdg_decoration": True}},
        focus=name,
    )
    client.command(f"title {name} SSD fixture")
    if case == "titlebar-inactive":
        client.command("app-xdg other 640 480 0 0 3060e0 default")
        expect(
            "both bars visible with only the second active",
            sizes={name: (320, 448 - height), "other": (320, 448 - height)},
            configure_sizes={name: (320, 448 - height), "other": (320, 448 - height)},
            fields={"other": {"xdg_mode": 2}},
            focus="other",
        )
        client.command("title other SSD fixture")
    elif case == "titlebar-svg-restore":
        client.command("maximize app")
        expect(
            "maximize chooses the distinct restore SVG without changing SSD inset",
            sizes={name: size},
            fields={name: {"maximized": True}},
            focus=name,
        )
    elif case == "titlebar-height":
        client.command("resize panel 0 64 64")
        expect("reservation plus custom height", sizes={name: (640, 416 - height)})
        client.command("resize panel 0 32 32")
        expect("reservation restored without changing bar", sizes={name: size})
    elif case == "titlebar-popup":
        client.command("popup popup launcher 40 50 100 60 d040c0")
        expect(
            "popup coordinates stay relative to custom-height content",
            sizes={"popup": (100, 60)},
            fields={"popup": {"popup_x": 40, "popup_y": 50}},
        )
    client.command("sync")
    return {"ssd": True, "overlay": overlay, "case": case, "theme": theme}


def drive(client, peer, case, checks):
    def expect(label, **kwargs):
        result = client.expect(label, **kwargs)
        checks.append(result)
        return result["state"]

    def expect_quiet(label, before):
        # A roundtrip alone cannot catch configures emitted by a later scene
        # reconciliation. Observe several turns and fail on any counter change.
        fields = {
            key: before[key]
            for key in (
                "configure_count",
                "commit_count",
                "ack_serial",
                "xdg_decoration_count",
            )
        }
        fields.update(mapped=False, configured=False)
        samples = []
        for _ in range(5):
            client.command("sync")
            time.sleep(0.12)
            state = client.command("state")
            assert LAYER.state_matches(
                state, sizes={name: (0, 0)}, fields={name: fields}
            ), (label, fields, state)
            samples.append(state)
        checks.append({"check": label, "samples": samples})

    client.command("layer panel 2 0 32 32 0 e04030")
    expect("panel configured", configured=("panel",))
    client.command("map panel")
    expect("panel reservation", sizes={"panel": (640, 32)})
    overlay = case in OVERLAYS
    name = "launcher" if overlay else "app"
    protocol = "kde" if case.startswith("kde-") else "xdg"
    initial_mode = "default"
    if case in (
        "xdg-client",
        "to-ssd-held",
        "to-ssd-committed",
        "xdg-unset",
        "kde-client",
    ):
        initial_mode = "client"
    elif case in ("xdg-server", "kde-server"):
        initial_mode = "server"
    elif case == "kde-none":
        initial_mode = "none"
    ssd = initial_mode not in ("client", "none") and case != "unnegotiated"
    if overlay:
        client.command("app background 640 480 0 0 30b050")
        expect("undecorated backdrop", sizes={"background": (640, 448)})
        # A contrasting seven-pixel SHM margin must not leak into the SSD body.
        client.command(f"app-xdg {name} 300 160 1 7 20c0d0 default")
    elif case == "unnegotiated":
        client.command("app app 640 480 0 0 30b050")
    elif case == "xdg-initial-deferred":
        client.command("app-xdg-deferred app 640 480 0 0 30b050 default")
        expect_quiet(
            "get_decoration defers configure until initial bufferless commit",
            {
                "configure_count": 0,
                "commit_count": 0,
                "ack_serial": 0,
                "xdg_decoration_count": 0,
            },
        )
        client.command("start app")
    else:
        client.command(f"app-{protocol} app 640 480 0 0 30b050 {initial_mode}")
    size = (300, 160) if overlay else (640, 448 - BAR_HEIGHT * ssd)
    mode = 2 if ssd else 0 if initial_mode == "none" else 1
    fields = {f"{protocol}_decoration": True, f"{protocol}_mode": mode}
    if case == "unnegotiated":
        fields = {
            "xdg_decoration": False,
            "kde_decoration": False,
            "xdg_decoration_count": 0,
            "kde_decoration_count": 0,
        }
    state = expect(
        "initial content and negotiation",
        sizes={name: size},
        fields={name: fields},
        focus=name,
    )
    if case != "unnegotiated":
        assert state["views"][name][f"{protocol}_decoration_count"] > 0
    if protocol == "kde":
        assert state["kde_default_count"] > 0 and state["kde_default_mode"] == 2, state
    client.command(f"title {name} SSD fixture")

    if case.startswith("to-"):
        target_ssd = case.startswith("to-ssd-")
        client.command(f"hold {name}")
        before = client.command("state")["views"][name]
        client.command(f"decorate {name} xdg {'server' if target_ssd else 'client'}")
        held = expect(
            "new decoration configure ACKed without committing a buffer",
            sizes={name: size},
            configure_sizes={name: (640, 448 - BAR_HEIGHT * target_ssd)},
            fields={
                name: {
                    "xdg_mode": 2 if target_ssd else 1,
                    "hold_commit": True,
                    "commit_count": before["commit_count"],
                }
            },
        )["views"][name]
        assert held["configure_count"] > before["configure_count"]
        assert held["ack_serial"] != before["ack_serial"]
        assert held["xdg_decoration_count"] > before["xdg_decoration_count"]
        if case.endswith("committed"):
            client.command(f"commit {name}")
            ssd = target_ssd
            size = (640, 448 - BAR_HEIGHT * ssd)
            expect(
                "mode takes effect with content commit",
                sizes={name: size},
                fields={name: {"hold_commit": False}},
            )
    elif case == "xdg-unset":
        client.command("decorate app xdg unset")
        ssd = True
        expect(
            "unset restores server default",
            sizes={name: (640, 416)},
            fields={name: {"xdg_mode": 2}},
        )
    elif case in ("xdg-destroy-held", "xdg-destroy-committed"):
        client.command("hold app")
        before = client.command("state")["views"][name]
        client.command("decorate app xdg destroy")
        held = expect(
            "destroy configures CSD but retains the old root buffer",
            sizes={name: (640, 416)},
            configure_sizes={name: (640, 448)},
            fields={
                name: {
                    "xdg_decoration": False,
                    "hold_commit": True,
                    "commit_count": before["commit_count"],
                }
            },
        )["views"][name]
        assert held["configure_count"] > before["configure_count"]
        assert held["ack_serial"] != before["ack_serial"]
        if case == "xdg-destroy-committed":
            client.command("commit app")
            ssd = False
            after = expect(
                "destroy removes SSD only with a root buffer commit",
                sizes={name: (640, 448)},
                fields={name: {"xdg_decoration": False, "hold_commit": False}},
            )["views"][name]
            assert after["commit_count"] > before["commit_count"]
        # The held case captures the old titlebar; the committed case captures
        # client content in its place. Both use the same total core frame.
    elif case == "xdg-destroy":
        client.command("decorate app xdg destroy")
        ssd = False
        expect(
            "destroy returns to CSD",
            sizes={name: (640, 448)},
            fields={name: {"xdg_decoration": False}},
        )
    elif case in ("xdg-recreate", "xdg-remap"):
        before = client.command("state")["views"][name]
        client.command("unmap app")
        expect_quiet("unmap sends no fresh configure before remap", before)
        if case == "xdg-recreate":
            client.command("decorate app xdg destroy")
            expect_quiet("unmapped decoration destroy sends no configure", before)
            client.command("decorate app xdg default")
            expect_quiet("replacement decoration waits for bufferless remap", before)
        client.command("remap app")
        after = expect(
            "fresh bufferless handshake and decorated remap",
            sizes={name: (640, 416)},
            fields={name: {"xdg_mode": 2}},
            focus=name,
        )["views"][name]
        assert after["configure_count"] > before["configure_count"]
        assert after["commit_count"] > before["commit_count"]
        assert after["ack_serial"] != before["ack_serial"]
        if case == "xdg-recreate":
            assert after["xdg_decoration_count"] > before["xdg_decoration_count"]
        client.command("title app SSD fixture")
    elif case in ("title-updated", "title-cleared"):
        old_title = "I" if case == "title-updated" else "MMMMMMMMMMMM"
        client.command(f"title {name} {old_title}")
        client.command("sync")
        # Let the old title enter the renderer cache before changing metadata.
        # No buffer commit accompanies either title request.
        time.sleep(0.5)
        client.command(
            f"title {name} MMMMMMMMMMMM" if case == "title-updated" else f"title {name}"
        )
    elif case == "maximized":
        client.command("app other 640 480 0 0 3060e0")
        expect(
            "SSD tile keeps total core height",
            sizes={name: (320, 416), "other": (320, 448)},
        )
        window = next(w["id"] for w in peer.state()["windows"] if w["app_id"] == name)
        peer.command("focus_window", window=window)
        client.command("maximize app")
        expect(
            "maximized SSD respects top reservation",
            sizes={name: (640, 416)},
            fields={name: {"maximized": True}},
            focus=name,
        )
        client.command("resize panel 0 64 64")
        expect("reservation resizes content, not titlebar", sizes={name: (640, 384)})
        client.command("resize panel 0 32 32")
        expect("reservation restored", sizes={name: (640, 416)})
        client.command("unmaximize app")
        expect(
            "restore retains tiled frame proportions",
            sizes={name: (320, 416)},
            fields={name: {"maximized": False}},
        )
        client.command("maximize app")
        expect(
            "maximized final capture",
            sizes={name: (640, 416)},
            fields={name: {"maximized": True}},
            focus=name,
        )
    elif case == "popup":
        client.command("popup popup launcher 40 50 100 60 d040c0")
        expect(
            "popup configured in parent content coordinates",
            sizes={"popup": (100, 60)},
            fields={"popup": {"popup_x": 40, "popup_y": 50}},
        )
    elif case == "kde-switch":
        for requested, accepted in (("client", 1), ("none", 0), ("server", 2)):
            before = client.command("state")["views"][name]
            client.command(f"decorate app kde {requested}")
            state = expect(
                f"KDE accepts {requested}",
                sizes={name: (640, 416 if accepted == 2 else 448)},
                fields={name: {"kde_mode": accepted}},
            )
            assert (
                state["views"][name]["kde_decoration_count"]
                > before["kde_decoration_count"]
            )
    client.command("sync")
    return {"ssd": ssd, "overlay": overlay, "case": case}


def check_frame(path, scene):
    width, height, pixels = LAYER.read_ppm(path)
    assert (width, height) == (640, 480), (width, height)
    checked = 0

    def pixel(x, y):
        offset = (y * width + x) * 3
        return tuple(pixels[offset : offset + 3])

    def expect(x, y, color):
        nonlocal checked
        actual = pixel(x, y)
        assert all(abs(a - b) <= 3 for a, b in zip(actual, color)), (
            scene["case"],
            x,
            y,
            actual,
            color,
        )
        checked += 1

    def ink(box):
        x, y, w, h = box
        return sum(
            min(pixel(px, py)) > 100 for py in range(y, y + h) for px in range(x, x + w)
        )

    for x, y in ((0, 0), (639, 31), (320, 16)):
        expect(x, y, LAYER.COLORS["panel"])
    bx, by, bw, bh = (170, 160, 300, 192) if scene["overlay"] else (0, 32, 640, 448)
    color = LAYER.COLORS["launcher" if scene["overlay"] else "app"]
    cy = by + BAR_HEIGHT * scene["ssd"]
    for x in (bx + 40, bx + bw // 2, bx + bw - 40):
        expect(x, cy + 2, color)
        expect(x, by + bh - 10, color)
    text_pixels = 0
    icons = []
    if scene["ssd"]:
        # Flat top and content seam: neither the old accent nor separator remains.
        for y in (by, by + 4, cy - 1):
            expect(bx + bw // 2, y, BAR)
        text_pixels = ink((bx + 12, by + 4, bw - 120, 24))
        assert text_pixels > 8, "SSD title text missing (install a sans-serif font)"
        if scene["case"] == "title-cleared":
            # Clear uses app_id ("launcher") when the title is empty. The long
            # previous title must not survive in the right side of the text area.
            assert ink((bx + 100, by + 4, 50, 24)) == 0, (
                "stale title after app_id fallback"
            )
        if scene["case"] == "title-updated":
            assert ink((bx + 80, by + 4, 50, 24)) > 8, "updated title not rasterized"
        for right in (96, 64, 32):
            count = ink((bx + bw - right + 8, by + 8, 16, 16))
            assert count > 4, ("missing minimize/maximize/close icon", right, count)
            icons.append(count)
    else:
        # The old SSD strip must be actual client content, not an empty titlebar.
        for y in range(by + 2, by + BAR_HEIGHT):
            for x in range(bx + 32, bx + bw - 32):
                expect(x, y, color)

    antialiased = 0
    if scene["overlay"]:
        background = LAYER.COLORS["app"]
        popup = (bx + 40, cy + 50, 100, 60) if scene["case"] == "popup" else None
        for y in range(by - 3, by + bh + 3):
            for x in range(bx - 3, bx + bw + 3):
                coverage = ROUNDED.coverage(
                    ROUNDED.distance(x + 0.5, y + 0.5, (bx, by, bw, bh), [24] * 4)
                )
                if (
                    popup
                    and popup[0] <= x < popup[0] + popup[2]
                    and popup[1] <= y < popup[1] + popup[3]
                ):
                    expect(x, y, LAYER.COLORS["popup"])
                    continue
                # Text and icon rasterization is font-independent above. On the
                # rounded boundary neither glyphs nor controls touch the mask.
                if y < cy and coverage == 1:
                    continue
                foreground = BAR if y < cy else color
                expected = tuple(
                    round(c * coverage + bg * (1 - coverage))
                    for c, bg in zip(foreground, background)
                )
                expect(x, y, expected)
                antialiased += 0 < coverage < 1
        assert antialiased > 10, "combined titlebar/body outline must be antialiased"
        # The seam is not a new rounded top edge for the client buffer.
        expect(bx, cy, color)
        expect(bx + bw - 1, cy, color)
        if popup:
            # Square popup corners. Parent-relative coordinates use the content
            # origin, independent of the seven-pixel XDG buffer-geometry margin.
            px, py, pw, ph = popup
            for x, y in (
                (px, py),
                (px + pw - 1, py),
                (px, py + ph - 1),
                (px + pw - 1, py + ph - 1),
            ):
                expect(x, y, LAYER.COLORS["popup"])
            expect(px, py - 1, color)
            expect(px - 1, py, color)
    return {
        "checked_pixels": checked,
        "antialiased_pixels": antialiased,
        "title_ink_pixels": text_pixels,
        "icon_ink_pixels": icons,
    }


def source_over(rgba, background, coverage=1.0):
    """Straight theme RGBA over an opaque, actual scene sample (byte RGB)."""
    alpha = rgba[3] * coverage
    return tuple(
        round(255 * c * alpha + b * (1 - alpha)) for c, b in zip(rgba, background)
    )


def titlebar_frames(scene):
    height = scene["theme"]["height"]
    if scene["overlay"]:
        # Center the whole frame in the panel-reserved 640x448 area, not just
        # the content, and never add the seven-pixel buffer-geometry margin.
        frame_height = 160 + height
        return [
            (
                (170, 32 + (448 - frame_height) // 2, 300, frame_height),
                True,
                LAYER.COLORS["launcher"],
            )
        ]
    if scene["case"] == "titlebar-inactive":
        return [
            ((0, 32, 320, 448), False, LAYER.COLORS["app"]),
            ((320, 32, 320, 448), True, LAYER.COLORS["bottom"]),
        ]
    return [((0, 32, 640, 448), True, LAYER.COLORS["app"])]


def control_boxes(frame, theme, maximized=False):
    x, y, width, _ = frame
    height = theme["height"]
    size = min(height - 12, 20)
    controls = []
    for index, name in enumerate(
        ("close", "restore" if maximized else "maximize", "minimize")
    ):
        left = (
            x + index * height
            if theme["controls_side"] == "left"
            else x + width - (index + 1) * height
        )
        controls.append(
            (
                name,
                (left, y, height, height),
                (left + (height - size) // 2, y + (height - size) // 2, size, size),
            )
        )
    return controls


def in_box(x, y, box):
    bx, by, width, height = box
    return bx <= x < bx + width and by <= y < by + height


def check_titlebar_frame(path, scene):
    width, height, pixels = LAYER.read_ppm(path)
    assert (width, height) == (640, 480), (width, height)
    theme = scene["theme"]
    case = scene["case"]
    square = case in SQUARE_CASES
    checked = antialiased = border_pixels = 0
    summaries = []

    def pixel(x, y):
        offset = (y * width + x) * 3
        return tuple(pixels[offset : offset + 3])

    def expect(x, y, expected):
        nonlocal checked
        actual = pixel(x, y)
        assert all(abs(a - b) <= 3 for a, b in zip(actual, expected)), (
            case,
            x,
            y,
            actual,
            expected,
        )
        checked += 1

    def underlying(x):
        # These colors belong to real, undecorated tiled clients, not an assumed
        # clear color. The same samples are checked outside the overlay as well.
        if (case == "titlebar-translucent" or square) and x >= 320:
            return LAYER.COLORS["bottom"]
        return LAYER.COLORS["app"]

    def glyph(x, y, background, foreground):
        nonlocal checked
        actual = pixel(x, y)
        delta = [f - b for f, b in zip(foreground, background)]
        strength = sum((a - b) * d for a, b, d in zip(actual, background, delta)) / sum(
            d * d for d in delta
        )
        assert -0.04 <= strength <= 1.04, (case, "foreground alpha", x, y, actual)
        expected = [b + min(1, max(0, strength)) * d for b, d in zip(background, delta)]
        assert all(abs(a - b) <= 3 for a, b in zip(actual, expected)), (
            case,
            "foreground tint",
            x,
            y,
            actual,
            foreground,
            background,
        )
        checked += 1
        return strength > 0.65

    for x, y in ((0, 0), (639, 31), (320, 16)):
        expect(x, y, LAYER.COLORS["panel"])
    for frame, active, content in titlebar_frames(scene):
        bx, by, bw, bh = frame
        cy = by + theme["height"]
        background = theme["active_background" if active else "inactive_background"]
        foreground = theme["active_foreground" if active else "inactive_foreground"]
        controls = control_boxes(frame, theme, case == "titlebar-svg-restore")
        text_left = (
            bx + 12 + (3 * theme["height"] if theme["controls_side"] == "left" else 0)
        )
        text_right = (
            bx + bw - (3 * theme["height"] if theme["controls_side"] == "right" else 0)
        )
        size = min(theme["height"] - 12, 20)
        icon = (
            (text_left, by + (theme["height"] - size) // 2, size, size)
            if theme["show_icon"]
            else None
        )
        if icon:
            text_left += size + 12
        text = (
            text_left,
            by + (theme["height"] - 20) // 2,
            text_right - text_left - 12,
            20,
        )
        popup = (bx + 40, cy + 50, 100, 60) if case == "titlebar-popup" else None
        counts = {"title": 0, "app_icon": 0, **{name: 0 for name, _, _ in controls}}
        for y in range(max(32, by - 3), min(height, by + bh + 3)):
            for x in range(max(0, bx - 3), min(width, bx + bw + 3)):
                if square:
                    # The border is strictly outside the total SSD frame. Check
                    # all four sides/corners and the first pixel outside the ring;
                    # never use its color as the titlebar's underlying scene.
                    if not in_box(x, y, frame):
                        ring = in_box(x, y, (bx - 2, by - 2, bw + 4, bh + 4))
                        expect(x, y, SQUARE_BORDER if ring else underlying(x))
                        border_pixels += ring
                        continue
                    coverage = 1
                else:
                    coverage = ROUNDED.coverage(
                        ROUNDED.distance(x + 0.5, y + 0.5, frame, [24] * 4)
                    )
                # Tiled corners expose the compositor's clear color, which is not
                # part of these tests. Overlay cases check the complete outline.
                if not scene["overlay"] and coverage != 1:
                    continue
                below = underlying(x)
                if popup and in_box(x, y, popup):
                    expect(x, y, LAYER.COLORS["popup"])
                    continue
                if y >= cy:
                    expect(
                        x,
                        y,
                        source_over([*(c / 255 for c in content), 1], below, coverage),
                    )
                    antialiased += 0 < coverage < 1
                    continue
                bar = source_over(background, below)
                bg = source_over([*(c / 255 for c in bar), 1], below, coverage)
                fg = source_over(foreground, bar)
                fg = source_over([*(c / 255 for c in fg), 1], below, coverage)
                control = next(
                    (entry for entry in controls if in_box(x, y, entry[1])), None
                )
                if control and case.startswith("titlebar-svg"):
                    name, _, raster = control
                    if in_box(x, y, raster):
                        expect(
                            x,
                            y,
                            source_over(
                                [*(c / 255 for c in SVG_COLORS[name]), 1],
                                below,
                                coverage,
                            ),
                        )
                        counts[name] += 1
                    else:
                        expect(x, y, bg)
                elif control and coverage == 1:
                    counts[control[0]] += glyph(x, y, bg, fg)
                elif icon and in_box(x, y, icon):
                    ix, iy = x - icon[0], y - icon[1]
                    stroke = ix in (0, size - 1) or iy in (
                        0,
                        size - 1,
                        max(1, size // 3),
                    )
                    expect(x, y, fg if stroke else bg)
                    counts["app_icon"] += stroke
                elif case == "titlebar-left" and x >= bx + bw - 3 * theme["height"]:
                    # The short fixture title ends well before this edge. Do not
                    # mistake stale right-hand controls for valid title glyphs.
                    expect(x, y, bg)
                elif theme["show_title"] and in_box(x, y, text) and coverage == 1:
                    counts["title"] += glyph(x, y, bg, fg)
                else:
                    # Includes the full opposite control edge, hidden title/icon,
                    # SVG raster-box surroundings, flat top row and content seam.
                    expect(x, y, bg)
                antialiased += 0 < coverage < 1
        if theme["show_title"]:
            assert counts["title"] > 8, (
                case,
                "title missing or wrong foreground",
                counts,
            )
        if icon:
            assert counts["app_icon"] > 20, (case, "generic app icon missing", counts)
        for name, _, _ in controls:
            assert counts[name] > 4, (
                case,
                "control missing or wrong foreground",
                counts,
            )
        if scene["overlay"]:
            # Neither content seam edge can acquire a new rounded corner.
            expect(bx, cy, content)
            expect(bx + bw - 1, cy, content)
            for x in (bx - 4, bx + bw + 4):
                expect(x, by + 12, underlying(x))
        summaries.append({"frame": frame, "active": active, "ink_pixels": counts})
    if square:
        assert border_pixels == 2048, (
            "complete two-pixel ring around the 300x208 SSD frame"
        )
        assert antialiased == 0, "square borders must not acquire rounded corners"
    elif scene["overlay"]:
        assert antialiased > 10, "combined custom-height outline must be antialiased"
    return {
        "checked_pixels": checked,
        "antialiased_pixels": antialiased,
        "border_pixels": border_pixels,
        "bars": summaries,
    }


def check_trace(path, case):
    events = [
        json.loads(line)
        for line in path.read_text().splitlines()
        if line.startswith("{")
    ]
    name = view_name(case)
    own = [e for e in events if e.get("name") == name]
    first_commit = next(i for i, e in enumerate(own) if e.get("event") == "commit")
    assert any(e.get("event") == "xdg-ack" for e in own[:first_commit])
    if case != "unnegotiated":
        event = "kde-decoration" if case.startswith("kde-") else "xdg-decoration"
        assert any(e.get("event") == event for e in own[:first_commit]), (
            "decoration must be negotiated before the initial buffer",
            own,
        )
    if case in ("xdg-remap", "xdg-recreate", "xdg-initial-deferred"):
        beginning = (
            "app-xdg-deferred app " if case == "xdg-initial-deferred" else "unmap app"
        )
        start_command = "start app" if case == "xdg-initial-deferred" else "remap app"
        unmap = next(
            i
            for i, e in enumerate(events)
            if e.get("command", "").startswith(beginning)
        )
        start = next(
            i for i, e in enumerate(events) if e.get("command") == start_command
        )
        premature = [
            e
            for e in events[unmap:start]
            if e.get("name") == name
            and e.get("event") in ("configure", "xdg-ack", "xdg-decoration", "commit")
        ]
        assert not premature, ("configure before bufferless handshake", premature)
        fresh = [e for e in events[start:] if e.get("name") == name]
        commit = next(i for i, e in enumerate(fresh) if e.get("event") == "commit")
        acks = [e for e in fresh[:commit] if e.get("event") == "xdg-ack"]
        assert acks and acks[0]["decoration_mode"] == 2, (
            "first fresh configure must be SSD, not eventual SSD",
            fresh,
        )
        configures = [e for e in fresh if e.get("event") == "configure"]
        all_acks = [e for e in fresh if e.get("event") == "xdg-ack"]
        assert configures, fresh
        assert all(e["decoration_mode"] == 2 for e in all_acks) and all(
            e["mode"] == 2 for e in fresh if e.get("event") == "xdg-decoration"
        ), ("transient non-SSD decoration mode", fresh)
        # Initial and remap handshakes may use the 640x480 bootstrap content
        # size before core mapping configures 640x416. Zero lets the client
        # choose; 448 is still a premature full-frame CSD content configure.
        assert all(
            e["width"] in (0, 640) and e["height"] in (0, 416, 480)
            for e in configures + all_acks
        ), ("transient CSD content size", fresh)
    if case in ("xdg-destroy-held", "xdg-destroy-committed"):
        destroyed = next(
            i
            for i, e in enumerate(events)
            if e.get("command") == "decorate app xdg destroy"
        )
        after_destroy = events[destroyed:]
        if case == "xdg-destroy-held":
            held = after_destroy
        else:
            released = next(
                i
                for i, e in enumerate(after_destroy)
                if e.get("command") == "commit app"
            )
            held = after_destroy[:released]
            assert any(
                e.get("name") == name and e.get("event") == "commit"
                for e in after_destroy[released:]
            )
        assert not any(
            e.get("name") == name and e.get("event") == "commit" for e in held
        ), "destroy must not trigger an implicit root commit"
        assert any(
            e.get("name") == name
            and e.get("event") == "xdg-ack"
            and e["held"]
            and (e["width"], e["height"]) == (640, 448)
            for e in held
        ), "destroy configure was not ACKed while held"


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / "config.toml"
    config.write_text(config_text(case))
    if case.startswith("titlebar-svg"):
        write_svg_controls(directory)
    case_env = dict(env, WAYLAND_DISPLAY=host_name)
    if case == "titlebar-icon":
        # No user desktop entry may accidentally resolve our unknown app ID.
        data = directory / "empty-data"
        data.mkdir(exist_ok=True)
        case_env.update(XDG_DATA_HOME=str(data), XDG_DATA_DIRS=str(data))
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    result_path = directory / "result.json"
    result_path.unlink(missing_ok=True)
    name = f"clear-decoration-{os.getpid()}-{case}"
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
                env=case_env,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            address = runtime / f"clear-{name}" / "shell.sock"
            LAYER.SMOKE.wait_socket(address, clear)
            peer = SHELL.Peer(address)
            client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
            driver = drive_titlebar if case in TITLEBAR_CASES else drive
            scene = driver(client, peer, case, checks)
            assert not frame.exists(), (
                "scenario finished after capture; increase --seconds"
            )
            peer.close()
            peer = None
            assert clear.wait(timeout=args.seconds + 5) == 0
            checker = check_titlebar_frame if case in TITLEBAR_CASES else check_frame
            result = checker(frame, scene)
            log.flush()
            text = (directory / "clear.log").read_text()
            assert "clear: stopped" in text and "panicked" not in text
        finally:
            if peer is not None:
                peer.close()
            if client is not None:
                client.stop()
            LAYER.SMOKE.stop(clear)
    check_trace(directory / "client.jsonl", case)
    result_path.write_text(
        json.dumps(
            {"case": case, "passed": True, "scene": scene, "checks": checks, **result},
            indent=2,
        )
        + "\n"
    )
    print(
        f"PASS: {case}: real negotiation/ACKs, content dimensions, final GPU pixels",
        flush=True,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-decoration-smoke")
    parser.add_argument("--seconds", type=int, default=12)
    parser.add_argument("--layer-xml", type=Path)
    parser.add_argument("--kde-xml", type=Path)
    parser.add_argument("--build-only", action="store_true")
    parser.add_argument("--case", action="append", choices=CASES)
    args = parser.parse_args()
    if not 10 <= args.seconds <= 30:
        parser.error("--seconds must be between 10 and 30")
    if not args.build_only and not Path(args.binary).is_file():
        parser.error("build the Clear binary separately before running")
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    fixture = LAYER.build_fixture(
        artifacts / "fixture", args.layer_xml, kde_xml=args.kde_xml
    )
    if args.build_only:
        print(f"Built C fixture: {fixture}")
        return
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
    name = f"clear-decoration-host-{os.getpid()}"
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
