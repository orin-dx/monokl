---
monokl: minor
---

**A match is credited to the block it starts in.** `monokl search` decides which block holds a matching line by where in the line each match starts, not by comparing the text of the line, so a match is never given to a block that only shares its line.

- **Why it matters.** A class or object keeps a match that is its own even when a method on the same line matches another term, and blocks that share a line (`} b() {`, `}, // note`, `const a = 1; const needle = 2;`) are told apart by the byte where the match begins. A test function and the module that holds it are never both credited.
- **Output.** Same columns and the same order. On the pinned repositories the shown rows are identical, `totalCount` is 1 to 3 lower in 4 of 137 queries, and `finalScore` moves in its later digits where fewer blocks change the document frequencies. Harness scores are unchanged: identifier 58/59, questions 17/18, members 15/16, Rust fd 22/24, held-out 23/24 and 12/12.
- **Limit.** A match that starts past the 4096th byte of a line credits no block. The cut-line warning already says such a match is not seen.

**Breaking:** `LineHit` has a new public field, `match_starts`, the offsets in the hit's text at which a match starts. Code that builds a `LineHit` must set it, and `text_search` fills it.
