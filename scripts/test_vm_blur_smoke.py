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
    def test_optical_sampling_is_bounded_to_nonzero_viewport(self):
        clip = (319, 0, 321, 241)
        def source(x, y):
            self.assertTrue(SMOKE.inside(x, y, clip), (x, y))
            return (48, 96, 144)
        effect = SMOKE.glass_filter(source, clip, clip, 0)
        for x, y in [(319, 0), (639, 240), (319, 120), (479, 120)]:
            color = effect(x, y)
            self.assertTrue(all(math.isfinite(c) and 0 <= c <= 255 for c in color))
        self.assertEqual(effect(479, 120), (48, 96, 144))

    def test_zero_bypasses_glass_and_oracle_rejects_missing_optics(self):
        for method, radius, passes in [("gaussian", 2, 3), ("kawase", 2, 3)]:
            for case in ("xdg", "layer-top", "stacking", "output-boundary-odd"):
                zero = SMOKE.oracle(case, 0, method, passes)
                glass_zero = SMOKE.oracle(case, 0, method, passes, True)
                plain = SMOKE.oracle(case, radius, method, passes)
                glass = SMOKE.oracle(case, radius, method, passes, True)
                points = SMOKE.sample_points(case)
                self.assertTrue(all(zero(x, y) == glass_zero(x, y) for x, y in points))
                with patch.object(SMOKE, "pixel", side_effect=lambda image, x, y: image(x, y)):
                    with self.assertRaises(AssertionError):
                        SMOKE.check_frame(plain, case, radius, method, passes, expected=glass)
                if case == "layer-top":
                    # A fully transparent client hole must expose the unfiltered scene.
                    self.assertEqual(glass(320, 220), zero(320, 220))
                if case == "xdg":
                    self.assertEqual(glass(240, 200), SMOKE.INK)

    def test_glass_option_is_explicit_and_independent_of_filter(self):
        self.assertFalse(SMOKE.parse_args([]).liquid_glass)
        for method in ("gaussian", "kawase"):
            args = SMOKE.parse_args(["--liquid-glass", "--method", method])
            self.assertTrue(args.liquid_glass)
            self.assertIn("[theme.liquid_glass]\nenabled = true", SMOKE.config_text("xdg", 2, method, liquid_glass=True))


class HarnessTests(unittest.TestCase):
    def test_cli_defaults_and_bounds(self):
        args = SMOKE.parse_args([])
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
        ):
            with (
                contextlib.redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                SMOKE.parse_args(argv)

    def test_config_defaults_and_parameters(self):
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
                patch.object(SMOKE, "oracle", side_effect=lambda *a: a) as oracle,
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
