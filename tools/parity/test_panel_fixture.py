import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

from panel_fixture import prepare


class PanelFixtureTests(unittest.IsolatedAsyncioTestCase):
    async def test_unknown_fixture_does_not_connect(self):
        with patch("panel_fixture.FixtureRpc") as rpc:
            with self.assertRaises(ValueError):
                await prepare({}, "unknown")
            rpc.assert_not_called()

    async def test_review_fixture_prepares_both_independent_servers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = {"configurations": {}, "workspaces": {}}
            for side in ("reference", "native"):
                config = root / (side + ".json")
                config.write_text(json.dumps({"environments": [{"id": side}]}))
                workspace = root / side
                workspace.mkdir()
                manifest["configurations"][side] = str(config)
                manifest["workspaces"][side] = str(workspace)
            clients = [AsyncMock(), AsyncMock()]
            with (
                patch("panel_fixture.FixtureRpc") as factory,
                patch("panel_fixture.configure_workspace") as configure,
            ):
                factory.side_effect = [
                    AsyncMock(__aenter__=AsyncMock(return_value=client))
                    for client in clients
                ]
                await prepare(manifest, "pr-guide")
                self.assertEqual(configure.call_count, 2)
                self.assertEqual(
                    [call.args[0]["id"] for call in factory.call_args_list],
                    ["reference", "native"],
                )
            for client in clients:
                calls = client.call.await_args_list
                self.assertEqual(calls[0].args[0], "orchestration.dispatchCommand")
                self.assertEqual(calls[0].args[1]["type"], "thread.create")
                self.assertEqual(
                    calls[1].args,
                    ("server.updateSettings", {"patch": {"reviewGuidesEnabled": True}}),
                )
            for side in ("reference", "native"):
                self.assertEqual(
                    (root / side / "panel-fixture.txt").read_text(),
                    "An uncommitted fixture for the diff panel.\n",
                )
