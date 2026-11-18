#!/usr/bin/env python3
import os
import shutil
import subprocess
import sys
from pathlib import Path

import env


def owned_generation(root, environment):
    state = env.read_json(root.parent / "state.json")
    slot_path = env.main_tree() / ".claude-dev/parity/slots" / str(state["slot"])
    ports, display = env.allocation(state["slot"])
    if (
        root.parent != slot_path
        or root != Path(state["generation"])
        or environment.get(env.MARKER) != state["owner"]
        or environment.get("DISPLAY") != display
        or state["ports"] != ports
        or not environment.get("DBUS_SESSION_BUS_ADDRESS")
    ):
        raise RuntimeError(
            "An owned parity generation and private D-Bus session are required"
        )
    return state


def main():
    root = Path.cwd()
    owned_generation(root, os.environ)
    daemon = shutil.which("gnome-keyring-daemon")
    if daemon is None:
        raise RuntimeError(
            "gnome-keyring-daemon is required for the private fixture keyring"
        )
    os.environ["XDG_DATA_HOME"] = str(root / "keyring-data")
    (root / "keyring-control").mkdir(exist_ok=True, mode=0o700)
    with (root / "keyring.log").open("ab") as log:
        subprocess.run(
            [
                daemon,
                "--unlock",
                "--components=secrets",
                "--control-directory=keyring-control",
            ],
            input=b"\n",
            stdout=log,
            stderr=log,
            check=True,
            timeout=15,
        )
    args = [
        "--password-store=gnome-libsecret" if arg == "--password-store=basic" else arg
        for arg in sys.argv[1:]
    ]
    os.execv(env.ELECTRON, [env.ELECTRON, *args])


if __name__ == "__main__":
    main()
