#!/usr/bin/env python3
"""Bounded Quickshell UI captures and hover regression on a private virtual KWin.

Run in an isolated environment. Fixture timers call QML hover entry/exit handlers;
no physical input is injected. Build Clear separately. Popup configure coordinates
come from the real Wayland trace; captures check panel separation and UI presence.
"""

import argparse
import importlib.util
import json
import os
import re
import shutil
import subprocess
import struct
import zlib
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("shell_smoke", Path(__file__).with_name("vm-shell-smoke.py"))
assert SPEC and SPEC.loader
SHELL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SHELL)
SMOKE = SHELL.SMOKE
SCENES = ("panel", "preview", "tooltip", "launcher", "launcher-empty", "notifications", "context-menu")


def backdrop_pixel(x, y):
    if (x + y) % 97 < 4:
        return (240, 230, 190)
    base = (210, 72, 145) if (x // 96 + y // 96) % 2 else (20, 175, 165)
    ramp = ((x % 512) // 2, (y % 512) // 2, ((x + y) % 512) // 2)
    return tuple((3 * channel + gradient) // 4 for channel, gradient in zip(base, ramp))


def write_backdrop(path, width, height):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\x00" + bytes(c for x in range(width) for c in backdrop_pixel(x, y)) for y in range(height))
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def fixture(source, directory, scene, glass):
    shutil.copytree(source, directory)
    # Instrument a copy so the real example and private delegate geometry are used.
    dock = directory / "AppDock.qml"
    text = dock.read_text()
    if scene == "preview":
        timers = '''
    Timer {
        interval: 700; running: true
        onTriggered: dock.queuePreview(appRepeater.itemAt(0).modelData.id, appRepeater.itemAt(0))
    }
    Timer {
        interval: 850; running: true
        onTriggered: {
            if (dock.hoveredApp !== "") console.error("UI FAIL: preview opened before dwell");
            dock.leavePreview();
        }
    }
    Timer {
        interval: 1350; running: true
        onTriggered: {
            if (dock.hoveredApp !== "") console.error("UI FAIL: cancelled hover opened preview");
            dock.queuePreview(appRepeater.itemAt(0).modelData.id, appRepeater.itemAt(0));
        }
    }
    Timer {
        interval: 2150; running: true
        onTriggered: {
            if (dock.hoveredApp !== "") console.error("UI FAIL: preview opened before one-second dwell");
        }
    }
    Timer {
        interval: 2800; running: true
        onTriggered: {
            const popup = previewLoader.item;
            if (!popup || !popup.visible) { console.error("UI FAIL: dwell did not open preview"); return; }
            console.info("UI popup " + JSON.stringify({x: popup.anchor.rect.x, y: popup.anchor.rect.y,
                width: popup.width, height: popup.height, panelHeight: dock.panelWindow.height}));
            console.info("UI READY");
        }
    }
'''
        text = text.replace('    id: dock\n', '    id: dock\n' + timers, 1)
        text = text.replace('    id: dock\n', '''    id: dock
    Timer {
        interval: 3600; running: true
        onTriggered: {
            dock.shellBridge.command({type: "set_minimized", window: dock.hoveredGroup.windows[0].id, minimized: true});
            console.info("UI MINIMIZED");
        }
    }
''', 1)
        app_marker = '                id: appButton\n'
        assert app_marker in text
        text = text.replace(app_marker, app_marker + '\n                Component.onCompleted: console.info("UI dock button created " + appId)\n', 1)
        card_marker = '                        id: windowCard\n'
        assert card_marker in text
        text = text.replace(card_marker, card_marker + '\n                        Component.onCompleted: console.info("UI card created " + modelData.id)\n', 1)
        live = directory / "LivePreview.qml"
        live_text = live.read_text().replace('    id: preview\n', '    id: preview\n    property bool fixtureHadFrame: false\n    onHasFrameChanged: {\n        if (fixtureHadFrame && !hasFrame) console.error("UI FAIL: displayed preview reverted to icon");\n        if (hasFrame) fixtureHadFrame = true;\n    }\n    onDisplayedSourceChanged: console.info("UI displayed frame " + displayedSource.length)\n', 1)
        live.write_text(live_text)
    elif scene == "context-menu":
        text = text.replace('    function showContextMenu(id, button) {',
            '    function showContextMenu(id, button) {\n        const fixtureStart = Date.now();', 1)
        marker = '            appearance.panelTopMargin + panelWindow.height + appearance.popupGap);'
        assert marker in text
        text = text.replace(marker, marker + '\n        console.info("UI menu dispatch ms " + (Date.now() - fixtureStart));', 1)
        text = text.replace('    id: dock\n', '''    id: dock
    Timer {
        interval: 700; running: true
        onTriggered: {
            if (!contextLoader.item) console.error("UI FAIL: context menu was not prepared before right-click");
        }
    }
    Timer {
        interval: 800; running: true
        onTriggered: dock.showContextMenu(appRepeater.itemAt(0).modelData.id, appRepeater.itemAt(0))
    }
    Timer {
        interval: 1100; running: true
        onTriggered: {
            const entry = contextLoader.item.entry;
            if (!entry || !entry.actions.length) console.error("UI FAIL: published desktop action missing");
            else entry.actions[0].execute();
            dock.shellBridge.command({type: "move_window_to_workspace", window: contextLoader.item.window.id, workspace: "2"});
        }
    }
    Timer {
        interval: 1600; running: true
        onTriggered: {
            if (!contextLoader.item || !contextLoader.item.opened)
                console.error("UI FAIL: context menu did not open");
            contextLoader.item.showWorkspaces = true;
            console.info("UI READY");
        }
    }
    Timer {
        interval: 3600; running: true
        onTriggered: {
            if (!contextLoader.item.entry || !contextLoader.item.entry.actions.length)
                console.error("UI FAIL: desktop action vanished after index refresh");
            if (!contextLoader.item.fixtureOwnsKeyboard())
                console.error("UI FAIL: painted menu body does not own keyboard focus");
            const box = contextLoader.item.bodyGeometry;
            console.info("UI menu bounds " + JSON.stringify({x: box.x, y: box.y, width: box.width, height: box.height}));
        }
    }
    Timer {
        interval: 1900; running: true
        onTriggered: contextLoader.item.dismiss()
    }
    Timer {
        interval: 2300; running: true
        onTriggered: {
            const menu = contextLoader.item;
            if (menu.opened || menu.width !== 1 || menu.height !== 1
                    || menu.bodyGeometry.width !== 1 || menu.bodyGeometry.height !== 1)
                console.error("UI FAIL: closed context menu retained full-output geometry");
            menu.open(menu.appId, menu.anchorX, menu.anchorY);
            menu.showWorkspaces = true;
        }
    }
''', 1)
    elif scene == "tooltip":
        text = text.replace('    id: dock\n', '''    id: dock
    ShellToolTip {
        id: fixtureTip
        appearance: dock.appearance
        target: dock
        belowPanel: true
        text: "Application windows"
    }
    Timer {
        interval: 700; running: true
        onTriggered: {
            // Expose the patterned wallpaper instead of a uniform opaque titlebar.
            dock.shellBridge.command({type: "set_minimized", window: dock.groups[0].windows[0].id, minimized: true});
            fixtureTip.hovered = true;
        }
    }
    Timer {
        interval: 2100; running: true
        onTriggered: {
            if (!fixtureTip.ready) console.error("UI FAIL: tooltip dwell did not complete");
            console.info("UI READY");
        }
    }
''', 1)
    text = text.replace("running: true", "running: dock.shellBridge.online")
    dock.write_text(text)
    if scene == "context-menu":
        menu = directory / "AppContextMenu.qml"
        menu_text = menu.read_text()
        menu_text = menu_text.replace("    function dismiss()", "    function fixtureOwnsKeyboard() { return content.activeFocus; }\n    function dismiss()", 1)
        marker = '        opened = true;'
        assert marker in menu_text
        menu_text = menu_text.replace(marker,
            '        console.info("UI menu requested size " + JSON.stringify([menuWidth, menuHeight]));\n' + marker, 1)
        menu.write_text(menu_text)
    assembly = directory / "DesktopShell.qml"
    text = assembly.read_text()
    marker = "            RowLayout {\n"
    assert marker in text
    text = text.replace(marker, marker + "                id: fixtureLayout\n", 1)
    if scene in ("launcher", "launcher-empty"):
        # Open, close and reopen the retained layer role before the final capture.
        action = '''launcherLoader.active = true; launcherLoader.item.open();'''
        final = '''launcherLoader.item.open();'''
        if scene == "launcher-empty":
            launcher = directory / "AppLauncher.qml"
            launcher.write_text(launcher.read_text().replace('    function dismiss()',
                '    function fixtureQuery() { search.text = "clear-no-such-application-7319"; }\n    function dismiss()', 1))
            final += '''launcherLoader.item.fixtureQuery();'''
        timers = f'''
            Timer {{ interval: 700; running: true; onTriggered: {{ {action} }} }}
            Timer {{ interval: 1200; running: true; onTriggered: launcherLoader.item.dismiss() }}
            Timer {{ interval: 1800; running: true; onTriggered: {{ {final} console.info("UI READY"); }} }}
'''
    elif scene == "notifications":
        timers = '''
            Timer {
                interval: 700; running: true
                onTriggered: {
                    notificationsLoader.active = true;
                    notificationsLoader.item.toggle();
                    console.info("UI READY");
                }
            }
'''
    else:
        timers = '' if scene in ("preview", "tooltip", "context-menu") else '            Timer { interval: 1000; running: true; onTriggered: console.info("UI READY") }\n'
    if scene == "panel":
        timers += '''
            Timer {
                interval: 700; running: panel.active
                onTriggered: bridge.command({type: "switch_workspace", output: panel.output.id,
                    workspace: bridge.state.workspaces[bridge.state.workspaces.length - 1].id})
            }
            Timer {
                interval: 1700; running: panel.active
                onTriggered: {
                    const selected = workspaceRow.children.find(child => child.selected === true);
                    if (!selected || selected.x < workspaceViewport.contentX - 1
                            || selected.x + selected.width > workspaceViewport.contentX + workspaceViewport.width + 1)
                        console.error("UI FAIL: selected workspace scrolled out of view");
                }
            }
'''
    timers += '''
            Timer {
                interval: 2400; running: panel.active
                onTriggered: {
                    for (const child of fixtureLayout.children) {
                        if (child.visible && child.width > 0
                                && (child.x < -1 || child.x + child.width > fixtureLayout.width + 1))
                            console.error("UI FAIL: panel control outside layout bounds");
                    }
                }
            }
'''
    marker = "            screen: modelData\n"
    assert marker in text
    assembly.write_text(text.replace(marker, timers.replace("running: true", "running: panel.active") + marker, 1))
    bridge = directory / "ClearBridge.qml"
    bridge_text = bridge.read_text()
    bridge_text = bridge_text.replace("            var message = Protocol.decodeMessage(line);", '''            var message = Protocol.decodeMessage(line);
            if (message.type === "window_previews") {
                let hash = 0;
                const images = message.frames.map(frame => frame.image || "").join("");
                for (let i = 0; i < images.length; ++i) hash = (hash * 31 + images.charCodeAt(i)) | 0;
                console.info("UI capture " + hash);
            }''', 1)
    if scene == "preview":
        bridge_text = bridge_text.replace('    id: bridge\n', '    id: bridge\n    property int fixturePreviewReplies: 0\n', 1)
        marker = '            var message = Protocol.decodeMessage(line);'
        bridge_text = bridge_text.replace(marker, marker + '''
            if (message.type === "window_previews") {
                ++fixturePreviewReplies;
                if (fixturePreviewReplies % 4 === 2) message.frames.forEach(frame => frame.image = null);
                if (fixturePreviewReplies % 4 === 3) message.frames.forEach(frame => frame.image = "data:image/png;base64,aW52YWxpZA==");
            }
''', 1)
    bridge.write_text(bridge_text)
    return directory / ("liquid-glass.qml" if glass else "shell.qml")


def run_scene(args, scene, root, env, runtime, host_name):
    directory = root / scene
    directory.mkdir(exist_ok=True)
    qml = fixture(Path(__file__).resolve().parents[1] / "examples/quickshell", directory / f"fixture-{os.getpid()}", scene, args.glass)
    config = directory / "config.toml"
    config.write_text(f'[[outputs]]\nname="ui"\nwidth={args.width}\nheight=720\n')
    if args.backdrop is not None:
        wallpaper = directory / "pattern.png"
        write_backdrop(wallpaper, args.width, 720)
        theme = '\n[wallpaper]\npath=' + json.dumps(str(wallpaper)) + '\nmode="fill"\n'
        theme += '\n[theme]\nblur_method="kawase"\nblur_radius=' + ('2' if args.backdrop in ('blur', 'both') else '0') + '\nblur_passes=3\n'
        if args.backdrop in ('glass', 'both'):
            theme += '\n[theme.liquid_glass]\nenabled=true\nrefraction_width=10\nzoom_level=1.5\n'
        config.write_text(config.read_text() + theme)

    clear_name = f"clear-ui-{os.getpid()}-{scene}"
    address = runtime / f"clear-{clear_name}" / "shell.sock"
    capture = directory / "frame.ppm"
    capture.unlink(missing_ok=True)
    data_home = directory / "data"
    applications = data_home / "applications"
    applications.mkdir(parents=True, exist_ok=True)
    action_marker = directory / "desktop-action-invoked"
    action_marker.unlink(missing_ok=True)
    if scene == "context-menu":
        (applications / "foot.desktop").write_text(
            '[Desktop Entry]\nType=Application\nName=Preview Fixture\nExec=foot\nIcon=utilities-terminal\nStartupWMClass=foot\nActions=Inspect;\n'
            f'[Desktop Action Inspect]\nName=Inspect fixture\nExec=/usr/bin/touch "{action_marker}"\n')
    clear = app = shell = peer = None
    log_path = directory / "quickshell.log"
    with (directory / "clear.log").open("w") as clear_log, log_path.open("w") as shell_log, (directory / "foot.log").open("w") as app_log:
        try:
            clear = subprocess.Popen([str(Path(args.binary).resolve()), "--config", str(config), "--socket", clear_name, "--exit-after", "10", "--capture", str(capture)], env=dict(env, WAYLAND_DISPLAY=host_name), stdout=clear_log, stderr=subprocess.STDOUT, start_new_session=True)
            SMOKE.wait_socket(address, clear)
            peer = SHELL.Peer(address)
            app_env = dict(env, WAYLAND_DISPLAY=clear_name, CLEAR_SOCKET=str(address))
            app = subprocess.Popen(["foot", "--title", "Clear · Shell UI", "sh", "-c", "i=0; while [ $i -lt 12 ]; do printf '\\033[H\\033[2J\\033]2;Clear preview title %s\\007Window preview frame %s\\n' \"$i\" \"$i\"; i=$((i+1)); sleep 1; done"], env=app_env, stdout=app_log, stderr=subprocess.STDOUT, start_new_session=True)
            SHELL.wait_for(lambda: peer.state()["windows"], "real application")
            shell = subprocess.Popen(["dbus-run-session", "--", args.quickshell, "-p", str(qml)], env=dict(app_env, QT_QPA_PLATFORM="wayland", QT_QUICK_BACKEND="software", WAYLAND_DEBUG="client", XDG_DATA_HOME=str(data_home)), stdout=shell_log, stderr=subprocess.STDOUT, start_new_session=True)
            SHELL.wait_for(lambda: "UI READY" in log_path.read_text(), "UI scene ready", seconds=7)
            if scene == "preview":
                SHELL.wait_for(lambda: peer.state()["windows"][0]["minimized"], "minimized window preview remains active", seconds=3)
            if scene == "context-menu":
                SHELL.wait_for(action_marker.exists, "published desktop action executed")
                SHELL.wait_for(lambda: peer.state()["windows"][0]["workspace"] == "2", "explicit context-menu workspace move")
                assert peer.state()["outputs"][0]["workspace"] == "1", "move revealed the destination workspace"
            refusal = peer.request("window_previews", windows=[peer.state()["windows"][0]["id"]])
            assert refusal["type"] == "error" and "owned by this process" in refusal["message"], refusal
            state = peer.state()
            assert state["outputs"][0]["area"]["y"] == (54 if args.glass else 52), state
            assert clear.wait(timeout=10) == 0
            log = re.sub(r"\x1b\[[0-9;]*m", "", log_path.read_text())
            if scene == "preview":
                assert "Unsupported image format" in log, "invalid-frame fixture did not exercise decode failure"
                checked_log = re.sub(r"^.*WARN scene: QML QQuickImage at @LivePreview.qml\[\d+:\d+\]: Error decoding: data:image/png;base64,aW52YWxpZA==: Unsupported image format$", "", log, flags=re.MULTILINE)
            else:
                checked_log = log
            for failure in ("UI FAIL", "WARN scene:", "ReferenceError", "TypeError", "Failed to load configuration", "Cannot assign", "is not a type"):
                assert failure not in checked_log, checked_log[-6000:]
            header, dimensions, maximum, pixels = capture.read_bytes().split(b"\n", 3)
            assert (header, dimensions, maximum) == (b"P6", f"{args.width} 720".encode(), b"255")
            def pixel(x, y):
                offset = (y * args.width + x) * 3
                return tuple(pixels[offset:offset + 3])
            # Transparent margin and rounded corner remain outside the bar.
            background = tuple(round(c * 255) for c in (0.07, 0.08, 0.10))
            for point in ((0, 0), (13, 11 if args.glass else 9)):
                assert all(abs(a - b) <= 1 for a, b in zip(pixel(*point), backdrop_pixel(*point) if args.backdrop is not None else background)), (point, pixel(*point))
            result = {"scene": scene, "width": args.width, "glass": args.glass}
            if scene == "context-menu":
                bounds = re.findall(r"UI menu bounds (\{.*\})", log)
                assert bounds, "no bounded menu geometry"
                box = json.loads(bounds[-1])
                result["menu"] = [int(box[key]) for key in ("x", "y", "width", "height")]
                assert 0 < box["width"] <= 320 and box["height"] < 720, box
                dispatch = re.findall(r"UI menu dispatch ms (\d+)", log)
                assert dispatch and max(map(int, dispatch)) < 150, ("blocking context-menu dispatch", dispatch)
                result["dispatch_ms"] = list(map(int, dispatch))
                layer_id = re.search(r'get_layer_surface\(new id zwlr_layer_surface_v1[@#](\d+), .*"clear-(?:glass-rounded-)?(?:app-)?context-menu"', log)
                assert layer_id, "no menu layer role"
                configs = re.findall(r'zwlr_layer_surface_v1[@#]' + layer_id[1] + r'\.configure\(\d+, (\d+), (\d+)\)', log)
                shown = [list(map(int, size)) for size in configs if size != ("1", "1")]
                requested = json.loads(re.findall(r"UI menu requested size (\[.*\])", log)[0])
                assert shown and shown[0] == requested, ("menu opened at an intermediate size", requested, shown)
                assert all(w == requested[0] and h >= requested[1] for w, h in shown), ("menu flickered through tiny sizes", shown)
                result["configured_sizes"] = shown

            if scene == "preview":
                hashes = re.findall(r"UI capture (-?\d+)", log)
                assert len(hashes) >= 3 and len(set(hashes)) >= 2, "preview did not update with real client pixels"
                result["distinct_frames"] = len(set(hashes))
                assert log.count("UI dock button created ") == 1, "metadata recreated the hovered dock button"
                cards = re.findall(r"UI card created (\d+)", log)
                assert len(cards) == 1, ("metadata recreated the preview card", cards)
                assert log.count("UI displayed frame ") >= 3, "live frames did not swap after null/invalid replies"
                hidden_hashes = re.findall(r"UI capture (-?\d+)", log.split("UI MINIMIZED", 1)[1])
                assert len(set(hidden_hashes)) >= 2, "minimized preview froze rather than delivering live commits"
                result["minimized_distinct_frames"] = len(set(hidden_hashes))
            if scene in ("preview", "tooltip"):
                configs = re.findall(r'xdg_popup[@#]\d+\.configure\((-?\d+), (-?\d+), (\d+), (\d+)\)', log)
                assert configs, "no real popup configure"
                x, y, width, height = map(int, configs[-1])
                assert y >= 44 + 8, (x, y, width, height)
                assert 0 <= x and x + width <= args.width - 24, (x, y, width, height)
                result["popup"] = [x, y, width, height]
                # The popup's fill is below the panel at its configured position.
                top = (10 if args.glass else 8) + y
                assert pixel(12 + x + width // 2, top + 2) != pixel(12 + x + width // 2, top - 2), "popup absent at configured position"
            (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            print(f"PASS: {scene} ({args.width}px, {'glass' if args.glass else 'standard'}); no physical input", flush=True)
            return result
        finally:
            if peer:
                peer.close()
            for process in (shell, app, clear):
                SMOKE.stop(process)


def compare_backdrops(root, scene, width):
    images = {}
    for mode in ("plain", "blur", "glass", "both"):
        data = (root / mode / scene / "frame.ppm").read_bytes().split(b"\n", 3)[3]
        images[mode] = data
    result = json.loads((root / "both" / scene / "result.json").read_text())
    if scene in ("preview", "tooltip"):
        x, y, w, h = result["popup"]
        x += 12
        y += 10
    else:
        x, y, w, h = result["menu"]
    def pixel(mode, px, py):
        offset = (py * width + px) * 3
        return images[mode][offset:offset + 3]
    # Blank, translucent gutter: no text, cards or captured client pixels.
    inside = ([(px, y + dy) for dy in (3, 5, 7) for px in range(x + 12, x + w - 12, 2)]
              if scene == "tooltip" else
              [(x + dx, py) for dx in (5, 7, 9) for py in range(y + 32, y + h - 32, 3)])
    report = {}
    for first, second in (("plain", "blur"), ("plain", "glass"), ("blur", "both"), ("glass", "both")):
        changed = sum(max(abs(a - b) for a, b in zip(pixel(first, px, py), pixel(second, px, py))) > 3 for px, py in inside)
        assert changed > 20, (scene, first, second, "missing distinct backdrop pass", changed)
        report[first + "_vs_" + second] = changed
    outside = [(x + 1, y + 1), (x + w - 2, y + 1), (x + 1, y + h - 2),
               (x - 4, y + h // 2), (x + w + 4, y + h // 2), (width - 3, 710)]
    for px, py in outside:
        if 0 <= px < width and 54 < py < 720:
            for mode in images:
                assert all(abs(a - b) <= 1 for a, b in zip(pixel(mode, px, py), backdrop_pixel(px, py))), (scene, mode, px, py, "effect escaped menu bounds")
    (root / (scene + "-backdrops.json")).write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: {scene} distinct blur/glass passes, rounded holes and untouched exterior", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--quickshell", default="quickshell")
    parser.add_argument("--artifacts", default="target/vm-shell-ui-smoke")
    parser.add_argument("--case", action="append", choices=SCENES)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--glass", action="store_true")
    parser.add_argument("--backdrop", choices=("plain", "blur", "glass", "both"))
    parser.add_argument("--verify-backdrops", action="store_true", help="Compare patterned plain/blur/glass/combined GPU captures for hover cards, tooltips and context menus")
    args = parser.parse_args()
    assert 480 <= args.width <= 1920
    if args.verify_backdrops and (not args.glass or args.backdrop is not None):
        parser.error("--verify-backdrops requires --glass and chooses its own backdrop modes")
    root = Path(args.artifacts).resolve()
    root.mkdir(parents=True, exist_ok=True)
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), QT_SCALE_FACTOR="1")
    for key in ("WAYLAND_SOCKET", "WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "CLEAR_SOCKET"):
        env.pop(key, None)
    host_name = f"clear-ui-host-{os.getpid()}"
    host = None
    with (root / "kwin.log").open("w") as host_log:
        try:
            host = subprocess.Popen(["dbus-run-session", "--", "kwin_wayland", "--virtual", "--width", "2000", "--height", "900", "--no-lockscreen", "--no-global-shortcuts", "--no-kactivities", "--socket", host_name], env=env, stdout=host_log, stderr=subprocess.STDOUT, start_new_session=True)
            SMOKE.wait_socket(runtime / host_name, host)
            if args.verify_backdrops:
                scenes = args.case or ("preview", "tooltip", "context-menu")
                if any(scene not in ("preview", "tooltip", "context-menu") for scene in scenes):
                    parser.error("backdrop comparisons support preview, tooltip and context-menu cases")
                results = []
                for mode in ("plain", "blur", "glass", "both"):
                    args.backdrop = mode
                    mode_root = root / mode
                    mode_root.mkdir(exist_ok=True)
                    results.extend(run_scene(args, scene, mode_root, env, runtime, host_name) for scene in scenes)
                for scene in scenes:
                    compare_backdrops(root, scene, args.width)
            else:
                results = [run_scene(args, scene, root, env, runtime, host_name) for scene in (args.case or SCENES)]
            (root / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        finally:
            SMOKE.stop(host)


if __name__ == "__main__":
    main()
