#!/usr/bin/env python3
"""Fails when a pending changeset breaks a rule of CONTRIBUTING.md's "Writing a changeset" that a script can judge:
no repository-internal terms (the harness, decision-log section numbers, script names, parameter grids) and at most
MAX_BULLETS bullets. Wording and length are left to review. Run as `python3 -I scripts/check-changesets.py [FILE...]`;
with no files it checks `.changeset/*.md` except the README.
"""

import re
import sys
from pathlib import Path

MAX_BULLETS = 4

# (pattern, what to say instead). Matched outside the frontmatter, case-insensitively.
FORBIDDEN = [
    (r"\bharness\b", "the harness is repository-internal; say what was measured, e.g. 'measured on 222 sample questions'"),
    (r"(?:§|\bsections?\s+\d)", "decision-log section numbers belong in docs/spec/05-research-and-decisions.md"),
    (r"\b[\w-]+\.(?:py|sh)\b", "script names are repository-internal"),
    (r"\bgrid of\b", "parameter grids belong in the decision log"),
    (r"docs/(?:spec|adr|architecture)/", "repository doc paths belong in the decision log"),
]
BULLET = re.compile(r"^\s*(?:[-*+]|\d+\.)\s")


def body(text):
    """The changeset text after its `---` frontmatter."""
    parts = text.split("---", 2)
    return parts[2] if text.startswith("---") and len(parts) == 3 else text


def problems(name, text):
    """One message per rule `text` breaks; empty when it passes."""
    found = []
    lines = body(text).splitlines()
    for number, line in enumerate(lines, 1):
        for pattern, advice in FORBIDDEN:
            match = re.search(pattern, line, re.IGNORECASE)
            if match:
                found.append(f"{name}: '{match.group(0)}' in the body, line {number}: {advice}")
    bullets = sum(1 for line in lines if BULLET.match(line))
    if bullets > MAX_BULLETS:
        found.append(f"{name}: {bullets} bullets, at most {MAX_BULLETS}")
    return found


def main(argv):
    root = Path(__file__).resolve().parent.parent
    files = [Path(a) for a in argv] or sorted(
        p for p in (root / ".changeset").glob("*.md") if p.name != "README.md"
    )
    found = [message for path in files for message in problems(path.name, path.read_text(encoding="utf-8"))]
    for message in found:
        print(message, file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
