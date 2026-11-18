import asyncio
import json
import socket
import subprocess
import uuid
from pathlib import Path

from fixture_rpc import FixtureRpc

PROJECT = "11111111-1111-4111-8111-111111111111"
THREAD = "88888888-8888-4888-8888-888888888888"


async def prepare(manifest, kind):
    if kind not in {"panel", "pr-guide", "services", "turn"}:
        raise ValueError(f"Unknown panel fixture: {kind}")
    for side, path in manifest["configurations"].items():
        await asyncio.to_thread(configure_workspace, manifest["workspaces"][side])
        environment = json.loads(Path(path).read_text())["environments"][0]
        async with FixtureRpc(environment) as rpc:
            await rpc.call(
                "orchestration.dispatchCommand",
                {
                    "commandId": str(uuid.uuid4()),
                    "type": "thread.create",
                    "createdBy": "user",
                    "creationSource": "web",
                    "threadId": THREAD,
                    "projectId": PROJECT,
                    "title": "Panel fixture",
                    "modelSelection": {"instanceId": "codex", "model": "gpt-6-luna"},
                    "runtimeMode": "full-access",
                    "interactionMode": "default",
                    "branch": "feature/parity",
                    "worktreePath": None,
                },
            )
            if kind == "pr-guide":
                await rpc.call(
                    "server.updateSettings", {"patch": {"reviewGuidesEnabled": True}}
                )
            if kind == "services":
                await rpc.call(
                    "scheduledTasks.upsert",
                    {
                        "id": "parity-automation",
                        "title": "Parity automation",
                        "prompt": "Say parity automation complete.",
                        "enabled": False,
                        "schedule": {"type": "interval", "everyMs": 3600000},
                        "projectId": PROJECT,
                        "threadId": THREAD,
                        "threadMode": "continue",
                        "workspaceStrategy": {"type": "root"},
                        "modelSelection": {
                            "instanceId": "codex",
                            "model": "gpt-6-luna",
                        },
                        "runtimeMode": "full-access",
                        "interactionMode": "default",
                        "createdBy": "user",
                        "creationSource": "web",
                    },
                )
            if kind == "services":
                port = manifest["ports"][side] + 3
                with socket.socket() as probe:
                    probe.bind(("127.0.0.1", port))
                terminal = "parity-server"
                await rpc.call(
                    "terminal.open",
                    {
                        "threadId": THREAD,
                        "terminalId": terminal,
                        "cwd": manifest["workspaces"][side],
                        "cols": 120,
                        "rows": 30,
                    },
                )
                command = f' env -u NODE_OPTIONS node -e \'require("http").createServer((q,r)=>r.end("Parity server")).listen({port},"127.0.0.1")\'\r'
                await rpc.call(
                    "terminal.write",
                    {"threadId": THREAD, "terminalId": terminal, "data": command},
                )
        workspace = Path(manifest["workspaces"][side])
        for filename, content in {
            "panel-fixture.txt": "An uncommitted fixture for the diff panel.\n",
            "zz-selected.txt": "Selected parity file.\n",
        }.items():
            (workspace / filename).write_text(content)
    if kind == "turn":
        return await asyncio.gather(
            *(
                prepare_turn(manifest, side, path)
                for side, path in manifest["configurations"].items()
            )
        )


async def prepare_turn(manifest, side, path):
    workspace = Path(manifest["workspaces"][side])
    await asyncio.to_thread(
        subprocess.run,
        ["git", "-C", str(workspace), "branch", "comparison-base"],
        check=True,
    )
    environment = json.loads(Path(path).read_text())["environments"][0]
    async with FixtureRpc(environment) as rpc:
        await rpc.call(
            "orchestration.dispatchCommand",
            {
                "commandId": str(uuid.uuid4()),
                "type": "message.dispatch",
                "threadId": THREAD,
                "messageId": str(uuid.uuid4()),
                "text": "Create the deterministic turn diff fixture.",
                "attachments": [],
                "createdBy": "user",
                "creationSource": "web",
                "deliveryIntent": "auto",
                "dispatchMode": {"type": "start_immediately"},
            },
        )
        await wait_running(rpc)
        await wait_baseline(workspace)
        (workspace / "turn-fixture.txt").write_text(
            "A file changed while the mock turn was running.\n"
        )
        await wait_checkpoint(rpc, 1)
        result = await rpc.call(
            "orchestration.getTurnDiff",
            {
                "threadId": THREAD,
                "fromTurnCount": 0,
                "toTurnCount": 1,
                "ignoreWhitespace": True,
            },
        )
        if "turn-fixture.txt" not in result["diff"]:
            raise AssertionError("The mock turn did not capture the fixture file")
        return {"side": side, "turn_diff": result}


def configure_workspace(workspace):
    for key, value in [
        ("user.name", "Parity fixture"),
        ("user.email", "parity@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ]:
        subprocess.run(
            ["git", "-C", workspace, "config", "--local", key, value], check=True
        )


def git_state(workspace):
    return {
        key: subprocess.check_output(["git", "-C", workspace, *args], text=True).strip()
        for key, args in {
            "tree": ["rev-parse", "HEAD^{tree}"],
            "subject": ["log", "-1", "--format=%s"],
            "status": ["status", "--porcelain"],
        }.items()
    }


async def verify_git(manifest, expected):
    for _ in range(50):
        states = {
            side: await asyncio.to_thread(git_state, path)
            for side, path in manifest["workspaces"].items()
        }
        passed = (
            all(
                all(state[key] == value for key, value in expected.items())
                for state in states.values()
            )
            and states["reference"]["tree"] == states["native"]["tree"]
        )
        if passed:
            break
        await asyncio.sleep(0.2)
    return {
        "status": "passed" if passed else "failed",
        "targets": states,
        "expected": expected,
    }


async def wait_baseline(workspace):
    async with asyncio.timeout(60):
        while True:
            refs = await asyncio.to_thread(
                subprocess.check_output,
                [
                    "git",
                    "-C",
                    str(workspace),
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/t3/orchestration-v2/checkpoints/",
                ],
                text=True,
            )
            if any(ref.endswith("/ordinal/0") for ref in refs.splitlines()):
                return
            await asyncio.sleep(0.2)


async def wait_checkpoint(rpc, ordinal):
    async with asyncio.timeout(60):
        while True:
            projection = await rpc.call(
                "orchestration.getThreadProjection", {"threadId": THREAD}
            )
            if any(
                checkpoint["status"] == "ready"
                and checkpoint["appRunOrdinal"] == ordinal
                for checkpoint in projection["checkpoints"]
            ):
                return
            await asyncio.sleep(0.2)


async def wait_running(rpc):
    async with asyncio.timeout(60):
        while True:
            projection = await rpc.call(
                "orchestration.getThreadProjection", {"threadId": THREAD}
            )
            if any(
                attempt["status"] == "running" for attempt in projection["attempts"]
            ):
                return
            if any(run["completedAt"] is not None for run in projection["runs"]):
                raise AssertionError(
                    "The mock run finished before the fixture file was written"
                )
            await asyncio.sleep(0.2)


async def verify_automation(manifest, expected):
    states = {}
    for side, path in manifest["configurations"].items():
        environment = json.loads(Path(path).read_text())["environments"][0]
        async with FixtureRpc(environment) as rpc:
            await rpc.socket.send(
                json.dumps(
                    {
                        "_tag": "Request",
                        "id": "tasks",
                        "tag": "scheduledTasks.subscribe",
                        "headers": [],
                        "payload": {},
                    }
                )
            )
            try:
                async with asyncio.timeout(10):
                    while True:
                        reply = json.loads(await rpc.socket.recv())
                        if (
                            reply.get("_tag") != "Chunk"
                            or reply.get("requestId") != "tasks"
                        ):
                            continue
                        tasks = [
                            task for value in reply["values"] for task in value["tasks"]
                        ]
                        task = next(
                            (task for task in tasks if task["id"] == expected["id"]), {}
                        )
                        states[side] = {key: task.get(key) for key in expected}
                        if states[side] == expected:
                            break
            except TimeoutError:
                states.setdefault(side, {})
    return {
        "status": "passed"
        if all(state == expected for state in states.values())
        else "failed",
        "targets": states,
        "expected": expected,
    }
