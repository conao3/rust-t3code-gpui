import os
import signal
import socket
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
from desktop_session import read_display, require_free_ports, stop_process_group
from fixture_validation import distinct, inspect_environments
from PIL import Image
from private_session import cleanup_bus_processes
from run import (
    assert_visible_regions,
    compare,
    execute,
    matches,
    wait_for,
)


class VisibleImageTests(unittest.TestCase):
    def test_visibility_rejects_blank_regions_in_both_themes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capture.png"
            for background, foreground in [(0, 255), (255, 0)]:
                image = Image.new("RGB", (12, 12), (background,) * 3)
                image.save(path)
                regions = [{"bounds": [0, 0, 12, 12], "minimum_pixels": 5}]
                with self.assertRaisesRegex(AssertionError, "contrasting pixels"):
                    assert_visible_regions(path, regions)
                for x in range(5):
                    image.putpixel((x, 1), (foreground,) * 3)
                image.save(path)
                assert_visible_regions(path, regions)


class KeyInputTests(unittest.IsolatedAsyncioTestCase):
    def test_native_click_scrolls_before_reading_pointer_coordinates(self):
        from native import Native

        app = object.__new__(Native)
        app.focus = Mock()
        app.xdo = Mock()
        app.atspi = SimpleNamespace(STATE_ENABLED="enabled", SCROLL_ANYWHERE="anywhere")
        node = Mock()
        node.name = "Settled 1"
        node.getRoleName.return_value = "push button"
        node.getState.return_value.contains.return_value = True
        app.nodes = Mock(return_value=[node])
        app.snapshot_controls = {("push button", "Settled 1"): [node]}
        events = []
        node.queryComponent.return_value.scrollTo.side_effect = lambda *_: (
            events.append("scroll")
        )

        def bounds():
            events.append("bounds")
            return [
                {
                    "role": "push button",
                    "name": "Settled 1",
                    "bounds": {"x": 10, "y": 20, "width": 100, "height": 32},
                }
            ]

        app.tree = bounds
        app.click("push button", "Settled 1")
        self.assertEqual(events, ["bounds", "scroll", "bounds"])
        self.assertIn(unittest.mock.call("mousemove", 60, 36), app.xdo.call_args_list)

    async def test_focus_uses_the_selected_control_and_rejects_disabled_targets(self):
        app = Cdp("http://127.0.0.1:39991")
        app.call = AsyncMock()
        app.tree = AsyncMock(
            return_value=[
                {
                    "role": "textbox",
                    "name": "Message",
                    "backend_id": 1,
                    "disabled": True,
                },
                {
                    "role": "textbox",
                    "name": "Message",
                    "backend_id": 2,
                    "disabled": False,
                },
            ]
        )
        with self.assertRaisesRegex(RuntimeError, "disabled"):
            await app.focus_control("textbox", "Message")
        app.call.assert_not_awaited()
        await app.focus_control("textbox", "Message", 1)
        app.call.assert_awaited_once_with("DOM.focus", backendNodeId=2)

    def test_native_focus_rejects_unfocusable_nodes_without_sending_pointer_input(self):
        from native import Native

        app = object.__new__(Native)
        app.focus = Mock()
        app.xdo = Mock()
        app.atspi = SimpleNamespace(
            STATE_ENABLED="enabled",
            STATE_FOCUSABLE="focusable",
            SCROLL_ANYWHERE="anywhere",
        )
        node = Mock(name="entry")
        node.name = "Message"
        node.getRoleName.return_value = "entry"
        node.getState.return_value.contains.side_effect = lambda state: (
            state == "enabled"
        )
        app.nodes = Mock(return_value=[node])
        app.snapshot_controls = {("entry", "Message"): [node]}
        app.tree = Mock(return_value=[{"role": "entry", "name": "Message"}])
        with self.assertRaisesRegex(RuntimeError, "cannot take focus"):
            app.focus_control("entry", "Message")
        node.queryComponent.assert_not_called()
        node.getState.return_value.contains.return_value = True
        node.getState.return_value.contains.side_effect = None
        node.queryComponent.return_value.grabFocus.return_value = True
        app.focus_control("entry", "Message")
        node.queryComponent.return_value.scrollTo.assert_called_once_with("anywhere")
        node.queryComponent.return_value.grabFocus.assert_called_once()
        app.xdo.assert_not_called()

    async def test_held_control_applies_to_a_number_and_is_released(self):
        app = Cdp("http://127.0.0.1:39991")
        app.call = AsyncMock()
        await app.modifier("ctrl", True)
        await app.press("1")
        await app.press("ctrl+2")
        await app.release_modifiers()
        events = [call.kwargs for call in app.call.await_args_list]
        self.assertEqual(
            [event["key"] for event in events],
            ["Control", "1", "1", "2", "2", "Control"],
        )
        self.assertEqual([event["modifiers"] for event in events], [2, 2, 2, 2, 2, 0])
        self.assertEqual(app.held_modifiers, 0)

    def test_native_number_keeps_held_control_and_cleanup_releases_it(self):
        from native import Native

        app = object.__new__(Native)
        app.focus = Mock()
        app.xdo = Mock()
        app.modifier("ctrl", True)
        app.press("1")
        app.release_modifiers()
        self.assertEqual(
            [call.args for call in app.xdo.call_args_list],
            [("keydown", "Control_L"), ("key", "1"), ("keyup", "Control_L")],
        )
        self.assertEqual(app.held_modifiers, set())

    async def test_reference_pointer_preserves_a_drag_and_rejects_outside_coordinates(
        self,
    ):
        app = Cdp("http://127.0.0.1:39991")
        app.call = AsyncMock()
        app.evaluate = AsyncMock(return_value={"width": 1400, "height": 900})
        await app.pointer("down", 300, 700)
        await app.pointer("move", 350, 720)
        await app.pointer("up", 350, 720)
        events = [call.kwargs for call in app.call.await_args_list]
        self.assertEqual(
            [event["type"] for event in events],
            ["mousePressed", "mouseMoved", "mouseReleased"],
        )
        self.assertEqual([event["buttons"] for event in events], [1, 1, 0])
        self.assertEqual([event["button"] for event in events], ["left"] * 3)
        with self.assertRaises(ValueError):
            await app.pointer("down", 1400, 700)
        self.assertEqual(app.call.await_count, 3)

    def test_native_pointer_translates_content_coordinates_without_releasing_a_drag(
        self,
    ):
        from native import Native

        app = object.__new__(Native)
        app.focus = Mock()
        app.xdo = Mock()
        app.content_bounds = Mock(
            return_value={"x": 62, "y": 62, "width": 1400, "height": 900}
        )
        app.pointer("down", 300, 700)
        app.pointer("move", 350, 720)
        app.pointer("up", 350, 720)
        self.assertEqual(
            [call.args for call in app.xdo.call_args_list],
            [
                ("mousemove", 362, 762),
                ("mousedown", 1),
                ("mousemove", 412, 782),
                ("mousemove", 412, 782),
                ("mouseup", 1),
            ],
        )
        with self.assertRaises(ValueError):
            app.pointer("down", 300, 900)

    def test_native_text_keeps_ascii_speed_and_all_unicode_in_order(self):
        from native import Native

        app = object.__new__(Native)
        app.focus = Mock()
        app.xdo = Mock()
        text = "ASCII界🙂tail"
        app.type(text)
        calls = app.xdo.call_args_list
        self.assertEqual("".join(call.args[4] for call in calls), text)
        self.assertEqual([call.args[3] for call in calls], [20, 300, 20])
        app.focus.assert_called_once()

    async def test_appearance_observes_the_applied_document_color_scheme(self):
        app = Cdp("http://127.0.0.1:39991")
        app.evaluate = AsyncMock(return_value="light")
        self.assertEqual(await app.appearance(), "light")
        app.evaluate.assert_awaited_once_with(
            "getComputedStyle(document.documentElement).colorScheme"
        )

    def test_duplicate_controls_require_the_selected_occurrence(self):
        nodes = [
            {"role": "button", "name": "Copy message", "value": text}
            for text in ["user", "assistant"]
        ]
        self.assertEqual(
            matches(nodes, {"role": "button", "name": "Copy message", "index": 1}),
            [nodes[1]],
        )
        self.assertEqual(
            matches(nodes, {"role": "button", "name": "Copy message", "index": 2}), []
        )
        for index in [-1, True, "1"]:
            with self.assertRaises(ValueError):
                matches(nodes, {"role": "button", "index": index})

    async def test_scoped_duplicates_dispatch_to_the_matching_global_occurrence(self):
        nodes = [
            {
                "role": "button",
                "name": "Settle thread",
                "ancestors": [{"role": "button", "name": title}],
            }
            for title in ["First thread", "Second thread"]
        ]
        selector = {
            "role": "button",
            "name": "Settle thread",
            "ancestor": {"role": "button", "name": "Second thread"},
        }
        self.assertEqual(matches(nodes, selector), [nodes[1]])
        app = Mock()
        app.tree = AsyncMock(return_value=nodes)
        app.click = AsyncMock()
        with tempfile.TemporaryDirectory() as directory:
            result = await execute(
                app,
                "reference",
                {"steps": [{"action": "click", "selector": selector}]},
                Path(directory),
            )
        self.assertEqual(result[0]["status"], "passed")
        app.click.assert_awaited_once_with("button", "Settle thread", 1)

    async def test_cdp_keeps_visible_ancestors_across_ignored_layout_nodes(self):
        app = Cdp("http://127.0.0.1:39991")
        app.call = AsyncMock(
            return_value={
                "nodes": [
                    {
                        "nodeId": "row",
                        "role": {"value": "button"},
                        "name": {"value": "Second thread"},
                    },
                    {"nodeId": "layout", "parentId": "row", "ignored": True},
                    {
                        "nodeId": "control",
                        "parentId": "layout",
                        "role": {"value": "button"},
                        "name": {"value": "Settle thread"},
                    },
                ]
            }
        )
        nodes = await app.tree()
        self.assertEqual(
            matches(
                nodes,
                {
                    "role": "button",
                    "name": "Settle thread",
                    "ancestor": {"role": "button", "name": "Second thread"},
                },
            ),
            [nodes[1]],
        )

    async def test_rendered_text_checkpoint_checks_text_content(self):
        app = Mock()
        app.tree = AsyncMock(
            side_effect=[
                [
                    {
                        "role": "document frame",
                        "name": "Assistant message",
                        "value": "stale",
                    }
                ],
                [
                    {
                        "role": "document frame",
                        "name": "Assistant message",
                        "value": "日本語の表示。",
                    }
                ],
            ]
        )
        with patch("run.asyncio.sleep", new=AsyncMock()):
            nodes = await wait_for(
                app, {"name": "Assistant message"}, text_contains="日本語の表示。"
            )
        self.assertEqual(app.tree.call_count, 2)
        self.assertEqual(nodes[0]["value"], "日本語の表示。")

    async def test_rendered_text_checkpoint_reads_reference_static_text(self):
        app = Mock()
        app.tree = AsyncMock(
            return_value=[
                {"role": "StaticText", "name": "日本語の表示。", "value": None}
            ]
        )
        nodes = await wait_for(
            app, {"role": "StaticText"}, text_contains="日本語の表示。"
        )
        self.assertEqual(len(nodes), 1)

    async def test_viewport_waits_for_both_accessible_and_x11_geometry_to_settle(self):
        from native import Native

        app = object.__new__(Native)
        app.window = "1"
        app.focus = Mock()
        app.xdo = Mock()
        desired = {"width": 1400, "height": 900}
        app.content_bounds = Mock(return_value=desired)
        app.geometry = Mock(
            side_effect=[{"WIDTH": "1400", "HEIGHT": "900"}]
            + [{"WIDTH": "1442", "HEIGHT": "942"}] * 3
        )
        with patch("native.asyncio.sleep", new=AsyncMock()):
            await app.viewport(1400, 900)
        self.assertEqual(app.geometry.call_count, 4)
        self.assertEqual(app.xdo.call_count, 1)

    async def test_viewport_applies_the_content_delta_once_after_stable_observations(
        self,
    ):
        from native import Native

        app = object.__new__(Native)
        app.window = "1"
        app.focus = Mock()
        app.xdo = Mock()
        app.content_bounds = Mock(
            side_effect=[{"width": 1442, "height": 942}] * 3
            + [{"width": 1400, "height": 900}] * 3
        )
        app.geometry = Mock(
            side_effect=[{"WIDTH": "1484", "HEIGHT": "984"}] * 3
            + [{"WIDTH": "1442", "HEIGHT": "942"}] * 3
        )
        with patch("native.asyncio.sleep", new=AsyncMock()):
            await app.viewport(1400, 900)
        self.assertEqual(app.xdo.call_args.args, ("windowsize", "1", 1442, 942))
        self.assertEqual(app.xdo.call_count, 2)

    async def test_startup_waits_for_the_single_renderer_after_connection_refusal(self):
        page = Cdp("http://127.0.0.1:1")
        page.wait_document = AsyncMock()
        page.call = AsyncMock()
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
                                "url": "about:blank",
                                "webSocketDebuggerUrl": "ws://127.0.0.1:1/initial",
                            }
                        ],
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
        page.wait_document.assert_awaited_once()
        page.call.assert_awaited_once_with("Accessibility.enable")

    async def test_initial_document_waits_for_controls_before_accessibility_requests(
        self,
    ):
        page = Cdp("http://127.0.0.1:1")
        page.evaluate = AsyncMock(side_effect=[False, False, True])
        with patch("cdp.asyncio.sleep", new=AsyncMock()):
            await page.wait_document()
        self.assertEqual(page.evaluate.await_count, 3)

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

    async def test_button_activation_includes_browser_character_events(self):
        page = Cdp("http://127.0.0.1:1")
        page.call = AsyncMock(return_value={})
        for chord, text in [("enter", "\r"), ("space", " ")]:
            page.call.reset_mock()
            await page.press(chord)
            events = [call.kwargs for call in page.call.await_args_list]
            self.assertEqual(
                [event["type"] for event in events], ["rawKeyDown", "char", "keyUp"]
            )
            self.assertEqual(events[1]["text"], text)
        page.call.reset_mock()
        await page.press("ctrl+enter")
        self.assertNotIn(
            "char", [call.kwargs["type"] for call in page.call.await_args_list]
        )

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
    def test_a_new_desktop_cannot_reuse_a_port_before_its_previous_owner_stops(self):
        with socket.socket() as owner:
            owner.bind(("127.0.0.1", 0))
            owner.listen()
            port = owner.getsockname()[1]
            with self.assertRaisesRegex(RuntimeError, "is in use"):
                require_free_ports([port])
        require_free_ports([port])
        with self.assertRaisesRegex(ValueError, "must be distinct"):
            require_free_ports([port, port])

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
        app.reveal_control = Mock()
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
