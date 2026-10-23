# Architecture

Topic docs for monokl's subsystems. Read the one your task touches, not all of them. Each distills the `docs/spec/` sections it cites and the edge-case findings that change what an implementer must do; where code exists, the code and its tests are authoritative and the doc should be kept in step with them.

## Task to file

| You are working on | Read |
| --- | --- |
| `error.rs`, `file_io.rs`, `MonoklError`, any `Result<T>` return | [error-taxonomy.md](error-taxonomy.md) |
| `types.rs`, `SymbolEntry`, `Visibility`, `LanguageId`, `CodeBlock` | [core-types.md](core-types.md) |
| `query/`, ranking, BM25, tie-breaking, determinism | [query-and-ranking.md](query-and-ranking.md) |
| The in-memory and disk cache, `digest.rs`, digest vs. mtime | [cache-architecture.md](cache-architecture.md); `Digest` is ADR 0002 |
| `LanguageAnalyzer`, `AnalyzerRegistry`, capability and precision gating | [language-analyzer-contract.md](language-analyzer-contract.md) |
| Daemon and session mode, `PrecisionUpgrader`, `PrecisionLedger` (not v0.1.0) | [daemon-lifetimes.md](daemon-lifetimes.md) |

## Cross-cutting invariants

- One `thiserror` error enum for the crate, `MonoklError`; file I/O failures are `FileIoError` wrapped transparently.
- `camino::Utf8Path` everywhere a path appears, except where the path is by definition not UTF-8.
- `FxHashMap`/`FxHashSet` for internal hot-path maps; `BTreeMap` for anything serialized, because output order must be deterministic.
- Analyzers return fully owned data. No arena-borrowed AST node outlives the parse.

## Elsewhere

- Decisions with a credible alternative: [`docs/adr/`](../adr/README.md).
- The full spec and its decision log: [`docs/spec/`](../spec/README.md), [`05-research-and-decisions.md`](../spec/05-research-and-decisions.md).
- The edge-case audit these docs draw on: [`07-edge-cases-and-failure-modes.md`](../spec/07-edge-cases-and-failure-modes.md).
