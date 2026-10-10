#!/usr/bin/env python3
"""Bounded animation GPU/protocol checks on a private virtual KWin host.

Slow linear effects make the single framebuffer capture fall inside a transition.
Real SHM clients and shell IPC drive final desktop policy independently of pixels.
No physical input, actual presentation cadence, or native DRM is tested.
"""
import argparse
import json
import struct
import zlib
import os
import subprocess
import time
from pathlib import Path
import importlib.util


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


BASE = load('animation_fullscreen', 'vm-fullscreen-smoke.py')
LAYER, SHELL = BASE.LAYER, BASE.SHELL
BLUR = load('animation_blur', 'vm-blur-smoke.py')
CASES = ('open', 'close', 'close-blur', 'minimize', 'restore', 'movement', 'maximize',
         'fullscreen', 'fullscreen-held', 'workspace', 'overview-enter', 'overview-exit')
EFFECT = dict(open='window_open', close='window_close', minimize='minimize',
              restore='minimize', movement='window_movement', maximize='maximize',
              fullscreen='fullscreen', workspace='workspace_switch',
              **{'overview-enter': 'overview', 'overview-exit': 'overview', 'close-blur': 'window_close', 'fullscreen-held': 'fullscreen'})


def configuration(case):
    effects = []
    for effect in dict.fromkeys(EFFECT.values()):
        enabled = effect == EFFECT[case]
        effects.append(f'''[animations.{effect}]
enabled = {str(enabled).lower()}
kind = "easing"
duration_ms = 2000
curve = "linear"
''')
    return '''gaps = 0
[theme]
border_width = 0
corner_radius = 16
background = [0.0, 0.0, 0.0, 1.0]
[animations]
enabled = true
speed = 0.2
frame_rate = 30
[[outputs]]
name = "test"
width = 1024
height = 768
[[workspaces]]
id = 1
name = "main"
mode = "floating"
[[workspaces]]
id = 2
name = "next"
mode = "floating"
''' + '\n'.join(effects)


def green_bounds(frame, largest_component=False):
    width, height, pixels = LAYER.read_ppm(frame)
    assert (width, height) == (1024, 768), (width, height)
    xs, ys, maximum = [], [], 0
    for y in range(height):
        for x in range(width):
            r, g, b = pixels[(y * width + x) * 3:(y * width + x) * 3 + 3]
            if g > 12 and g > r * 2 and g > b * 1.7:
                xs.append(x)
                ys.append(y)
                maximum = max(maximum, g)
    assert xs, 'No retained/live green window in captured transition'
    if largest_component:
        active = bytearray(width * height)
        for x, y in zip(xs, ys): active[y * width + x] = 1
        components = []
        for seed in range(width * height):
            if not active[seed]: continue
            active[seed] = 0
            pending = [seed]
            count, left, top, right, bottom, green = 0, width, height, 0, 0, 0
            while pending:
                pixel = pending.pop()
                x, y = pixel % width, pixel // width
                count += 1
                left, top, right, bottom = min(left, x), min(top, y), max(right, x), max(bottom, y)
                green = max(green, pixels[pixel * 3 + 1])
                for adjacent in (pixel - 1 if x else -1, pixel + 1 if x + 1 < width else -1,
                                 pixel - width if y else -1, pixel + width if y + 1 < height else -1):
                    if adjacent >= 0 and active[adjacent]:
                        active[adjacent] = 0
                        pending.append(adjacent)
            components.append((count, left, top, right, bottom, green))
        _, left, top, right, bottom, maximum = max(components)
        return dict(left=left, top=top, right=right, bottom=bottom,
                    width=right-left+1, height=bottom-top+1, green=maximum,
                    components=len(components))
    return dict(left=min(xs), top=min(ys), right=max(xs), bottom=max(ys),
                width=max(xs)-min(xs)+1, height=max(ys)-min(ys)+1, green=maximum)


def verify(case, frame):
    if case == 'close-blur':
        width, height, pixels = LAYER.read_ppm(frame)
        assert (width, height) == (1024, 768)
        levels = [[], []]
        for y in range(350, 400):
            for x in range(480, 544):
                actual = tuple(pixels[(y * width + x)*3:(y * width + x)*3+3])
                assert max(actual) - min(actual) <= 1, actual
                levels[(x//4+y//4) % 2].append(actual[0])
        low, high = (sum(v)/len(v) for v in levels)
        # A disappearing fully transparent rounded frame must reveal sharp checks
        # progressively. Full geometric blur has almost no contrast; no blur has 192.
        assert 45 < low < 105 and 151 < high < 211, (low, high)
        assert 70 < high-low < 170, (low, high)
        return dict(low=low, high=high, contrast=high-low)
    b = green_bounds(frame, largest_component=case.startswith("overview"))
    if case == 'fullscreen-held':
        assert b['left'] == 0 and b['right'] == 1023 and 30 <= b['top'] <= 50, b
        assert 720 <= b['height'] <= 738 and b['bottom'] == 767, b
        assert b['green'] == 176, b
    elif case in ('open', 'close'):
        assert 800 < b['width'] < 850 and 600 < b['height'] < 640, b
        assert 15 < b['green'] < 170, b
    elif case in ('minimize', 'restore'):
        assert 80 < b['width'] < 760 and 60 < b['height'] < 570, b
        assert 15 < b['green'] < 170, b
        assert (b['top'] + b['bottom']) / 2 > 390, b
    elif case in ('maximize', 'fullscreen'):
        assert 805 < b['width'] < 1018 and 605 < b['height'] < 762, b
        assert b['green'] == 176, b
    elif case == 'movement':
        assert 805 < b['width'] < 1018 and 605 < b['height'] < 762, b
    elif case == 'workspace':
        assert b['left'] == 0 and 0 < b['right'] < 780, b
    elif case.startswith('overview'):
        assert b['width'] != 800 and b['height'] != 600, b
        assert 40 < b['width'] < 800 and 40 < b['height'] < 600, b
        b['wallpaper'] = overview_wallpaper_bounds(frame)
    return b


def overview_wallpaper(path):
    """Portable RGB PNG with a full canvas color and source-coordinate stripe."""
    width, height = 1024, 768
    row = b"\x00" + b"".join(bytes((220, 20, 40) if 256 <= x < 276 else (70, 0, 90)) for x in range(width))
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack("!2I5B", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(row * height)) + chunk(b"IEND", b""))


def overview_wallpaper_bounds(frame):
    width, height, pixels = LAYER.read_ppm(frame)
    positions = [(pixel % width, pixel // width) for pixel in range(width * height)
                 if abs(pixels[pixel*3] - 70) <= 1 and pixels[pixel*3+1] <= 1
                 and abs(pixels[pixel*3+2] - 90) <= 1]
    assert positions, "No undimmed moving overview wallpaper canvas"
    left, right = min(p[0] for p in positions), max(p[0] for p in positions)
    top, bottom = min(p[1] for p in positions), max(p[1] for p in positions)
    canvas_width = right - left + 1
    # Neither full-output nor settled inset geometry may describe this capture.
    assert 0 < left < 71 and 0 < top < 107, (left, top, right, bottom)
    progress = left / 71
    assert abs(top - 107 * progress) <= 2, (left, top, progress)
    assert abs((width - 1 - right) - left) <= 1, (left, right)
    # The workspace strip can overlap the top of the expanding canvas on exit.
    # Its lower edge remains free of cards, captions and strip overlays.
    row = bottom - 2
    stripe = [x for x in range(left, right + 1)
              if pixels[(row*width+x)*3] >= 215 and pixels[(row*width+x)*3+1] <= 22
              and pixels[(row*width+x)*3+2] <= 42]
    assert stripe, "Source wallpaper stripe missing from scaled canvas"
    expected_left = left + 256 * canvas_width / width
    expected_right = left + 276 * canvas_width / width
    assert abs(min(stripe) - expected_left) <= 2, (stripe, expected_left)
    assert abs(max(stripe) + 1 - expected_right) <= 2, (stripe, expected_right)
    return dict(left=left, top=top, right=right, bottom=bottom,
                stripe_left=min(stripe), stripe_right=max(stripe), approximate_progress=progress)


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / 'config.toml'
    config_text = configuration(case)
    if case.startswith('overview'):
        overview_wallpaper(directory / 'overview.png')
        config_text += '\n[wallpaper]\npath="overview.png"\nmode="fill"\n'
    if case == 'close-blur':
        BLUR.png(directory / 'checks.png', 1024, 768, BLUR.pattern)
        config_text = config_text.replace('border_width = 0', 'border_width = 0\nblur_radius = 2\nblur_method = "kawase"\nblur_passes = 3')
        config_text += '\n[wallpaper]\npath="checks.png"\nmode="stretch"\n'
    config.write_text(config_text)
    frame = directory / 'frame.ppm'
    frame.unlink(missing_ok=True)
    name = f'clear-animation-{os.getpid()}-{case}'
    seconds = 19 if case in ('restore', 'overview-exit') else (15 if case == 'fullscreen-held' else 8)
    clear = client = peer = None
    checks = []
    with (directory / 'clear.log').open('w') as log, (directory / 'client.jsonl').open('w') as trace:
        try:
            started = time.monotonic()
            clear = subprocess.Popen([str(Path(args.binary).resolve()), '--config', str(config),
                                      '--socket', name, '--exit-after', str(seconds),
                                      '--capture', str(frame)],
                                     env=dict(env, WAYLAND_DISPLAY=host_name), stdout=log,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            LAYER.SMOKE.wait_socket(runtime / f'clear-{name}' / 'shell.sock', clear)
            peer = SHELL.Peer(runtime / f'clear-{name}' / 'shell.sock')
            client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
            if case == 'fullscreen-held':
                client.command('app-xdg app 800 600 0 0 30b050 server')
                checks.append(client.expect('initial SSD float', sizes={'app': (800, 568)}, focus='app'))
            else:
                client.command('app app 800 600 0 0 30b050')
                checks.append(client.expect('initial float', sizes={'app': (800, 600)}, focus='app'))
            identity = BASE.window(peer, 'app')['id']
            time.sleep(0.8)  # Let the initial committed source be submitted before destruction.
            if case == 'close-blur':
                client.command('alpha app 0')
                time.sleep(0.8)
            if case in ('close', 'close-blur'):
                client.command('destroy app')
                SHELL.wait_for(lambda: not peer.state()['windows'], 'destroyed policy')
            elif case in ('minimize', 'restore'):
                peer.command('set_minimized', window=identity, minimized=True)
                BASE.policy(peer, 'app', minimized=True)
                if case == 'restore':
                    time.sleep(10.5)
                    peer.command('focus_window', window=identity)
                    BASE.policy(peer, 'app', minimized=False)
            elif case == 'movement':
                peer.command('set_mode', output='1', mode='columns')
                checks.append(client.expect('final tiled configure', sizes={'app': (1024, 768)}))
            elif case == 'fullscreen-held':
                client.command('hold app')
                peer.command('set_fullscreen', window=identity, fullscreen=True)
                BASE.policy(peer, 'app', fullscreen=True)
                checks.append(client.expect('ACKed held full configure', sizes={'app': (800, 568)},
                                            fields={'app': {'fullscreen': True, 'committed_fullscreen': False,
                                                            'configure_width': 1024, 'configure_height': 768,
                                                            'hold_commit': True}}))
            elif case in ('maximize', 'fullscreen'):
                peer.command('set_' + ('maximized' if case == 'maximize' else 'fullscreen'),
                             window=identity, **{('maximized' if case == 'maximize' else 'fullscreen'): True})
                BASE.policy(peer, 'app', **{('maximized' if case == 'maximize' else 'fullscreen'): True})
                checks.append(client.expect('final full configure', sizes={'app': (1024, 768)}))
            elif case == 'workspace':
                peer.command('switch_workspace', output='1', workspace='2')
            elif case.startswith('overview'):
                peer.command('toggle_overview')
                if case == 'overview-exit':
                    time.sleep(10.5)
                    peer.command('toggle_overview')
            checks.append({'policy': peer.state(), 'action_complete_seconds': time.monotonic()-started})
            assert not frame.exists(), 'Action missed capture deadline'
            assert clear.wait(timeout=seconds + 5) == 0
            pixels = verify(case, frame)
            log.flush()
            text = (directory / 'clear.log').read_text()
            assert 'clear: stopped' in text and 'panicked' not in text, text
        finally:
            if peer: peer.close()
            if client: client.stop()
            LAYER.SMOKE.stop(clear)
    (directory / 'result.json').write_text(json.dumps(dict(case=case, passed=True,
                                                           checks=checks, pixels=pixels), indent=2) + '\n')
    print(f'PASS: {case}: final policy and intermediate GPU geometry/opacity', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/clear')
    parser.add_argument('--artifacts', default='target/vm-animation-smoke')
    parser.add_argument('--layer-xml', type=Path)
    parser.add_argument('--case', action='append', choices=CASES)
    args = parser.parse_args()
    if not Path(args.binary).is_file(): parser.error('build the binary separately')
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    fixture = LAYER.build_fixture(artifacts / 'fixture', args.layer_xml)
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}'))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), QT_SCALE_FACTOR='1', RUST_BACKTRACE='1')
    for key in ('WAYLAND_SOCKET', 'WAYLAND_DISPLAY', 'DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'CLEAR_SOCKET'):
        env.pop(key, None)
    host_name = f'clear-animation-host-{os.getpid()}'
    host = None
    with (artifacts / 'kwin.log').open('w') as log:
        try:
            host = subprocess.Popen(['dbus-run-session', '--', 'kwin_wayland', '--virtual',
                                     '--width', '1200', '--height', '900', '--no-lockscreen',
                                     '--no-global-shortcuts', '--no-kactivities', '--socket', host_name],
                                    env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            LAYER.SMOKE.wait_socket(runtime / host_name, host)
            for case in args.case or CASES:
                run_case(args, fixture, artifacts, env, runtime, host_name, case)
        finally:
            LAYER.SMOKE.stop(host)
    print(f'Artifacts: {artifacts}')


if __name__ == '__main__': main()
