# Language Analyzer Contract — Semantic Model

Source: `docs/spec/01-core-architecture.md` §11 (`analysis/lang.rs`, the `LanguageAnalyzer` trait) and
`docs/spec/03-multi-language-platform.md` §3 (`analysis/registry.rs`, `AnalyzerRegistry`). The code in
`crates/monokl/src/analysis/` is smaller than the spec: it has what `monokl search` reads.

## `LanguageAnalyzer` trait (`analysis/lang.rs`)

```rust
pub trait LanguageAnalyzer: Send + Sync {
    fn languages(&self) -> &[Language];
    fn supports(&self, path: &Utf8Path) -> bool;
    fn analyze(&self, path: &Utf8Path, source: String) -> Result<Arc<FileAnalysis>>;
}
```

- `languages` is a slice because one instance can own several languages (`TsAnalyzer` owns `Language::TypeScript` and
  `Language::JavaScript`). Do not collapse it back to one analyzer per language.
- `supports` asks `Language::of(path)`, so the extensions and the declaration-file exclusion are defined once in
  `analysis/lang.rs`, not per analyzer.
- `analyze` takes the text, not a closure: search has already read the file (and replaced invalid UTF-8) before it asks.
  It returns `blocks`, `test_blocks`, `had_parse_errors` and `degradation`, which is all search reads.
- `Language` has the languages an analyzer exists for: `TypeScript`, `JavaScript` and `Rust`. It is also the `--language` value.

Removed because no command read them: `language_for`, `capabilities`, `AnalyzerCapabilities`, `CapabilityPrecision`,
`analyze_with_profile` and `AnalysisProfile` (`Dependencies` and `Structural` were never requested; search always asked
for `Full`). A command that gates on precision, or needs a cheaper pass, restores the one it uses together with the
analyzer output it reads. The spec's shapes for them (`languages() -> &[Language]`, a per-language
`capabilities(language)`, `language_for(path)`) are the ones to restore, so an analyzer that owns several languages
stays expressible.

## `AnalyzerRegistry` (`analysis/registry.rs`)

```rust
pub struct AnalyzerRegistry { analyzers: Vec<Arc<dyn LanguageAnalyzer>> }

impl AnalyzerRegistry {
    pub fn new(analyzers: Vec<Arc<dyn LanguageAnalyzer>>) -> Self;
    pub fn analyzer_for(&self, path: &Utf8Path) -> Option<&dyn LanguageAnalyzer>;
    pub fn handles(&self, language: Language) -> bool;
}
```

- **First-match-by-registration-order, not specificity**: linear `find`, the first `supports(path)` wins.
- **No match is `None`, with no diagnostic from the registry itself.** The caller decides what to say.
- `Arc<dyn LanguageAnalyzer>` throughout so the registry is shareable across rayon workers.

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
