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

    def test_diff_http_fixture_supports_preflight_and_never_reaches_the_host(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node is required for the server fixture")
        tools = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "fixture.json"
            shutil.copyfile(tools / "pr-rpc-fixture.json", fixture)
            result = subprocess.run(
                [
                    node,
                    "--require",
                    str(tools / "pr_rpc_fixture.cjs"),
                    "-e",
                    """
const { createServer } = require('node:http');
const { writeFileSync } = require('node:fs');
const server = createServer(() => { throw new Error('Fixture reached real handler'); });
server.listen(0, '127.0.0.1', async () => {
  const url = `http://127.0.0.1:${server.address().port}/api/pull-requests/diff`;
  try {
    const origin = 'http://127.0.0.1:42040';
    const preflight = await fetch(url, { method: 'OPTIONS', headers: {
      origin, 'access-control-request-method': 'POST',
      'access-control-request-headers': 'content-type,authorization'
    }});
    const headers = { origin, 'content-type': 'application/json' };
    const input = { repository: 'parity/fixture', number: 42 };
    const response = await fetch(url, { method: 'POST', headers, body: JSON.stringify(input) });
    const body = await response.json();
    writeFileSync(process.env.T3_PARITY_PR_FIXTURE, '{}');
    const missing = await fetch(url, { method: 'POST', headers, body: JSON.stringify(input) });
    process.stdout.write(JSON.stringify({
      preflight: preflight.status,
      origin: preflight.headers.get('access-control-allow-origin'),
      allowed: preflight.headers.get('access-control-allow-headers'),
      status: response.status, body, missing: missing.status
    }));
  } finally { server.close(); }
});
""",
                ],
                env=os.environ
                | {
                    "T3_PARITY_OWNER": "unit-test",
                    "T3CODE_HOME": directory,
                    "T3_PARITY_PR_FIXTURE": str(fixture),
                },
                capture_output=True,
                text=True,
                check=True,
                timeout=20,
            )
            reply = json.loads(result.stdout)
            self.assertEqual(reply["preflight"], 204)
            self.assertEqual(reply["origin"], "http://127.0.0.1:42040")
            self.assertEqual(reply["allowed"], "content-type,authorization")
            self.assertEqual(reply["status"], 200)
            self.assertIn("diff --git", reply["body"]["patch"])
            self.assertEqual(reply["missing"], 500)
            log = [
                json.loads(line)
                for line in (Path(directory) / "pr-rpc.jsonl").read_text().splitlines()
            ]
            self.assertEqual(len(log), 2)
            self.assertTrue(
                all(item["method"] == "http:/api/pull-requests/diff" for item in log)
            )
            self.assertTrue(all(item["input"]["number"] == 42 for item in log))
