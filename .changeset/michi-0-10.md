---
monokl: minor
---

**A cut cell ends in `…` and is listed in a `truncated` block.** monokl moves to `michi-core` 0.10.0. The search rows are unchanged except where a cell is over 200 characters, which in practice is a long `symbolSignature`. 23 of the 222 harness outputs (10%) have one, and the total output grows 0.5% (385,471 bytes, was 383,506). Every harness score stays at its baseline.

- **Cut cell.** The cell keeps 199 characters and ends in `…`. It used to end in `(N more chars)`.
- **`truncated` block.** After the rows, `truncated[N]{row,field,removed}:` lists each cut cell: `row` is 0-based, `field` the column, `removed` the characters cut.
- **Row budget.** `search_row_texts` counted every line after the header as a row, so the new block would have been counted as rows (a debug build panicked, a release build gave the budget a wrong count). It returns the rows only.
- **Errors.** stderr keeps its `causes[N]` lines.
- **Not added.** No `budget` or `provenance` block: about 66 tokens a response for nothing the caller lacks.

**Breaking:** A cut cell ends in `…`, not `(N more chars)`, and the output gains a `truncated[N]{row,field,removed}:` block after the rows when a cell was cut. `ToDomainError::to_domain_error` returns michi 0.10.0's `DomainError`, so a caller that names michi types needs `michi-core` 0.10.0.

**Action:** A parser of `(N more chars)` reads the `truncated` block instead. A parser that takes every indented line of the output as a row must read only the indented lines directly under the `search[N]{...}:` header, because the lines of the `truncated` block are indented too.
