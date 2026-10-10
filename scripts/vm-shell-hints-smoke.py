#!/usr/bin/env python3
"""Bounded real Quickshell panel/icon-hint IPC checks on private virtual KWin.

Copies the example into ignored artifacts and logs its actual discovery/report
replies, without adding production diagnostics. Checks accepted nonempty icon
rectangles, foreign-process discovery/refusal, unchanged desktop policy, and
rotated panel identities after restart. No physical input, scroll gesture, GPU
pixel oracle, or minimize animation is tested. Build Clear separately.
"""

import argparse
import importlib.util
import json
import os
import shutil
import subprocess
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("hints_shell", Path(__file__).with_name("vm-shell-smoke.py"))
assert SPEC is not None and SPEC.loader is not None
SHELL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SHELL)
SMOKE = SHELL.SMOKE


def fixture(source, directory):
    shutil.copytree(source, directory)
    bridge = directory / "ClearBridge.qml"
    text = bridge.read_text()
    marker = '    property var animationPanels: []\n'
    assert marker in text
    text = text.replace(marker, marker + '    property var fixtureReports: ({})\n', 1)
    marker = '        socket.write(Protocol.encodeRequest(nextId, request));\n'
    assert marker in text
    text = text.replace(marker, '''        if (request.type === "set_animation_targets" && request.targets.length)
            fixtureReports[nextId] = request;
''' + marker, 1)
    marker = '            var message = Protocol.decodeMessage(line);\n'
    assert marker in text
    text = text.replace(marker, marker + '''            if (message.type === "animation_panels" && message.panels.length)
                console.info("hint fixture: panels " + JSON.stringify(message.panels));
            if (fixtureReports[message.id]) {
                console.info("hint fixture: report " + JSON.stringify({
                    request: fixtureReports[message.id], response: message}));
                delete fixtureReports[message.id];
            }
''', 1)
    bridge.write_text(text)
    return directory / "shell.qml"


def records(log, kind):
    marker = f"hint fixture: {kind} "
    return [json.loads(line.split(marker, 1)[1]) for line in log.read_text().splitlines() if marker in line]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--quickshell", default="quickshell")
    parser.add_argument("--artifacts", default="target/vm-shell-hints-smoke")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    assert binary.is_file(), "build Clear separately"
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    qml = fixture(Path(__file__).resolve().parents[1] / "examples/quickshell", artifacts / f"fixture-{os.getpid()}")
    config = artifacts / "config.toml"
    config.write_text('[[outputs]]\nname="left"\nwidth=640\nheight=480\n[[outputs]]\nname="right"\nwidth=640\nheight=480\n')
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), QT_SCALE_FACTOR="1")
    for key in ("WAYLAND_SOCKET", "WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "CLEAR_SOCKET"):
        env.pop(key, None)
    host_name = f"clear-hints-host-{os.getpid()}"
    clear_name = f"clear-hints-{os.getpid()}"
    address = runtime / f"clear-{clear_name}" / "shell.sock"
    log_path = artifacts / "quickshell.log"
    host = clear = app = shell = peer = None
    result = {}
    with (artifacts / "kwin.log").open("w") as host_log, (artifacts / "clear.log").open("w") as clear_log, log_path.open("w") as shell_log, (artifacts / "foot.log").open("w") as app_log:
        try:
            host = subprocess.Popen(["dbus-run-session", "--", "kwin_wayland", "--virtual", "--width", "1400", "--height", "800", "--no-lockscreen", "--no-global-shortcuts", "--no-kactivities", "--socket", host_name], env=env, stdout=host_log, stderr=subprocess.STDOUT, start_new_session=True)
            SMOKE.wait_socket(runtime / host_name, host)
            clear = subprocess.Popen([str(binary), "--config", str(config), "--socket", clear_name, "--exit-after", "25"], env=dict(env, WAYLAND_DISPLAY=host_name), stdout=clear_log, stderr=subprocess.STDOUT, start_new_session=True)
            SMOKE.wait_socket(address, clear)
            peer = SHELL.Peer(address)
            assert "animation_targets_v1" in peer.state()["capabilities"]
            app_env = dict(env, WAYLAND_DISPLAY=clear_name, CLEAR_SOCKET=str(address))
            app = subprocess.Popen(["foot", "--title", "Panel hint smoke", "sh", "-c", "sleep 30"], env=app_env, stdout=app_log, stderr=subprocess.STDOUT, start_new_session=True)
            SHELL.wait_for(lambda: peer.state()["windows"], "normal window")

            def start_shell():
                return subprocess.Popen(["dbus-run-session", "--", args.quickshell, "-p", str(qml)], env=dict(app_env, QT_QPA_PLATFORM="wayland", QT_QUICK_BACKEND="software"), stdout=shell_log, stderr=subprocess.STDOUT, start_new_session=True)

            def accepted():
                reports = records(log_path, "report")
                return next((record for record in reports if record["response"]["type"] == "ok"), None)

            shell = start_shell()
            report = SHELL.wait_for(accepted, "real Quickshell nonempty icon report accepted", seconds=10)
            panels = records(log_path, "panels")[-1]
            descriptor = next(panel for panel in panels if panel["panel"] == report["request"]["panel"])
            assert len(panels) == 2, panels
            assert report["request"]["targets"], report
            for target in report["request"]["targets"]:
                rect = target["rect"]
                assert rect["width"] == rect["height"] == 20, target
                assert 0 <= rect["x"] <= descriptor["width"] - 20
                assert 0 <= rect["y"] <= descriptor["height"] - 20
            before = peer.state()
            assert before["focused_window"] == before["windows"][0]["id"]
            query = peer.request("animation_panels")
            assert query["type"] == "animation_panels" and query["panels"] == [], query
            refusal = peer.request("set_animation_targets", **{key: value for key, value in report["request"].items() if key != "type"})
            assert refusal["type"] == "error" and "another Wayland process" in refusal["message"], refusal
            assert peer.state() == before, "query/refused hint changed policy"
            result.update(first_report=report, same_process_panels=panels, foreign_discovery=query, foreign_registration=refusal, unchanged_policy=True)
            old_ids = {panel["panel"] for panel in panels}
            SMOKE.stop(shell)
            shell = None
            SHELL.wait_for(lambda: all(o["area"]["y"] == 0 for o in peer.state()["outputs"]), "unmapped panels release reservation")
            stale = peer.request("set_animation_targets", **{key: value for key, value in report["request"].items() if key != "type"})
            assert stale["type"] == "error" and "stale panel" in stale["message"], stale
            previous = len(records(log_path, "report"))
            shell = start_shell()
            fresh = SHELL.wait_for(lambda: next((record for record in records(log_path, "report")[previous:] if record["response"]["type"] == "ok"), None), "restarted panel report accepted", seconds=10)
            new_panels = records(log_path, "panels")[-1]
            assert old_ids.isdisjoint({panel["panel"] for panel in new_panels}), (panels, new_panels)
            result.update(stale_mapping=stale, restarted_report=fresh, restarted_panels=new_panels)
            errors = [record for record in records(log_path, "report") if record["response"]["type"] == "error"]
            assert not errors, errors
            assert shell.poll() is None
            assert clear.wait(timeout=28) == 0
            for failure in ("ReferenceError", "TypeError", "Failed to load configuration", "Cannot assign", "is not a type"):
                assert failure not in log_path.read_text(), failure
        finally:
            if peer is not None:
                peer.close()
            for process in (shell, app, clear, host):
                SMOKE.stop(process)
    result["passed"] = True
    (artifacts / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"PASS: real Quickshell nonempty icon hints, process ownership and panel remapping; artifacts: {artifacts}")


if __name__ == "__main__":
    main()
