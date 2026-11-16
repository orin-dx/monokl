---
monokl: minor
---

**A Rust row costs fewer bytes.** On the 222 rows of the harness the total output falls from 420,946 to 383,903 bytes (8.8%). The fd Rust median falls from 1,442 to 1,300 bytes (p90 2,957 to 2,565) and the held-out Rust median from 3,560 to 3,156 (p90 4,692 to 3,903). Every harness score and every one-turn answer count stays at its baseline.

- **Snippet.** A matched code line is shown before a comment or attribute line: a line starting with `//`, `/*`, `*/`, `* `, `#[` or `#![` (a Rust doc comment, an attribute, a JSDoc line, a comment in a body) is shown only when too few code lines matched or it is the only match. 59% of the Rust rows with a snippet showed a doc comment or an attribute. The shown lines keep their order in the block. TypeScript follows the same rule.
- **Rust signature.** `symbolSignature` ends before the `where` clause and says `where …` in its place, so `impl<L, M, S, F> Debug for WithGracefulShutdown<L, M, S, F> where L: Debug + 'static, M: Debug, ...` becomes `impl<L, M, S, F> Debug for WithGracefulShutdown<L, M, S, F> where …`. A line of the clause that matched is a snippet line, after the code lines. Ranking still reads the whole signature.
- **More lines marker.** The count of snippet lines past the shown ones reads `+3 more` (was `...and 3 more matching lines`).
- **Not changed, with the numbers.** No signature starts with an attribute or a doc comment already. The `parent` column is `kind name` and costs 5% of the bytes. Test-path rows are 1.4% of the rows. Capping the signature length and rounding `finalScore` were measured and not taken (docs/spec/05-research-and-decisions.md, section 47).
- **Still above the rg policy.** The fd Rust median is 1,300 bytes against 731. What remains is the floor of 8 rows.

**Breaking:** The text of a row changed. A Rust row's `symbolSignature` drops the where clause and ends in ` where …`; the snippet line is a code line where one matched, and for a Rust item a where-clause line can now be a snippet line; the marker is `+N more` instead of `...and N more matching line(s)`.

**Action:** Code that parses the marker, or reads a bound out of a Rust `symbolSignature`, must change: the marker is `+N more`, and a where clause is in the file at `lineStart`–`lineEnd`. `search_row_texts` and `render_search` keep their signatures. There is no flag for the old rows.
