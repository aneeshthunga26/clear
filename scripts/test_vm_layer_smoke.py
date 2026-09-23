#!/usr/bin/env python3
"""Fast, compositor-free regression tests for fixture state and GPU assertions."""

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "vm_layer_smoke", Path(__file__).with_name("vm-layer-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


class Frame:
    def __init__(self):
        self.pixels = bytearray(bytes(SMOKE.COLORS["app"]) * (640 * 480))

    def rect(self, x, y, w, h, color):
        row = bytes(color) * w
        for py in range(y, y + h):
            start = (py * 640 + x) * 3
            self.pixels[start : start + w * 3] = row

    def read_bytes(self):
        return b"P6\n640 480\n255\n" + self.pixels


def expected_frame(case):
    frame = Frame()
    if case in (
        "panel-mapped",
        "panel-resized",
        "launcher",
        "launcher-resized",
        "layer-popup",
    ):
        frame.rect(
            0, 0, 640, 64 if case == "panel-resized" else 32, SMOKE.COLORS["panel"]
        )
    if case == "panel-remapped":
        frame.rect(0, 432, 640, 48, SMOKE.COLORS["panel"])
    if case == "layer-popup":
        frame.rect(120, 32, 160, 100, SMOKE.COLORS["popup"])
    if case.startswith("pending-layer-"):
        frame.rect(0, 0, 640, 8, SMOKE.COLORS["bottom"])
        frame.rect(200, 170, 240, 140, SMOKE.COLORS["panel"])
        if case == "pending-layer-committed":
            frame.rect(240, 190, 160, 100, SMOKE.COLORS["overlay"])
    if case in ("priority-top", "priority-overlay"):
        frame.rect(200, 170, 240, 140, SMOKE.COLORS["panel"])
    if case == "priority-overlay":
        frame.rect(240, 190, 160, 100, SMOKE.COLORS["overlay"])
    if case in ("launcher", "launcher-resized"):
        w, h = (180, 100) if case == "launcher" else (260, 140)
        x, y = (640 - w) // 2, 32 + (448 - h) // 2
        frame.rect(x - 7, y - 7, w + 14, h + 14, (160, 48, 192))
        frame.rect(x, y, w, h, SMOKE.COLORS["launcher"])
    return frame


class CaptureTests(unittest.TestCase):
    def test_all_expected_checkpoints(self):
        for case in SMOKE.CASES:
            with self.subTest(case=case):
                SMOKE.check_frame(expected_frame(case), case)

    def test_blank_capture_rejected(self):
        frame = Frame()
        frame.rect(0, 0, 640, 480, (0, 0, 0))
        for case in SMOKE.CASES:
            with self.subTest(case=case), self.assertRaises(AssertionError):
                SMOKE.check_frame(frame, case)

    def test_stale_reservation_rejected(self):
        with self.assertRaises(AssertionError):
            SMOKE.check_frame(expected_frame("panel-mapped"), "panel-unmapped")

    def test_wrong_layer_order_rejected(self):
        with self.assertRaises(AssertionError):
            SMOKE.check_frame(expected_frame("priority-top"), "priority-overlay")
        frame = expected_frame("priority-bottom")
        frame.rect(170, 150, 300, 180, SMOKE.COLORS["bottom"])
        with self.assertRaises(AssertionError):
            SMOKE.check_frame(frame, "priority-bottom")

    def test_surface_origin_instead_of_geometry_origin_rejected(self):
        frame = expected_frame("launcher")
        frame.rect(220, 190, 210, 130, SMOKE.COLORS["app"])
        # Wrongly centering the surface leaves the geometry displaced by its offset.
        frame.rect(237, 213, 180, 100, SMOKE.COLORS["launcher"])
        with self.assertRaises(AssertionError):
            SMOKE.check_frame(frame, "launcher")

    def test_launcher_stale_committed_size_rejected(self):
        with self.assertRaises(AssertionError):
            SMOKE.check_frame(expected_frame("launcher"), "launcher-resized")

    def test_remap_requires_new_placement_and_no_bufferless_reservation(self):
        for stale in ("panel-mapped", "panel-resized", "panel-remapped"):
            with self.subTest(stale=stale), self.assertRaises(AssertionError):
                SMOKE.check_frame(expected_frame(stale), "panel-reconfigured")
        for stale in ("panel-configured", "panel-mapped", "panel-resized"):
            with self.subTest(stale=stale), self.assertRaises(AssertionError):
                SMOKE.check_frame(expected_frame(stale), "panel-remapped")

    def test_pending_layer_neither_applies_early_nor_gets_lost(self):
        for actual, expected in (
            ("pending-layer-committed", "pending-layer-uncommitted"),
            ("pending-layer-uncommitted", "pending-layer-committed"),
        ):
            with self.subTest(expected=expected), self.assertRaises(AssertionError):
                SMOKE.check_frame(expected_frame(actual), expected)

    def test_popup_missing_clipped_or_misplaced_rejected(self):
        for bounds in (
            None,
            (120, 32, 160, 32),
            (121, 32, 160, 100),
            (120, 64, 160, 100),
            (0, 32, 160, 100),
        ):
            frame = expected_frame("panel-mapped")
            if bounds:
                frame.rect(*bounds, SMOKE.COLORS["popup"])
            with self.subTest(bounds=bounds), self.assertRaises(AssertionError):
                SMOKE.check_frame(frame, "layer-popup")

    def test_small_rgb_rounding_tolerated(self):
        frame = Frame()
        frame.rect(0, 0, 640, 480, (49, 175, 82))
        SMOKE.check_frame(frame, "panel-configured")

    def test_incomplete_capture_rejected(self):
        frame = Frame()
        frame.pixels.pop()
        with self.assertRaisesRegex(AssertionError, "truncated"):
            SMOKE.read_ppm(frame)


class StateTests(unittest.TestCase):
    def test_committed_geometry_independent_of_last_configure(self):
        state = {
            "focus": "launcher",
            "views": {
                "launcher": {
                    "width": 260,
                    "height": 140,
                    "configure_width": 180,
                    "configure_height": 100,
                    "configured": True,
                    "mapped": True,
                }
            },
        }
        self.assertTrue(
            SMOKE.state_matches(
                state,
                sizes={"launcher": (260, 140)},
                configure_sizes={"launcher": (180, 100)},
                focus="launcher",
            )
        )
        self.assertFalse(SMOKE.state_matches(state, sizes={"launcher": (180, 100)}))
        self.assertFalse(
            SMOKE.state_matches(state, configure_sizes={"launcher": (260, 140)})
        )

    def test_bufferless_configure_is_not_a_commit(self):
        state = {
            "focus": "app",
            "views": {
                "panel": {
                    "width": 0,
                    "height": 0,
                    "configure_width": 640,
                    "configure_height": 48,
                    "configured": True,
                    "mapped": False,
                }
            },
        }
        self.assertTrue(
            SMOKE.state_matches(
                state,
                sizes={"panel": (0, 0)},
                configure_sizes={"panel": (640, 48)},
                configured=("panel",),
                fields={"panel": {"mapped": False}},
            )
        )
        self.assertFalse(SMOKE.state_matches(state, sizes={"panel": (640, 48)}))
        state["views"]["panel"]["configured"] = False
        self.assertFalse(SMOKE.state_matches(state, configured=("panel",)))

    def test_pending_state_checks_reject_implicit_commit_and_early_focus(self):
        state = {"focus": "top", "views": {"pending": {"commit_count": 1}}}
        expected = {"focus": "top", "fields": {"pending": {"commit_count": 1}}}
        self.assertTrue(SMOKE.state_matches(state, **expected))
        state["views"]["pending"]["commit_count"] = 2
        self.assertFalse(SMOKE.state_matches(state, **expected))
        state["views"]["pending"]["commit_count"] = 1
        state["focus"] = "pending"
        self.assertFalse(SMOKE.state_matches(state, **expected))


if __name__ == "__main__":
    unittest.main()
