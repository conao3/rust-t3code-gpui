import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import env
import host_fixture
import keyring_reference


class HostFixtureTests(unittest.TestCase):
    def test_foreign_manifest_is_rejected_before_creating_servers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "foreign.json"
            path.write_text(json.dumps({"slot": 5}))
            with (
                patch("env.main_tree", return_value=root),
                patch("subprocess.Popen") as launch,
            ):
                with self.assertRaisesRegex(RuntimeError, "owned slot manifest"):
                    host_fixture.prepare(path, "secondary", "primary")
                launch.assert_not_called()

    def test_slot_allocation_cannot_be_redirected_to_production(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / ".claude-dev/parity/slots/5/manifest.json"
            path.parent.mkdir(parents=True)
            path.write_text(
                json.dumps({"slot": 5, "ports": {"reference": 3773}, "display": ":0"})
            )
            with (
                patch("env.main_tree", return_value=root),
                patch("subprocess.Popen") as launch,
            ):
                with self.assertRaisesRegex(RuntimeError, "allocation"):
                    host_fixture.prepare(path, "secondary", "primary")
                launch.assert_not_called()

    def test_keyring_requires_the_slot_owner_display_and_private_bus(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            generation = root / ".claude-dev/parity/slots/5/run-test"
            generation.mkdir(parents=True)
            ports, display = env.allocation(5)
            state = {
                "slot": 5,
                "generation": str(generation),
                "owner": "fixture-owner",
                "ports": ports,
            }
            (generation.parent / "state.json").write_text(json.dumps(state))
            environment = {
                env.MARKER: "fixture-owner",
                "DISPLAY": display,
                "DBUS_SESSION_BUS_ADDRESS": "unix:path=private-fixture-bus",
            }
            with patch("env.main_tree", return_value=root):
                self.assertEqual(
                    keyring_reference.owned_generation(generation, environment), state
                )
                for changed in [
                    {env.MARKER: "another-owner"},
                    {"DISPLAY": ":0"},
                    {"DBUS_SESSION_BUS_ADDRESS": ""},
                ]:
                    with self.assertRaisesRegex(
                        RuntimeError, "owned parity generation"
                    ):
                        keyring_reference.owned_generation(
                            generation, environment | changed
                        )


if __name__ == "__main__":
    unittest.main()
