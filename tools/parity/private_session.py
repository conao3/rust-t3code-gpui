"""Own an isolated accessibility bus and one native test application."""

import argparse
import json
import os
import signal
import subprocess
import time
from pathlib import Path

from fixture_validation import inspect_environments


def cleanup_bus_processes(address):
    count = 0
    for process in Path("/proc").glob("[0-9]*"):
        if int(process.name) == os.getpid():
            continue
        try:
            environment = dict(
                item.split(b"=", 1)
                for item in (process / "environ").read_bytes().split(b"\0")
                if b"=" in item
            )
            if environment.get(b"DBUS_SESSION_BUS_ADDRESS") == address.encode():
                os.kill(int(process.name), signal.SIGTERM)
                count += 1
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--session", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/t3code-gpui"))
    args = parser.parse_args()
    environments = inspect_environments(args.binary, args.config)
    args.session.parent.mkdir(parents=True, exist_ok=True)
    launcher = Path(os.environ["T3CODE_ATSPI_LAUNCHER"])
    children = []
    try:
        for binary, suffix in [
            (launcher, "bus"),
            (launcher.with_name("at-spi2-registryd"), "registry"),
        ]:
            log = args.session.parent / f"atspi-{suffix}.log"
            with log.open("w") as output:
                command = [str(binary)] + (
                    ["--launch-immediately"] if suffix == "bus" else []
                )
                children.append(
                    subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
                )
            time.sleep(1)
        from gi.repository import Gio, GLib

        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        bus.call_sync(
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.freedesktop.DBus.Properties",
            "Set",
            GLib.Variant(
                "(ssv)", ("org.a11y.Status", "IsEnabled", GLib.Variant("b", True))
            ),
            None,
            Gio.DBusCallFlags.NONE,
            5000,
            None,
        )
        env = os.environ.copy()
        env["T3CODE_HOME"] = str((args.session.parent / "native-home").resolve())
        env["XDG_STATE_HOME"] = str((args.session.parent / "native-state").resolve())
        env["T3CODE_GPUI_RENDERER_INFO"] = "1"
        with (args.session.parent / "native.log").open("w") as output:
            app = subprocess.Popen(
                [str(args.binary.resolve()), "--config", str(args.config.resolve())],
                env=env,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
        children.append(app)
        manifest = {
            "pid": app.pid,
            "dbus_address": os.environ["DBUS_SESSION_BUS_ADDRESS"],
            "binary": str(args.binary.resolve()),
            "display": env["DISPLAY"],
            "environments": environments,
        }
        args.session.write_text(json.dumps(manifest, indent=2) + "\n")
        args.session.chmod(0o600)
        print(json.dumps({"pid": app.pid, "session": str(args.session)}), flush=True)
        signal.signal(signal.SIGTERM, lambda *_: app.terminate())
        result = app.wait()
        if result != 0:
            raise SystemExit(result if result > 0 else 128 - result)
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        cleanup_bus_processes(os.environ["DBUS_SESSION_BUS_ADDRESS"])


if __name__ == "__main__":
    main()
