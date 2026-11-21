#!/usr/bin/env python3
"""Checks scripts/check-changesets.py. Run as `python3 -I scripts/test_check_changesets.py`; `just changesets` runs it."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("check_changesets", ROOT / "scripts" / "check-changesets.py")
CC = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(CC)

FRONT = "---\nmonokl: minor\n---\n\n"
GOOD = FRONT + "**A lead-in.** Value.\n\n- One.\n- Two.\n\n**Breaking:** Old to new.\n"


class CheckChangesets(unittest.TestCase):
    def test_clean_changeset_passes(self):
        self.assertEqual(CC.problems("a.md", GOOD), [])

    def test_harness_is_rejected_in_any_case(self):
        self.assertEqual(
            [m.split(" in the body")[0] for m in CC.problems("a.md", GOOD + "On the 222 rows of the Harness.\n")],
            ["a.md: 'Harness'"],
        )

    def test_decision_log_section_numbers_are_rejected(self):
        for text in ("(decision log, section 52)", "see Section 7", "see §52"):
            with self.subTest(text=text):
                self.assertEqual(len(CC.problems("a.md", GOOD + text + "\n")), 1)

    def test_script_names_and_grids_and_doc_paths_are_rejected(self):
        for text in ("breaks `scripts/agent-cost.py`", "run check.sh", "chosen from a grid of 18 settings",
                     "see docs/spec/05-research-and-decisions.md"):
            with self.subTest(text=text):
                self.assertTrue(CC.problems("a.md", GOOD + text + "\n"))

    def test_the_word_section_without_a_number_is_allowed(self):
        self.assertEqual(CC.problems("a.md", GOOD + "Each section of a file.\n"), [])

    def test_more_than_four_bullets_are_rejected(self):
        text = FRONT + "**Lead.** Value.\n\n" + "- Bullet.\n" * 5
        self.assertEqual(CC.problems("a.md", text), ["a.md: 5 bullets, at most 4"])

    def test_four_bullets_pass_and_sub_bullets_count(self):
        self.assertEqual(CC.problems("a.md", FRONT + "- a\n" * 4), [])
        self.assertEqual(CC.problems("a.md", FRONT + "- a\n- b\n- c\n- d\n  - e\n"), ["a.md: 5 bullets, at most 4"])

    def test_frontmatter_is_not_checked(self):
        self.assertEqual(CC.problems("a.md", "---\nharness: minor\n---\n\nBody.\n"), [])

    def test_main_exit_code_follows_the_files(self):
        with tempfile.TemporaryDirectory() as directory:
            good, bad = Path(directory, "good.md"), Path(directory, "bad.md")
            good.write_text(GOOD)
            bad.write_text(GOOD + "harness\n")
            self.assertEqual(CC.main([str(good)]), 0)
            self.assertEqual(CC.main([str(good), str(bad)]), 1)


if __name__ == "__main__":
    unittest.main()
