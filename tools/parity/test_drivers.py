import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock, patch
from urllib.error import URLError

from audit_commands import reference_commands
from cdp import Cdp
from desktop_session import read_display, stop_process_group
from fixture_validation import distinct, inspect_environments
from PIL import Image
from private_session import cleanup_bus_processes
from run import compare, matches, wait_for


class KeyInputTests(unittest.IsolatedAsyncioTestCase):
    async def test_startup_waits_for_the_single_renderer_after_connection_refusal(self):
        page = Cdp("http://127.0.0.1:1")
        socket = Mock()
        with (
            patch(
                "cdp.asyncio.to_thread",
                new=AsyncMock(
                    side_effect=[
                        URLError(ConnectionRefusedError()),
                        [],
                        [
                            {
                                "type": "page",
                                "url": "t3code://app/",
                                "webSocketDebuggerUrl": "ws://127.0.0.1:1/page",
                            }
                        ],
                    ]
                ),
            ),
            patch("cdp.asyncio.sleep", new=AsyncMock()),
            patch("cdp.websockets.connect", new=AsyncMock(return_value=socket)),
        ):
            await page.__aenter__()
        self.assertIs(page.socket, socket)
        self.assertEqual(page.page_url, "t3code://app/")

    async def test_a_disabled_control_is_not_reported_as_a_successful_click(self):
        page = Cdp("http://127.0.0.1:1")
        page.tree = AsyncMock(
            return_value=[
                {"role": "button", "name": "Send", "backend_id": 1, "disabled": True}
            ]
        )
        page.call = AsyncMock()
        with self.assertRaisesRegex(RuntimeError, "disabled"):
            await page.click("button", "Send")
        page.call.assert_not_called()

    async def test_failed_key_dispatch_releases_held_modifiers(self):
        page = Cdp("http://127.0.0.1:1")
        events = []

        async def call(method, **params):
            events.append(params)
            if params["key"] == "Enter":
                raise RuntimeError("Renderer rejected event")
            return {}

        page.call = call
        with self.assertRaisesRegex(RuntimeError, "Renderer rejected"):
            await page.press("ctrl+shift+enter")
        self.assertEqual([event["key"] for event in events[-2:]], ["Shift", "Control"])
        self.assertEqual([event["type"] for event in events[-2:]], ["keyUp", "keyUp"])
        self.assertEqual(events[-1]["modifiers"], 0)

    async def test_unsupported_key_leaves_no_pressed_modifiers(self):
        page = Cdp("http://127.0.0.1:1")
        page.call = AsyncMock()
        with self.assertRaises(ValueError):
            await page.press("ctrl+unsupported-key")
        page.call.assert_not_called()

    async def test_text_does_not_turn_a_newline_into_an_accidental_submit(self):
        from native import Native

        page = Cdp("http://127.0.0.1:1")
        page.call = AsyncMock()
        with self.assertRaises(ValueError):
            await page.type("first\nsecond")
        page.call.assert_not_called()
        native = object.__new__(Native)
        with self.assertRaises(ValueError):
            native.type("first\nsecond")

    async def test_desktop_origin_is_read_without_returning_bootstrap_credentials(self):
        page = Cdp("http://127.0.0.1:1")
        page.page_url = "t3code://app/"
        page.evaluate = AsyncMock(return_value="http://127.0.0.1:2")
        self.assertEqual(await page.server_origin(), "http://127.0.0.1:2")
        self.assertIn("?.httpBaseUrl", page.evaluate.call_args.args[0])
        page.evaluate.return_value = None
        with self.assertRaisesRegex(TypeError, "did not expose"):
            await page.server_origin()

    async def test_missing_native_input_value_fails_the_checkpoint(self):
        class App:
            async def tree(self):
                return [{"role": "entry", "name": "Message", "value": None}]

        with self.assertRaisesRegex(AssertionError, "Timed out"):
            await wait_for(
                App(), {"role": "textbox", "name": "Message"}, value="text", timeout=0
            )


class ComparisonTests(unittest.TestCase):
    def test_only_removed_accessible_objects_are_retried(self):
        from native import stale_object

        self.assertTrue(stale_object(Exception("Unknown object")))
        error = SimpleNamespace(
            domain="atspi_error", code=1, message="/org/a11y/atspi/accessible/0/123"
        )
        self.assertTrue(stale_object(error))
        error.message = "Service unavailable"
        self.assertFalse(stale_object(error))
        error.message = "/org/a11y/atspi/accessible/0/123"
        error.code = 2
        self.assertFalse(stale_object(error))

    def test_native_click_uses_the_retrying_snapshot_instead_of_stale_objects(self):
        from native import Native

        app = object.__new__(Native)
        app.focus = Mock()
        app.tree = Mock(
            return_value=[
                {
                    "role": "button",
                    "name": "Choose editor",
                    "bounds": {"x": 10, "y": 20, "width": 32, "height": 32},
                }
            ]
        )
        app.nodes = Mock(side_effect=RuntimeError("Stale accessibility object"))
        app.xdo = Mock()
        with patch("native.time.sleep"):
            app.click("button", "Choose editor")
        app.nodes.assert_not_called()
        self.assertEqual(app.xdo.call_args_list[0].args, ("mousemove", 26, 36))

    def test_platform_accessibility_roles_keep_the_same_control_semantics(self):
        for native, reference in [
            ("entry", "textbox"),
            ("combo box", "combobox"),
            ("landmark", "region"),
            ("radio menu item", "menuitemradio"),
        ]:
            node = {"role": native, "name": "Control"}
            self.assertEqual(
                matches([node], {"role": reference, "name": "Control"}), [node]
            )
            self.assertEqual(matches([node], {"role": "button", "name": "Control"}), [])

    def test_changed_content_and_wrong_viewport_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            left, right, output = [
                directory / name for name in ["left.png", "right.png", "result"]
            ]
            Image.new("RGB", (32, 32), "black").save(left)
            Image.new("RGB", (32, 32), "black").save(right)
            self.assertEqual(compare(left, right, output)["status"], "passed")
            Image.new("RGB", (32, 32), "white").save(right)
            self.assertEqual(compare(left, right, output)["status"], "failed")
            Image.new("RGB", (33, 32), "black").save(right)
            self.assertEqual(compare(left, right, output)["status"], "failed")

    def test_upstream_spreads_are_part_of_the_inventory(self):
        source = """
        const EXTRA = ["terminal.toggle", "terminal.split"] as const;
        export const STATIC_KEYBINDING_COMMANDS = ["chat.new", ...EXTRA] as const;
        """
        self.assertEqual(
            reference_commands(source),
            ["chat.new", "terminal.split", "terminal.toggle"],
        )


class FixtureTests(unittest.TestCase):
    def test_shared_origin_and_shared_server_identity_are_rejected(self):
        for url in ["http://localhost", "http://127.0.0.1:80", "http://[::1]:80"]:
            with self.assertRaisesRegex(ValueError, "same server origin"):
                distinct("http://127.0.0.1", [{"origin": url}])
        with self.assertRaisesRegex(ValueError, "same server identity"):
            distinct(
                "http://127.0.0.1:1",
                [{"origin": "http://127.0.0.1:2", "environment_id": "one"}],
                "one",
            )
        with self.assertRaisesRegex(ValueError, "missing"):
            distinct("http://127.0.0.1:1", [])
        distinct(
            "http://127.0.0.1:1",
            [{"origin": "http://127.0.0.1:2", "environment_id": "two"}],
            "one",
        )

    def test_unauthenticated_fixture_cannot_start_a_comparison(self):
        import json
        from types import SimpleNamespace

        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "environments.json"
            config.write_text(
                json.dumps(
                    {
                        "environments": [
                            {"label": "Fixture", "httpBaseUrl": "http://127.0.0.1:2"}
                        ]
                    }
                )
            )
            with (
                patch(
                    "fixture_validation.subprocess.run",
                    return_value=SimpleNamespace(
                        stdout=json.dumps({"Authenticated": False})
                    ),
                ),
                self.assertRaisesRegex(RuntimeError, "did not authenticate"),
            ):
                inspect_environments(Path(directory) / "app", config)


class DisplayTests(unittest.TestCase):
    def test_display_number_waits_for_its_complete_line(self):
        read_fd, write_fd = os.pipe()

        def write():
            os.write(write_fd, b"12")
            time.sleep(0.03)
            os.write(write_fd, b"\n")
            os.close(write_fd)

        writer = threading.Thread(target=write)
        writer.start()
        try:
            self.assertEqual(read_display(read_fd), ":12")
        finally:
            os.close(read_fd)
            writer.join()

    def test_eof_without_newline_is_a_startup_failure(self):
        read_fd, write_fd = os.pipe()
        os.write(write_fd, b"1")
        os.close(write_fd)
        try:
            with self.assertRaisesRegex(RuntimeError, "exited before"):
                read_display(read_fd)
        finally:
            os.close(read_fd)


@unittest.skipUnless(
    sys.platform.startswith("linux"), "Private bus process ownership uses Linux procfs"
)
class CleanupTests(unittest.TestCase):
    def test_owned_descendants_stop_even_when_the_launcher_has_exited(self):
        with tempfile.TemporaryDirectory() as directory:
            pid_file = Path(directory) / "child.pid"
            launcher = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    (
                        "import subprocess,sys; from pathlib import Path; "
                        "p=subprocess.Popen([sys.executable,'-c',"
                        "'import time; time.sleep(30)']); "
                        "Path(sys.argv[1]).write_text(str(p.pid))"
                    ),
                    str(pid_file),
                ],
                start_new_session=True,
            )
            launcher.wait(timeout=3)
            descendant = int(pid_file.read_text())
            stop_process_group(launcher)
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                stat = Path(f"/proc/{descendant}/stat")
                if not stat.exists() or stat.read_text().split()[2] == "Z":
                    break
                time.sleep(0.01)
            else:
                self.fail("The owned descendant survived launcher cleanup")

    def test_only_processes_on_the_owned_bus_are_stopped(self):
        address = f"parity-test:{os.getpid()}:{time.monotonic_ns()}"
        command = [sys.executable, "-c", "import time; time.sleep(30)"]
        owned = subprocess.Popen(
            command, env=os.environ | {"DBUS_SESSION_BUS_ADDRESS": address}
        )
        protected = subprocess.Popen(
            command, env=os.environ | {"DBUS_SESSION_BUS_ADDRESS": address + "-other"}
        )
        try:
            self.assertEqual(cleanup_bus_processes(address), 1)
            self.assertEqual(owned.wait(timeout=3), -signal.SIGTERM)
            self.assertIsNone(protected.poll())
        finally:
            for process in [owned, protected]:
                if process.poll() is None:
                    process.terminate()
                process.wait(timeout=3)


if __name__ == "__main__":
    unittest.main()
