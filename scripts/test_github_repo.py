#!/usr/bin/env python3
"""Checks scripts/github-repo.sh. Run as `python3 -I scripts/test_github_repo.py`; `just check-drift` runs it."""

import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "github-repo.sh"


def run(*args):
    done = subprocess.run([str(SCRIPT), *args], capture_output=True, text=True)
    return done.returncode, done.stdout, done.stderr


class GithubRepo(unittest.TestCase):
    def test_accepted_forms_print_owner_and_repo(self):
        for url, want in [
            ("https://github.com/orin-dx/monokl", "orin-dx/monokl"),
            ("https://github.com/orin-dx/monokl.git", "orin-dx/monokl"),
            ("https://github.com/orin-dx/monokl/", "orin-dx/monokl"),
            ("https://github.com/orin-dx/monokl.git/", "orin-dx/monokl"),
            ("ssh://git@github.com/orin-dx/monokl.git", "orin-dx/monokl"),
            ("ssh://git@github.com/orin-dx/monokl", "orin-dx/monokl"),
            ("ssh://git@github.com:22/orin-dx/monokl.git", "orin-dx/monokl"),
            ("ssh://github.com/orin-dx/monokl.git", "orin-dx/monokl"),
            ("git@github.com:orin-dx/monokl.git", "orin-dx/monokl"),
            ("git@github.com:orin-dx/monokl", "orin-dx/monokl"),
            ("git@github.com:orin-dx/monokl/", "orin-dx/monokl"),
            ("https://github.com/o/my.repo-1_x.git", "o/my.repo-1_x"),
        ]:
            with self.subTest(url=url):
                self.assertEqual(run(url), (0, want + "\n", ""))

    def test_unparsable_urls_fail_naming_the_url(self):
        for url in [
            "https://gitlab.com/o/r.git",
            "github.com/o/r",
            "https://github.com/o",
            "https://github.com/o/r/extra",
            "ssh://git@github.com/o/",
            "",
        ]:
            with self.subTest(url=url):
                code, out, err = run(url)
                self.assertEqual((code, out), (3 if url else 2, ""))
                self.assertEqual(err.count("\n"), 1)
                if url:
                    self.assertIn(f"'{url}'", err)

    def test_forms_callisto_refuses_are_refused(self):
        # Callisto accepts only HTTPS without userinfo and SSH remotes, so `callisto release` would fail on these too.
        for url in [
            "http://github.com/o/r.git",
            "git://github.com/o/r.git",
            "https://user@github.com/o/r.git",
        ]:
            with self.subTest(url=url):
                code, out, err = run(url)
                self.assertEqual((code, out), (3, ""))
                self.assertIn(f"'{url}'", err)


if __name__ == "__main__":
    unittest.main()
