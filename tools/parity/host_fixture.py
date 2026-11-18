import argparse
import asyncio
import fcntl
import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path
from urllib.error import URLError
from urllib.request import Request, urlopen

import env
from cdp import Cdp
from fixture_validation import environment_descriptor

PROJECT_ID = "22222222-2222-4222-8222-222222222222"


def add_project(configuration, workspace):
    subprocess.run(
        [
            "git",
            "-C",
            str(workspace),
            "remote",
            "add",
            "origin",
            "https://example.invalid/parity/multi-host.git",
        ],
        check=True,
        capture_output=True,
    )
    for command in [
        {
            "type": "project.create",
            "title": "Multi host fixture",
            "workspaceRoot": str(workspace),
        },
        {
            "type": "project.update",
            "defaultModelSelection": {"instanceId": "codex", "model": "gpt-6-luna"},
        },
    ]:
        request = Request(
            configuration["httpBaseUrl"] + "/api/projects/mutate",
            data=json.dumps(
                command | {"projectId": PROJECT_ID, "commandId": str(uuid.uuid4())}
            ).encode(),
            headers={
                "Authorization": "Bearer " + configuration["token"],
                "Content-Type": "application/json",
                "x-t3-orchestration-protocol": "2",
            },
        )
        with urlopen(request, timeout=30) as response:
            json.load(response)


async def register_reference(endpoint, configuration, identity):
    connection = "parity-secondary"
    target = {
        "_tag": "BearerConnectionTarget",
        "connectionId": connection,
        "environmentId": identity,
        "label": configuration["label"],
    }
    profile = target | {
        "_tag": "BearerConnectionProfile",
        "httpBaseUrl": configuration["httpBaseUrl"],
        "wsBaseUrl": configuration["httpBaseUrl"].replace("http", "ws", 1) + "/ws",
    }
    credential = {
        "connectionId": connection,
        "credential": {
            "_tag": "BearerConnectionCredential",
            "token": configuration["token"],
        },
    }
    async with Cdp(endpoint) as app:
        catalog = await app.evaluate("window.desktopBridge.getConnectionCatalog()")
        catalog = (
            json.loads(catalog)
            if catalog
            else {
                "schemaVersion": 1,
                "targets": [],
                "profiles": [],
                "credentials": [],
                "remoteDpopTokens": [],
                "disabledEnvironmentIds": [],
            }
        )
        if any(t["environmentId"] == identity for t in catalog["targets"]):
            raise RuntimeError("Secondary environment is already registered")
        catalog["targets"].append(target)
        catalog["profiles"].append(profile)
        catalog["credentials"].append(credential)
        saved = await app.evaluate(
            "window.desktopBridge.setConnectionCatalog("
            + json.dumps(json.dumps(catalog))
            + ")"
        )
        if saved is not True:
            raise RuntimeError("Desktop did not save the secondary connection")
        await app.call("Page.reload")


def prepare(manifest_path, label, primary_label):
    if label == primary_label:
        raise ValueError("Host labels must differ")
    manifest = env.read_json(manifest_path)
    slot = manifest["slot"]
    slot_path = env.main_tree() / ".claude-dev/parity/slots" / str(slot)
    if manifest_path.resolve() != (slot_path / "manifest.json").resolve():
        raise RuntimeError("Manifest is not the owned slot manifest")
    ports, display = env.allocation(slot)
    if manifest["ports"] != ports or manifest["display"] != display:
        raise RuntimeError("Slot allocation does not match its manifest")
    with (slot_path / "control.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        state = env.read_json(slot_path / "state.json")
        if (
            state["status"] != "ready"
            or state["generation"] != manifest["generation"]
            or not env.owned_processes(state["owner"])
        ):
            raise RuntimeError("The slot must be running")
        generation = Path(manifest["generation"])
        destination = generation / "multi-host"
        destination.mkdir(mode=0o700)
        configurations = {}
        identities = {}
        workspaces = {}
        children = []
        try:
            for target, port in [
                ("reference", ports["reference"] + 3),
                ("native", ports["reference"] + 4),
            ]:
                with socket.socket() as probe:
                    probe.bind(("127.0.0.1", port))
                base = destination / target
                shutil.copytree(generation / "seed", base)
                provider = base / "provider"
                provider.mkdir()
                wrapper = base / "mock-codex"
                wrapper.write_text(
                    "#!/bin/sh\nexec "
                    + shlex.join(
                        [
                            sys.executable,
                            str(env.TOOLS / "replay_codex.py"),
                            "--transcript",
                            str(base / "transcript.ndjson"),
                            "--stream-ms",
                            str(state["options"]["stream_ms"]),
                        ]
                    )
                    + ' "$@"\n'
                )
                wrapper.chmod(0o700)
                settings = env.read_json(
                    generation / target / "home/userdata/settings.json"
                )
                settings["providerInstances"]["codex"]["config"] |= {
                    "binaryPath": str(wrapper),
                    "homePath": str(provider),
                }
                env.write_json(base / "home/userdata/settings.json", settings)
                process_environment = os.environ | {
                    env.MARKER: state["owner"],
                    "T3CODE_HOME": str(base / "home"),
                    "GIT_CEILING_DIRECTORIES": str(generation.parent),
                }
                with (base / "server.log").open("w") as log:
                    child = subprocess.Popen(
                        [
                            state["options"]["t3_binary"],
                            "serve",
                            "--port",
                            str(port),
                            "--host",
                            "127.0.0.1",
                            "--base-dir",
                            str(base / "home"),
                            "--no-browser",
                            str(base / "workspace"),
                        ],
                        env=process_environment,
                        stdout=log,
                        stderr=log,
                        start_new_session=True,
                    )
                children.append(child)
                origin = f"http://127.0.0.1:{port}"
                deadline = time.monotonic() + 60
                while True:
                    try:
                        descriptor = environment_descriptor(origin)
                        break
                    except (URLError, TimeoutError):
                        if child.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError(
                                "Secondary server did not become ready"
                            ) from None
                        time.sleep(0.1)
                configuration = {
                    "label": label,
                    "httpBaseUrl": origin,
                    "token": env.issue(state["options"]["t3_binary"], base / "home"),
                }
                configurations[target] = configuration
                identities[target] = descriptor["environmentId"]
                add_project(configuration, base / "workspace")
                primary = env.read_json(Path(manifest["configurations"][target]))[
                    "environments"
                ][0]
                workspace = generation / target / "multi-host-workspace"
                shutil.copytree(generation / "seed/workspace", workspace)
                add_project(primary, workspace)
                workspaces[target] = {
                    "primary": str(workspace),
                    "secondary": str(base / "workspace"),
                }
                for home in [generation / target / "home", base / "home"]:
                    keys = home / "userdata/keybindings.json"
                    bindings = env.read_json(keys)
                    bindings.append(
                        {
                            "key": "mod+alt+h",
                            "command": "composer.cycleHost",
                            "when": "!terminalFocus",
                        }
                    )
                    env.write_json(keys, bindings)
                asyncio.run(env.verify_provider(configuration))
            if (
                len(
                    set(identities.values())
                    | {e["environment_id"] for e in manifest["environments"].values()}
                )
                != 4
            ):
                raise RuntimeError("Expected four independent environment identities")
            env.verify_credentials(configurations)
            env.write_json(destination / "configurations.json", configurations)
            asyncio.run(
                register_reference(
                    manifest["endpoint"],
                    configurations["reference"],
                    identities["reference"],
                )
            )
            native_path = Path(manifest["configurations"]["native"])
            native = env.read_json(native_path)
            native["environments"][0]["label"] = primary_label
            native["environments"].append(configurations["native"])
            env.write_json(native_path, native)
            result = {
                "slot": slot,
                "project_id": PROJECT_ID,
                "label": label,
                "environments": {
                    target: {
                        "httpBaseUrl": configurations[target]["httpBaseUrl"],
                        "environmentId": identities[target],
                    }
                    for target in configurations
                },
                "configurations": str(destination / "configurations.json"),
                "workspaces": workspaces,
                "cycle_host_key": "ctrl+alt+h",
            }
            env.write_json(destination / "fixture.json", result)
            return result
        except BaseException:
            for child in children:
                child.terminate()
            raise


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--label", default="parity-secondary")
    parser.add_argument("--primary-label", required=True)
    args = parser.parse_args()
    result = prepare(args.manifest, args.label, args.primary_label)
    subprocess.run(
        [
            sys.executable,
            str(env.TOOLS / "env.py"),
            "restart",
            "--slot",
            str(result["slot"]),
            "--target",
            "native",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
