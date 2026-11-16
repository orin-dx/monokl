---
monokl: minor
---

**Search ranks a declaration by its abbreviated name, and ranks `impl` blocks lower.** Scores and row order change; no API or wire shape changes.

- **Abbreviated names.** A name token of at least 3 characters now matches a query term that starts with it, so `ListenerExt` holds `listener` and `extension`, and `reqIdGenFactory` holds `request`, `id`, `generator` and `factory`. A token never matches a term it is longer than (`parser` is not `parse`), and a token of 1 or 2 characters has to equal the term. A prefix match counts as much as an equal one in the name boost, including the bonus for a name that is exactly the query. Measured alone: answerable in the first three rows 204 of 222 to 206, held-out Rust 34 of 36 to 35 (axum `listener extension` from absent to rank 2), no harness set loses a hit.
- **Impl prior.** The multiplier for a Rust `impl` block falls from 1.1 to 0.9, so the struct or method an `impl` wraps outranks the block around it. Measured alone it changes no harness count; it puts `OwnerFilter` and `is_within` ahead of their `impl` blocks and, with the prefix rule, removes a regression the prefix rule causes on fd `directory entry stripped path`. Dropping it was tried and measured worse: the prefix rule alone changes 6 first rows (3 better, 3 worse), both together change 8 (5 better, 1 equal, 2 worse).
- **Tradeoff.** The prefix rule that fixes `ListenerExt` also matches where it should not. `RequestPartsExt` now matches `extractor` through `ext` and scores as exactly the query, so axum `extractor from request parts` no longer has `FromRequestParts` in the first five rows, and zod `global registry metadata` is worse (`meta` starts `metadata`). 47 of the 48 queries of the earlier blind review keep their first row.
