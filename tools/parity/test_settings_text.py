import unittest

from settings_text import compare


class SettingsTextTests(unittest.TestCase):
    def node(self, role, name, value=None):
        return {
            "role": role,
            "name": name,
            "value": value,
            "ancestors": [{"role": "dialog", "name": "Settings"}],
        }

    def test_native_labels_and_browser_static_text_match(self):
        result = compare(
            [
                self.node("StaticText", "Follow-ups"),
                self.node("InlineTextBox", "Follow-ups"),
                self.node("combobox", "Follow-up behavior"),
                self.node("StaticText", "Steer"),
            ],
            [
                self.node("label", "Follow-ups"),
                self.node("combo box", "Follow-up behavior"),
                self.node("label", "Steer", "Steer"),
            ],
        )
        self.assertEqual(result["missing"], [])
        self.assertEqual(result["extra"], [])

    def test_reworded_or_inaccessible_text_fails(self):
        result = compare(
            [
                self.node("StaticText", "Follow-ups"),
                self.node("StaticText", "Show formatted Markdown as you type."),
            ],
            [self.node("label", "While an agent is running")],
        )
        self.assertEqual(len(result["missing"]), 2)
        self.assertEqual(result["extra"], ["While an agent is running"])

    def test_background_window_text_is_excluded(self):
        result = compare(
            [{"role": "StaticText", "name": "Thread title", "ancestors": []}], []
        )
        self.assertEqual(result["reference_count"], 0)

    def test_numeric_values_preserve_both_the_label_and_zero(self):
        result = compare(
            [self.node("spinbutton", "Contrast", 0)],
            [self.node("spin button", "Contrast", "0")],
        )
        self.assertEqual(result["reference_count"], 2)
        self.assertEqual(result["missing"], [])
        self.assertEqual(result["extra"], [])


if __name__ == "__main__":
    unittest.main()
