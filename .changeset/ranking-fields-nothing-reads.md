---
monokl: minor
---

`finalScore` in `monokl search` rows no longer includes a constant 1.0, and `RankedBlock` and `SearchResponse` lose fields nothing read.

- **`finalScore` drops by exactly 1.0.** Every printed block holds a matching line, so the coverage term was always 1.0. Row order is unchanged.
- **Library.** Removed `rank::coverage_boost`, `RankedBlock::{bm25_score, coverage_boost, node_type_boost}` and `SearchResponse::{total_bytes, total_tokens}`. `SymbolKind` now derives `PartialOrd` and `Ord` in declaration order, which ranking uses as its tie-break.
