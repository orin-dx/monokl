---
monokl: minor
---

`monokl search` now ranks a block above others when its declared name holds the query words, so a question like `build full path base url` lists `buildFullPath` before code that merely calls it.

- **Name match.** The name is the block's signature, or for a block without one its first line that is not a comment or decorator, cut before the first `(`, `<`, `{`, `=` or `:`. The score is multiplied by `1 + 2 * matched / query terms`, from 1.0 to 3.0. Parameters, return types and the body do not count.
- **Scores and order change.** `finalScore` values are higher for blocks that match by name, and rows can reorder. On the pinned-repo harness the answer file of a multi-word question is in the first three files for 17 of 18 questions (was 14), and a declaration is in the first three files for 58 of 59 identifiers (was 56). The median output of a question falls from 9,876 to 9,633 bytes.
- **Library.** `rank::name_match_boost` is new, and `rank_blocks` multiplies it into `final_score`.
