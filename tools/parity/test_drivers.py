import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock

from audit_commands import reference_commands
from cdp import Cdp
from PIL import Image
from run import compare, wait_for


class KeyInputTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_missing_native_input_value_fails_the_checkpoint(self):
        class App:
            async def tree(self):
                return [{"role": "entry", "name": "Message", "value": None}]

        with self.assertRaisesRegex(AssertionError, "Timed out"):
            await wait_for(
                App(), {"role": "textbox", "name": "Message"}, value="text", timeout=0
            )


class ComparisonTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
