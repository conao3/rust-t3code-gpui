import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from workspace_fixture import independent, observe


class WorkspaceTests(unittest.TestCase):
    def test_initial_files_git_store_and_final_branch_are_all_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            reference = Path(temporary) / "reference"
            native = Path(temporary) / "native"
            subprocess.run(
                ["git", "init", "-q", "-b", "main", str(reference)], check=True
            )
            (reference / "tracked.txt").write_text("fixture\n")
            subprocess.run(
                ["git", "-C", str(reference), "add", "tracked.txt"], check=True
            )
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(reference),
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "maintenance.auto=false",
                    "-c",
                    "gc.auto=0",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "commit",
                    "-qm",
                    "Fixture",
                ],
                check=True,
            )
            shutil.copytree(reference, native)
            independent(reference, native)
            with self.assertRaisesRegex(ValueError, "independent"):
                independent(reference, reference)
            directories = {"reference": reference, "native": native}
            self.assertEqual(
                observe(directories, {"branch": "main"})["status"], "passed"
            )
            self.assertEqual(
                observe(directories, {"branch": "feature"})["status"], "failed"
            )
            (native / "tracked.txt").write_text("changed\n")
            with self.assertRaisesRegex(ValueError, "different initial"):
                independent(reference, native)
            self.assertEqual(
                observe(directories, {"branch": "main"})["status"], "failed"
            )
            (reference / "tracked.txt").write_text("changed\n")
            independent(reference, native)
            subprocess.run(["git", "-C", str(native), "branch", "extra"], check=True)
            with self.assertRaisesRegex(ValueError, "different initial"):
                independent(reference, native)


if __name__ == "__main__":
    unittest.main()
