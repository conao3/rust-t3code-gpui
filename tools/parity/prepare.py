import asyncio
import json
import subprocess
import time
import uuid
from datetime import UTC, datetime, timedelta
from pathlib import Path

from env import PROJECT
from fixture_rpc import FixtureRpc

MODEL = {"instanceId": "codex", "model": "gpt-6-luna"}


def identity(name):
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "t3-parity/" + name))


class Preparation:
    def __init__(self, rpc):
        self.rpc = rpc
        self.commands = []

    async def call(self, method, payload):
        self.commands.append({"method": method, "payload": payload})
        return await self.rpc.call(method, payload)

    async def dispatch(self, kind, **payload):
        return await self.call(
            "orchestration.dispatchCommand",
            {
                "commandId": identity(f"command/{len(self.commands)}"),
                "type": kind,
                **payload,
            },
        )

    async def thread(self, title, model=None, branch=None):
        thread = identity(title)
        await self.dispatch(
            "thread.create",
            threadId=thread,
            projectId=PROJECT,
            createdBy="user",
            creationSource="web",
            title=title,
            modelSelection=model or MODEL,
            runtimeMode="full-access",
            interactionMode="default",
            branch=branch,
            worktreePath=None,
        )
        return thread

    async def message(self, thread, text, queued=False):
        await self.dispatch(
            "message.dispatch",
            threadId=thread,
            messageId=identity(f"message/{len(self.commands)}"),
            text=text,
            attachments=[],
            createdBy="user",
            creationSource="web",
            dispatchMode={
                "type": "queue_after_active" if queued else "start_immediately"
            },
        )

    async def wait(self, thread, predicate):
        deadline = time.monotonic() + 60
        while True:
            projection = await self.rpc.call(
                "orchestration.getThreadProjection", {"threadId": thread}
            )
            if predicate(projection):
                return projection
            if time.monotonic() >= deadline:
                raise RuntimeError(
                    f"Fixture projection did not reach requested state: {thread}"
                )
            await asyncio.sleep(0.1)

    async def completed(self, title="Fixture response", prompts=None):
        thread = await self.thread(title)
        for ordinal, text in enumerate(
            prompts or ["Run the deterministic comparison fixture."], 1
        ):
            await self.message(thread, text)
            await self.wait(
                thread,
                lambda p, ordinal=ordinal: (
                    len(p["runs"]) == ordinal
                    and all(r["status"] == "completed" for r in p["runs"])
                ),
            )
        await self.dispatch("thread.metadata.update", threadId=thread, title=title)
        await self.dispatch(
            "thread.visit",
            threadId=thread,
            visitedAt=datetime.now(UTC).isoformat().replace("+00:00", "Z"),
        )
        return thread

    async def apply(self, helper, args):
        if helper == "threads":
            for title in args["titles"]:
                await self.thread(title, args.get("model"), args.get("branch"))
        elif helper == "completed":
            await self.completed(**args)
        elif helper == "queue":
            thread = await self.thread("Composer queue fixture")
            await self.message(thread, "Composer queue start")
            await self.wait(
                thread, lambda p: any(r["status"] == "running" for r in p["runs"])
            )
            for count, text in enumerate(args["messages"], 1):
                await self.message(thread, text, queued=True)
                await self.wait(
                    thread,
                    lambda p, count=count: (
                        sum(run["status"] == "queued" for run in p["runs"]) == count
                    ),
                )
            await self.dispatch("thread.stop", threadId=thread)
            await self.wait(
                thread,
                lambda p: (
                    p["runs"]
                    and not any(
                        r["status"] in ["running", "starting"] for r in p["runs"]
                    )
                    and sum(
                        run["status"] == "queued" and run["queueHeld"]
                        for run in p["runs"]
                    )
                    == len(args["messages"])
                ),
            )
        elif helper == "running":
            thread = await self.thread(args["title"])
            await self.message(thread, "Keep the deterministic fixture running.")
            await self.wait(
                thread, lambda p: any(r["status"] == "running" for r in p["runs"])
            )
        elif helper == "lineage_stop":
            thread = await self.thread("Lineage stop fixture")
            await self.message(thread, "Keep the parent mock running.")
            projection = await self.wait(
                thread, lambda p: any(r["status"] == "running" for r in p["runs"])
            )
            run = next(r for r in projection["runs"] if r["status"] == "running")
            await self.dispatch(
                "delegated_task.request",
                createdBy="user",
                creationSource="web",
                parentThreadId=thread,
                parentRunId=run["id"],
                parentNodeId=run["rootNodeId"],
                task="Keep the child mock running.",
                title="Subagent: /root/parity_stop",
                modelSelection=MODEL,
                runtimeMode="full-access",
                interactionMode="default",
                workspaceStrategy={"type": "root"},
            )
            await self.wait(
                thread, lambda p: any(a["status"] == "running" for a in p["subagents"])
            )
        elif helper == "image":
            from io import BytesIO
            from urllib.request import Request, urlopen

            from PIL import Image

            image = BytesIO()
            Image.new("RGB", (320, 160), (38, 126, 224)).save(image, format="PNG")
            png = image.getvalue()
            attachment = {
                "type": "image",
                "name": "composer-fixture.png",
                "mimeType": "image/png",
                "sizeBytes": len(png),
            }
            issued = await self.call("attachments.createUploadUrl", attachment)
            request = Request(
                self.rpc.environment["httpBaseUrl"] + issued["relativeUrl"],
                data=png,
                headers={
                    "Authorization": "Bearer " + self.rpc.environment["token"],
                    "Content-Type": "image/png",
                },
                method="POST",
            )

            def upload():
                with urlopen(request, timeout=20) as response:
                    response.read()

            await asyncio.to_thread(upload)
            thread = await self.thread("Composer image fixture")
            await self.dispatch(
                "message.dispatch",
                threadId=thread,
                messageId=identity("image-message"),
                text="Image fixture",
                attachments=[attachment | {"id": issued["attachmentId"]}],
                createdBy="user",
                creationSource="web",
                dispatchMode={"type": "start_immediately"},
            )
            await self.wait(thread, lambda p: bool(p["runs"]))
            await self.dispatch("thread.stop", threadId=thread)
            await self.wait(
                thread, lambda p: not any(r["status"] == "running" for r in p["runs"])
            )
        elif helper == "fork":
            thread = await self.completed()
            child = identity("Fixture response fork")
            await self.dispatch(
                "thread.fork",
                createdBy="user",
                creationSource="web",
                sourceThreadId=thread,
                targetThreadId=child,
                sourcePoint={"type": "latest_stable"},
                title="Fixture response fork",
            )
            await self.dispatch(
                "thread.visit",
                threadId=child,
                visitedAt=datetime.now(UTC).isoformat().replace("+00:00", "Z"),
            )
        elif helper == "snooze":
            await self.dispatch(
                "thread.snooze",
                threadId=identity(args["title"]),
                snoozedUntil=(datetime.now(UTC) + timedelta(hours=8))
                .isoformat()
                .replace("+00:00", "Z"),
            )
        elif helper == "settle":
            await self.dispatch("thread.settle", threadId=identity(args["title"]))
        elif helper == "archive":
            for title in args["titles"]:
                thread = await self.thread(title)
                await self.dispatch("thread.archive", threadId=thread)
        elif helper == "usage":
            now = datetime.now(UTC)
            result = await self.call(
                "server.getUsageSummary",
                {
                    "sinceDay": (now - timedelta(days=7)).date().isoformat(),
                    "untilDay": (now + timedelta(days=1)).date().isoformat(),
                    "timeZone": "UTC",
                    "resolution": "day",
                    "awaitRefresh": True,
                },
            )
            if not result["buckets"]:
                raise RuntimeError("Mock usage records were not discovered")
        elif helper == "settings":
            await self.call("server.updateSettings", {"patch": args})
        elif helper == "provider":
            await self.call(
                "server.updateSettings",
                {
                    "patch": {},
                    "providerInstanceMutation": {"operation": "create", **args},
                },
            )
        else:
            raise ValueError(f"Unknown fixture helper: {helper}")


async def prepare(manifest, declarations, output):
    output.mkdir(parents=True)
    results = {}
    for target, configuration in manifest["configurations"].items():
        environment = json.loads(Path(configuration).read_text())["environments"][0]
        async with FixtureRpc(environment) as rpc:
            preparation = Preparation(rpc)
            try:
                for declaration in declarations:
                    helper, args = declaration["helper"], declaration["arguments"]
                    if helper == "mock":
                        path = (
                            Path(manifest["generation"]) / target / "mock-settings.json"
                        )
                        settings = json.loads(path.read_text())
                        if set(args) - {"stream_ms", "text", "usage_models"}:
                            raise ValueError("Unknown mock setting")
                        path.write_text(json.dumps(settings | args) + "\n")
                        continue
                    if helper in ["actions", "reload_clients"]:
                        continue
                    if helper == "panel":
                        from panel_fixture import prepare as prepare_panel

                        result = await prepare_panel(
                            manifest | {"configurations": {target: configuration}},
                            args["kind"],
                        )
                        (output / f"{target}-panel.json").write_text(
                            json.dumps(result, indent=2) + "\n"
                        )
                        continue
                    if helper == "workspace":
                        workspace = Path(manifest["workspaces"][target])
                        for name, content in args["files"].items():
                            path = (workspace / name).resolve()
                            if not path.is_relative_to(workspace.resolve()):
                                raise ValueError("Fixture file escapes the workspace")
                            path.parent.mkdir(parents=True, exist_ok=True)
                            path.write_text(content)
                        for key, value in [
                            ("user.name", "Parity fixture"),
                            ("user.email", "parity@example.invalid"),
                            ("commit.gpgsign", "false"),
                            ("core.hooksPath", "/dev/null"),
                        ]:
                            await asyncio.to_thread(
                                subprocess.run,
                                [
                                    "git",
                                    "-C",
                                    str(workspace),
                                    "config",
                                    "--local",
                                    key,
                                    value,
                                ],
                                check=True,
                            )
                    else:
                        await preparation.apply(helper, args)
                results[target] = {"status": "passed", "commands": preparation.commands}
            except Exception as error:
                results[target] = {
                    "status": "failed",
                    "error": str(error),
                    "commands": preparation.commands,
                }
                raise
            finally:
                (output / "rpc.json").write_text(json.dumps(results, indent=2) + "\n")
    return results


async def main():
    import argparse

    from cdp import Cdp
    from native import Native
    from run import execute

    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--scenario", type=Path, required=True)
    parser.add_argument("--target", choices=["reference", "native"], required=True)
    parser.add_argument("--index", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    scenario = json.loads(args.scenario.read_text())
    scenario["steps"] = scenario["prepare"][args.index]["arguments"]["steps"]
    async with Cdp(manifest["endpoint"]) as reference:
        app = reference if args.target == "reference" else Native(manifest["session"])
        if args.target == "reference":
            await app.viewport(*scenario["viewport"], manifest["display"])
        else:
            await app.viewport(*scenario["viewport"])
        rows = await execute(app, args.target, scenario, args.output)
    raise SystemExit(0 if all(row["status"] == "passed" for row in rows) else 1)


if __name__ == "__main__":
    asyncio.run(main())
