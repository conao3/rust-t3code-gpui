import argparse
import copy
import json
import os
import sys
import threading
import time
import uuid
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--transcript", type=Path, required=True)
parser.add_argument("--stream-ms", type=int, default=15000)
args, extra = parser.parse_known_args()
if "--version" in extra:
    print("codex-cli 0.167.2")
    raise SystemExit(0)
if "mcp" in extra and "list" in extra:
    print("[]")
    raise SystemExit(0)
if "exec" in extra:
    sys.stdin.read()
    schema = json.loads(Path(extra[extra.index("--output-schema") + 1]).read_text())

    def structured(value, key=""):
        if "enum" in value:
            return value["enum"][0]
        if "$ref" in value:
            node = schema
            for part in value["$ref"].removeprefix("#/").split("/"):
                node = node[part]
            return structured(node, key)
        if "anyOf" in value:
            return structured(value["anyOf"][0], key)
        kind = value.get("type")
        if kind == "object" or "properties" in value:
            return {
                name: structured(item, name)
                for name, item in value.get("properties", {}).items()
            }
        if kind == "array":
            return []
        if kind == "boolean":
            return False
        if kind in ["integer", "number"]:
            return value.get("minimum", 0)
        if kind == "null":
            return None
        return "fixture-response" if "branch" in key.lower() else "Fixture response"

    Path(extra[extra.index("--output-last-message") + 1]).write_text(
        json.dumps(structured(schema))
    )
    raise SystemExit(0)
entries = [json.loads(line) for line in args.transcript.read_text().splitlines()]
responses = {
    e["label"]: e["frame"]["result"]
    for e in entries
    if e["type"] == "emit_inbound" and "result" in e.get("frame", {})
}
notifications = [
    e["frame"]
    for e in entries
    if e["type"] == "emit_inbound"
    and e.get("frame", {}).get("method", "").startswith(("turn/", "item/"))
]
old_thread = responses["thread/start"]["thread"]["id"]
old_turn = responses["turn/start"]["turn"]["id"]
thread_id = str(uuid.uuid4())
lock = threading.Lock()
interrupted = threading.Event()
active_turn = None


def write(value):
    with lock:
        print(json.dumps(value), flush=True)


def replace(value, mapping):
    if isinstance(value, dict):
        return {k: replace(v, mapping) for k, v in value.items()}
    if isinstance(value, list):
        return [replace(v, mapping) for v in value]
    if isinstance(value, int) and 1_500_000_000_000 < value < 2_500_000_000_000:
        return int(time.time() * 1000)
    if isinstance(value, int) and 1_500_000_000 < value < 2_500_000_000:
        return int(time.time())
    if isinstance(value, str):
        for source, target in mapping.items():
            value = value.replace(source, target)
    return value


def stream(turn_id, cwd):
    mapping = {
        old_thread: thread_id,
        old_turn: turn_id,
        "/home/replay-user/t3code/packages/effect-codex-app-server": cwd,
    }
    for frame in notifications:
        item = frame.get("params", {}).get("item")
        if item:
            mapping[item["id"]] = "fixture-item-" + uuid.uuid4().hex
    text = "".join(
        f"Fixture output {i:03d}: deterministic text, **Markdown**, 日本語の表示。\n\n"
        for i in range(200)
    )
    agent_item = None
    for frame in notifications:
        frame = replace(copy.deepcopy(frame), mapping)
        method, params = frame["method"], frame["params"]
        if method == "item/agentMessage/delta":
            continue
        if method == "item/started" and params["item"]["type"] == "agentMessage":
            agent_item = params["item"]["id"]
            write(frame)
            started = time.monotonic()
            for index, chunk in enumerate(text.splitlines(keepends=True)):
                if interrupted.is_set():
                    break
                write(
                    {
                        "method": "item/agentMessage/delta",
                        "params": {
                            "threadId": thread_id,
                            "turnId": turn_id,
                            "itemId": agent_item,
                            "delta": chunk,
                        },
                    }
                )
                delay = (
                    started
                    + (index + 1)
                    * args.stream_ms
                    / 1000
                    / len(text.splitlines(keepends=True))
                    - time.monotonic()
                )
                if delay > 0:
                    interrupted.wait(delay)
            continue
        if method == "item/completed" and params["item"]["type"] == "agentMessage":
            params["item"]["text"] = text
        if method == "turn/completed":
            for item in params["turn"]["items"]:
                if item["type"] == "agentMessage":
                    item["text"] = text
            params["turn"]["durationMs"] = args.stream_ms
            if interrupted.is_set():
                params["turn"]["status"] = "interrupted"
        write(frame)


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    params = message.get("params") or {}
    request = message.get("id")
    result = None
    if method == "initialize":
        result = {
            "userAgent": "t3-parity-fixture/0.167.2",
            "codexHome": os.environ["CODEX_HOME"],
            "platformFamily": "unix",
            "platformOs": "linux",
        }
    elif method == "account/read":
        result = {"account": {"type": "apiKey"}, "requiresOpenaiAuth": False}
    elif method == "model/list":
        result = {
            "data": [
                {
                    "id": "gpt-6-luna",
                    "model": "gpt-6-luna",
                    "displayName": "GPT-6-Luna",
                    "description": "Deterministic local test provider",
                    "hidden": False,
                    "isDefault": True,
                    "supportedReasoningEfforts": [],
                    "defaultReasoningEffort": "medium",
                }
            ],
            "nextCursor": None,
        }
    elif method == "skills/list":
        result = {"data": []}
    elif method == "config/read":
        result = {"config": {}}
    elif method == "account/rateLimits/read":
        result = {
            "rateLimits": None,
            "rateLimitsByLimitId": None,
            "rateLimitResetCredits": None,
            "accountId": None,
        }
    elif method in ["thread/start", "thread/resume"]:
        thread_id = params.get("threadId", thread_id)
        result = replace(
            copy.deepcopy(responses["thread/start"]),
            {
                old_thread: thread_id,
                "/home/replay-user/t3code/packages/effect-codex-app-server": params.get(
                    "cwd", os.getcwd()
                ),
            },
        )
    elif method == "turn/start":
        interrupted.clear()
        active_turn = str(uuid.uuid4())
        result = replace(
            copy.deepcopy(responses["turn/start"]), {old_turn: active_turn}
        )
        write({"id": request, "result": result})
        threading.Thread(
            target=stream,
            args=(active_turn, params.get("cwd", os.getcwd())),
            daemon=True,
        ).start()
        continue
    elif method == "turn/interrupt":
        interrupted.set()
        result = {}
    elif request is not None:
        write(
            {
                "id": request,
                "error": {
                    "code": -32601,
                    "message": f"Unsupported fixture request: {method}",
                },
            }
        )
        continue
    if request is not None:
        write({"id": request, "result": result})
