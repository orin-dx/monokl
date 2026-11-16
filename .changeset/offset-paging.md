---
monokl: minor
---

**`monokl search --offset N` prints the next page of rows.** It skips the first N ranked rows and prints the next page under the same cut and caps, so the rows the lean default hides are reachable without paying for the first ones again. The relevance cut stays relative to the top row of the whole result, so page 1 plus page 2 equals one run with a larger `--max-results`. `--max-tokens` bounds the page that is returned, not the rows skipped. An offset past the last row prints no rows and the `totalCount`, and exits 0; a non-number is a usage error (exit 2).

- **Help line.** When rows remain it names the next value: `showing 8 of 31; next: --offset 8, or refine with +term/-term`. It replaces the `--max-results N` and `--max-tokens N` advice, which re-paid for the rows already seen; `--min-score 0` is still named when the relevance cut dropped rows, and offsets keep counting the same ranking with it. An empty page the caps caused (a row larger than the token cap) still names the cap.
- **Library.** `Budget` and `SearchResponse` each have a new public field, `offset`, so code that builds them must set it (`Budget::default()` has 0).
- **Stateless.** Every call re-runs the search. Pages are exact for an unchanged tree.
