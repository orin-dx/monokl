# Semantic Model — Load Index

Read this first. Load only the files your task requires — never load all files at once.

monokl is spec-stage: no crate exists yet. Every file below cites the `docs/spec/` section
it distills rather than a source path — treat the cited spec section as authoritative until
the corresponding code exists, at which point these files should be re-pointed at the source
and kept in sync via `canon/drift`.

## Task → Files to Load

| You are working on... | Load these files |
|---|---|
| `error.rs`, `MonoklError`, any `Result<T>` return | `error-taxonomy.md` |
| `types.rs`, `SymbolEntry`, `Visibility`, `LanguageId`, `CodeBlock` | `core-types.md` |
| `query/`, `rank/`, BM25, tie-breaking, determinism | `query-and-ranking.md` |
| `analysis/cache.rs`, `analysis/persist.rs`, content-hash vs. mtime | `cache-architecture.md` |
| `analysis/lang.rs`, `AnalyzerRegistry`, capability/precision, `--kind`/`--format` gating | `language-analyzer-contract.md` |
| Daemon/session-mode work, `PrecisionUpgrader`, `PrecisionLedger` | `daemon-lifetimes.md` |
| Implementing a specific track | `.claude/specs/<track>.json` only |
| Starting fresh, no task assigned | `.claude/plans/ACTIVE.md` |

Do NOT load semantic-model files when implementing a track unless the spec explicitly
references them. The spec@1 is the authority; these files are the reference layer beneath it.

## File Map

| File | Covers |
|---|---|
| `error-taxonomy.md` | `MonoklError` variants, when each fires, five known error-handling contract gaps from the edge-case audit |
| `core-types.md` | `SymbolEntry`, `Visibility`, `LanguageId`, `CodeBlock`, and the Part 4 naming mismatches (`owner`/`impl_owner`, missing `Visibility::Super`) that must be resolved, not ported as-is |
| `query-and-ranking.md` | Query grammar, `QueryPlan`, `rank_blocks`, the determinism gaps found in the audit (candidate-set ordering, missing tie-break) |
| `cache-architecture.md` | Two-tier cache, `ContentHash` authority vs. mtime/size fast path, the Tier-1 gap found in Part 3 |
| `language-analyzer-contract.md` | `LanguageAnalyzer` trait (post-fix `languages()`/`language_for()`/`capabilities(language)`), `AnalyzerRegistry` dispatch, `CapabilityPrecision` |
| `daemon-lifetimes.md` | `PrecisionUpgrader`, `ResourceLifetime`/`SessionOwnership`, `PrecisionLedger` — Part 6, not required for v0.1.0 |

## What Does NOT Live Here

- Source code — none exists yet; read `docs/spec/` directly
- Spec@1 artifacts — live in `.claude/specs/`
- Active task tracking — lives in `.claude/plans/ACTIVE.md`
- The full research/decision history — lives in `docs/spec/05-research-and-decisions.md`, treat as human reference, not invariant source
- The edge-case audit itself — lives in `docs/spec/07-edge-cases-and-failure-modes.md`; these files distill only the audit findings that change what an implementer must do differently from the original spec text
