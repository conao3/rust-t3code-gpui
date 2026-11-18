import asyncio
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from prepare import Preparation, identity, prepare
from run_all import markdown, run, scenarios, summarize


class RegressionTests(unittest.IsolatedAsyncioTestCase):
    async def test_parallel_sweep_continues_after_reference_failure_and_cleans_both_slots(
        self,
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "native"
            binary.write_bytes(b"fixture executable")
            for name in ["a", "b", "c"]:
                (root / f"{name}.json").write_text(
                    json.dumps({"theme": "dark", "steps": [{"action": "checkpoint"}]})
                )
            commands = []

            async def command(arguments, log):
                commands.append(arguments)
                await asyncio.sleep(0.01)
                if Path(arguments[0]).name != "run.py":
                    return 0
                output = Path(arguments[arguments.index("--output") + 1])
                output.mkdir(parents=True)
                failed = output.parent.name == "b"
                reference = {"index": 0, "status": "passed"}
                if failed:
                    reference.update(status="failed", error="fixture missing")
                (output / "report.json").write_text(
                    json.dumps(
                        {
                            "targets": {
                                "reference": [reference],
                                "native": [{"index": 0, "status": "passed"}],
                            },
                            "status": "failed" if failed else "passed",
                        }
                    )
                )
                return int(failed)

            args = SimpleNamespace(
                output=root / "results",
                scenarios=root,
                scenario=None,
                repeat=1,
                native_binary=binary,
                slots=[8, 9],
            )
            with (
                patch("run_all.command", side_effect=command),
                patch("run_all.main_tree", return_value=root),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(await run(args), 1)
            rows = json.loads((args.output / "results.json").read_text())
            self.assertEqual(
                [row["scenario"] for row in rows], ["a.json", "b.json", "c.json"]
            )
            self.assertEqual({row["slot"] for row in rows}, {8, 9})
            self.assertEqual(rows[1]["category"], "scenario_or_fixture_error")
            self.assertEqual(sum(args[1] == "down" for args in commands), 5)

    async def test_queue_dispatch_does_not_auto_steer_into_the_active_run(self):
        rpc = AsyncMock()
        setup = Preparation(rpc)
        await setup.message("thread", "Queued message", queued=True)
        payload = rpc.call.call_args.args[1]
        self.assertEqual(payload["dispatchMode"], {"type": "queue_after_active"})
        self.assertNotIn("deliveryIntent", payload)

    async def test_prepare_creates_identical_threads_on_both_servers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            configurations = {}
            for target in ["reference", "native"]:
                path = root / f"{target}.json"
                path.write_text(json.dumps({"environments": [{"httpBaseUrl": target}]}))
                configurations[target] = str(path)
            clients = []

            def rpc(environment):
                client = AsyncMock()
                client.__aenter__.return_value = client
                clients.append((environment, client))
                return client

            with patch("prepare.FixtureRpc", side_effect=rpc):
                result = await prepare(
                    {"configurations": configurations},
                    [{"helper": "threads", "arguments": {"titles": ["Alpha", "Beta"]}}],
                    root / "result",
                )
            self.assertEqual(
                [environment["httpBaseUrl"] for environment, _ in clients],
                ["reference", "native"],
            )
            self.assertEqual(
                clients[0][1].call.call_args_list, clients[1][1].call.call_args_list
            )
            self.assertEqual(result["reference"]["status"], "passed")
            self.assertNotEqual(identity("Alpha"), identity("Beta"))

    async def test_unknown_helper_fails_and_saves_preparation_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = root / "config.json"
            config.write_text(json.dumps({"environments": [{}]}))
            with patch("prepare.FixtureRpc") as rpc:
                rpc.return_value.__aenter__ = AsyncMock(return_value=AsyncMock())
                rpc.return_value.__aexit__ = AsyncMock(return_value=False)
                with self.assertRaisesRegex(ValueError, "Unknown fixture helper"):
                    await prepare(
                        {"configurations": {"reference": str(config)}},
                        [{"helper": "misspelled", "arguments": {}}],
                        root / "result",
                    )
            self.assertEqual(
                json.loads((root / "result/rpc.json").read_text())["reference"][
                    "status"
                ],
                "failed",
            )

    def test_native_only_and_reference_failures_are_separate_from_visual_differences(
        self,
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scenario = root / "sample.json"
            scenario.write_text(json.dumps({"steps": [{"action": "checkpoint"}]}))
            (root / "fixture.json").write_text("{}")
            self.assertEqual(scenarios(root), [scenario])
            report = {
                "targets": {
                    "reference": [
                        {"index": 0, "status": "failed", "error": "missing | fixture"}
                    ],
                    "native": [{"index": 0, "status": "passed"}],
                },
                "status": "failed",
            }
            (root / "report.json").write_text(json.dumps(report))
            row = summarize(scenario, root, 8)
            self.assertEqual(row["category"], "scenario_or_fixture_error")
            self.assertEqual(
                row["counts"]["reference"],
                {"passed": 0, "failed": 1, "not_run": 0, "total": 1},
            )
            self.assertEqual(row["failures"][0]["step"], 1)
            self.assertIn("missing \\| fixture", markdown([row]))
            report["targets"]["reference"][0] = {"index": 0, "status": "passed"}
            report["status"] = "passed"
            (root / "report.json").write_text(json.dumps(report))
            self.assertEqual(summarize(scenario, root, 8)["category"], "passed")
            report["workspace_after"] = {"status": "failed"}
            report["status"] = "failed"
            (root / "report.json").write_text(json.dumps(report))
            self.assertEqual(
                summarize(scenario, root, 8)["category"], "workspace_failure"
            )
            del report["workspace_after"]
            report["visual"] = [{"status": "failed"}]
            (root / "report.json").write_text(json.dumps(report))
            self.assertEqual(
                summarize(scenario, root, 8)["category"], "visual_difference"
            )
            scenario.write_text(
                json.dumps({"targets": ["native"], "steps": [{"action": "checkpoint"}]})
            )
            row = summarize(scenario, root, 8)
            self.assertEqual(row["kind"], "native")
            self.assertNotIn("reference", row["counts"])
