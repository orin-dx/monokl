---
monokl: minor
---

**A search row drops `rank` and shows `relevance` instead of `finalScore`.** Each `monokl search` answer costs about 9% fewer tokens (measured on 222 sample questions) with the same results.

- The row has 8 columns, ending in `relevance`: `file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance`.
- `rank` is gone. Rows are in rank order, so a row's position is its rank.
- `relevance` is the row's score as a whole percent of the top row (top row 100). Compare rows within one query, not across queries. With `--offset` it still measures against the top row of the whole result, so the first row of page 2 is not 100.
- It lines up with `--min-score`: the paging hint now reads `--min-score 0 for the 2 rows below relevance 30`.

**Breaking:** A row had 9 cells and now has 8. `rank` and `finalScore` (a float) are gone, and `relevance` (an integer from 0 to 100) is last.

**Action:** Read cells by header name, not by position, and use row order where you used `rank`. The header changes from `…,snippet,rank,finalScore` to `…,snippet,relevance`, and there is no flag for the old columns. For library users: set `top_score` on a `SearchResponse` you build (`apply_budget` does) and `min_score` on a `SearchView` (`SearchView::default()` has it); `SearchView` is no longer `Eq`.
