---
monokl: minor
---

**Search ranks a declaration by its abbreviated name.** Scores and row order change; no API or wire shape changes.

- **Abbreviated names.** A name token of at least 3 characters now matches a query term that starts with it, so `ListenerExt` holds `listener` and `extension`, and `reqIdGenFactory` holds `request`, `id`, `generator` and `factory`. A token never matches a term it is longer than (`parser` is not `parse`), and a token of 1 or 2 characters has to equal the term. A prefix match counts as much as an equal one in the name boost, including the bonus for a name that is exactly the query.
- **Measured.** Answerable in the first three rows 204 of 222 to 206, held-out Rust 34 of 36 to 35 (axum `listener extension` from absent to rank 2). No harness set loses a hit. 6 of 250 queries change their first row: 3 better, 3 worse. 47 of the 48 queries of the earlier blind review keep their first row.
- **Tradeoff.** The same rule that fixes `ListenerExt` also matches where it should not. `RequestPartsExt` now matches `extractor` through `ext` and scores as exactly the query, so axum `extractor from request parts` no longer has `FromRequestParts` in the first five rows; zod `global registry metadata` is worse (`meta` starts `metadata`); and fd `directory entry stripped path` shows `impl DirEntry` (`dir` starts `directory`) before `stripped_path`.
