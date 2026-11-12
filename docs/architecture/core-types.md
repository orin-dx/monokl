# Core Types — Semantic Model

Source: `docs/spec/01-core-architecture.md` §4 ("types.rs — all public types verbatim"). `crates/monokl/src/types.rs`
implements the part of it that search reads. This file states what `types.rs` contains.

None of these types derives `Serialize` or `Deserialize`: wire shapes belong to the output layer. A type with no
caller is not defined (see the decision-log entry "Removed public types nothing builds or consumes" in
`05-research-and-decisions.md`), so the spec's `SearchOptions`, `SearchLimits`, `SymbolsResult`, `DependentsResult`,
`TsconfigMode`, `WorkspaceOptions` and `ExtractRequest` are not in `types.rs` until a command uses them.

## CodeBlock / RankedBlock / ParentContext

```rust
pub struct CodeBlock {
    pub file: Utf8PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub node_kind: SymbolKind,
    pub code: String,
    pub symbol_signature: Option<String>,
    pub matched_lines: Vec<usize>,
    pub parent: Option<ParentContext>,
}

pub struct RankedBlock {
    pub block: CodeBlock,
    pub final_score: f64,
    pub rank: usize,
}

pub struct ParentContext { pub kind: SymbolKind, pub name: String }
```

`RankedBlock` nests its `CodeBlock`; the output layer picks the columns it prints.
`CodeBlock.parent` is the enclosing class or object literal of a member block and `None` for a top-level block; the analyzer sets it, and the agent output prints it as the `parent` column (`<kind> <name>`).
`final_score` is the only score `RankedBlock` carries; see `query-and-ranking.md` for the ranking algorithm itself, not covered here.

## `Language`

One enum names the languages monokl searches, and `analysis/lang.rs` holds every file fact about them:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[non_exhaustive]
pub enum Language { TypeScript, JavaScript, Rust }
```

- `Language::of(path)` is the language of a file, `None` for an unsearched extension and for `.d.ts`, `.d.mts` and `.d.cts`.
- `Language::rules()` has the extensions, the excluded suffixes and the test-file names; `is_test_path` adds the test directories.
- The CLI's `--language` flag, the `--language` filter in the pipeline, `LanguageAnalyzer::supports` and `languages`, text search's test-file skip and the rg-compare and bench scripts (through the hidden `monokl language-rules`) all read these rules. Add an extension in `lang.rs` only.
- `LanguageAnalyzer::languages` is a slice because one analyzer can own several languages: `TsAnalyzer` owns `TypeScript` and `JavaScript`.
- A variant exists when a language has an analyzer. `Python` was removed because nothing handled it: `--language python` is a usage error (exit 2) that lists the valid values.

## Diagnostic / DiagnosticKind

```rust
pub struct Diagnostic { pub kind: DiagnosticKind, pub path: Option<Utf8PathBuf>, pub message: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind { Degraded, Skipped, Warning, LossyRead }
```

Every result type (today only `SearchResponse`) carries a `diagnostics: Vec<Diagnostic>` field. Per the
edge-case audit, **population of that field is inconsistently specified** — do not assume every
degradation path pushes a `Diagnostic`. Confirmed gaps: `dependents` drops unanalyzable files
with only a `tracing::warn!`, never a `Diagnostic` (07-edge-cases..., Part 1 finding #1); most
Part 2 commands declare the field but never describe it being populated (Part 2 finding #3).
When implementing a command that can degrade, check that command's own spec text for whether
`diagnostics` is actually supposed to be pushed to — don't infer it from the field's existence.

## Minor types (one line each)

- `SymbolKind` — universal (`Function, Method, Constructor, Class, Struct, Enum, Interface, TypeAlias, Property, Field, Variable, Module`) + Rust-only (`Impl, Macro`) + `Other` catch-all. `Copy`.
- `SearchResponse { results: Vec<RankedBlock>, total_blocks_before_truncation, truncated, diagnostics }` — top-level `search` command output. Counts only: the output layer words the truncation hint.
- `LineHit { line_number, text }` — internal grep-hit representation only.

## Types removed until a command reads them

`SymbolEntry`, `Visibility`, `DependencyRecord`, `DependencyBinding`, `DependencyTarget`, `BindingKind`, `ExportRecord`,
`JsxAttribute`, `JsxElementEntry`, `TsData` and `LangData` were built for every file and read by no command, so they
are deleted, along with `FileAnalysis`'s `symbols`, `dependencies`, `exports`, `lang`, `digest`, `profile` and
`source_path` fields, `ParentContext.line`, `CodeBlock.matched_keywords` and `RankedBlock.parent_context` (a copy of
`block.parent`). Search reads `FileAnalysis`'s `blocks`, `test_blocks`, `had_parse_errors` and `degradation` only. A
command that needs one of them restores it with its analyzer output and a reader. When it does, the spec's names
stand: `owner` (not `impl_owner`) on a symbol, and `pub(super)` maps to `Visibility::Module`, with no `Super` variant.
