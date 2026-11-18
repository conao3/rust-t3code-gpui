import argparse
import asyncio
import copy
import json
import shlex
import uuid
from pathlib import Path

from fixture_rpc import FixtureRpc

PROJECT = "11111111-1111-4111-8111-111111111111"
THREAD = "77777777-7777-4777-8777-777777777777"
WORKSPACE = "/home/replay-user/t3code/packages/effect-codex-app-server"
MARKDOWN = """## Timeline fixture

Plain text, **bold**, *italic*, ~~deleted~~, and `inline code`.

> A quoted paragraph.

- First item
- Second item

| Name | Value |
| --- | --- |
| Alpha | 1 |
| Beta | 2 |

```rust
fn main() {
    println!("timeline");
}
```

[Example link](https://example.com/)

The final paragraph contains 日本語 and an emoji 🌱.
"""


def items(kind):
    if kind == "activity":
        return [
            {
                "type": "reasoning",
                "id": "fixture-reasoning",
                "summary": ["**Inspecting the fixture**\n\nCompare the timeline rows."],
                "content": [],
            },
            {
                "type": "commandExecution",
                "id": "fixture-command",
                "command": "printf 'timeline output\\n'",
                "cwd": WORKSPACE,
                "processId": "12345",
                "source": "agent",
                "status": "completed",
                "commandActions": [
                    {"type": "unknown", "command": "printf 'timeline output\\n'"}
                ],
                "aggregatedOutput": "timeline output\n",
                "exitCode": 0,
                "durationMs": 100,
            },
            {
                "type": "fileChange",
                "id": "fixture-change",
                "changes": [
                    {
                        "path": WORKSPACE + "/timeline.txt",
                        "kind": {"type": "add"},
                        "diff": "timeline fixture\n",
                    }
                ],
                "status": "completed",
            },
        ]
    if kind == "plan":
        return [
            {
                "type": "plan",
                "id": "fixture-plan",
                "text": "## Plan\n\n1. Inspect the timeline.\n2. Compare the controls.\n",
            }
        ]
    if kind == "subagent":
        return [
            {
                "type": "collabAgentToolCall",
                "id": "fixture-agent",
                "tool": "spawnAgent",
                "status": "completed",
                "senderThreadId": "fixture-parent",
                "receiverThreadIds": ["fixture-child"],
                "prompt": "Inspect the timeline fixture.",
                "model": "gpt-6-luna",
                "reasoningEffort": "medium",
                "agentsStates": {
                    "fixture-child": {
                        "status": "completed",
                        "message": "Timeline inspection complete.",
                    }
                },
            }
        ]
    return []


def transcript(source, kind):
    entries = [json.loads(line) for line in source.read_text().splitlines()]
    before = next(
        i
        for i, entry in enumerate(entries)
        if entry.get("frame", {}).get("method") == "item/started"
        and entry["frame"]["params"]["item"]["type"] == "agentMessage"
    )
    template = entries[before]["frame"]["params"]
    extra = []
    if kind in ["approval", "question"]:
        params = {
            "threadId": template["threadId"],
            "turnId": template["turnId"],
            "itemId": "fixture-request-item",
        }
        if kind == "approval":
            method = "item/commandExecution/requestApproval"
            params.update(
                command="printf 'approved output\\n'",
                cwd=WORKSPACE,
                reason="Run the timeline fixture command.",
                startedAtMs=1794960000000,
            )
        else:
            method = "item/tool/requestUserInput"
            params.update(
                isBlocking=True,
                questions=[
                    {
                        "id": "fixture-choice",
                        "header": "Timeline",
                        "question": "Which timeline should we inspect?",
                        "isOther": True,
                        "isSecret": False,
                        "options": [
                            {
                                "label": "Current thread",
                                "description": "Inspect this conversation.",
                            },
                            {
                                "label": "Side chat",
                                "description": "Inspect the side conversation.",
                            },
                        ],
                    }
                ],
            )
        extra.append(
            {
                "type": "emit_inbound",
                "label": method,
                "frame": {"id": "fixture-request", "method": method, "params": params},
            }
        )
    for item in items(kind):
        for method in ["item/started", "item/completed"]:
            params = {
                "threadId": template["threadId"],
                "turnId": template["turnId"],
                "item": copy.deepcopy(item),
                (
                    "startedAtMs" if method == "item/started" else "completedAtMs"
                ): 1794960000000,
            }
            if item["type"] == "collabAgentToolCall":
                params["item"]["senderThreadId"] = template["threadId"]
            if method == "item/started" and "status" in item:
                params["item"]["status"] = "inProgress"
            extra.append(
                {
                    "type": "emit_inbound",
                    "label": method,
                    "frame": {"method": method, "params": params},
                }
            )
    entries[before:before] = extra
    if kind == "error":
        for entry in entries:
            if entry.get("frame", {}).get("method") == "turn/completed":
                turn = entry["frame"]["params"]["turn"]
                turn["status"] = "failed"
                turn["error"] = {
                    "message": "The timeline fixture failed.",
                    "codexErrorInfo": "other",
                }
    return "".join(json.dumps(entry) + "\n" for entry in entries)


async def prepare(manifest, kind, output):
    for side in ["reference", "native"]:
        base = Path(manifest["generation"]) / side
        wrapper = base / "mock-codex"
        argv = shlex.split(wrapper.read_text().split("exec ", 1)[1].removesuffix("\n"))
        source = Path(argv[argv.index("--transcript") + 1])
        fixture = base / f"timeline-{kind}.ndjson"
        fixture.write_text(transcript(source, kind))
        body = "Timeline fixture complete.\n"
        if kind == "markdown":
            body = MARKDOWN
        elif kind == "long":
            body = (
                "\n\n".join(
                    f"Paragraph {index:03d}: a long conversation keeps the reader's scroll position."
                    for index in range(100)
                )
                + "\n"
            )
        argv[argv.index("--transcript") + 1] = str(fixture)
        settings_path = Path(argv[argv.index("--settings") + 1])
        settings = json.loads(settings_path.read_text())
        settings["text"] = body
        settings["stream_ms"] = 300
        settings_path.write_text(json.dumps(settings))
        wrapper.write_text(
            "#!/bin/sh\nexec "
            + " ".join('"$@"' if arg == "$@" else shlex.quote(arg) for arg in argv)
            + "\n"
        )
        environment = json.loads(Path(manifest["configurations"][side]).read_text())[
            "environments"
        ][0]
        async with FixtureRpc(environment) as rpc:

            async def dispatch(**command):
                return await rpc.call(
                    "orchestration.dispatchCommand",
                    {"commandId": str(uuid.uuid4()), **command},
                )

            await dispatch(
                type="thread.create",
                createdBy="user",
                creationSource="web",
                threadId=THREAD,
                projectId=PROJECT,
                title="Timeline fixture",
                modelSelection={"instanceId": "codex", "model": "gpt-6-luna"},
                runtimeMode="full-access",
                interactionMode="default",
                branch=None,
                worktreePath=None,
            )
            await dispatch(
                type="message.dispatch",
                threadId=THREAD,
                messageId=str(uuid.uuid4()),
                createdBy="user",
                creationSource="web",
                text=(
                    "**First line**\nSecond line\n\n- First item\n- Second item\n\n`inline code` and [Example link](https://example.com/)\n<mark>literal HTML</mark>"
                    if kind == "user"
                    else "Compare the timeline fixture."
                ),
                attachments=[],
                deliveryIntent="auto",
                dispatchMode={"type": "start_immediately"},
            )
            for _ in range(120):
                await asyncio.sleep(0.5)
                projection = await rpc.call(
                    "orchestration.getThreadProjection", {"threadId": THREAD}
                )
                if kind in ["approval", "question"] and any(
                    request.get("status") == "pending"
                    for request in projection.get("runtimeRequests", [])
                ):
                    break
                if projection["runs"] and all(
                    run["status"] in ["completed", "failed", "interrupted"]
                    for run in projection["runs"]
                ):
                    break
            else:
                raise RuntimeError(f"{side}: fixture did not complete")
            expected_status = "failed" if kind == "error" else "completed"
            if kind not in ["approval", "question"] and any(
                run["status"] != expected_status for run in projection["runs"]
            ):
                raise RuntimeError(f"{side}: fixture run failed")
            await dispatch(
                type="thread.metadata.update", threadId=THREAD, title="Timeline fixture"
            )
            (output / f"{side}-projection.json").write_text(
                json.dumps(projection, indent=2) + "\n"
            )
            present = {item["type"] for item in projection["turnItems"]}
            required = {
                "activity": {"reasoning", "command_execution", "file_change"},
                "plan": {"proposed_plan"},
                "subagent": {"subagent"},
                "approval": {"approval_request"},
                "question": {"user_input_request"},
                "error": {"error"},
            }.get(kind, {"assistant_message"})
            if missing := required - present:
                raise RuntimeError(f"{side}: fixture rows missing: {sorted(missing)}")
            print(
                side,
                [item.get("type") for item in projection.get("turnItems", [])],
                flush=True,
            )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument(
        "--kind",
        choices=[
            "activity",
            "markdown",
            "user",
            "plan",
            "subagent",
            "approval",
            "question",
            "error",
            "long",
        ],
        required=True,
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    asyncio.run(prepare(json.loads(args.manifest.read_text()), args.kind, args.output))
