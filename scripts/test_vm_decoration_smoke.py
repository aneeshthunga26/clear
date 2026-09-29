#!/usr/bin/env python3
"""CPU-only SSD harness/oracle regressions, not compositor or GPU validation.

Run: python3 -B scripts/test_vm_decoration_smoke.py
Synthetic glyph blocks deliberately avoid depending on an installed font. They
exercise the oracle's color/count constraints, not actual text/control rendering.
"""

import functools
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

import tomllib

SPEC = importlib.util.spec_from_file_location(
    "decoration_smoke", Path(__file__).with_name("vm-decoration-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


def blend(rgb, alpha, below):
    return tuple(round(a * alpha + b * (1 - alpha)) for a, b in zip(rgb, below))


def set_pixel(image, x, y, color):
    offset = (y * 640 + x) * 3
    image[offset : offset + 3] = bytes(color)


def rectangle(image, box, color):
    x, y, w, h = box
    for py in range(y, y + h):
        offset = (py * 640 + x) * 3
        image[offset : offset + w * 3] = bytes(color) * w


@functools.cache
def synthetic(case):
    """Paint independently placed shapes; never use the oracle's layout helpers."""
    themed = case in SMOKE.TITLEBAR_CASES
    theme = (
        SMOKE.titlebar_theme(case)
        if themed
        else {
            "height": 32,
            "active_background": [35 / 255, 40 / 255, 52 / 255, 1],
            "active_foreground": [239 / 255, 243 / 255, 250 / 255, 1],
            "show_title": True,
            "show_icon": False,
            "controls_side": "right",
        }
    )
    overlay = case in SMOKE.OVERLAYS
    square = case in SMOKE.SQUARE_CASES
    split_background = case == "titlebar-translucent" or square
    height = theme["height"]
    scene = {"case": case, "overlay": overlay, "ssd": True, "theme": theme}
    green, blue, cyan = (48, 176, 80), (48, 96, 224), (32, 192, 208)
    image = bytearray(bytes(green) * (640 * 480))
    if split_background:
        rectangle(image, (320, 32, 320, 448), blue)
    rectangle(image, (0, 0, 640, 32), (224, 64, 48))
    if overlay:
        # Expected positions for the explicitly chosen heights, not derived from
        # the implementation helper whose centering is under test.
        frames = [
            (170, 152 if themed else 160, 300, 208 if themed else 192, True, cyan)
        ]
    elif case == "titlebar-inactive":
        frames = [(0, 32, 320, 448, False, green), (320, 32, 320, 448, True, blue)]
    else:
        frames = [(0, 32, 640, 448, True, green)]
    for bx, by, bw, bh, active, content in frames:
        below = bytes(image)
        if square:
            # Independently paint the four explicit strips around (170,152,300,208).
            for box in (
                (168, 150, 304, 2),
                (168, 360, 304, 2),
                (168, 152, 2, 208),
                (470, 152, 2, 208),
            ):
                rectangle(image, box, (255, 0, 0))
        background = theme["active_background" if active else "inactive_background"]
        foreground = theme["active_foreground" if active else "inactive_foreground"]
        rectangle(image, (bx, by + height, bw, bh - height), content)
        for x in range(bx, bx + bw):
            under = blue if split_background and x >= 320 else green
            bar = blend([c * 255 for c in background[:3]], background[3], under)
            rectangle(image, (x, by, 1, height), bar)
        left = theme["controls_side"] == "left"
        # A short opaque glyph block in the fixture title's legal area.
        title_x = bx + (3 * height if left else 0) + 12
        size = 12 if case == "titlebar-svg-small" else 20
        if theme["show_icon"]:
            icon_y = by + (height - size) // 2
            for y in range(size):
                for x in range(size):
                    if x in (0, size - 1) or y in (0, size - 1, size // 3):
                        offset = ((icon_y + y) * 640 + title_x + x) * 3
                        color = blend(
                            [c * 255 for c in foreground[:3]],
                            foreground[3],
                            image[offset : offset + 3],
                        )
                        set_pixel(image, title_x + x, icon_y + y, color)
            title_x += size + 12
        shapes = []
        if theme["show_title"]:
            shapes.append(((title_x, by + (height - 20) // 2 + 4, 60, 8), None))
        for index, name in enumerate(
            (
                "close",
                "restore" if case == "titlebar-svg-restore" else "maximize",
                "minimize",
            )
        ):
            x = bx + index * height if left else bx + bw - (index + 1) * height
            if case.startswith("titlebar-svg"):
                shapes.append(
                    (
                        (
                            x + (height - size) // 2,
                            by + (height - size) // 2,
                            size,
                            size,
                        ),
                        SMOKE.SVG_COLORS[name],
                    )
                )
            else:
                shapes.append(((x + height // 2 - 4, by + height // 2 - 4, 8, 8), None))
        for (x, y, w, h), literal in shapes:
            for py in range(y, y + h):
                for px in range(x, x + w):
                    offset = (py * 640 + px) * 3
                    color = literal or blend(
                        [c * 255 for c in foreground[:3]],
                        foreground[3],
                        image[offset : offset + 3],
                    )
                    set_pixel(image, px, py, color)
        if overlay and not square:
            # Rounding is shared with the existing independently tested rounded
            # oracle; mutation tests below still require its boundary checks.
            for y in range(by, by + bh):
                for x in range(bx, bx + bw):
                    if bx + 24 <= x < bx + bw - 24 or by + 24 <= y < by + bh - 24:
                        continue
                    coverage = SMOKE.ROUNDED.coverage(
                        SMOKE.ROUNDED.distance(
                            x + 0.5, y + 0.5, (bx, by, bw, bh), [24] * 4
                        )
                    )
                    offset = (y * 640 + x) * 3
                    set_pixel(
                        image,
                        x,
                        y,
                        blend(
                            image[offset : offset + 3],
                            coverage,
                            below[offset : offset + 3],
                        ),
                    )
        if case == "titlebar-popup":
            rectangle(image, (210, 250, 100, 60), (208, 64, 192))
    return bytes(image), scene


class DecorationSmokeTests(unittest.TestCase):
    def check(self, case, mutation=None):
        original, scene = synthetic(case)
        image = bytearray(original)
        if mutation:
            mutation(image)
        with patch.object(SMOKE.LAYER, "read_ppm", return_value=(640, 480, image)):
            checker = (
                SMOKE.check_titlebar_frame
                if case in SMOKE.TITLEBAR_CASES
                else SMOKE.check_frame
            )
            return checker(Path("unused.ppm"), scene)

    def reject_pixel(self, case, x, y, color):
        with self.assertRaises(AssertionError):
            self.check(case, lambda image: set_pixel(image, x, y, color))

    def test_all_focused_oracles_accept_valid_synthetic_frames(self):
        for case in SMOKE.TITLEBAR_CASES:
            with self.subTest(case=case):
                result = self.check(case)
                self.assertGreater(result["checked_pixels"], 50000)

    def test_flat_default_keeps_rounded_boundary_and_seam_checks(self):
        result = self.check("launcher")
        self.assertGreater(result["antialiased_pixels"], 10)
        self.reject_pixel("launcher", 320, 160, (111, 168, 255))
        self.reject_pixel("launcher", 320, 191, (27, 30, 38))
        coverage = SMOKE.ROUNDED.coverage(
            SMOKE.ROUNDED.distance(188.5, 160.5, (170, 160, 300, 192), [24] * 4)
        )
        self.reject_pixel(
            "launcher", 188, 160, blend((111, 168, 255), coverage, (48, 176, 80))
        )
        self.reject_pixel("launcher", 170, 192, (48, 176, 80))

    def test_default_title_and_each_control_remain_required(self):
        for box in (
            (182, 164, 180, 24),
            (374, 168, 24, 16),
            (406, 168, 24, 16),
            (438, 168, 24, 16),
        ):
            with self.subTest(box=box), self.assertRaises(AssertionError):
                self.check(
                    "launcher", lambda image, box=box: rectangle(image, box, SMOKE.BAR)
                )

    def test_height_launcher_and_popup_geometry_reject_old_origins(self):
        self.reject_pixel("titlebar-height", 320, 70, (48, 176, 80))
        self.reject_pixel("titlebar-launcher", 320, 154, (48, 176, 80))
        self.reject_pixel("titlebar-popup", 210, 250, (32, 192, 208))
        self.reject_pixel("titlebar-popup", 210, 249, (208, 64, 192))
        self.reject_pixel("titlebar-popup", 209, 250, (208, 64, 192))

    def test_translucency_uses_both_real_backdrops(self):
        # Opaque bar, alpha applied against black, and blending against the left
        # scene on the right all differ decisively from source-over the scene.
        for x, color in (
            (280, (80, 32, 96)),
            (280, (40, 16, 48)),
            (360, (64, 104, 88)),
        ):
            with self.subTest(x=x, color=color):
                self.reject_pixel("titlebar-translucent", x, 156, color)
        self.reject_pixel("titlebar-translucent", 474, 164, (48, 176, 80))

    def test_square_titlebar_uses_scene_not_solid_border_fill(self):
        for case, alpha, colors in (
            ("titlebar-translucent-square", 0.5, ((64, 104, 88), (64, 64, 160))),
            ("titlebar-transparent-square", 0.0, ((48, 176, 80), (48, 96, 224))),
        ):
            with self.subTest(case=case):
                image, scene = synthetic(case)
                self.assertEqual(scene["theme"]["active_background"][3], alpha)
                for x, expected in zip((280, 360), colors):
                    offset = (156 * 640 + x) * 3
                    self.assertEqual(tuple(image[offset : offset + 3]), expected)
                    # Reproduce the old full-outer-rectangle fill behind SSD,
                    # independently blending the titlebar over opaque red.
                    self.reject_pixel(
                        case, x, 156, blend((80, 32, 96), alpha, (255, 0, 0))
                    )
                # Also reject forced opaque titlebars, even at alpha zero.
                self.reject_pixel(case, 280, 156, (80, 32, 96))

    def test_square_border_ring_is_present_and_exactly_two_pixels(self):
        for case in ("titlebar-translucent-square", "titlebar-transparent-square"):
            with self.subTest(case=case):
                result = self.check(case)
                self.assertEqual(result["border_pixels"], 2048)
                self.assertEqual(result["antialiased_pixels"], 0)
                for x, y in (
                    (168, 150),
                    (169, 151),
                    (320, 150),
                    (168, 250),
                    (471, 250),
                    (320, 361),
                    (471, 361),
                ):
                    under = (48, 176, 80) if x < 320 else (48, 96, 224)
                    self.reject_pixel(case, x, y, under)
                # Too thick outward, or a ring inset into the titlebar/body.
                for x, y in (
                    (167, 250),
                    (472, 250),
                    (280, 149),
                    (280, 362),
                    (170, 152),
                    (469, 152),
                    (170, 359),
                    (469, 359),
                ):
                    self.reject_pixel(case, x, y, (255, 0, 0))

    def test_inactive_background_and_foreground_are_not_active_colors(self):
        self.reject_pixel("titlebar-inactive", 160, 36, (80, 32, 96))
        self.reject_pixel("titlebar-inactive", 296, 56, (148, 154, 58))
        self.reject_pixel("titlebar-height", 616, 56, (239, 243, 250))

    def test_left_controls_and_right_absence(self):
        self.reject_pixel("titlebar-left", 616, 56, (208, 147, 58))
        with self.assertRaises(AssertionError):
            self.check(
                "titlebar-left",
                lambda image: rectangle(image, (0, 44, 144, 24), (80, 32, 96)),
            )

    def test_hidden_title_and_icon_stay_empty(self):
        self.reject_pixel("titlebar-hidden", 20, 46, (208, 147, 58))
        self.reject_pixel("titlebar-icon", 48, 46, (208, 147, 58))
        self.reject_pixel("titlebar-icon", 12, 38, (80, 32, 96))
        self.reject_pixel("titlebar-icon", 20, 49, (208, 147, 58))

    def test_svg_colors_restore_selection_and_raster_boxes(self):
        self.reject_pixel("titlebar-svg", 616, 56, (208, 147, 58))
        self.reject_pixel("titlebar-svg-restore", 568, 56, SMOKE.SVG_COLORS["maximize"])
        self.reject_pixel("titlebar-svg-small", 628, 44, (80, 32, 96))
        # The 24-high bar gets a 12x12 raster, not the usual 20x20 raster.
        self.reject_pixel("titlebar-svg-small", 620, 44, SMOKE.SVG_COLORS["close"])
        self.reject_pixel("titlebar-svg", 604, 56, SMOKE.SVG_COLORS["close"])

    def test_generated_toml_and_relative_distinct_svg_files(self):
        self.assertNotIn(
            "titlebar", tomllib.loads(SMOKE.config_text("xdg-default"))["theme"]
        )
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            SMOKE.write_svg_controls(directory)
            for case in SMOKE.TITLEBAR_CASES:
                with self.subTest(case=case):
                    config = tomllib.loads(SMOKE.config_text(case))
                    theme = config["theme"]["titlebar"]
                    self.assertEqual(
                        config["theme"]["corner_radius"],
                        0 if case in SMOKE.SQUARE_CASES else 24,
                    )
                    if case in SMOKE.SQUARE_CASES:
                        self.assertEqual(config["theme"]["border_width"], 2)
                        self.assertEqual(config["theme"]["blur_radius"], 0)
                        self.assertEqual(
                            config["theme"]["active_border"], [1.0, 0.0, 0.0, 1.0]
                        )
                    self.assertEqual(config["outputs"][0]["height"], 480)
                    for key, value in SMOKE.titlebar_theme(case).items():
                        self.assertEqual(theme[key], value)
                    if case.startswith("titlebar-svg"):
                        for name, path in theme["controls"].items():
                            self.assertFalse(Path(path).is_absolute())
                            color = "#" + "".join(
                                f"{c:02x}" for c in SMOKE.SVG_COLORS[name]
                            )
                            self.assertIn(
                                f'fill="{color}"', (directory / path).read_text()
                            )

    def test_driver_asserts_configures_and_committed_sizes(self):
        for case in SMOKE.TITLEBAR_CASES:
            with self.subTest(case=case):
                client = MagicMock()
                client.expect.return_value = {"state": {}}
                checks = []
                scene = SMOKE.drive_titlebar(client, MagicMock(), case, checks)
                self.assertEqual(scene["theme"], SMOKE.titlebar_theme(case))
                initial = next(
                    call.kwargs
                    for call in client.expect.call_args_list
                    if call.args[0].startswith("configured titlebar height")
                )
                name = SMOKE.view_name(case)
                overlay = case in SMOKE.OVERLAYS
                size = (300, 160) if overlay else (640, 448 - scene["theme"]["height"])
                self.assertEqual(initial["sizes"][name], size)
                self.assertEqual(
                    initial["configure_sizes"][name], (0, 0) if overlay else size
                )
                self.assertEqual(initial["fields"][name]["xdg_mode"], 2)
                self.assertEqual(initial["focus"], name)
                if case in SMOKE.SQUARE_CASES:
                    client.command.assert_any_call(
                        "subsurface background-right background 320 448 320 0 3060e0"
                    )
                    client.command.assert_any_call(
                        "app-xdg launcher 300 160 1 0 20c0d0 default"
                    )
                if case == "titlebar-svg-restore":
                    client.command.assert_any_call("maximize app")
                if case == "titlebar-popup":
                    client.command.assert_any_call(
                        "popup popup launcher 40 50 100 60 d040c0"
                    )

    def test_new_cases_still_require_negotiation_before_first_commit(self):
        for case in SMOKE.TITLEBAR_CASES:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "trace.jsonl"
                name = SMOKE.view_name(case)
                events = [
                    {"event": event, "name": name}
                    for event in ("xdg-decoration", "xdg-ack", "commit")
                ]
                path.write_text("\n".join(json.dumps(event) for event in events))
                SMOKE.check_trace(path, case)
                path.write_text("\n".join(json.dumps(event) for event in events[1:]))
                with self.assertRaises(AssertionError):
                    SMOKE.check_trace(path, case)


if __name__ == "__main__":
    unittest.main()
