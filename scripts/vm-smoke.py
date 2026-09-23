#!/usr/bin/env python3
"""Bounded, unprivileged compositor smoke test on an isolated virtual KWin host.

Starts an isolated KWin virtual display, Clear, and real foot clients. Neither
sudo nor a logged-in graphical session is required. Logs remain in --artifacts.
"""

import argparse
import os
import signal
import subprocess
import time
from pathlib import Path


def wait_socket(path, process, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(
                f"process exited before creating {path}: {process.returncode}"
            )
        if path.is_socket():
            return
        time.sleep(0.1)
    raise TimeoutError(f"socket did not appear: {path}")


def stop(process):
    if process is None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/clear")
    parser.add_argument("--artifacts", default="target/vm-smoke")
    parser.add_argument(
        "--script", help="Optional Rhai file providing a columns(ctx) function"
    )
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    artifacts = Path(args.artifacts).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), RUST_BACKTRACE="1")
    env.pop("WAYLAND_SOCKET", None)
    env.pop("DISPLAY", None)
    host_name = f"clear-test-host-{os.getpid()}"
    clear_name = f"clear-test-{os.getpid()}"
    host = clear = None
    clients = []
    config = artifacts / "config.toml"
    source = 'gaps = 12\n[[outputs]]\nname = "left"\nwidth = 640\nheight = 480\n[[outputs]]\nname = "right"\nwidth = 640\nheight = 480\n'
    if args.script:
        import json

        source = (
            "script = " + json.dumps(str(Path(args.script).resolve())) + "\n" + source
        )
        source += (
            '\n[[workspaces]]\nid = 1\nname = "scripted"\nmode = "script:columns"\n'
        )
    config.write_text(source)
    with (
        (artifacts / "kwin.log").open("w") as host_log,
        (artifacts / "clear.log").open("w") as clear_log,
        (artifacts / "clients.log").open("w") as client_log,
    ):
        try:
            host = subprocess.Popen(
                [
                    "dbus-run-session",
                    "--",
                    "kwin_wayland",
                    "--virtual",
                    "--width",
                    "1400",
                    "--height",
                    "900",
                    "--no-lockscreen",
                    "--no-global-shortcuts",
                    "--no-kactivities",
                    "--socket",
                    host_name,
                ],
                env=env,
                stdout=host_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            wait_socket(runtime / host_name, host)
            clear = subprocess.Popen(
                [
                    str(binary),
                    "--config",
                    str(config),
                    "--socket",
                    clear_name,
                    "--exit-after",
                    "12",
                    "--capture",
                    str(artifacts / "frame.ppm"),
                ],
                env=dict(env, WAYLAND_DISPLAY=host_name),
                stdout=clear_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            wait_socket(runtime / clear_name, clear)
            for index in range(3):
                clients.append(
                    subprocess.Popen(
                        [
                            "foot",
                            "--app-id",
                            f"clear-test-{index}",
                            "--title",
                            f"Clear test {index}",
                            "sh",
                            "-c",
                            f"printf 'Clear integration test {index}\\n'; sleep 20",
                        ],
                        env=dict(env, WAYLAND_DISPLAY=clear_name),
                        stdout=client_log,
                        stderr=subprocess.STDOUT,
                        start_new_session=True,
                    )
                )
            result = clear.wait(timeout=25)
            clear_log.flush()
            log = (artifacts / "clear.log").read_text()
            if result != 0:
                raise RuntimeError(f"Clear exited with status {result}:\n{log}")
            assert "outputs=2" in log, log
            assert log.count("clear: mapped window ") == 3, log
            assert "clear: stopped" in log, log
            assert "panicked" not in log, log
            assert "Rhai layout" not in log, log
            frame = (artifacts / "frame.ppm").read_bytes()
            magic, dimensions, maximum, pixels = frame.split(b"\n", 3)
            width, height = map(int, dimensions.split())
            assert magic == b"P6" and maximum == b"255", "invalid capture header"
            assert len(pixels) == width * height * 3, "incomplete capture"
            colors = {pixels[i : i + 3] for i in range(0, len(pixels), 3)}
            assert len(colors) > 16, "capture appears blank (no rendered client text)"
            print(
                "PASS: two virtual outputs, three real XDG clients, nonblank GPU capture, graceful exit"
            )
            print(f"Logs: {artifacts}")
        finally:
            for client in clients:
                stop(client)
            stop(clear)
            stop(host)


if __name__ == "__main__":
    main()
