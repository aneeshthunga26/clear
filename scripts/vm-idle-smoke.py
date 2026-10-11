#!/usr/bin/env python3
"""Bounded demand-driven rendering checks on a private virtual KWin host.

Checks real callbacks, idle submission counters, IPC wakeups and captured pixels.
Build the Rust binary separately. No physical input or presentation timing oracle.
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
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


BASE = load('idle_fullscreen', 'vm-fullscreen-smoke.py')
LAYER, SHELL = BASE.LAYER, BASE.SHELL
CASES = ('empty', 'static', 'callbacks', 'animated', 'hud', 'ipc',
         'subsurface', 'subsurface-sync', 'popup-destroy', 'layer-unmap',
         'disconnect', 'overview', 'animation-after-idle', 'partial-damage')
CONFIG = '''gaps = 0
[theme]
border_width = 0
background = [0.0, 0.0, 0.0, 1.0]
[[outputs]]
name = "test"
width = 960
height = 720
[[workspaces]]
id = 1
name = "main"
mode = "floating"
[[workspaces]]
id = 2
name = "next"
mode = "floating"
'''


def stats_from_log(text):
    lines = [line.removeprefix('clear: render stats ') for line in text.splitlines()
             if line.startswith('clear: render stats ')]
    assert len(lines) == 1, f'Expected one shutdown render summary, found {len(lines)}'
    return json.loads(lines[0])


def verify(case, stats, frame, callback_frames):
    # These are generous settlement bounds, far below six seconds of continuous
    # 60 Hz submissions. Exact initial exposure counts depend on the host.
    upper = 16 if case in ('empty', 'static', 'callbacks', 'hud') else 40
    if case == 'animated':
        assert callback_frames >= 20, callback_frames
        assert 20 <= stats['submissions'] < 180, stats
    elif case == 'animation-after-idle':
        assert 10 <= stats['submissions'] < 180, stats
    elif case == 'overview':
        assert stats['submissions'] < 220, stats
    else:
        assert stats['submissions'] <= upper, stats
    if case == 'callbacks':
        assert callback_frames >= 30 and stats['callback_only'] >= 30, (callback_frames, stats)
    if case in ('empty', 'static', 'hud'):
        assert stats['loop_turns'] < 100, stats
    if case == 'hud':
        assert 1 <= stats['diagnostic_frames'] <= 3, stats
    if case not in ('empty', 'static', 'callbacks', 'hud') and 'last_scene_submission_ms' in stats:
        # A deadline capture alone would conceal a missing visual wakeup in the
        # final pixels. Require real scene work after the late command, then quiet
        # settlement well before the capture/exit deadline.
        assert 1000 <= stats['last_scene_submission_ms'] < 6000, stats
    width, height, pixels = LAYER.read_ppm(frame)
    assert (width, height) == (960, 720), (width, height)
    def pixel(x, y):
        return tuple(pixels[(y * width + x) * 3:(y * width + x) * 3 + 3])
    expected = ((0, 0, 0) if case in ('empty', 'ipc', 'disconnect') else
                (16, 56, 104) if case == 'partial-damage' else LAYER.COLORS['app'])
    if case not in ('overview', 'animated', 'animation-after-idle'):
        assert pixel(480, 360) == expected, (case, pixel(480, 360), expected)
    if case in ('subsurface', 'subsurface-sync'):
        assert pixel(170, 130) == (112, 224, 144), pixel(170, 130)
    return dict(center=pixel(480, 360))


def run_case(args, fixture, artifacts, env, runtime, host_name, case):
    directory = artifacts / case
    directory.mkdir(parents=True, exist_ok=True)
    config = directory / 'config.toml'
    text = ('show_fps = true\n' if case == 'hud' else '') + CONFIG
    if case in ('overview', 'animation-after-idle'):
        text += '\n[animations]\nenabled = true\nspeed = 1.0\n'
    config.write_text(text)
    frame = directory / 'frame.ppm'
    frame.unlink(missing_ok=True)
    name = f'clear-idle-{os.getpid()}-{case}'
    seconds = 8
    clear = client = peer = None
    callback_frames = 0
    partial_frame = directory / 'partial.ppm'
    partial_frame.unlink(missing_ok=True)
    recovery_frame = partial_frame.with_suffix('.uncertain.ppm')
    recovery_frame.unlink(missing_ok=True)
    with (directory / 'clear.log').open('w') as log, (directory / 'client.jsonl').open('w') as trace:
        try:
            clear_env = dict(env, WAYLAND_DISPLAY=host_name, CLEAR_RENDER_STATS='1')
            if case == 'partial-damage':
                clear_env['CLEAR_PARTIAL_FRAME_CAPTURE'] = str(partial_frame)
            clear = subprocess.Popen([str(Path(args.binary).resolve()), '--config', str(config),
                                      '--socket', name, '--exit-after', str(seconds),
                                      '--capture', str(frame)],
                                     env=clear_env,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            LAYER.SMOKE.wait_socket(runtime / f'clear-{name}' / 'shell.sock', clear)
            peer = SHELL.Peer(runtime / f'clear-{name}' / 'shell.sock')
            if case != 'empty':
                client = LAYER.Client(fixture, dict(env, WAYLAND_DISPLAY=name), trace)
                client.command('app app 800 600 0 0 30b050')
                client.expect('mapped static float', sizes={'app': (800, 600)}, focus='app')
            time.sleep(1.2)
            if case in ('callbacks', 'animated'):
                client.command(('callbacks' if case == 'callbacks' else 'animate') + ' app')
                time.sleep(1.2)
                callback_frames = client.command('state')['views']['app']['frame_count']
                client.command('stop-frames app')
            elif case == 'ipc':
                identity = BASE.window(peer, 'app')['id']
                peer.command('switch_workspace', output='1', workspace='2')
                assert BASE.window(peer, 'app')['id'] == identity
            elif case in ('subsurface', 'subsurface-sync'):
                client.command(f'{case} child app 80 60 80 60 e04030')
                client.command('alpha child 255')
                # Change pixels without involving the parent's configure lifecycle.
                client.command('destroy child')
                client.command(f'{case} child2 app 80 60 80 60 70e090')
                if case == 'subsurface-sync':
                    client.command('empty-commit app')
            elif case == 'popup-destroy':
                client.command('popup menu app 80 60 80 60 d040c0')
                client.expect('popup mapped', sizes={'menu': (80, 60)})
                time.sleep(0.3)
                client.command('destroy menu')
            elif case == 'layer-unmap':
                client.command('layer panel 2 0 32 32 0 e04030')
                client.expect('panel configured', configured=('panel',))
                client.command('map panel')
                client.expect('panel mapped', sizes={'panel': (960, 32)})
                client.command('unmap panel')
            elif case == 'disconnect':
                client.stop()
                client = None
                SHELL.wait_for(lambda: not peer.state()['windows'], 'disconnected policy')
            elif case == 'overview':
                peer.command('toggle_overview')
                time.sleep(1.0)
                peer.command('toggle_overview')
            elif case == 'animation-after-idle':
                peer.command('set_maximized', window=BASE.window(peer, 'app')['id'], maximized=True)
                client.expect('maximized after idle', sizes={'app': (960, 720)})
            elif case == 'partial-damage':
                def partial_matches(color, path=partial_frame):
                    if not path.exists():
                        return False
                    width, height, pixels = LAYER.read_ppm(path)
                    if (width, height) != (960, 720):
                        return False
                    return tuple(pixels[(360 * width + 480) * 3:(360 * width + 480) * 3 + 3]) == color
                client.command('color app 2070d0')
                SHELL.wait_for(lambda: partial_matches((32, 112, 208)), 'blue partial draw before forced capture')
                time.sleep(0.3)
                client.command('alpha app 128')
                SHELL.wait_for(lambda: partial_matches((16, 56, 104)), 'transparent partial draw before forced capture')
                # Unknown replacement contents without damage must recover fully,
                # rather than silently acknowledging an empty tracker result.
                client.command('color-undamaged app 30b050')
                SHELL.wait_for(lambda: partial_matches((24, 88, 40), recovery_frame),
                               'undamaged replacement recovery before forced capture')
                client.command('color app 2070d0')
                SHELL.wait_for(lambda: partial_matches((16, 56, 104)), 'partial draw after undamaged recovery')
            assert clear.wait(timeout=seconds + 5) == 0
            log.flush()
            output = (directory / 'clear.log').read_text()
            assert 'clear: stopped' in output and 'panicked' not in output, output
            stats = stats_from_log(output)
            if case == 'partial-damage':
                assert stats['partial_captures'] >= 2 and stats['partial_frames'] >= 2, stats
                assert stats['uncertain_frames'] >= 1 and stats['last_uncertain_submission_ms'] >= 1000, stats
            pixels = verify(case, stats, frame, callback_frames)
        finally:
            if peer:
                peer.close()
            if client:
                client.stop()
            LAYER.SMOKE.stop(clear)
    (directory / 'result.json').write_text(json.dumps(dict(case=case, passed=True,
        stats=stats, callback_frames=callback_frames, pixels=pixels), indent=2) + '\n')
    print(f'PASS: {case}: {stats}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/clear')
    parser.add_argument('--artifacts', default='target/vm-idle-smoke')
    parser.add_argument('--layer-xml', type=Path)
    parser.add_argument('--case', action='append', choices=CASES)
    args = parser.parse_args()
    if not Path(args.binary).is_file():
        parser.error('build the binary separately')
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    fixture = LAYER.build_fixture(artifacts / 'fixture', args.layer_xml)
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}'))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), QT_SCALE_FACTOR='1', RUST_BACKTRACE='1')
    for key in ('WAYLAND_SOCKET', 'WAYLAND_DISPLAY', 'DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'CLEAR_SOCKET'):
        env.pop(key, None)
    host_name = f'clear-idle-host-{os.getpid()}'
    host = None
    with (artifacts / 'kwin.log').open('w') as log:
        try:
            host = subprocess.Popen(['dbus-run-session', '--', 'kwin_wayland', '--virtual',
                '--width', '1000', '--height', '800', '--no-lockscreen',
                '--no-global-shortcuts', '--no-kactivities', '--socket', host_name],
                env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            LAYER.SMOKE.wait_socket(runtime / host_name, host)
            for case in args.case or CASES:
                run_case(args, fixture, artifacts, env, runtime, host_name, case)
        finally:
            LAYER.SMOKE.stop(host)
    print(f'Artifacts: {artifacts}')


if __name__ == '__main__':
    main()
