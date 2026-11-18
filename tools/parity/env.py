import argparse
import asyncio
import fcntl
import json
import os
import shlex
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import time
import traceback
import uuid
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from fixture_rpc import FixtureRpc
from fixture_validation import (
    distinct,
    environment_descriptor,
    inspect_environments,
    origin,
)
from workspace_fixture import independent

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parent.parent
PROJECT = "11111111-1111-4111-8111-111111111111"
T3 = "/nix/store/5l5pkbxsxw62xyvdd3cdnx2q8px2rvs7-t3code-0.1.9/bin/t3"
ELECTRON = "/nix/store/jyicc2sxcg9llp2hvj3lf2wny5a3nkfx-t3code-0.1.9-desktop/bin/t3code-desktop"
MARKER = "T3_PARITY_OWNER"


class SlotError(RuntimeError):
    pass


def main_tree():
    return Path(
        subprocess.check_output(
            [
                "git",
                "-C",
                str(REPO),
                "rev-parse",
                "--path-format=absolute",
                "--git-common-dir",
            ],
            text=True,
        ).strip()
    ).parent


def allocation(slot):
    if not 1 <= slot <= 100:
        raise ValueError("Slot must be between 1 and 100")
    base = 42000 + slot * 10
    return {"reference": base, "native": base + 1, "cdp": base + 2}, f":{100 + slot}"


def write_json(path, value):
    temporary = path.with_name(path.name + ".tmp")
    with open(temporary, "w", opener=lambda p, f: os.open(p, f, 0o600)) as output:
        json.dump(value, output, indent=2)
        output.write("\n")
    temporary.chmod(0o600)
    temporary.replace(path)


def read_json(path):
    return json.loads(path.read_text())


def process_identity(pid):
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        if fields[0] == "Z":
            return None
        return {"pid": int(pid), "group": int(fields[2]), "start": fields[19]}
    except (FileNotFoundError, ProcessLookupError):
        return None


def owned_processes(owner):
    found = []
    marker = f"{MARKER}={owner}".encode()
    for path in Path("/proc").glob("[0-9]*"):
        try:
            if marker in (path / "environ").read_bytes().split(b"\0"):
                identity = process_identity(int(path.name))
                if identity:
                    found.append(identity)
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return found


def signal_owned(owner, signum):
    processes = owned_processes(owner)
    groups = {p["group"] for p in processes if p["pid"] != os.getpid()}
    for group in groups:
        members = [p for p in processes if p["group"] == group]
        if group == os.getpgrp():
            raise SlotError("Refusing to signal the caller's process group")
        if not any(process_identity(p["pid"]) == p for p in members):
            continue
        try:
            os.killpg(group, signum)
        except ProcessLookupError:
            pass


def stop_owned(owner):
    for signum, timeout in [(signal.SIGTERM, 10), (signal.SIGKILL, 5)]:
        signal_owned(owner, signum)
        deadline = time.monotonic() + timeout
        while owned_processes(owner) and time.monotonic() < deadline:
            time.sleep(0.1)
        if not owned_processes(owner):
            return
    raise SlotError("Owned processes did not exit")


def require_free(ports, display):
    for port in ports.values():
        with socket.socket() as probe:
            try:
                probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                probe.bind(("127.0.0.1", port))
            except OSError:
                raise SlotError(f"Slot port {port} is unavailable") from None
    number = display.removeprefix(":")
    if (
        Path(f"/tmp/.X{number}-lock").exists()
        or Path(f"/tmp/.X11-unix/X{number}").exists()
    ):
        raise SlotError(f"Slot display {display} is unavailable")


def wait_free(ports, display, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        try:
            require_free(ports, display)
            return
        except SlotError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.1)


def quiet(command, **kwargs):
    result = subprocess.run(
        command, check=False, capture_output=True, timeout=60, **kwargs
    )
    if result.returncode:
        raise SlotError(f"Fixture command failed: {Path(command[0]).name}")
    return result.stdout


def fixture(generation, options):
    seed = generation / "seed"
    home = seed / "home"
    (home / "userdata").mkdir(parents=True)
    with sqlite3.connect(home / "userdata/statev2.sqlite") as database:
        database.execute("VACUUM")
    workspace = seed / "workspace"
    workspace.mkdir()
    (workspace / "README.md").write_text(
        "# UI parity\n\nDeterministic local workspace.\n"
    )
    git_env = os.environ | {
        "GIT_AUTHOR_NAME": "Parity fixture",
        "GIT_AUTHOR_EMAIL": "parity@example.invalid",
        "GIT_COMMITTER_NAME": "Parity fixture",
        "GIT_COMMITTER_EMAIL": "parity@example.invalid",
        "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
        "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
    }
    for args in [
        ["init", "-b", "main"],
        ["add", "."],
        ["-c", "commit.gpgsign=false", "commit", "-m", "Initialize parity fixture"],
        ["branch", "feature/parity"],
    ]:
        quiet(["git", "-C", str(workspace), *args], env=git_env)
    transcript = (
        Path(options["reference_source"])
        / "apps/server/src/orchestration-v2/testkit/fixtures/simple/codex_transcript.ndjson"
    )
    shutil.copyfile(transcript, seed / "transcript.ndjson")
    for target in ["reference", "native"]:
        base = generation / target
        shutil.copytree(seed, base)
        provider = base / "provider"
        provider.mkdir()
        wrapper = base / "mock-codex"
        write_json(
            base / "mock-settings.json",
            {
                "stream_ms": options["stream_ms"],
                "usage_models": [],
                "text": "".join(
                    f"Fixture output {i:03d}: deterministic text, **Markdown**, 日本語の表示。\n\n"
                    for i in range(200)
                ),
            },
        )
        command = [
            sys.executable,
            str(TOOLS / "replay_codex.py"),
            "--transcript",
            str(base / "transcript.ndjson"),
            "--settings",
            str(base / "mock-settings.json"),
        ]
        wrapper.write_text("#!/bin/sh\nexec " + shlex.join(command) + ' "$@"\n')
        wrapper.chmod(0o700)
        write_json(
            base / "home/userdata/settings.json",
            {
                "providerInstances": {
                    "codex": {
                        "driver": "codex",
                        "enabled": True,
                        "config": {
                            "setupMode": "existing",
                            "binaryPath": str(wrapper),
                            "homePath": str(provider),
                        },
                    },
                    **{
                        driver: {"driver": driver, "enabled": False}
                        for driver in [
                            "claudeAgent",
                            "cursor",
                            "grok",
                            "fx",
                            "opencode",
                            "antigravity",
                            "kiro",
                            "pi",
                            "muse",
                            "acpRegistry",
                        ]
                    },
                },
            },
        )
    independent(generation / "reference/workspace", generation / "native/workspace")


def issue(t3, home):
    token = quiet(
        [
            t3,
            "auth",
            "session",
            "issue",
            "--ttl",
            "12h",
            "--label",
            "parity-slot",
            "--token-only",
            "--base-dir",
            str(home),
        ],
        text=True,
    ).strip()
    if not token or "\n" in token:
        raise SlotError("Fixture token issue returned invalid output")
    return token


def mutate(environment, command):
    request = Request(
        environment["httpBaseUrl"] + "/api/projects/mutate",
        data=json.dumps(
            command | {"commandId": str(uuid.uuid4()), "projectId": PROJECT}
        ).encode(),
        headers={
            "Authorization": "Bearer " + environment["token"],
            "Content-Type": "application/json",
            "x-t3-orchestration-protocol": "2",
        },
    )
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def verify_credentials(configurations):
    reference, native = configurations["reference"], configurations["native"]
    if reference["token"] == native["token"]:
        raise SlotError("Fixture credentials must differ")
    for target, other in [(reference, native), (native, reference)]:
        request = Request(
            target["httpBaseUrl"] + "/api/auth/websocket-ticket",
            method="POST",
            headers={"Authorization": "Bearer " + other["token"]},
        )
        try:
            with urlopen(request, timeout=10):
                raise SlotError("A fixture accepted the other server's credentials")
        except HTTPError as error:
            if error.code not in [401, 403]:
                raise SlotError("Cross-server authentication check failed") from None


async def verify_provider(environment):
    async with FixtureRpc(environment) as rpc:
        await rpc.call("server.refreshProviders", {})
        async with asyncio.timeout(60):
            while True:
                config = await rpc.call("server.getConfig", {})
                enabled = [
                    provider
                    for provider in config["providers"]
                    if provider.get("enabled", True)
                ]
                if any(provider["instanceId"] != "codex" for provider in enabled):
                    raise SlotError("Only the local mock provider may be enabled")
                if (
                    len(enabled) == 1
                    and enabled[0].get("installed")
                    and enabled[0].get("models")
                ):
                    return
                await asyncio.sleep(0.2)


async def set_theme(manifest, theme):
    from cdp import Cdp
    from native import Native
    from run import invoke, wait_for

    async with Cdp(manifest["endpoint"]) as reference:
        if await reference.appearance() != theme:
            await reference.click("button", "Settings")
            await wait_for(reference, {"role": "treeitem", "name": "Appearance"})
            await reference.click("treeitem", "Appearance")
            await wait_for(reference, {"role": "button", "name": f"Use {theme} mode"})
            await reference.click("button", f"Use {theme} mode")
            async with asyncio.timeout(10):
                while await reference.appearance() != theme:
                    await asyncio.sleep(0.1)
            await wait_for(reference, {"role": "button", "name": "Close"})
            await reference.click("button", "Close")
        if await reference.appearance() != theme:
            raise SlotError("Reference appearance did not change")
    native = Native(manifest["session"])
    settings_path = Path(native.session["home"]) / "userdata/client-settings.json"
    settings = read_json(settings_path) if settings_path.exists() else {}
    if settings.get("gpuiAppearance", "dark") != theme:
        await invoke(native, "click", "button", "Settings")
        await wait_for(native, {"role": "tree item", "name": "Appearance"})
        await invoke(native, "click", "tree item", "Appearance")
        await wait_for(native, {"role": "toggle button", "name": f"Use {theme} mode"})
        await invoke(native, "click", "toggle button", f"Use {theme} mode")
        await invoke(native, "click", "button", "Close")
    deadline = time.monotonic() + 10
    while True:
        settings = read_json(settings_path) if settings_path.exists() else {}
        if settings.get("gpuiAppearance", "dark") == theme:
            break
        if time.monotonic() >= deadline:
            raise SlotError("Native appearance did not change")
        await asyncio.sleep(0.1)


def worker(slot_path):
    state_path = slot_path / "state.json"
    state = read_json(state_path)
    generation = Path(state["generation"])
    options = state["options"]
    state["supervisor"] = process_identity(os.getpid())
    children = []
    launches = {}
    stopping = False

    def stop(*_):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    env = os.environ.copy()
    env[MARKER] = state["owner"]
    env["DISPLAY"] = state["display"]
    for key in [
        "WAYLAND_DISPLAY",
        "SESSION_MANAGER",
        "DESKTOP_AUTOSTART_ID",
        "DBUS_SESSION_BUS_ADDRESS",
        "T3CODE_HOME",
        "T3CODE_PORT",
        "T3CODE_AUTH_TOKEN",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
    ]:
        env.pop(key, None)
    isolated_home = generation / "os-home"
    isolated_home.mkdir()
    env.update(
        {
            "HOME": str(isolated_home),
            "XDG_CONFIG_HOME": str(isolated_home / "config"),
            "XDG_STATE_HOME": str(isolated_home / "state"),
            "XDG_CACHE_HOME": str(isolated_home / "cache"),
            "GIT_CEILING_DIRECTORIES": str(generation),
        }
    )

    def save(stage):
        state["stage"] = stage
        write_json(state_path, state)

    def launch(name, command, extra=None):
        process = subprocess.Popen(
            command,
            env=env | (extra or {}),
            cwd=generation,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        children.append(process)
        launches[name] = (process, command, extra)
        state.setdefault("processes", {})[name] = process_identity(process.pid)
        save(name)
        return process

    def wait(predicate, timeout=120):
        deadline = time.monotonic() + timeout
        while True:
            if stopping:
                raise InterruptedError("Slot startup cancelled")
            if any(child.poll() is not None for child in children):
                raise SlotError("A slot process exited during startup")
            try:
                if predicate():
                    return
            except (URLError, ConnectionError, TimeoutError):
                pass
            if time.monotonic() >= deadline:
                raise TimeoutError("Slot startup timed out")
            time.sleep(0.1)

    try:
        save("fixture")
        fixture(generation, options)
        rpc_env = {}
        if options.get("pr_rpc_fixture"):
            rpc_env = {
                "NODE_OPTIONS": "--require=" + str(TOOLS / "pr_rpc_fixture.cjs"),
                "T3_PARITY_PR_FIXTURE": options["pr_rpc_fixture"],
            }
        ports = state["ports"]
        require_free(ports, state["display"])
        launch(
            "xvfb",
            [
                "Xvfb",
                state["display"],
                "-nolisten",
                "tcp",
                "-noreset",
                "-screen",
                "0",
                "1920x1080x24",
            ],
        )
        wait(lambda: Path(f"/tmp/.X11-unix/X{state['display'][1:]}").exists(), 10)
        launch(
            "window-manager",
            [
                "dbus-run-session",
                "--",
                "xfwm4",
                "--compositor=off",
                "--sm-client-disable",
            ],
        )
        launch(
            "native-server",
            [
                options["t3_binary"],
                "serve",
                "--port",
                str(ports["native"]),
                "--host",
                "127.0.0.1",
                "--base-dir",
                str(generation / "native/home"),
                "--no-browser",
                str(generation / "native/workspace"),
            ],
            rpc_env | {"T3CODE_HOME": str(generation / "native/home")},
        )
        launch(
            "reference-electron",
            [
                "dbus-run-session",
                "--",
                options["reference_binary"],
                "--password-store=basic",
                "--force-device-scale-factor=1",
                "--remote-debugging-address=127.0.0.1",
                f"--remote-debugging-port={ports['cdp']}",
            ],
            rpc_env
            | {
                "T3CODE_HOME": str(generation / "reference/home"),
                "T3CODE_PORT": str(ports["reference"]),
                "XDG_CONFIG_HOME": str(generation / "electron-profile"),
            },
        )
        configurations = {}
        for target in ["reference", "native"]:
            server_origin = f"http://127.0.0.1:{ports[target]}"
            save(target + "-authentication")
            wait(
                lambda server_origin=server_origin: environment_descriptor(
                    server_origin
                )
            )
            environment = {
                "label": "UI parity",
                "httpBaseUrl": server_origin,
                "token": issue(options["t3_binary"], generation / target / "home"),
            }
            configurations[target] = environment
            write_json(
                generation / f"{target}-environments.json",
                {"environments": [environment]},
            )
            mutate(
                environment,
                {
                    "type": "project.create",
                    "title": "UI parity",
                    "workspaceRoot": str(generation / target / "workspace"),
                },
            )
            mutate(
                environment,
                {
                    "type": "project.update",
                    "defaultModelSelection": {
                        "instanceId": "codex",
                        "model": "gpt-6-luna",
                    },
                },
            )
        verify_credentials(configurations)
        inspected = {
            target: inspect_environments(
                Path(options["native_binary"]),
                generation / f"{target}-environments.json",
            )[0]
            for target in ["reference", "native"]
        }
        distinct(
            inspected["reference"]["origin"],
            [inspected["native"]],
            inspected["reference"]["environment_id"],
        )
        session = generation / "desktop/native-session.json"
        session.parent.mkdir()
        launch(
            "native-desktop",
            [
                "dbus-run-session",
                "--",
                sys.executable,
                str(TOOLS / "private_session.py"),
                "--binary",
                options["native_binary"],
                "--config",
                str(generation / "native-environments.json"),
                "--session",
                str(session),
            ],
        )
        wait(session.exists)
        manifest = {
            "slot": state["slot"],
            "display": state["display"],
            "ports": ports,
            "endpoint": f"http://127.0.0.1:{ports['cdp']}",
            "session": str(session),
            "environments": inspected,
            "theme": options["theme"],
            "workspaces": {
                target: str(generation / target / "workspace") for target in inspected
            },
            "configurations": {
                target: str(generation / f"{target}-environments.json")
                for target in inspected
            },
            "generation": str(generation),
        }
        save("desktop-readiness")
        from cdp import Cdp
        from run import wait_for

        async def ready(reference_ready=True):
            for target, environment in configurations.items():
                save(target + "-provider")
                await asyncio.to_thread(
                    wait,
                    lambda environment=environment: environment_descriptor(
                        environment["httpBaseUrl"]
                    ),
                )
                await verify_provider(environment)
            if reference_ready:
                save("reference-accessibility")
                async with Cdp(manifest["endpoint"]) as reference:
                    actual = origin(await reference.server_origin())
                    if actual != inspected["reference"]["origin"]:
                        raise SlotError("Electron connected to an unexpected origin")
                    distinct(
                        actual,
                        [inspected["native"]],
                        environment_descriptor(actual)["environmentId"],
                    )
                    await wait_for(reference, {"role": "button", "name": "New thread"})
            save("native-accessibility")
            with (generation / "readiness.log").open("a") as log:
                await asyncio.to_thread(
                    subprocess.run,
                    [
                        sys.executable,
                        "-c",
                        (
                            "import asyncio,sys; from native import Native; from run import wait_for; "
                            "asyncio.run(wait_for(Native(sys.argv[1]), {'role':'button','name':'New thread'}))"
                        ),
                        str(session),
                    ],
                    cwd=TOOLS,
                    stdout=log,
                    stderr=log,
                    timeout=120,
                    check=True,
                )
            if reference_ready:
                save("theme")
                with (generation / "readiness.log").open("a") as log:
                    await asyncio.to_thread(
                        subprocess.run,
                        [
                            sys.executable,
                            "-c",
                            (
                                "import asyncio,json,sys; from env import set_theme; "
                                "asyncio.run(set_theme(json.loads(sys.argv[1]), sys.argv[2]))"
                            ),
                            json.dumps(manifest),
                            options["theme"],
                        ],
                        cwd=TOOLS,
                        stdout=log,
                        stderr=log,
                        timeout=120,
                        check=True,
                    )

        asyncio.run(ready())
        write_json(slot_path / "manifest.json", manifest)
        state["status"] = "ready"
        save("ready")
        while not stopping:
            restart_path = generation / "restart.json"
            if restart_path.exists():
                request = read_json(restart_path)
                state["status"] = "restarting"
                save("restart")
                names = ["native-desktop"]
                if request["target"] == "both":
                    names.append("reference-electron")
                for name in names:
                    process, command, extra = launches[name]
                    identity = state["processes"][name]
                    if process_identity(process.pid) != identity:
                        raise SlotError("Restart target identity changed")
                    if identity not in owned_processes(state["owner"]):
                        raise SlotError("Restart target is not owned by this slot")
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
                    children.remove(process)
                    if name == "native-desktop":
                        session.unlink(missing_ok=True)
                    launch(name, command, extra)
                wait(session.exists)
                asyncio.run(ready(reference_ready=request["target"] == "both"))
                restart_path.unlink()
                state["restart_completed"] = request["id"]
                state["status"] = "ready"
                save("ready")
            if any(child.poll() is not None for child in children):
                raise SlotError("A slot process exited")
            time.sleep(0.2)
    except BaseException as error:
        state["status"] = "failed"
        state["error"] = (
            str(error) if isinstance(error, SlotError) else type(error).__name__
        )
        state["failure_trace"] = [
            {
                "file": Path(frame.filename).name,
                "line": frame.lineno,
                "function": frame.name,
            }
            for frame in traceback.extract_tb(error.__traceback__)
        ]
        save(state["stage"])
        raise
    finally:
        stop_owned(state["owner"])
        for child in children:
            child.wait()
        if state["status"] != "failed":
            state["status"] = "stopped"
        write_json(state_path, state)


def down(slot_path):
    state_path = slot_path / "state.json"
    if not state_path.exists():
        return
    state = read_json(state_path)
    supervisor = state.get("supervisor")
    if supervisor and process_identity(supervisor["pid"]) == supervisor:
        os.kill(supervisor["pid"], signal.SIGTERM)
        deadline = time.monotonic() + 30
        while (
            process_identity(supervisor["pid"]) == supervisor
            and time.monotonic() < deadline
        ):
            time.sleep(0.1)
        if process_identity(supervisor["pid"]) == supervisor:
            os.kill(supervisor["pid"], signal.SIGKILL)
    stop_owned(state["owner"])
    wait_free(state["ports"], state["display"])
    state = read_json(state_path)
    state["status"] = "stopped"
    write_json(state_path, state)


def up(slot_path, slot, options):
    state_path = slot_path / "state.json"
    if state_path.exists():
        old = read_json(state_path)
        supervisor = old.get("supervisor")
        if (
            owned_processes(old["owner"])
            or supervisor
            and process_identity(supervisor["pid"]) == supervisor
        ):
            raise SlotError("Slot has running processes; use down or reset first")
    ports, display = allocation(slot)
    require_free(ports, display)
    for key in ["native_binary", "t3_binary", "reference_binary"]:
        if not os.access(options[key], os.X_OK):
            raise SlotError(f"Executable is unavailable: {key}")
    (slot_path / "manifest.json").unlink(missing_ok=True)
    generation = slot_path / ("run-" + uuid.uuid4().hex)
    generation.mkdir(mode=0o700)
    write_json(
        state_path,
        {
            "slot": slot,
            "owner": uuid.uuid4().hex,
            "ports": ports,
            "display": display,
            "generation": str(generation),
            "options": options,
            "status": "starting",
        },
    )
    process = subprocess.Popen(
        [sys.executable, str(TOOLS / "env.py"), "_worker", "--slot", str(slot)],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    try:
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            state = read_json(state_path)
            if state["status"] == "ready":
                return
            if state["status"] == "failed" or process.poll() is not None:
                raise SlotError(
                    f"Slot startup failed at {state.get('stage', 'launch')} ({state.get('error', 'process exit')})"
                )
            time.sleep(0.2)
        raise TimeoutError("Slot startup timed out")
    except BaseException:
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        down(slot_path)
        raise


def status(slot_path, slot):
    path = slot_path / "state.json"
    if not path.exists():
        return {"slot": slot, "status": "absent"}
    state = read_json(path)
    supervisor = state.get("supervisor")
    alive = bool(supervisor and process_identity(supervisor["pid"]) == supervisor)
    processes = owned_processes(state["owner"])
    return {
        "slot": slot,
        "status": state["status"]
        if alive or not processes and state["status"] == "stopped"
        else "orphaned"
        if processes
        else "stopped",
        "stage": state.get("stage"),
        "supervisor_alive": alive,
        "processes": processes,
        "ports": state["ports"],
        "display": state["display"],
        "manifest": str(slot_path / "manifest.json"),
    }


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(
        description="Own independent T3 parity slots on Linux"
    )
    commands = parser.add_subparsers(dest="operation", required=True)
    parsers = {}
    for name in ["up", "down", "status", "theme", "reset", "restart", "_worker"]:
        command = commands.add_parser(name)
        command.add_argument("--slot", required=True, type=int)
        parsers[name] = command
    parsers["theme"].add_argument("--theme", choices=["dark", "light"], required=True)
    parsers["restart"].add_argument(
        "--target", choices=["native", "both"], default="both"
    )
    launch = parsers["up"]
    launch.add_argument("--theme", choices=["dark", "light"], default="dark")
    launch.add_argument(
        "--native-binary", type=Path, default=REPO / "target/debug/t3code-gpui"
    )
    launch.add_argument("--t3-binary", type=Path, default=Path(T3))
    launch.add_argument("--reference-binary", type=Path, default=Path(ELECTRON))
    launch.add_argument(
        "--reference-source",
        type=Path,
        default=main_tree() / ".claude-dev/scratch/parity/reference",
    )
    launch.add_argument("--pr-rpc-fixture", type=Path)
    launch.add_argument("--stream-ms", type=int, default=15000)
    args = parser.parse_args()
    if not 1 <= args.slot <= 100:
        parser.error("Slot must be between 1 and 100")
    if args.operation == "up" and args.stream_ms < 0:
        parser.error("Stream duration must be nonnegative")
    slot_path = main_tree() / ".claude-dev/parity/slots" / str(args.slot)
    slot_path.mkdir(parents=True, exist_ok=True, mode=0o700)
    if args.operation == "status":
        print(json.dumps(status(slot_path, args.slot), indent=2))
        return
    if args.operation == "_worker":
        with (slot_path / "owner.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            worker(slot_path)
        return
    with (slot_path / "control.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.operation == "up":
            options = {
                key: str(getattr(args, key).resolve())
                for key in [
                    "native_binary",
                    "t3_binary",
                    "reference_binary",
                    "reference_source",
                ]
            }
            if args.pr_rpc_fixture is not None:
                options["pr_rpc_fixture"] = str(
                    args.pr_rpc_fixture.resolve(strict=True)
                )
            options.update(theme=args.theme, stream_ms=args.stream_ms)
            up(slot_path, args.slot, options)
        elif args.operation == "down":
            down(slot_path)
        elif args.operation == "reset":
            options = read_json(slot_path / "state.json")["options"]
            if (slot_path / "manifest.json").exists():
                options["theme"] = read_json(slot_path / "manifest.json")["theme"]
            down(slot_path)
            up(slot_path, args.slot, options)
        elif args.operation == "theme":
            if status(slot_path, args.slot)["status"] != "ready":
                raise SlotError("Theme requires a ready slot")
            manifest = read_json(slot_path / "manifest.json")
            asyncio.run(set_theme(manifest, args.theme))
            manifest["theme"] = args.theme
            write_json(slot_path / "manifest.json", manifest)
        elif args.operation == "restart":
            if status(slot_path, args.slot)["status"] != "ready":
                raise SlotError("Restart requires a ready slot")
            state = read_json(slot_path / "state.json")
            request_id = uuid.uuid4().hex
            write_json(
                Path(state["generation"]) / "restart.json",
                {"id": request_id, "target": args.target},
            )
            deadline = time.monotonic() + 180
            while True:
                state = read_json(slot_path / "state.json")
                if state.get("restart_completed") == request_id:
                    break
                if state["status"] == "failed" or time.monotonic() >= deadline:
                    raise SlotError("Slot restart did not complete")
                time.sleep(0.2)
        print(json.dumps(status(slot_path, args.slot), indent=2))


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as error:
        detail = str(error) if isinstance(error, SlotError) else type(error).__name__
        print(f"Parity environment failed: {detail}", file=sys.stderr)
        raise SystemExit(1) from None
