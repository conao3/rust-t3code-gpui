import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


class PullRequestFixtureTests(unittest.TestCase):
    def test_blocks_unconfigured_pull_request_methods(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node is required for the server fixture")
        tools = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [
                    node,
                    "--require",
                    str(tools / "pr_rpc_fixture.cjs"),
                    "-e",
                    """
const { EventEmitter } = require('node:events');
const socket = new EventEmitter();
socket._isServer = true;
socket.send = (value) => process.stdout.write(value + '\\n');
socket.on('message', () => { throw new Error('Fixture reached real handler'); });
for (const tag of ['pullRequests.list', 'pullRequests.runAction']) {
  socket.emit('message', Buffer.from(JSON.stringify({ _tag: 'Request', id: tag, tag, payload: {} })));
}
""",
                ],
                env=os.environ
                | {
                    "T3_PARITY_OWNER": "unit-test",
                    "T3CODE_HOME": directory,
                    "T3_PARITY_PR_FIXTURE": str(tools / "pr-rpc-fixture.json"),
                },
                capture_output=True,
                text=True,
                check=True,
            )
            replies = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(replies[0]["exit"]["value"]["entries"][0]["number"], 42)
            self.assertEqual(replies[1]["exit"]["_tag"], "Failure")
            log = [
                json.loads(line)
                for line in (Path(directory) / "pr-rpc.jsonl").read_text().splitlines()
            ]
            self.assertEqual(
                [entry["method"] for entry in log],
                ["pullRequests.list", "pullRequests.runAction"],
            )
