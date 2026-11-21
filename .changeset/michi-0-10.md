---
monokl: minor
---

**A cut cell now ends in `…` and is listed in a `truncated` block.** You can tell a cut cell from a whole one and see how much was lost. Only cells over 200 characters are cut, in practice a long `symbolSignature`: about 1 search in 10 on 222 sample questions.

- The cell keeps 199 characters and ends in `…`. It used to end in `(N more chars)`.
- After the rows, `truncated[N]{row,field,removed}:` lists each cut cell: `row` is 0-based, `field` is the column, `removed` is the number of characters cut.
- Rows without a cut cell are unchanged.

**Breaking:** A cut cell ends in `…`, not `(N more chars)`, and a `truncated[N]{row,field,removed}:` block follows the rows when a cell was cut.

**Action:** Read the `truncated` block instead of parsing `(N more chars)`. If you take every indented line of the output as a row, read only the indented lines directly under the `search[N]{...}:` header, because the `truncated` block's lines are indented too. For library users: `ToDomainError::to_domain_error` returns the `DomainError` of `michi-core` 0.10.0, so code that names michi types needs that version.
