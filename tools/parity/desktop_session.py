"""Own a private X11 desktop for reproducible comparison without focus conflicts."""

import argparse
import json
import os
import select
import signal
import subprocess
import sys
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-binary", required=True)
    parser.add_argument("--reference-home", type=Path, required=True)
    parser.add_argument("--reference-profile", type=Path, required=True)
    parser.add_argument("--reference-port", type=int, required=True)
    parser.add_argument("--cdp-port", type=int, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--session", type=Path, required=True)
    args = parser.parse_args()
    if args.session.exists() or args.reference_profile.exists():
        raise FileExistsError(
            "Use a fresh session directory and reference profile for each comparison"
        )
    args.session.parent.mkdir(parents=True, exist_ok=True)
    children = []
    stopping = False

    def launch(command, name, env=None, **kwargs):
        with (args.session.parent / (name + ".log")).open("w") as output:
            process = subprocess.Popen(
                command, env=env, stdout=output, stderr=subprocess.STDOUT, **kwargs
            )
        children.append(process)
        return process

    try:
        read_fd, write_fd = os.pipe()
        try:
            launch(
                [
                    "Xvfb",
                    "-displayfd",
                    str(write_fd),
                    "-nolisten",
                    "tcp",
                    "-noreset",
                    "-screen",
                    "0",
                    "1920x1080x24",
                ],
                "xvfb",
                pass_fds=(write_fd,),
            )
            os.close(write_fd)
            if not select.select([read_fd], [], [], 10)[0]:
                raise RuntimeError("Xvfb did not assign a display")
            display = ":" + os.read(read_fd, 64).decode().strip()
            if not display[1:].isdigit():
                raise RuntimeError("Xvfb failed to start; inspect xvfb.log")
        finally:
            os.close(read_fd)
        env = os.environ.copy()
        env["DISPLAY"] = display
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("SESSION_MANAGER", None)
        env.pop("DESKTOP_AUTOSTART_ID", None)
        launch(
            ["xfwm4", "--compositor=off", "--sm-client-disable"], "window-manager", env
        )
        time.sleep(0.5)
        reference_env = env | {
            "T3CODE_HOME": str(args.reference_home.resolve()),
            "XDG_CONFIG_HOME": str(args.reference_profile.resolve()),
            "T3CODE_PORT": str(args.reference_port),
        }
        reference = launch(
            [
                args.reference_binary,
                "--remote-debugging-address=127.0.0.1",
                f"--remote-debugging-port={args.cdp_port}",
            ],
            "reference-electron",
            reference_env,
        )
        native = launch(
            [
                "dbus-run-session",
                "--",
                sys.executable,
                str(Path(__file__).with_name("private_session.py")),
                "--config",
                str(args.config.resolve()),
                "--session",
                str(args.session.resolve()),
            ],
            "native-session",
            env,
        )
        (args.session.parent / "desktop-session.json").write_text(
            json.dumps(
                {
                    "pid": os.getpid(),
                    "display": display,
                    "reference_pid": reference.pid,
                    "native_launcher_pid": native.pid,
                    "endpoint": f"http://127.0.0.1:{args.cdp_port}",
                },
                indent=2,
            )
            + "\n"
        )
        print(
            json.dumps({"display": display, "session": str(args.session)}), flush=True
        )

        def stop(*_):
            nonlocal stopping
            stopping = True
            reference.terminate()

        signal.signal(signal.SIGTERM, stop)
        while reference.poll() is None and native.poll() is None:
            time.sleep(0.5)
        if not stopping and (
            reference.poll() not in [None, 0] or native.poll() not in [None, 0]
        ):
            raise RuntimeError(
                "A comparison application exited unsuccessfully; inspect session logs"
            )
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


if __name__ == "__main__":
    main()
