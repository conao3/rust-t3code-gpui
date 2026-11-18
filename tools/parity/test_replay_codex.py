import json
import queue
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path


class ReplayTests(unittest.TestCase):
    def test_interrupting_one_thread_keeps_the_other_stream_running(self):
        entries = [
            {
                "label": "thread/start",
                "frame": {"result": {"thread": {"id": "old-thread"}}},
            },
            {"label": "turn/start", "frame": {"result": {"turn": {"id": "old-turn"}}}},
            {
                "frame": {
                    "method": "item/started",
                    "params": {
                        "threadId": "old-thread",
                        "turnId": "old-turn",
                        "item": {"id": "old-item", "type": "agentMessage"},
                    },
                }
            },
            {
                "frame": {
                    "method": "turn/completed",
                    "params": {
                        "threadId": "old-thread",
                        "turn": {"id": "old-turn", "status": "completed", "items": []},
                    },
                }
            },
        ]
        with tempfile.TemporaryDirectory() as directory:
            transcript = Path(directory) / "transcript.jsonl"
            transcript.write_text(
                "\n".join(
                    json.dumps(dict(type="emit_inbound", **entry)) for entry in entries
                )
            )
            settings = Path(directory) / "settings.json"
            settings.write_text(
                json.dumps(
                    {"stream_ms": 3000, "text": "fixture\n" * 200, "usage_models": []}
                )
            )
            with subprocess.Popen(
                [
                    sys.executable,
                    str(Path(__file__).with_name("replay_codex.py")),
                    "--transcript",
                    str(transcript),
                    "--settings",
                    str(settings),
                ],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            ) as process:
                frames = queue.Queue()

                def read_frames():
                    for line in process.stdout:
                        frames.put(json.loads(line))

                reader = threading.Thread(target=read_frames, daemon=True)
                reader.start()
                observed = []

                def receive():
                    frame = frames.get(timeout=10)
                    observed.append(frame)
                    return frame

                def request(identifier, method, params):
                    process.stdin.write(
                        json.dumps(
                            {"id": identifier, "method": method, "params": params}
                        )
                        + "\n"
                    )
                    process.stdin.flush()
                    while True:
                        frame = receive()
                        if frame.get("id") == identifier:
                            return frame["result"]

                try:
                    first = request(1, "thread/start", {})["thread"]["id"]
                    second = request(2, "thread/start", {})["thread"]["id"]
                    self.assertNotEqual(first, second)
                    self.assertEqual(
                        request(3, "thread/resume", {"threadId": first})["thread"][
                            "id"
                        ],
                        first,
                    )
                    request(4, "turn/start", {"threadId": first})
                    turn = request(5, "turn/start", {"threadId": second})["turn"]["id"]
                    request(6, "turn/interrupt", {"threadId": second, "turnId": turn})
                    completed = {
                        frame["params"]["threadId"]: frame["params"]["turn"]["status"]
                        for frame in observed
                        if frame.get("method") == "turn/completed"
                    }
                    while len(completed) < 2:
                        frame = receive()
                        if frame.get("method") == "turn/completed":
                            completed[frame["params"]["threadId"]] = frame["params"][
                                "turn"
                            ]["status"]
                    self.assertEqual(
                        completed, {first: "completed", second: "interrupted"}
                    )
                    deltas = [
                        frame["params"]["threadId"]
                        for frame in observed
                        if frame.get("method") == "item/agentMessage/delta"
                    ]
                    self.assertGreater(deltas.count(first), deltas.count(second))
                finally:
                    process.terminate()
                    process.wait(timeout=5)
                    reader.join(timeout=5)
