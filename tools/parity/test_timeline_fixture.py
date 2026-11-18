import json
import queue
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

from timeline_fixture import transcript


class TimelineFixtureTests(unittest.TestCase):
    def test_question_blocks_output_until_the_client_responds(self):
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
            source = Path(directory) / "transcript.ndjson"
            source.write_text(
                "\n".join(
                    json.dumps({"type": "emit_inbound", **entry}) for entry in entries
                )
            )
            source.write_text(transcript(source, "question"))
            response = Path(directory) / "response.md"
            response.write_text("**Answer received.**\n")
            with subprocess.Popen(
                [
                    sys.executable,
                    str(Path(__file__).with_name("replay_codex.py")),
                    "--transcript",
                    str(source),
                    "--response-file",
                    str(response),
                    "--stream-ms",
                    "0",
                ],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            ) as process:
                frames = queue.Queue()
                reader = threading.Thread(
                    target=lambda: [
                        frames.put(json.loads(line)) for line in process.stdout
                    ],
                    daemon=True,
                )
                reader.start()

                def send(frame):
                    process.stdin.write(json.dumps(frame) + "\n")
                    process.stdin.flush()

                try:
                    send(
                        {
                            "id": 1,
                            "method": "turn/start",
                            "params": {"threadId": "new-thread"},
                        }
                    )
                    self.assertEqual(frames.get(timeout=10)["id"], 1)
                    request = frames.get(timeout=10)
                    self.assertEqual(request["method"], "item/tool/requestUserInput")
                    self.assertEqual(request["params"]["threadId"], "new-thread")
                    with self.assertRaises(queue.Empty):
                        frames.get(timeout=0.2)
                    send(
                        {
                            "id": request["id"],
                            "result": {
                                "answers": {
                                    "fixture-choice": {"answers": ["Current thread"]}
                                }
                            },
                        }
                    )
                    output = ""
                    while True:
                        frame = frames.get(timeout=10)
                        if frame.get("method") == "item/agentMessage/delta":
                            output += frame["params"]["delta"]
                        if frame.get("method") == "turn/completed":
                            break
                    self.assertEqual(output, response.read_text())
                finally:
                    process.terminate()
                    process.wait(timeout=5)
                    reader.join(timeout=5)
