import unittest

from composer_text import compare, texts


class ComposerTextTests(unittest.TestCase):
    def test_platform_listbox_roles_keep_the_same_option_text(self):
        reference = [
            {
                "role": "option",
                "name": "New worktree",
                "ancestors": [{"role": "listbox"}],
            }
        ]
        native = [
            {
                "role": "list item",
                "name": "New worktree",
                "ancestors": [{"role": "list box"}],
            }
        ]
        self.assertEqual(compare(reference, native)["missing"], [])
        self.assertEqual(compare(reference, native)["extra"], [])
        self.assertEqual(compare(reference, [])["missing"], ["New worktree"])

    def test_unrelated_lists_do_not_become_composer_text(self):
        self.assertEqual(
            texts(
                [
                    {
                        "role": "list item",
                        "name": "Other",
                        "ancestors": [{"role": "list"}],
                    }
                ]
            ),
            set(),
        )


if __name__ == "__main__":
    unittest.main()
