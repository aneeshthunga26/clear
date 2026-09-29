#!/usr/bin/env python3
"""Bounded backdrop-blur GPU smoke on a private virtual KWin host (run in a VM).

Build Rust separately. Requires Python 3.10+, cc, pkg-config, wayland-scanner,
Wayland client development files/protocol XMLs, dbus-run-session, KWin with its
virtual backend, and working EGL/GLES. No Pillow, numpy, foot, shell UI, physical
input injection, or logged-in desktop is used. Only the existing C fixture builds.

Cases capture radius zero and --radius; xdg also captures an omitted blur_radius
(default-zero regression). Gaussian defaults to radius 12; Kawase to radius 2 and
3 passes. --seconds bounds EACH capture, not the entire suite.
Add --liquid-glass to verify refraction, dispersion and edge lighting against
an independent scalar optical oracle after either filter. Radius zero still bypasses optics.
Focused Kawase runs: --method kawase --case stacking --case output-boundary-odd
with --radius 1.5 and --passes 1, 3, or 6 (use separate --artifacts).
output-boundary-odd filters a 321x241 viewport at (319, 0), beside solid magenta;
it selects the second output using private shell IPC, not physical input.
All PNGs, PPMs, protocol traces, configs, build logs and results stay under target/.
--build-only validates the fixture without launching KWin or Clear.

The independent Gaussian oracle uses direct exp() weights and separable filtering.
The Dual Kawase oracle uses ceil-half pyramids, center-aligned bilinear sampling,
source-level offsets, edge clamping, and RGBA8 quantization after every step.
It does not import renderer code or derive expected blur from captured imagery.
Per-channel tolerances are 2 at radius zero and 4 with blur enabled, allowing SHM
premultiplication, UNORM intermediates, and GLES rounding, not spatial offsets.
Window output is F + (C-A)*blurred + (1-C)*original. Layers/popups instead use
blur weight A*(1-A): fully transparent holes stay clear. Transparent pixels INSIDE
an XDG geometric body intentionally blur, while rounded cut-outs stay unchanged.
"""

import argparse
import importlib.util
import json
import math
import os
import statistics
import struct
import subprocess
import sys
import time
import zlib
from functools import cache
from pathlib import Path

# Do not leave Python cache artifacts beside the existing scripts, even without -B.
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "blur_layer_smoke", Path(__file__).with_name("vm-layer-smoke.py")
)
assert SPEC is not None and SPEC.loader is not None
LAYER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LAYER)

CASES = (
    "xdg",
    "rounded-ssd",
    "layer-background",
    "layer-bottom",
    "layer-top",
    "layer-overlay",
    "popup",
    "stacking",
    "output-boundary",
    "output-boundary-odd",
)
BOUNDARIES = ("output-boundary", "output-boundary-odd")
KINDS = {"background": 0, "bottom": 1, "top": 2, "overlay": 3}
TINT = (32, 192, 208)
INK = (248, 232, 80)
LOWER = (24, 72, 224)
UPPER = (240, 32, 48)
RIGHT = (248, 16, 224)
# Ordered back-to-front, in client geometry coordinates. The gap x=104..135 is
# genuinely transparent, not an alpha-zero child over an already painted parent.
PATCHES = (
    ("left", (0, 0, 104, 160), TINT, 128),
    ("right", (136, 0, 104, 160), TINT, 128),
    ("overlap", (60, 70, 24, 24), (224, 64, 48), 128),
    ("stem", (40, 40, 3, 40), INK, 255),
    ("cap", (40, 40, 16, 3), INK, 255),
)


def inside(x, y, box):
    bx, by, w, h = box
    return bx <= x < bx + w and by <= y < by + h


def pattern(x, y):
    # Four-pixel two-dimensional checks exercise both filter axes. Neutral
    # colors make cross-output contamination from magenta particularly visible.
    value = 224 if (x // 4 + y // 4) % 2 else 32
    return (value,) * 3


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


def gaussian(source, clip, radius):
    """Independent normalized kernel; quantize each opaque RGB render target."""
    if radius == 0:
        return source
    support = math.ceil(radius)
    sigma = max(radius / 3, 0.5)
    weights = [math.exp(-0.5 * (i / sigma) ** 2) for i in range(-support, support + 1)]
    total = sum(weights)
    taps = [(i - support, w / total) for i, w in enumerate(weights)]
    bx, by, width, height = clip

    @cache
    def horizontal(x, y):
        samples = [
            (source(max(bx, min(bx + width - 1, x + dx)), y), weight)
            for dx, weight in taps
        ]
        return tuple(round(sum(p[c] * w for p, w in samples)) for c in range(3))

    @cache
    def blurred(x, y):
        samples = [
            (horizontal(x, max(by, min(by + height - 1, y + dy))), weight)
            for dy, weight in taps
        ]
        return tuple(round(sum(p[c] * w for p, w in samples)) for c in range(3))

    return blurred


def kawase_sizes(width, height, passes):
    """Exact destination sizes; stop only when both axes reach one pixel."""
    sizes = [(width, height)]
    for _ in range(passes):
        if (width, height) == (1, 1):
            break
        width, height = (width + 1) // 2, (height + 1) // 2
        sizes.append((width, height))
    return sizes


def kawase_step(source, source_size, dest_size, radius, down):
    """Memoized RGBA8 grid, not a recursive re-evaluation of entire pyramids.

    Compact byte grids bound storage by the sum of the pyramid areas. Precomputed
    axis coordinates avoid repeating clamping/floor arithmetic for each channel.
    Only pixels needed by the assertion grid (and their dependencies) are evaluated.
    """
    sw, sh = source_size
    dw, dh = dest_size
    diagonals = [
        (x * radius, y * radius, 1 if down else 2) for x in (-1, 1) for y in (-1, 1)
    ]
    taps = (
        [(0, 0, 4)]
        if down
        else [
            (-2 * radius, 0, 1),
            (2 * radius, 0, 1),
            (0, -2 * radius, 1),
            (0, 2 * radius, 1),
        ]
    ) + diagonals
    divisor = 8 if down else 12

    def axis(source_length, dest_length, offset):
        result = []
        for d in range(dest_length):
            s = (d + 0.5) * source_length / dest_length - 0.5 + offset
            s = max(0, min(source_length - 1, s))
            low = math.floor(s)
            result.append((low, min(low + 1, source_length - 1), s - low))
        return result

    xs = {dx: axis(sw, dw, dx) for dx, _, _ in taps}
    ys = {dy: axis(sh, dh, dy) for _, dy, _ in taps}
    sampling = [(xs[dx], ys[dy], weight) for dx, dy, weight in taps]
    pixels = bytearray(dw * dh * 4)
    ready = bytearray(dw * dh)

    def sample(x, y):
        index = y * dw + x
        start = index * 4
        if not ready[index]:
            sums = [0.0] * 4
            for xt, yt, weight in sampling:
                x0, x1, fx = xt[x]
                y0, y1, fy = yt[y]
                a, b = source(x0, y0), source(x1, y0)
                c, d = source(x0, y1), source(x1, y1)
                for channel in range(4):
                    top = a[channel] * (1 - fx) + b[channel] * fx
                    bottom = c[channel] * (1 - fx) + d[channel] * fx
                    sums[channel] += (top * (1 - fy) + bottom * fy) * weight
            pixels[start : start + 4] = bytes(round(v / divisor) for v in sums)
            ready[index] = 1
        return tuple(pixels[start : start + 4])

    return sample


def kawase(source, clip, radius, passes):
    """Independent clip-local Dual Kawase filter; source may supply RGB or RGBA."""
    if radius == 0:
        return source
    bx, by, width, height = clip
    # Copy the entire viewport, not the surface bounds. Never sample adjacent
    # outputs, and never reuse a previous surface's filtered lower scene.
    pixels = bytearray()
    channels = len(source(bx, by))
    for y in range(by, by + height):
        for x in range(bx, bx + width):
            value = source(x, y)
            pixels.extend(value if channels == 4 else (*value, 255))

    def level_zero(x, y):
        start = (y * width + x) * 4
        return tuple(pixels[start : start + 4])

    sizes = kawase_sizes(width, height, passes)
    grid = level_zero
    for small, large in zip(sizes[1:], sizes):
        grid = kawase_step(grid, large, small, radius, down=True)
    for small, large in reversed(list(zip(sizes[1:], sizes))):
        grid = kawase_step(grid, small, large, radius, down=False)
    return lambda x, y: grid(x - bx, y - by)[:channels]


def viewport(case):
    if case == "output-boundary":
        return (0, 0, 320, 240)
    if case == "output-boundary-odd":
        return (319, 0, 321, 241)
    return (0, 0, 640, 480)


def frame_size(case):
    return (640, viewport(case)[3])


def rounded_coverage(x, y, box, radius):
    # Signed distance at the pixel center, with one-pixel smoothstep AA. All
    # tested outlines have uniform radius 24 and are too large to require fitting.
    bx, by, w, h = box
    px, py = x + 0.5 - bx, y + 0.5 - by
    distance = min(px, py, w - px, h - py)
    if radius:
        cx = min(max(px, radius), w - radius)
        cy = min(max(py, radius), h - radius)
        distance = min(distance, radius - math.hypot(px - cx, py - cy))
    t = max(0, min(1, distance + 0.5))
    return t * t * (3 - 2 * t)


def geometry(case):
    if case in BOUNDARIES:
        return viewport(case), viewport(case)
    if case == "rounded-ssd":
        return (200, 144, 240, 192), (200, 176, 240, 160)
    return (200, 160, 240, 160), (200, 160, 240, 160)


def window_case(case):
    return case in ("xdg", "rounded-ssd", "stacking")


def background(case, x, y):
    if case in BOUNDARIES:
        if not inside(x, y, viewport(case)):
            return RIGHT
        return pattern(x - viewport(case)[0], y)
    if case == "stacking" and inside(x, y, (280, 200, 80, 80)):
        return LOWER
    if case == "popup" and y < 16:
        return LOWER
    return pattern(x, y)


def foreground(case, x, y):
    if case == "stacking" or case in BOUNDARIES:
        return tuple(c * 128 // 255 for c in TINT) + (128,)
    value = (0, 0, 0, 0)
    for _, box, color, alpha in PATCHES:
        if inside(x, y, box):
            source = tuple(c * alpha // 255 for c in color) + (alpha,)
            value = tuple(
                round(s + d * (1 - alpha / 255)) for s, d in zip(source, value)
            )
    return value


def glass_filter(source, clip, box, radius):
    """Independent scalar Snell-law oracle for the default liquid-glass settings.

    Coordinates are desktop pixel centers, unlike the shader's flipped UVs.
    Samples use bilinear interpolation of the already quantized blur result.
    """
    bx, by, w, h = box
    cx, cy, cw, ch = clip

    def distance(x, y):
        px, py = x - bx, y - by
        d = min(px, py, w - px, h - py)
        if radius:
            qx = min(max(px, radius), w - radius)
            qy = min(max(py, radius), h - radius)
            d = min(d, radius - math.hypot(px - qx, py - qy))
        return d

    def sample(x, y):
        x, y = max(cx, min(cx + cw - 1, x)), max(cy, min(cy + ch - 1, y))
        ix, iy = math.floor(x), math.floor(y)
        fx, fy = x - ix, y - iy
        return tuple(sum(
            source(min(ix + dx, cx + cw - 1), min(iy + dy, cy + ch - 1))[c] * wx * wy
            for dx, wx in ((0, 1 - fx), (1, fx))
            for dy, wy in ((0, 1 - fy), (1, fy))
        ) for c in range(3))

    def filtered(x, y):
        px, py = x + 0.5, y + 0.5
        d = distance(px, py)
        if d < 0:
            return source(x, y)
        t = max(0, min(1, d / min(24, w / 2, h / 2)))
        edge = 1 - t * t * (3 - 2 * t)
        gx = distance(px - 0.5, py) - distance(px + 0.5, py)
        gy = distance(px, py - 0.5) - distance(px, py + 0.5)
        length = max(math.hypot(gx, gy), 0.0001)
        gx, gy = gx / length, gy / length
        nx = gx * edge * 2 + ((px - bx - w / 2) / (w / 2)) * 0.5 * 0.35
        ny = gy * edge * 2 + ((py - by - h / 2) / (h / 2)) * 0.5 * 0.35
        length = math.sqrt(nx * nx + ny * ny + 1)
        nx, ny, nz = nx / length, ny / length, 1 / length
        channels = []
        for c, ior in enumerate((1.5 - 0.15 * 0.15, 1.5, 1.5 + 0.15 * 0.15)):
            eta = 1 / ior
            # Refract incident (0,0,-1) at the normalized surface normal.
            factor = eta * nz - math.sqrt(1 - eta * eta * (1 - nz * nz))
            channels.append(sample(x + nx * factor * 12, y + ny * factor * 12)[c])
        light = max(-(gx + gy) / math.sqrt(2), 0)
        reflection = min(1, 0.25 * (edge * edge * light * 0.6 + (1 - nz) ** 3))
        return tuple(c * (1 - reflection) + 255 * reflection for c in channels)

    return filtered


def oracle(case, radius, method="gaussian", passes=3, liquid_glass=False):
    frame, body = geometry(case)
    clip = viewport(case)
    source = lambda x, y: background(case, x, y)
    blurred = (
        kawase(source, clip, radius, passes)
        if method == "kawase"
        else gaussian(source, clip, radius)
    )

    if liquid_glass and radius:
        blurred = glass_filter(blurred, clip, frame, 24 if case == "rounded-ssd" else 0)

    def expected(x, y):
        original = source(x, y)
        # This opaque overlay is deliberately absent from source(): a lower
        # window must never sample surfaces that will be composited above it.
        if case == "stacking" and inside(x, y, (308, 190, 24, 100)):
            return UPPER
        coverage = rounded_coverage(x, y, frame, 24 if case == "rounded-ssd" else 0)
        if not coverage:
            return original
        if case == "rounded-ssd" and y < body[1]:
            # Font/control pixels are compared between captures, not predicted.
            return None
        if not inside(x, y, body):
            return original
        f = foreground(case, x - body[0], y - body[1])
        f = tuple(round(channel * coverage) for channel in f)
        alpha = f[3] / 255
        weight = (
            max(coverage, alpha) - alpha if window_case(case) else alpha * (1 - alpha)
        )
        filtered = blurred(x, y) if radius and weight else original
        return tuple(
            round(f[c] + weight * filtered[c] + (1 - alpha - weight) * original[c])
            for c in range(3)
        )

    return expected


def sample_points(case):
    frame, body = geometry(case)
    bx, by, w, h = body
    width, height = frame_size(case)
    points = {
        (x, y)
        for y in range(max(0, frame[1] - 4), min(height, by + h + 4), 3)
        for x in range(max(0, bx - 4), min(width, bx + w + 4), 3)
    }
    # Exact glyph edges, alpha overlap, transparent holes and rounded AA pixels;
    # sparse grid alone could miss a three-pixel opaque stroke or a one-pixel halo.
    for box in ((38, 38, 20, 44), (58, 68, 28, 28), (104, 35, 32, 90)):
        x, y, rw, rh = box
        points.update(
            (bx + px, by + py) for py in range(y, y + rh) for px in range(x, x + rw)
        )
    for cx in (bx, bx + w - 24):
        for cy in (frame[1], by + h - 24):
            points.update(
                (x, y) for y in range(cy, cy + 24) for x in range(cx, cx + 24)
            )
    if case == "output-boundary":
        points.update((x, y) for x in range(292, 349) for y in range(8, 232, 3))
        points.update((x, y) for x in range(8, 32) for y in range(13))
    if case == "output-boundary-odd":
        points.update((x, y) for x in range(292, 350) for y in range(8, 233, 3))
        points.update((x, y) for x in range(319, 352) for y in range(13))
    if case == "stacking":
        points.update((x, y) for x in range(294, 346) for y in range(204, 278, 2))
    points.update((x, y) for x in (1, 100, 500, 638) for y in (1, 100, height - 2))
    return sorted((x, y) for x, y in points if 0 <= x < width and 0 <= y < height)


def read_frame(path, case):
    width, height, pixels = LAYER.read_ppm(path)
    expected = frame_size(case)
    LAYER.require(
        (width, height) == expected,
        f"{path}: expected {expected}, got {width}x{height}",
    )
    return width, height, pixels


def pixel(image, x, y):
    width, height, data = image
    LAYER.require(
        0 <= x < width and 0 <= y < height, f"sample outside capture: {x},{y}"
    )
    offset = (y * width + x) * 3
    return tuple(data[offset : offset + 3])


def check_frame(image, case, radius, method="gaussian", passes=3, expected=None):
    expected = expected or oracle(case, radius, method, passes)
    tolerance = 4 if radius else 2
    maximum = checked = 0
    for x, y in sample_points(case):
        wanted = expected(x, y)
        if wanted is None:
            continue
        actual = pixel(image, x, y)
        error = max(abs(a - b) for a, b in zip(actual, wanted))
        LAYER.require(
            error <= tolerance,
            f"{case} {method} radius={radius} passes={passes} ({x},{y}): expected {wanted}, got {actual}; tolerance={tolerance}",
        )
        maximum = max(maximum, error)
        checked += 1
    LAYER.require(checked > 3000, f"insufficient oracle coverage: {checked}")
    return {
        "oracle_pixels": checked,
        "max_channel_error": maximum,
        "tolerance": tolerance,
    }


def compare_pair(
    zero, blurred, case, radius=12, method="gaussian", passes=3, expected=None
):
    frame, body = geometry(case)
    bx, by, _, _ = body
    # One uniform-alpha region with no glyphs/overlaps or outline edge. A paired
    # variance check rejects no-op blur independently of absolute pixel tolerances.
    roi = [(bx + x, by + y) for x in range(20, 36) for y in range(30, 66)]
    variances = [
        statistics.pvariance(pixel(image, x, y)[0] for x, y in roi)
        for image in (zero, blurred)
    ]
    LAYER.require(
        variances[0] > 1500,
        f"{case}: zero-radius patterned backdrop missing: {variances}",
    )
    ratio = variances[1] / variances[0]
    mean_delta = statistics.mean(
        abs(pixel(zero, x, y)[0] - pixel(blurred, x, y)[0]) for x, y in roi
    )
    predicted = {}
    if method == "gaussian" and radius == 12:
        # Preserve the original default regression checks exactly.
        limit = 0.05 if window_case(case) else 0.40
        LAYER.require(
            ratio < limit,
            f"{case}: blur did not reduce variance enough: {variances}, limit={limit}",
        )
        LAYER.require(
            mean_delta > 15, f"{case}: radius 0/12 imagery barely changed: {mean_delta}"
        )
    else:
        expected = expected or oracle(case, radius, method, passes)
        expected_zero = oracle(case, 0, method, passes)
        wanted, baseline = [], []
        for x, y in roi:
            filtered, original = expected(x, y), expected_zero(x, y)
            assert filtered is not None and original is not None
            wanted.append(filtered[0])
            baseline.append(original[0])
        variance = statistics.pvariance(wanted)
        delta = statistics.mean(abs(a - b) for a, b in zip(baseline, wanted))
        # A per-pixel error <=4 implies a standard-deviation error <=4. In
        # particular, shallow/integer-offset Kawase can alias the checks rather
        # than suppress them. Do not impose Gaussian's fixed variance ratio.
        LAYER.require(
            abs(math.sqrt(variances[1]) - math.sqrt(variance)) <= 4,
            f"{case}: variance {variances[1]} disagrees with oracle {variance}",
        )
        LAYER.require(
            abs(mean_delta - delta) <= 6,
            f"{case}: paired delta {mean_delta} disagrees with oracle {delta}",
        )
        predicted = {"oracle_variance": variance, "oracle_mean_absolute_delta": delta}
    unchanged = []
    if case != "stacking" and case not in BOUNDARIES:
        # Every opaque glyph pixel including its edge must remain crisp.
        for _, box, _, alpha in PATCHES:
            if alpha == 255:
                x, y, w, h = box
                unchanged.extend(
                    (bx + px, by + py)
                    for px in range(x, x + w)
                    for py in range(y, y + h)
                )
        if not window_case(case):
            unchanged.extend(
                (bx + x, by + y) for x in range(112, 128) for y in range(40, 120)
            )
    if case == "rounded-ssd":
        unchanged.extend(
            (x, y)
            for x, y in sample_points(case)
            if inside(x, y, frame) and rounded_coverage(x, y, frame, 24) == 0
        )
        # Interior titlebar includes real glyphs/controls; no font-specific oracle.
        unchanged.extend(
            (x, y) for x in range(bx + 24, bx + 216) for y in range(frame[1] + 2, by)
        )
        LAYER.require(
            pixel(zero, bx + 120, frame[1] + 4) == (35, 40, 52), "SSD titlebar absent"
        )
    if case == "stacking":
        unchanged.extend((x, y) for x in range(308, 332) for y in range(190, 290))
    if case == "output-boundary":
        unchanged.extend((x, y) for x in range(320, 349) for y in range(8, 232, 3))
    if case == "output-boundary-odd":
        unchanged.extend((x, y) for x in range(292, 319) for y in range(8, 233, 3))
    for x, y in unchanged:
        a, b = pixel(zero, x, y), pixel(blurred, x, y)
        LAYER.require(
            max(abs(c - d) for c, d in zip(a, b)) <= 2,
            f"{case}: crisp/clear region changed at {x},{y}: {a} -> {b}",
        )
    return {
        "variance_zero": variances[0],
        **predicted,
        **(
            {"variance_12": variances[1]}
            if method == "gaussian" and radius == 12
            else {}
        ),
        "variance_blurred": variances[1],
        "variance_ratio": ratio,
        "mean_absolute_delta": mean_delta,
        "unchanged_pixels": len(unchanged),
    }


def drive(client, case):
    checks = []

    def expect(label, **kwargs):
        checks.append(client.expect(label, **kwargs))

    def layer(name, kind, w, h, color):
        client.command(f"layer {name} {kind} {w} {h} -1 0 {color}")
        expect(f"{name} configured", configured=(name,))
        client.command(f"map {name}")
        expect(f"{name} mapped", sizes={name: (w or 640, h)})

    if case == "stacking":
        layer("lower", 1, 80, 80, "1848e0")
    if window_case(case):
        mode = "server" if case == "rounded-ssd" else "client"
        # The stacking root is translucent, so the fixture's magenta geometry
        # margin would also be visible: square floating clients retain overflow.
        # Keep stacking geometry exact; the other XDG cases still exercise offsets.
        offset = 0 if case == "stacking" else 7
        client.command(f"app-xdg launcher 240 160 1 {offset} 20c0d0 {mode}")
        expect(
            "XDG content and decoration mode",
            sizes={"launcher": (240, 160)},
            fields={"launcher": {"xdg_mode": 2 if mode == "server" else 1}},
            focus="launcher",
        )
        name = "launcher"
    elif case == "popup":
        layer("panel", 2, 0, 16, "1848e0")
        client.command("popup popup panel 200 160 240 160 20c0d0")
        expect(
            "independent popup geometry",
            sizes={"popup": (240, 160)},
            fields={"popup": {"popup_x": 200, "popup_y": 160}},
        )
        name, offset = "popup", 0
    else:
        w, h = viewport(case)[2:] if case in BOUNDARIES else (240, 160)
        kind = 2 if case in BOUNDARIES else KINDS[case.removeprefix("layer-")]
        layer("surface", kind, w, h, "20c0d0")
        name, offset = "surface", 0
    client.command(
        f"alpha {name} {128 if case == 'stacking' or case in BOUNDARIES else 0}"
    )
    if case != "stacking" and case not in BOUNDARIES:
        for child, (x, y, w, h), color, alpha in PATCHES:
            rgb = "".join(f"{c:02x}" for c in color)
            client.command(
                f"subsurface {child} {name} {w} {h} {x + offset} {y + offset} {rgb}"
            )
            if alpha != 255:
                client.command(f"alpha {child} {alpha}")
        expect(
            "composed alpha tree and opaque glyphs",
            sizes={child: box[2:] for child, box, _, _ in PATCHES},
        )
    if case == "stacking":
        layer("upper", 3, 24, 100, "f02030")
    client.command("sync")
    expect("final mapped tree", fields={name: {"mapped": True, "configured": True}})
    return checks


def config_text(case, radius, method="gaussian", passes=3, liquid_glass=False):
    blur = "" if radius is None else f"blur_radius = {float(radius)}\n"
    # Leave Gaussian defaults implicit to retain coverage of the default method.
    if method != "gaussian" or passes != 3:
        blur += f'blur_method = "{method}"\nblur_passes = {passes}\n'
    # Layer/popup tests intentionally configure rounded windows to verify that
    # their own square surface trees do not acquire a window outline.
    corners = 0 if case in ("xdg", "stacking") or case in BOUNDARIES else 24
    text = f"""gaps = 0
[theme]
border_width = 0
corner_radius = {corners}
{blur}[shell]
launcher_app_ids = ["launcher"]
panels = []
[wallpaper]
path = "pattern.png"
mode = "stretch"
"""
    if case == "output-boundary":
        text += """[wallpaper.outputs.right]
path = "right.png"
mode = "stretch"
[[outputs]]
name = "left"
width = 320
height = 240
[[outputs]]
name = "right"
width = 320
height = 240
"""
    elif case == "output-boundary-odd":
        text += """[wallpaper.outputs.left]
path = "right.png"
mode = "stretch"
[[outputs]]
name = "left"
width = 319
height = 241
[[outputs]]
name = "right"
width = 321
height = 241
"""
    else:
        text += '[[outputs]]\nname = "test"\nwidth = 640\nheight = 480\n'
    if liquid_glass:
        text += "[theme.liquid_glass]\nenabled = true\n"
    return text


def focus_odd_output(runtime, name, clear):
    """Select the second viewport without input injection or fixture changes."""
    spec = importlib.util.spec_from_file_location(
        "blur_shell_smoke", Path(__file__).with_name("vm-shell-smoke.py")
    )
    assert spec is not None and spec.loader is not None
    shell = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(shell)
    address = runtime / f"clear-{name}" / "shell.sock"
    LAYER.SMOKE.wait_socket(address, clear)
    peer = shell.Peer(address)
    try:
        output = next(o for o in peer.state()["outputs"] if o["name"] == "right")
        peer.command("focus_output", output=output["id"])
    finally:
        peer.close()


def capture(args, fixture, directory, env, runtime, host_name, case, radius):
    directory.mkdir(parents=True, exist_ok=True)
    width, height = viewport(case)[2:]
    png(directory / "pattern.png", width, height, pattern)
    if case in BOUNDARIES:
        png(directory / "right.png", 640 - width, height, lambda x, y: RIGHT)
    config = directory / "config.toml"
    config.write_text(config_text(case, radius, args.method, args.passes, args.liquid_glass))
    frame = directory / "frame.ppm"
    frame.unlink(missing_ok=True)
    name = f"clear-blur-{os.getpid()}-{case}-{radius}"
    clear = client = None
    with (
        (directory / "clear.log").open("w") as log,
        (directory / "client.jsonl").open("w") as trace,
    ):
        try:
            started = time.monotonic()
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
            if case == "output-boundary-odd":
                focus_odd_output(runtime, name, clear)
            client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
            checks = drive(client, case)
            (directory / "protocol.json").write_text(
                json.dumps(checks, indent=2) + "\n"
            )
            LAYER.require(
                not frame.exists() and time.monotonic() - started < args.seconds - 1,
                "scenario too close to capture; increase --seconds (maximum 30)",
            )
            result = clear.wait(timeout=args.seconds + 5)
            LAYER.require(
                result == 0, f"Clear exited {result}; see {directory / 'clear.log'}"
            )
            log.flush()
            text = (directory / "clear.log").read_text()
            LAYER.require(
                "clear: stopped" in text and "panicked" not in text, "unclean shutdown"
            )
            LAYER.require(
                "clear: wallpaper" not in text.lower(),
                f"wallpaper load error; see {directory / 'clear.log'}",
            )
            return read_frame(frame, case)
        finally:
            try:
                if client is not None:
                    client.stop()
            finally:
                LAYER.SMOKE.stop(clear)


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    report = {
        "case": case,
        "method": args.method,
        "liquid_glass": args.liquid_glass,
        "radius": args.radius,
        "passes": args.passes,
        "viewport": viewport(case),
        "passed": False,
        "captures": {},
    }
    try:
        images = {}
        expected = None
        radii = (None, 0, args.radius) if case == "xdg" else (0, args.radius)
        for radius in dict.fromkeys(radii):
            label = "default" if radius is None else f"radius-{radius:g}"
            image = capture(
                args, fixture, directory / label, env, runtime, host_name, case, radius
            )
            expected = oracle(case, radius or 0, args.method, args.passes, args.liquid_glass)
            report["captures"][label] = check_frame(
                image, case, radius or 0, args.method, args.passes, expected
            )
            images[radius] = image
        if case == "xdg":
            LAYER.require(
                images[None] == images[0],
                "omitted blur_radius differs from explicit zero",
            )
            report["default_equals_zero"] = True
        assert expected is not None
        report.update(
            compare_pair(
                images[0],
                images[args.radius],
                case,
                args.radius,
                args.method,
                args.passes,
                expected,
            )
        )
        if args.liquid_glass and args.radius:
            plain = oracle(case, args.radius, args.method, args.passes)
            differences = [
                max(abs(a - b) for a, b in zip(expected(x, y), plain(x, y)))
                for x, y in sample_points(case)
                if expected(x, y) is not None and plain(x, y) is not None
            ]
            # Ensure the oracle grid would actually catch an omitted glass pass.
            report["glass_sensitive_pixels"] = sum(d > 4 for d in differences)
            LAYER.require(
                report["glass_sensitive_pixels"] > 20,
                "glass test does not distinguish ordinary blur",
            )
        report["passed"] = True
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (directory / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        f"PASS: {case}: {args.method} oracle, radius 0/{args.radius:g}, "
        f"passes={args.passes}, paired variance, crisp/clear regions",
        flush=True,
    )
    return report


def parse_args(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--method", choices=("gaussian", "kawase"), default="gaussian")
    parser.add_argument(
        "--radius",
        type=float,
        help="blur radius (0..32); default: 12 Gaussian, 2 Kawase",
    )
    parser.add_argument(
        "--passes",
        type=int,
        choices=range(1, 7),
        default=3,
        help="Kawase pyramid depth (default: 3); ignored by Gaussian",
    )
    parser.add_argument(
        "--liquid-glass", action="store_true",
        help="test default glass optics with either blur method",
    )
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-blur-smoke")
    parser.add_argument(
        "--seconds", type=int, default=8, help="seconds per capture (8..30)"
    )
    parser.add_argument("--layer-xml", type=Path)
    parser.add_argument("--kde-xml", type=Path)
    parser.add_argument(
        "--case",
        action="append",
        choices=CASES,
        help="repeat to select cases; default: all ten",
    )
    parser.add_argument(
        "--build-only",
        action="store_true",
        help="only compile the real C fixture; no GPU processes",
    )
    args = parser.parse_args(argv)
    if args.radius is None:
        args.radius = 2 if args.method == "kawase" else 12
    if not math.isfinite(args.radius) or not 0 <= args.radius <= 32:
        parser.error("--radius must be finite and between 0 and 32")
    if not 8 <= args.seconds <= 30:
        parser.error("--seconds must be between 8 and 30")
    return args


def main():
    args = parse_args()
    # File/dependency validation belongs to execution, not CPU/CLI self-tests.
    parser = argparse.ArgumentParser()
    if not args.build_only and not Path(args.binary).is_file():
        parser.error("build the Clear binary separately before running")
    artifacts = Path(args.artifacts).resolve()
    target = Path(__file__).resolve().parent.parent / "target"
    if not artifacts.is_relative_to(target.resolve()):
        parser.error("--artifacts must be under this checkout's target/ directory")
    artifacts.mkdir(parents=True, exist_ok=True)
    print(f"Artifacts: {artifacts}", flush=True)
    fixture = LAYER.build_fixture(
        artifacts / "fixture", args.layer_xml, kde_xml=args.kde_xml
    )
    if args.build_only:
        print(f"Fixture built: {fixture}; no GPU tests run", flush=True)
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
    host_name = f"clear-blur-host-{os.getpid()}"
    host = None
    reports = []
    selected = list(dict.fromkeys(args.case or CASES))
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
            for case in selected:
                reports.append(
                    run_case(args, fixture, artifacts, env, runtime, host_name, case)
                )
        finally:
            LAYER.SMOKE.stop(host)
            (artifacts / "results.json").write_text(
                json.dumps(
                    {
                        "method": args.method,
                        "liquid_glass": args.liquid_glass,
                        "radius": args.radius,
                        "passes": args.passes,
                        "selected": selected,
                        "passed": len(reports) == len(selected),
                        "cases": reports,
                        "note": "No physical input injection; final GPU captures and protocol state only.",
                    },
                    indent=2,
                )
                + "\n"
            )


if __name__ == "__main__":
    main()
