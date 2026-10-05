#!/usr/bin/env python3
"""Deterministic, compositor-free Dual Kawase oracle and harness regressions.

Run: python3 -B scripts/test_vm_blur_smoke.py
The deliberately slow Fraction reference is only used for tiny images; the
production oracle's 640x480 assertion grid is exercised separately.
"""

import contextlib
import importlib.util
import io
import math
import unittest
from fractions import Fraction as F
from pathlib import Path
from unittest.mock import MagicMock, patch

SPEC = importlib.util.spec_from_file_location(
    "vm_blur_smoke", Path(__file__).with_name("vm-blur-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


def reference_step(image, width, height, radius, down):
    """Direct rational-coordinate reference, independent of optimized grids."""
    sh, sw = len(image), len(image[0])

    def bilinear(x, y, channel):
        x, y = max(0, min(sw - 1, x)), max(0, min(sh - 1, y))
        ix, iy = math.floor(x), math.floor(y)
        fx, fy = x - ix, y - iy
        return sum(
            image[min(iy + dy, sh - 1)][min(ix + dx, sw - 1)][channel] * wx * wy
            for dx, wx in ((0, 1 - fx), (1, fx))
            for dy, wy in ((0, 1 - fy), (1, fy))
        )

    r = F(radius)
    if down:
        kernel = [(0, 0, 4), (-r, -r, 1), (-r, r, 1), (r, -r, 1), (r, r, 1)]
        divisor = 8
    else:
        kernel = [
            (-2 * r, 0, 1),
            (2 * r, 0, 1),
            (0, -2 * r, 1),
            (0, 2 * r, 1),
            (-r, -r, 2),
            (-r, r, 2),
            (r, -r, 2),
            (r, r, 2),
        ]
        divisor = 12
    result = []
    for y in range(height):
        row = []
        for x in range(width):
            sx = F(2 * x + 1, 2) * F(sw, width) - F(1, 2)
            sy = F(2 * y + 1, 2) * F(sh, height) - F(1, 2)
            row.append(
                tuple(
                    round(
                        sum(
                            bilinear(sx + dx, sy + dy, c) * weight
                            for dx, dy, weight in kernel
                        )
                        / divisor
                    )
                    for c in range(4)
                )
            )
        result.append(row)
    return result


def reference(image, radius, passes):
    if radius == 0:
        return image
    sizes = []
    for _ in range(passes):
        h, w = len(image), len(image[0])
        if w == h == 1:
            break
        sizes.append((w, h))
        image = reference_step(image, (w + 1) // 2, (h + 1) // 2, radius, True)
    for w, h in reversed(sizes):
        image = reference_step(image, w, h, radius, False)
    return image


def varied(x, y):
    return (
        (x * 67 + y * 19) % 256,
        (x * 31 + y * 97) % 256,
        (x * 43 + y * 53 + 79) % 256,
        (x * 17 + y * 29 + 13) % 256,
    )


class KawaseTests(unittest.TestCase):
    def test_sizes_ceil_half_and_exact_reverse_targets(self):
        self.assertEqual(
            SMOKE.kawase_sizes(321, 241, 3),
            [(321, 241), (161, 121), (81, 61), (41, 31)],
        )
        self.assertEqual(SMOKE.kawase_sizes(1, 7, 6), [(1, 7), (1, 4), (1, 2), (1, 1)])
        self.assertEqual(SMOKE.kawase_sizes(1, 1, 6), [(1, 1)])
        self.assertEqual(len(SMOKE.kawase_sizes(640, 480, 6)), 7)

    def test_downsample_known_one_dimensional_ramp(self):
        values = [0, 64, 128, 192, 255]
        source = lambda x, y: (values[x],) * 4
        grid = SMOKE.kawase_step(source, (5, 1), (3, 1), 1, down=True)
        self.assertEqual([grid(x, 0)[0] for x in range(3)], [32, 128, 223])

    def test_exact_reference_odd_tiny_and_fractional(self):
        for w, h in ((5, 3), (7, 9), (1, 7), (2, 1), (1, 1)):
            image = [[varied(x, y) for x in range(w)] for y in range(h)]
            for radius in (F(1, 2), F(3, 2), F(2)):
                for passes in (1, 3, 6):
                    with self.subTest(size=(w, h), radius=radius, passes=passes):
                        wanted = reference(image, radius, passes)

                        # Nonzero origins in BOTH axes must not change local sampling.
                        def source(x, y, w=w, h=h, image=image):
                            self.assertTrue(19 <= x < 19 + w and 11 <= y < 11 + h)
                            return image[y - 11][x - 19]

                        grid = SMOKE.kawase(
                            source, (19, 11, w, h), float(radius), passes
                        )
                        actual = [
                            [grid(x + 19, y + 11) for x in range(w)] for y in range(h)
                        ]
                        self.assertEqual(actual, wanted)

    def test_constants_all_channels_and_rgb(self):
        for size in ((1, 1), (1, 9), (9, 1), (13, 7)):
            for color in (
                (0, 0, 0, 0),
                (255, 255, 255, 255),
                (7, 61, 129, 203),
                (17, 128, 239),
            ):
                for passes in (1, 3, 6):
                    grid = SMOKE.kawase(
                        lambda x, y, color=color: color, (3, 5, *size), 1.5, passes
                    )
                    self.assertTrue(
                        all(
                            grid(x + 3, y + 5) == color
                            for y in range(size[1])
                            for x in range(size[0])
                        )
                    )

    def test_zero_is_identity_and_level_zero_is_fresh(self):
        source = lambda x, y: varied(x, y)
        self.assertIs(SMOKE.kawase(source, (0, 0, 7, 5), 0, 6), source)
        color = [10, 30, 50, 70]
        first = SMOKE.kawase(lambda x, y: tuple(color), (0, 0, 7, 5), 2, 3)
        color[:] = [80, 100, 120, 140]
        second = SMOKE.kawase(lambda x, y: tuple(color), (0, 0, 7, 5), 2, 3)
        self.assertEqual(first(3, 2), (10, 30, 50, 70))
        self.assertEqual(second(3, 2), tuple(color))

    def test_pass_depth_and_fractional_radius_are_observable(self):
        # Include lower-frequency structure, not just checks which can alias to
        # the same constant at several pyramid depths.
        source = lambda x, y: (255 if x < 17 and y < 13 else 0,) * 4
        points = [(x, y) for y in range(25) for x in range(33)]
        outputs = []
        for radius, passes in ((1.5, 1), (1.5, 3), (1.5, 6), (0.75, 3), (2, 3)):
            grid = SMOKE.kawase(source, (0, 0, 33, 25), radius, passes)
            outputs.append([grid(x, y)[0] for x, y in points])
        for i, a in enumerate(outputs):
            for b in outputs[i + 1 :]:
                self.assertGreater(max(abs(x - y) for x, y in zip(a, b)), 8)

    def test_full_viewport_assertion_grid_and_parameter_aware_variance(self):
        for case, radius, passes in (
            ("stacking", 1.5, 6),
            ("output-boundary-odd", 0.25, 1),
        ):
            with self.subTest(case=case):
                expected = SMOKE.oracle(case, radius, "kawase", passes)
                zero = SMOKE.oracle(case, 0, "kawase", passes)
                with patch.object(
                    SMOKE, "pixel", side_effect=lambda image, x, y: image(x, y)
                ):
                    report = SMOKE.check_frame(
                        expected, case, radius, "kawase", passes, expected=expected
                    )
                    self.assertEqual(report["max_channel_error"], 0)
                    SMOKE.compare_pair(
                        zero,
                        expected,
                        case,
                        radius,
                        "kawase",
                        passes,
                        expected=expected,
                    )
                    with self.assertRaises(AssertionError):
                        SMOKE.check_frame(
                            zero, case, radius, "kawase", passes, expected=expected
                        )


class GlassTests(unittest.TestCase):
    def test_refraction_width_preserves_rim_center_and_continuity(self):
        for width in (0.25, 1, 3, 10):
            for x, y in ((0, 75), (75, 0), (135, 150), (210, 75), (105, 75),
                         (75 - 75 / math.sqrt(2), 75 - 75 / math.sqrt(2))):
                u, v = SMOKE.glass_refraction_uv(x / 210, y / 150, width)
                self.assertAlmostEqual(u, x / 210)
                self.assertAlmostEqual(v, y / 150)
            # Both joins to the straight strips and the central line are continuous.
            for x, y, dx, dy in ((75, 30, 1e-5, 0), (135, 120, 1e-5, 0),
                                 (105, 75, 0, 1e-5)):
                a = SMOKE.glass_refraction_uv((x - dx) / 210, (y - dy) / 150, width)
                b = SMOKE.glass_refraction_uv((x + dx) / 210, (y + dy) / 150, width)
                self.assertLess(math.dist(a, b), 1e-5)
        for x, y in ((30, 60), (105, 12), (170, 85)):
            self.assertEqual(SMOKE.glass_refraction_uv(x / 210, y / 150, 1),
                             (x / 210, y / 150))
        depths = [SMOKE.glass_refraction_uv(.5, .2, w)[1] for w in (.25, 1, 3, 10)]
        self.assertTrue(all(a > b for a, b in zip(depths, depths[1:])))
        # Twenty source pixels inward is almost flat originally, but refracts
        # appreciably when widened; the unchanged PNG supplies this profile.
        green = [SMOKE.glass_map_sample('displacement',
                 *SMOKE.glass_refraction_uv(.5, 20 / 150, w))[1] for w in (1, 3, 10)]
        self.assertLess(green[0], 129)
        self.assertGreater(green[1], 150)
        self.assertGreater(green[2], 200)

    def test_zero_refraction_width_disables_only_refraction(self):
        args = (lambda x, y: (x, y, 50), (0, 0, 240, 180), (0, 0, 210, 150), 75)
        disabled = SMOKE.glass_filter(*args, refraction_level=10, refraction_width=0)
        zero_level = SMOKE.glass_filter(*args, refraction_level=0)
        for x, y in ((52, 75), (105, 12), (150, 90)):
            self.assertEqual(disabled(x, y), zero_level(x, y))

    def test_map_fitting_preserves_reference_coordinates_and_window_perimeter(self):
        for x, y in ((0.5, 75.5), (60.5, 30.5), (105.5, 75.5), (190.5, 110.5)):
            u, v = SMOKE.glass_edge_uv(x, y, (0, 0, 210, 150), (75,) * 4)
            self.assertAlmostEqual(u, x / 210)
            self.assertAlmostEqual(v, y / 150)
        # The reference's arc must fit the native corner, not float inside a rectangle.
        corner = 20 * (1 - 1 / math.sqrt(2))
        u, v = SMOKE.glass_edge_uv(corner, corner, (0, 0, 400, 200), (20,) * 4)
        self.assertAlmostEqual(math.hypot(u * 210 - 75, v * 150 - 75), 75)
        for radii in ((0,) * 4, (20, 0, 40, 12), (0.5,) * 4):
            for x, y in ((0.5, 0.5), (200, 100), (399.5, 199.5)):
                self.assertTrue(all(math.isfinite(v) for v in
                                    SMOKE.glass_edge_uv(x, y, (0, 0, 400, 200), radii)))

    def test_reference_map_bytes_and_dimensions(self):
        import hashlib
        maps = (("magnifying", 210, 150, "991d5a0b7b8ce67a14e03e5ff4bea56d7432eaef87c8def94b8d4fbc5482e6b0"),
                ("displacement", 420, 300, "4b65b346a2d5c50b3dae3e6436e4c9d6acb16104b317cdce256e090b9a4dfcff"),
                ("specular", 420, 300, "f49768994e5f39532370c7c37d00487fbd8efd93046418ef10017cf1249b7d6a"))
        for name, width, height, digest in maps:
            path = Path(__file__).resolve().parents[1] / "src/platform/smithay/glass-maps" / (name + ".png")
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)
            self.assertEqual(SMOKE.glass_map(name)[:2], (width, height))
        self.assertEqual(SMOKE.glass_map('magnifying')[2][75][52], (192, 128, 0, 255))

    def test_zoom_endpoints_and_default_are_independent_of_refraction(self):
        for zoom, expected_x in ((0, 52), (0.5, 55), (1, 58), (2, 64)):
            effect = SMOKE.glass_filter(lambda x, y: (x, y, 0), (0, 0, 240, 180),
                                       (0, 0, 210, 150), 75, specular_opacity=0,
                                       specular_saturation=1, refraction_level=0,
                                       zoom_level=zoom)
            # SVG scale*(channel/255-.5), followed by the RGBA8 intermediate.
            self.assertEqual(effect(52, 75), (expected_x, 75, 0))

    def test_svg_saturation_and_specular_opacity_are_independent(self):
        color = (40, 100, 180)
        transparent = (0, 0, 0, 0)
        self.assertEqual(SMOKE.glass_specular(color, transparent, 1, 50), color)
        # Saturation remains effective with specular opacity zero.
        gray = SMOKE.glass_specular(color, (255, 255, 255, 255), 0, 0)
        wanted = .213 * 40 + .715 * 100 + .072 * 180
        for channel in gray:
            self.assertAlmostEqual(channel, wanted)
        self.assertEqual(SMOKE.glass_specular(color, (255, 255, 255, 255), 1, 9), (255, 255, 255))
        result = SMOKE.glass_specular(color, (64, 64, 64, 128), .5, 1)
        for actual, channel in zip(result, color):
            self.assertAlmostEqual(actual, 32 + channel * (1 - 64 / 255))

    def test_reference_glass_keeps_foreground_holes_and_output_clamps(self):
        for case in ("xdg", "layer-top", "rounded-asymmetric", "output-boundary-odd"):
            effect = SMOKE.oracle(case, 0, liquid_glass=True)
            plain = SMOKE.oracle(case, 0)
            self.assertEqual(effect(0, 0), plain(0, 0))
            if case == "xdg":
                self.assertEqual(effect(240, 200), SMOKE.INK)
            cx, cy, cw, ch = SMOKE.viewport(case)
            constant = SMOKE.glass_filter(lambda x, y: (20, 30, 40), (cx, cy, cw, ch),
                                         SMOKE.geometry(case)[0], SMOKE.case_radii(case),
                                         specular_opacity=0, specular_saturation=1)
            for x, y in ((cx, cy), (cx + cw - 1, cy + ch - 1)):
                for got, want in zip(constant(x, y), (20, 30, 40)):
                    self.assertAlmostEqual(got, want)


class HarnessTests(unittest.TestCase):
    def test_cli_defaults_and_bounds(self):
        args = SMOKE.parse_args([])
        self.assertEqual(args.zoom_level, 1)
        self.assertEqual(args.refraction_width, 1)
        for width in (0, 0.5, 1, 3, 10):
            self.assertEqual(SMOKE.parse_args(["--refraction-width", str(width)]).refraction_width, width)
        for level in (0, 1, 4, 7.5, 10):
            self.assertEqual(SMOKE.parse_args(["--refraction-level", str(level)]).refraction_level, level)
        self.assertEqual((args.specular_opacity, args.specular_saturation, args.refraction_level), (.5, 9, 1))
        self.assertEqual((args.method, args.radius, args.passes), ("gaussian", 12, 3))
        args = SMOKE.parse_args(["--method", "kawase"])
        self.assertEqual((args.radius, args.passes), (2, 3))
        args = SMOKE.parse_args(
            ["--method", "kawase", "--radius", "1.5", "--passes", "6"]
        )
        self.assertEqual((args.radius, args.passes), (1.5, 6))
        for argv in (
            ["--passes", "0"],
            ["--passes", "7"],
            ["--radius", "nan"],
            ["--radius", "inf"],
            ["--radius", "-1"],
            ["--radius", "33"],
            ["--refraction-level", "nan"],
            ["--refraction-width", "nan"],
            ["--refraction-width", "inf"],
            ["--refraction-width", "-0.01"],
            ["--refraction-width", "10.01"],
            ["--refraction-level", "10.01"],
            ["--refraction-level", "inf"],
            ["--refraction-level", "-0.01"],
            ["--specular-opacity", "-1"],
            ["--specular-saturation", "51"],
            ["--glass-model", "lens"],
            ["--lens-area", "25"],
            ["--mirror-strength", "1"],
            ["--zoom-level", "nan"],
            ["--zoom-level", "inf"],
            ["--zoom-level", "-1"],
            ["--zoom-level", "2.01"],
        ):
            with (
                contextlib.redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                SMOKE.parse_args(argv)

    def test_config_defaults_and_parameters(self):
        self.assertIn("refraction_width = 3", SMOKE.config_text(
            "xdg", 2, liquid_glass=True, refraction_width=3))
        self.assertIn("zoom_level = 1.75", SMOKE.config_text(
            "xdg", 2, liquid_glass=True, zoom_level=1.75))
        self.assertIn("refraction_level = 10", SMOKE.config_text(
            "xdg", 2, liquid_glass=True, refraction_level=10))
        default = SMOKE.config_text("xdg", None)
        self.assertNotIn("blur_", default)
        self.assertIn("blur_radius = 12.0", SMOKE.config_text("xdg", 12))
        for passes in (1, 3, 6):
            text = SMOKE.config_text("output-boundary-odd", 1.5, "kawase", passes)
            self.assertIn('blur_method = "kawase"', text)
            self.assertIn(f"blur_passes = {passes}", text)
            self.assertIn("blur_radius = 1.5", text)
            self.assertIn("width = 321\nheight = 241", text)
        self.assertEqual(SMOKE.viewport("output-boundary-odd"), (319, 0, 321, 241))
        self.assertEqual(SMOKE.background("output-boundary-odd", 318, 0), SMOKE.RIGHT)
        self.assertEqual(
            SMOKE.background("output-boundary-odd", 319, 0), SMOKE.pattern(0, 0)
        )

    def test_cli_parameters_reach_captures_oracle_and_report(self):
        for passes in (1, 3, 6):
            args = SMOKE.parse_args(
                ["--method", "kawase", "--radius", "1.5", "--passes", str(passes)]
            )
            with (
                patch.object(
                    SMOKE, "capture", side_effect=lambda *a: a[-1] or 0
                ) as capture,
                patch.object(SMOKE, "oracle", side_effect=lambda *a, **kw: a) as oracle,
                patch.object(SMOKE, "check_frame", return_value={}) as check,
                patch.object(SMOKE, "compare_pair", return_value={}) as compare,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                report = SMOKE.run_case(
                    args, None, MagicMock(), {}, None, "host", "xdg"
                )
            self.assertEqual(
                [c.args[-1] for c in capture.call_args_list], [None, 0, 1.5]
            )
            self.assertEqual(
                [c.args for c in oracle.call_args_list],
                [("xdg", r, "kawase", passes, False) for r in (0, 0, 1.5)],
            )
            self.assertEqual(check.call_args.args[2:5], (1.5, "kawase", passes))
            self.assertTrue(all(c.kwargs == {"specular_opacity": 0.5, "specular_saturation": 9, "refraction_level": 1, "zoom_level": 1, "refraction_width": 1}
                                for c in oracle.call_args_list))
            self.assertEqual(report["refraction_level"], 1)
            self.assertEqual(compare.call_args.args[3:6], (1.5, "kawase", passes))
            self.assertEqual(
                (report["method"], report["radius"], report["passes"]),
                ("kawase", 1.5, passes),
            )
            self.assertTrue(report["passed"] and report["default_equals_zero"])

    def test_all_case_samples_are_in_frame(self):
        for case in SMOKE.CASES:
            w, h = SMOKE.frame_size(case)
            points = SMOKE.sample_points(case)
            self.assertGreater(len(points), 3000)
            self.assertTrue(all(0 <= x < w and 0 <= y < h for x, y in points))


if __name__ == "__main__":
    unittest.main()
