---
monokl: minor
---

**A match is credited to the block it sits in.** `monokl search` decides which block holds a matching line by where in the line each match starts, not by comparing the text of the line, so a match is never given to a block that only shares its line, and a line a block covers is never lost.

- **Why it matters.** A class or object keeps a match that is its own even when a method on the same line matches another term, and blocks that share a line (`} b() {`, `}, // note`, `const a = 1; const needle = 2;`) are told apart by the byte where the match begins. A test function and the module that holds it are never both credited.
- **Terms are searched one by one.** `regex:class.* m` finds the method `m` that starts on the line `class.*` matches; the terms used to be joined, and the first match hid the second.
- **End of a declaration.** `regex:$` and other matches at the end of a declaration belong to it.
- **Output.** Same columns and the same order. On the pinned repositories the shown rows are identical, `totalCount` is 1 to 3 lower in a few queries, and `finalScore` moves in its later digits where fewer blocks change the document frequencies. Harness scores are unchanged.
- **A shrunken file.** Matching lines past the end of a file that changed after it was searched are counted in a warning.

**Breaking:** `LineHit` has a new public field, `match_starts`, the offsets in the hit's text at which a match starts. Code that builds a `LineHit` must set it, and `text_search` fills it.
