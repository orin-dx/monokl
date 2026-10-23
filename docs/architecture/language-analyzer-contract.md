# Language Analyzer Contract — Semantic Model

Source: `docs/spec/01-core-architecture.md` §11 (`analysis/lang.rs` — `LanguageAnalyzer` trait
verbatim, lines ~1290-1381); `docs/spec/03-multi-language-platform.md` §3 (`analysis/registry.rs`
— `AnalyzerRegistry`, lines ~219-291) and §11 (capability-driven query gating,
`require_precision_capability`/`add_missing_analyzer_diagnostic`, lines ~1780-1990).
Cross-referenced against `docs/spec/07-edge-cases-and-failure-modes.md` Part 3 findings #6, #10, #14.

No crate exists yet — code blocks below are spec-verbatim Rust, not source files. Treat this doc
as authoritative until code exists, then re-point at source paths.

## `LanguageAnalyzer` trait (`analysis/lang.rs`)

The most load-bearing contract in the crate — `AnalyzerRegistry`, capability gating, and every
command's diagnostic behavior are built on top of it.

```rust
pub trait LanguageAnalyzer: Send + Sync {
    fn languages(&self) -> &[LanguageId];
    fn supports(&self, path: &Utf8Path) -> bool;
    fn language_for(&self, path: &Utf8Path) -> Option<LanguageId>;
    fn capabilities(&self, language: LanguageId) -> AnalyzerCapabilities;

    fn analyze_with_profile(
        &self,
        path: &Utf8Path,
        source: Box<dyn FnOnce() -> Result<String>>,
        profile: AnalysisProfile,
    ) -> Result<Arc<FileAnalysis>>;

    /// Full-fidelity convenience wrapper — the only profile v0.1.0 code ever requests.
    fn analyze(&self, path: &Utf8Path, source: Box<dyn FnOnce() -> Result<String>>) -> Result<Arc<FileAnalysis>> {
        self.analyze_with_profile(path, source, AnalysisProfile::Full)
    }
}
```

### Signature fix — must not regress to single-language shape (invariant)

§11 carries a "Post-research correction" note: an earlier draft declared only `supports`/`analyze`,
leaving Part 3's `AnalyzerRegistry` calling `language_id()`/`capabilities()`/`analyze_with_profile()`
that didn't exist on the trait — a compile-breaking gap caught in a post-hoc audit. Corrected shape:

1. **`languages(&self) -> &[LanguageId]`** — a slice, not a zero-arg `language_id() -> LanguageId`.
   One analyzer instance can own several languages: `TsAnalyzer`/`RustAnalyzer` return a
   one-element slice; the generic tree-sitter fallback tier (05-research-and-decisions.md §10)
   returns several (C, Cpp, Bash, CSharp, Ruby, Php, Kotlin, Scala).
2. **`language_for(&self, path) -> Option<LanguageId>`** — new method, absent from the broken
   draft. Which specific language among `languages()` a path maps to; `None` only if `supports`
   would also be `false`. Single-language analyzers: `supports(path).then_some(<their one id>)`.
   Fallback tier: its own extension table.
3. **`capabilities(&self, language) -> AnalyzerCapabilities`** — parameterized, not zero-arg.
   Necessary because one fallback-tier instance has different precision ceilings per language
   (C reaches `Structural`, C++ stays `Heuristic` pending the optional Clang tier, §12).

**Do not collapse back to one-analyzer-one-language.** That shape can't express the fallback tier
and is the exact defect the correction fixed — any impl reverting to it is implementing the wrong
contract.

## `AnalysisProfile` / `CapabilityPrecision`

```rust
pub enum AnalysisProfile { Dependencies, Structural, Full }
// Dependencies: skips symbol/export/block extraction. Structural: skips inspect-entry pass.
// Full: everything, including per-language InspectEntry classification.

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityPrecision { Unsupported, Heuristic, Structural, Exact }
```

`Ord` derived in this order — gating code compares with `<` (`precision < CapabilityPrecision::Structural`), not just `==`.

- `Unsupported` — can't answer this surface at all for this language.
- `Heuristic` — regex/extension-based, no real parse (Rust's fallback when `lang-rs` off; the
  tree-sitter fallback tier for most languages).
- `Structural` — real AST parse, approximate resolution — e.g. Rust's `use` heuristic, which can't
  distinguish a submodule from an external crate without crate-graph resolution.
- `Exact` — real resolution. Only `TsAnalyzer.resolved_import_graph` reaches this (via
  `oxc_resolver`); TS's own `refs`/`definition`/`inspect_detail` are `Structural`, not `Exact`.

## `AnalyzerCapabilities`

```rust
pub struct AnalyzerCapabilities {
    pub blocks: bool,
    pub classified_imports: bool,
    pub resolved_import_graph: CapabilityPrecision,
    pub refs: CapabilityPrecision,
    pub definition: CapabilityPrecision,
    pub inspect_detail: CapabilityPrecision,
}
```

Per-language, not per-analyzer — callers must only pass a `language` value this analyzer's own
`languages()`/`language_for()` actually returned.

## `AnalyzerRegistry` (`analysis/registry.rs`)

```rust
pub struct AnalyzerRegistry { analyzers: Vec<Arc<dyn LanguageAnalyzer>>, config_hash: String }

impl AnalyzerRegistry {
    pub fn analyzer_for(&self, path: &Utf8Path) -> Option<&dyn LanguageAnalyzer> {
        self.analyzers.iter().find(|a| a.supports(path)).map(Arc::as_ref)
    }
    pub fn supports_language(&self, path: &Utf8Path, languages: &[LanguageId]) -> bool {
        self.analyzer_for(path).and_then(|a| a.language_for(path)).is_some_and(|l| languages.contains(&l))
    }
}
```

- **First-match-by-registration-order, not specificity** — linear `find`, first `supports(path)`
  wins. TS registers first when `lang-ts` is enabled, Rust always after.
- `config_hash` = each analyzer's own hash joined with `::` (`TsAnalyzer::config_hash_for(opts)` +
  literal `"rust-analyzer-v1"`) — feeds `analysis::persist::init`'s cache key.
- **No match → `None`, no diagnostic from the registry itself.** Caller's job — see
  `add_missing_analyzer_diagnostic` below, which has its own gap (#10).
- `Arc<dyn LanguageAnalyzer>` throughout so the registry is thread-shareable across rayon workers.

## Capability-driven query gating

`require_precision_capability` (03-multi-language-platform.md §11) is the single decision point
every gated command (`search`, `inspect`, `dependents`, `refs`, `definition`) calls through — no
command hand-rolls a per-language check anymore. Below `requirement.minimum` → `false` (caller
skips) + one `Diagnostic{kind: Skipped}` per language, deduped via `BTreeSet<LanguageId>` (not
per-file). `Structural`-but-not-`Exact` → one `Diagnostic{kind: Warning}` per language, same dedup.
`add_missing_analyzer_diagnostic` is the separate "no analyzer at all" path — see #10.

## Diagnostic-signal gaps — must be resolved, not ported as-is

Open questions for spec@1 (edge-case audit Part 3, findings #6/#10/#14), not resolved designs.

1. **#6 — macro-generated code and inactive `#[cfg(...)]` branches are invisible, zero
   capability/precision signal**, unlike every other Rust precision limitation in the spec. The
   crate-graph heuristic gets a signal (`resolved_import_graph: Structural`); `pick_main_struct`
   — which can arbitrarily pick a platform-dead struct as "the" struct — gets none.
   `AnalyzerCapabilities` has no field for "syntactically present but semantically
   unreachable/unexpanded." Decide: new capability field, or a per-result `Diagnostic` instead of
   a static one. Do not ship `pick_main_struct` silently without picking one.
2. **#10 — the "no analyzer" diagnostic conflates two facts.** `add_missing_analyzer_diagnostic`
   derives its message from `candidate_language(path)`, extension-only (`.ts` → `TypeScript`), and
   doesn't see `TsAnalyzer::supports`'s explicit `.d.ts` exclusion. A `.d.ts` file — TS fully
   supported, this one file intentionally out of scope by design — gets the same
   `"{op} is not supported for TypeScript files in this build"` message a truly-unsupported
   language would get. An agent parsing diagnostics can't tell "no analyzer in this build" from
   "supported language, this file deliberately excluded." Decide: distinguish
   `supports() == false but language recognized`, or add a distinct message template for
   intentional per-file exclusions.
3. **#14 — `Vec<Diagnostic>` has no stated ceiling**, unlike every other result list in the spec
   (symbols: 50/file, 500 total; dependents: 200). Lower priority than #6/#10 — resource-bound
   (pathological workspace → unbounded list), not a correctness/signal gap.
