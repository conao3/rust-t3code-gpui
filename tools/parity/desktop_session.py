"""Own a private X11 desktop for reproducible comparison without focus conflicts."""

import argparse
import json
import os
import select
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

from fixture_validation import distinct


def require_free_ports(ports):
    if len(set(ports)) != len(ports):
        raise ValueError("Reference and CDP ports must be distinct")
    for port in ports:
        with socket.socket() as probe:
            probe.settimeout(1)
            if probe.connect_ex(("127.0.0.1", port)) == 0:
                raise RuntimeError(
                    f"Test port {port} is in use; finish stopping its owner before launching"
                )


def read_display(read_fd, timeout=10):
    deadline = time.monotonic() + timeout
    number = b""
    while not number.endswith(b"\n"):
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([read_fd], [], [], remaining)[0]:
            raise RuntimeError("Xvfb did not assign a complete display number")
        chunk = os.read(read_fd, 64)
        if not chunk:
            raise RuntimeError("Xvfb exited before assigning a display")
        number += chunk
    display = number.decode().strip()
    if not display.isdigit():
        raise RuntimeError("Xvfb returned an invalid display number")
    return ":" + display


def stop_process_group(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        process.wait()
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--display", help="Use an existing X11 desktop; own only the test applications"
    )
    parser.add_argument("--reference-binary", required=True)
    parser.add_argument("--reference-home", type=Path, required=True)
    parser.add_argument("--reference-profile", type=Path, required=True)
    parser.add_argument("--reference-port", type=int, required=True)
    parser.add_argument("--cdp-port", type=int, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument(
        "--native-binary", type=Path, default=Path("target/debug/t3code-gpui")
    )
    parser.add_argument("--session", type=Path, required=True)
    args = parser.parse_args()
    configuration = json.loads(args.config.read_text())
    distinct(
        f"http://127.0.0.1:{args.reference_port}",
        [{"origin": e["httpBaseUrl"]} for e in configuration["environments"]],
    )
    if args.session.exists() or args.reference_profile.exists():
        raise FileExistsError(
            "Use a fresh session directory and reference profile for each comparison"
        )
    require_free_ports([args.reference_port, args.cdp_port])
    args.session.parent.mkdir(parents=True, exist_ok=True)
    children = []
    stopping = False

    def launch(command, name, env=None, **kwargs):
        with (args.session.parent / (name + ".log")).open("w") as output:
            process = subprocess.Popen(
                command,
                env=env,
                stdout=output,
                stderr=subprocess.STDOUT,
                start_new_session=True,
                **kwargs,
            )
        children.append(process)
        return process

    try:
        if args.display is None:
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
                display = read_display(read_fd)
            finally:
                os.close(read_fd)
        else:
            display = args.display
        env = os.environ.copy()
        env["DISPLAY"] = display
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("SESSION_MANAGER", None)
        env.pop("DESKTOP_AUTOSTART_ID", None)
        if args.display is None:
            launch(
                ["xfwm4", "--compositor=off", "--sm-client-disable"],
                "window-manager",
                env,
            )
            time.sleep(0.5)
        reference_env = env | {
            "T3CODE_HOME": str(args.reference_home.resolve()),
            "XDG_CONFIG_HOME": str(args.reference_profile.resolve()),
            "T3CODE_PORT": str(args.reference_port),
        }
        reference = launch(
            [
                "dbus-run-session",
                "--",
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
                "--binary",
                str(args.native_binary.resolve()),
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
            stop_process_group(child)


if __name__ == "__main__":
    main()
