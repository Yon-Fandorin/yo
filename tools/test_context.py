import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("context", Path(__file__).with_name("context.py"))
context = importlib.util.module_from_spec(spec)
spec.loader.exec_module(context)


class ContextTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        (self.root / "src").mkdir()
        (self.root / "src/prompt.rs").write_text("fn draw() {}\n")
        self.corpus = [("guide.md", "| Prompt cursor | [owner](src/prompt.rs) | `cargo test prompt` |\n")]

    def test_find_returns_existing_route_and_no_confident_fallback(self):
        rows = context.find(self.root, self.corpus, "prompt cursor")
        self.assertEqual(rows[0]["line"], 1)
        self.assertEqual(rows[0]["matched"], ["cursor", "prompt"])
        self.assertEqual(context.find(self.root, self.corpus, "unrelated"), [])

    def test_deleted_anchor_is_broken_and_still_has_impact(self):
        self.assertEqual(context.check(self.root, self.corpus), [])
        (self.root / "src/prompt.rs").unlink()
        self.assertEqual(context.check(self.root, self.corpus)[0]["missing"], "src/prompt.rs")
        self.assertEqual(context.impact(self.root, self.corpus, ["src/prompt.rs"])[0]["document"], "guide.md")

    def test_outcome_match_beats_incidental_mentions(self):
        corpus = [("a.md", "| Transcript | `prompt cursor` |\n"), *self.corpus]
        self.assertEqual(context.find(self.root, corpus, "prompt cursor")[0]["document"], "guide.md")

    def test_directory_boundary_does_not_match_similar_prefix(self):
        corpus = [("guide.md", "[owner](src/)\n")]
        self.assertEqual(context.impact(self.root, corpus, ["src-other/prompt.rs"]), [])
        self.assertEqual(len(context.impact(self.root, corpus, ["src/prompt.rs"])), 1)

    def test_external_links_ignored_and_escaping_local_links_rejected(self):
        self.assertEqual(context.check(self.root, [("guide.md", "[web](https://example.org/missing)")]), [])
        with self.assertRaises(ValueError):
            context.check(self.root, [("guide.md", "[outside](../outside)")])

    def test_output_has_hard_byte_budget_and_reports_omissions(self):
        result = context.bounded_result("find", [{"excerpt": "가" * 8000}] * 30, 20)
        self.assertLessEqual(len(json.dumps(result, ensure_ascii=False).encode()), context.MAX_OUTPUT_BYTES)
        self.assertTrue(result["truncated"])
        self.assertEqual(result["total"], 30)

    def test_changed_includes_staged_unstaged_deleted_and_untracked(self):
        def git(*args):
            subprocess.run(["git", "-C", str(self.root), *args], check=True,
                           capture_output=True, timeout=10)
        git("init", "-q")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "core.hooksPath=/dev/null",
            "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "base")
        for name in ("deleted", "staged", "unstaged", "renamed-old"):
            (self.root / name).write_text("old")
        git("add", ".")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "core.hooksPath=/dev/null",
            "-c", "commit.gpgsign=false", "commit", "-qm", "seed")
        (self.root / "deleted").unlink()
        (self.root / "staged").write_text("new")
        git("add", "staged")
        (self.root / "unstaged").write_text("new")
        (self.root / "new file").write_text("new")
        git("mv", "renamed-old", "renamed-new")
        self.assertEqual(set(context.changed_paths(self.root)),
                         {"deleted", "staged", "unstaged", "new file", "renamed-old", "renamed-new"})


if __name__ == "__main__":
    unittest.main()
