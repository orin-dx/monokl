---
monokl: minor
---

**Search ranks a declaration by its abbreviated name, and ranks `impl` blocks lower.** Scores and row order change; no API or wire shape changes.

- **Abbreviated names.** A name token of at least 3 characters now matches a query term that starts with it, so `ListenerExt` holds `listener` and `extension`, and `reqIdGenFactory` holds `request`, `id`, `generator` and `factory`. A token never matches a term it is longer than (`parser` is not `parse`), and a token of 1 or 2 characters has to equal the term. A prefix match counts as much as an equal one in the name boost, including the bonus for a name that is exactly the query.
- **Impl prior.** The multiplier for a Rust `impl` block falls from 1.1 to 0.9, so the struct or method an `impl` wraps outranks the block around it.
- **Measured.** Answerable in the first three rows 204 of 222 to 206, held-out Rust 34 of 36 to 35. No harness set loses a hit. Known cost: axum `extractor from request parts` no longer has `FromRequestParts` in the first five rows, because `RequestPartsExt` (`ext` starts `extractor`) now scores as exactly the query.
