---
monokl: minor
---

**A search row drops `rank` and shows `relevance` in place of `finalScore`.** The columns are now `file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance`. On the 222 rows of the harness the total output falls from 117,884 to 107,201 `o200k_base` tokens (9.1%), the median from 447.5 to 405 tokens and the p90 from 1,112 to 1,010; in bytes the total falls from 383,506 to 365,518 (4.7%). Every harness score and every one-turn answer count stays at its baseline (206 of 222).

- **`rank` is gone.** It equalled the row's position in every one of the 2,038 rows measured, and rows are still printed best first.
- **`relevance` replaces `finalScore`.** It is the row's score as a whole percent of the top row's score (top row 100, rounded to the nearest whole number). It compares the rows of one query; a raw score cannot be compared across queries. A page printed with `--offset` measures against the top row of the whole result, so its first row is not 100.
- **It lines up with `--min-score`.** The cut keeps rows scoring at least `--min-score` times the top, so the rows it drops have a `relevance` below 100 times it. The help line says so: `--min-score 0 for the 2 rows below relevance 30`. The first 8 rows are kept whatever their `relevance`.
- **Rejected, with the measurements.** Merging `lineStart` and `lineEnd` breaks `scripts/agent-cost.py` (206 to 0 answerable), grouping rows by file reorders 136 of 222 queries, and one decimal of score makes the top two rows tie in 7 queries (docs/spec/05-research-and-decisions.md, section 52).

**Breaking:** The columns `rank` and `finalScore` are gone from `monokl search` output, and `relevance` is new and last. The row has 8 cells, was 9, and the last cell is an integer from 0 to 100, was a float. `SearchResponse` has a new public field, `top_score`, the best score of the whole ranking, and `SearchView` has a new public field, `min_score`, and is no longer `Eq`. The paging hint reads `--min-score 0 for the N rows below relevance P`, was `below the cut`.

**Action:** Parse rows by header name, not by position: `rank` is no longer the second to last cell. To rank-order rows, use row order. `relevance` is a percent of the top row, so use it to see how far a row is from the first, not as a score. Code that builds a `SearchResponse` must set `top_score` (`apply_budget` sets it from the ranking), and code that builds a `SearchView` must set `min_score` (`SearchView::default()` carries the default). There is no flag for the old columns.
