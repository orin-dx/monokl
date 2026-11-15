---
monokl: minor
---

**`monokl search` prints less by default.** The default output is the rows that matter, with one snippet line each, so an answer costs about a fifth of the bytes. On the 222 rows of the harness the median output falls from 10,012 to 1,567 bytes, the 90th percentile from 13,351 to 3,997 and the total from 1,714,350 to 420,946, and every harness score stays at its baseline.

- **Relevance cut.** A row is kept when its `finalScore` is at least 0.3 times the top row's (`--min-score FRACTION`, `0` turns the cut off). The first 8 rows are always kept when that many exist. `--max-results` caps the result on top of that, now 20 by default (was 50), and it wins over the floor of 8. `--min-score` takes 0 to 1; anything else is a usage error (exit 2).
- **Snippet.** Each row shows its first matching line and counts the rest (`...and 2 more matching lines`). `--snippet-lines N` shows 0 to 3 lines (3 is the old snippet). `0` prints no snippet, so a variable or type row, which has no signature, shows no declaration text.
- **Help line.** When rows were cut the help names the flag that brings them back: `showing 9 of 37; --min-score 0 or --max-results N for more, or refine with +term/-term`. It names `--max-tokens N` when the tokens ran out first. There is no `--offset` yet.
- **Parameters.** Chosen from a grid of 18 settings (floor 3, 5 or 8; fraction 0.15, 0.2 or 0.3; cap 15 or 20) as the smallest total output that loses no harness question against the baseline. The first choice, floor 3 and cap 15, lost a member and a Rust question.
- **Not measured.** Whether a model finds the answer in the leaner rows, and what snippet lines 2 and 3 are worth when an agent reads for context instead of answering in one turn.

**Breaking:** The default output changed: at most 20 rows (was 50), none below 0.3 times the top score except the first 8, and one snippet line per row (was up to 3). `SearchResponse` has a new public field, `blocks_cut_by_relevance`, `Budget` has a new public field, `min_score` (and no longer implements `Eq`), `render_search` takes a `&SearchView` and `search_row_texts` a snippet length. The truncation help line is reworded.

**Action:** To get the earlier output, run `monokl search <query> --min-score 0 --max-results 50 --snippet-lines 3`. Code that builds a `SearchResponse` or a `Budget`, or calls `render_search` or `search_row_texts`, must pass the new values: `Budget::default()` carries the new defaults, and `SearchView::default()` the new snippet length.
