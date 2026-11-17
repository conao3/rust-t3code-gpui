import contextlib
import fcntl
import io
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid
from pathlib import Path
from unittest.mock import AsyncMock, patch

from env import (
    MARKER,
    allocation,
    fixture,
    main,
    owned_processes,
    process_identity,
    quiet,
    require_free,
    signal_owned,
    stop_owned,
    verify_credentials,
    verify_provider,
    wait_free,
    write_json,
)
from workspace_fixture import independent


class SlotTests(unittest.TestCase):
    def test_status_can_run_while_a_mutation_holds_the_slot_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            slot = root / ".claude-dev/parity/slots/1"
            slot.mkdir(parents=True)
            output = io.StringIO()
            previous_umask = os.umask(0o077)
            try:
                with (
                    (slot / "control.lock").open("a") as lock,
                    patch("env.main_tree", return_value=root),
                    patch("sys.argv", ["env.py", "status", "--slot", "1"]),
                    contextlib.redirect_stdout(output),
                ):
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    main()
            finally:
                os.umask(previous_umask)
            self.assertEqual(json.loads(output.getvalue())["status"], "absent")

    def test_allocations_are_disjoint_and_exclude_existing_sessions(self):
        ports, displays = set(), set()
        for slot in range(1, 101):
            assigned, display = allocation(slot)
            self.assertFalse(ports.intersection(assigned.values()))
            self.assertTrue(all(p > 39991 for p in assigned.values()))
            self.assertNotIn(display, displays)
            ports.update(assigned.values())
            displays.add(display)
        for slot in [-1, 0, 101]:
            with self.assertRaises(ValueError):
                allocation(slot)

    def test_busy_port_is_rejected_without_disrupting_listener(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            with self.assertRaisesRegex(RuntimeError, "unavailable"):
                require_free({"native": port}, ":9001")
            with socket.create_connection(("127.0.0.1", port)):
                pass

    def test_json_files_are_private_even_when_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text("old")
            path.chmod(0o644)
            write_json(path, {"token": "test-secret"})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(path.read_text()), {"token": "test-secret"})

    def test_shutdown_waits_for_the_listener_to_release_its_port(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            release = threading.Timer(0.1, listener.close)
            release.start()
            try:
                wait_free({"native": port}, ":9001", timeout=2)
            finally:
                release.join()

    def test_shutdown_keeps_a_foreign_busy_port_intact(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            with self.assertRaisesRegex(RuntimeError, "unavailable"):
                wait_free({"native": port}, ":9001", timeout=0.1)
            with socket.create_connection(("127.0.0.1", port)):
                pass

    def test_fixture_copies_have_identical_independent_stores(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            transcript = (
                source
                / "apps/server/src/orchestration-v2/testkit/fixtures/simple/codex_transcript.ndjson"
            )
            transcript.parent.mkdir(parents=True)
            transcript.write_text("{}\n")
            fixture(root, {"reference_source": str(source), "stream_ms": 100})
            independent(root / "reference/workspace", root / "native/workspace")
            reference = root / "reference/home/userdata/statev2.sqlite"
            native = root / "native/home/userdata/statev2.sqlite"
            self.assertEqual(reference.read_bytes(), native.read_bytes())
            self.assertNotEqual(reference.stat().st_ino, native.stat().st_ino)
            for target in ["reference", "native"]:
                config = json.loads(
                    (root / target / "home/userdata/settings.json").read_text()
                )
                provider = config["providerInstances"]["codex"]["config"]
                self.assertEqual(Path(provider["homePath"]), root / target / "provider")
                self.assertIn(
                    "replay_codex.py", Path(provider["binaryPath"]).read_text()
                )
            (root / "reference/workspace/README.md").write_text("changed")
            self.assertNotEqual(
                (root / "reference/workspace/README.md").read_text(),
                (root / "native/workspace/README.md").read_text(),
            )

    def test_failed_command_does_not_expose_captured_credentials(self):
        with self.assertRaises(RuntimeError) as error:
            quiet(
                [
                    sys.executable,
                    "-c",
                    "import sys; print('bearer-secret'); print('token-secret', file=sys.stderr); sys.exit(1)",
                ]
            )
        self.assertNotIn("bearer-secret", str(error.exception))
        self.assertNotIn("token-secret", str(error.exception))

    def test_reused_pid_is_not_signalled(self):
        identity = {"pid": 123, "group": 123, "start": "1"}
        with (
            patch("env.owned_processes", return_value=[identity]),
            patch("env.process_identity", return_value=identity | {"start": "2"}),
            patch("env.os.killpg") as kill,
        ):
            signal_owned("owner", signal.SIGTERM)
            kill.assert_not_called()

    def test_stop_leaves_an_unrelated_identical_command_running(self):
        owner = uuid.uuid4().hex
        command = [sys.executable, "-c", "import time; time.sleep(60)"]
        unrelated = subprocess.Popen(command, start_new_session=True)
        owned = subprocess.Popen(
            command, env=os.environ | {MARKER: owner}, start_new_session=True
        )
        try:
            stop_owned(owner)
            owned.wait(timeout=5)
            self.assertIsNone(unrelated.poll())
            self.assertEqual(owned_processes(owner), [])
        finally:
            unrelated.terminate()
            unrelated.wait()
            if owned.poll() is None:
                owned.kill()
                owned.wait()

    def test_stop_finds_descendant_after_group_leader_exit(self):
        owner = uuid.uuid4().hex
        command = [
            sys.executable,
            "-c",
            "import os,time; pid=os.fork(); time.sleep(60) if pid == 0 else None",
        ]
        process = subprocess.Popen(
            command, env=os.environ | {MARKER: owner}, start_new_session=True
        )
        process.wait(timeout=5)
        try:
            self.assertTrue(owned_processes(owner))
            stop_owned(owner)
            self.assertEqual(owned_processes(owner), [])
        finally:
            stop_owned(owner)

    def test_kill_escalation_is_limited_to_owned_group(self):
        owner = uuid.uuid4().hex
        process = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('ready',flush=True); time.sleep(60)",
            ],
            env=os.environ | {MARKER: owner},
            start_new_session=True,
            stdout=subprocess.PIPE,
            text=True,
        )
        try:
            self.assertEqual(process.stdout.readline().strip(), "ready")
            signal_owned(owner, signal.SIGTERM)
            time.sleep(0.05)
            self.assertIsNone(process.poll())
            signal_owned(owner, signal.SIGKILL)
            process.wait(timeout=5)
            self.assertIsNone(process_identity(process.pid))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdout.close()


class ProviderTests(unittest.IsolatedAsyncioTestCase):
    async def test_enabled_external_provider_is_rejected(self):
        rpc = AsyncMock()
        rpc.call.side_effect = [
            {},
            {"providers": [{"instanceId": "claudeAgent", "enabled": True}]},
        ]
        with patch("env.FixtureRpc") as factory:
            factory.return_value.__aenter__.return_value = rpc
            with self.assertRaisesRegex(RuntimeError, "Only the local mock"):
                await verify_provider({})

    async def test_installed_mock_and_disabled_external_providers_are_ready(self):
        rpc = AsyncMock()
        rpc.call.side_effect = [
            {},
            {
                "providers": [
                    {
                        "instanceId": "codex",
                        "enabled": True,
                        "installed": True,
                        "models": ["fixture"],
                    },
                    {"instanceId": "claudeAgent", "enabled": False},
                ]
            },
        ]
        with patch("env.FixtureRpc") as factory:
            factory.return_value.__aenter__.return_value = rpc
            await verify_provider({})

    def test_shared_credential_is_rejected_before_network_access(self):
        with patch("env.urlopen") as request:
            with self.assertRaisesRegex(RuntimeError, "must differ"):
                verify_credentials(
                    {"reference": {"token": "same"}, "native": {"token": "same"}}
                )
            request.assert_not_called()

    def test_cross_server_credential_acceptance_is_rejected(self):
        with (
            patch("env.urlopen"),
            self.assertRaisesRegex(RuntimeError, "accepted"),
        ):
            verify_credentials(
                {
                    "reference": {
                        "token": "a",
                        "httpBaseUrl": "http://127.0.0.1:42010",
                    },
                    "native": {
                        "token": "b",
                        "httpBaseUrl": "http://127.0.0.1:42011",
                    },
                }
            )
