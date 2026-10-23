# Core Types — Semantic Model

Source: `docs/spec/01-core-architecture.md` §4 ("types.rs — all public types verbatim"),
cross-referenced with `docs/spec/03-multi-language-platform.md`'s `SymbolEntry` field
additions (§ "`SymbolEntry` — gained `owner`, `trait_impl`, `visibility`, `kind_detail`").
No code exists yet — monokl is spec-stage. This file states what `types.rs` must actually
contain when written, which in two places (below) differs from what `docs/spec/04-analysis-fidelity.md`
currently says.

## SymbolEntry

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_impl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_detail: Option<String>,
}
```

Identical in `01-core-architecture.md` §4 and `03-multi-language-platform.md`'s confirmed
field list — no drift between the two source sections on this struct.

Field notes:

- `name` — display name only (e.g. `Widget<T>` for an impl's self type), not a qualified path.
- `kind` — see `SymbolKind` below.
- `line` — declaration line.
- `signature` — first-line-collapsed by the existing path (`first_line_signature`: truncates
  at the first newline). `04-analysis-fidelity.md`'s proposed `normalize_signature`
  (whitespace-collapse across the *full* multi-line span) is a materially different behavior
  that is not yet reconciled with the existing one — treat as unresolved, not as the new
  canonical behavior, until that's settled (07-edge-cases-and-failure-modes.md, Part 4 finding #3).
- `owner: Option<String>` — **the canonical field name.** For `Impl`/`Method`/`Constructor`
  symbols nested in an `impl` block, the self type of that impl (e.g. `Some("Widget<T>")`),
  propagated down to each associated `fn`. `None` for free-standing items.
- `trait_impl: Option<String>` — the trait name when the symbol comes from a `impl Trait for X`
  block (e.g. `impl Render for Widget<T>` → `Some("Render")`). `None` for inherent impls/methods
  and for symbols that aren't impl-related.
- `visibility: Option<Visibility>` — see `Visibility` below.
- `kind_detail: Option<String>` — stable kebab-case discriminator, independent of `kind`.
  Confirmed Rust values: `"rust-struct"`, `"rust-enum"`, `"rust-trait"`, `"rust-function"`,
  `"rust-type"`, `"rust-const"`, `"rust-module"`, `"rust-static"`, `"rust-impl"`. Ties
  symbols-level entries back to the parent `InspectEntry` variant they came from.

## Visibility

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    Public,
    Crate,
    Module,
    Private,
}
```

Exactly four variants. This is the complete, canonical enum from `01-core-architecture.md` §4,
and `03-multi-language-platform.md` does not add to it.

> **`Visibility::Super` does NOT exist and must not be added as a fifth variant.**
>
> `pub(super)` maps to `Visibility::Module` — this is the correct, intentional mapping, not a
> gap to fill. It is confirmed by the pre-existing visibility mapper in
> `02-inspection-and-analysis.md` (`parse_visibility`):
>
> ```rust
> fn parse_visibility(prefix: &str) -> Visibility {
>     let trimmed = prefix.trim();
>     if trimmed.is_empty() { return Visibility::Private; }
>     if trimmed == "pub" { return Visibility::Public; }
>     if trimmed.starts_with("pub(crate)") { return Visibility::Crate; }
>     if trimmed.starts_with("pub(super)") || trimmed.starts_with("pub(in") {
>         return Visibility::Module;
>     }
>     if trimmed.starts_with("pub") { return Visibility::Public; }
>     Visibility::Private
> }
> ```
>
> `04-analysis-fidelity.md`'s `rust_visibility(node)` instead claims to map
> `pub, pub(crate), pub(super), pub(self)` to `Visibility::{Public, Crate, Super, Module}` —
> i.e. it invents a fifth variant for `pub(super)` specifically. That does not compile against
> this enum. Treat `04-analysis-fidelity.md`'s mapping as wrong; treat `parse_visibility`'s
> `Module` mapping as the target behavior when `rust_visibility` is implemented for real.

## CodeBlock / RankedBlock / ParentContext

```rust
pub struct CodeBlock {
    pub file: Utf8PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub node_kind: SymbolKind,
    pub code: String,
    pub symbol_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub matched_lines: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub matched_keywords: Vec<String>,
}

pub struct RankedBlock {
    #[serde(flatten)] pub block: CodeBlock,
    pub bm25_score: f64,
    pub coverage_boost: f64,
    pub node_type_boost: f64,
    pub final_score: f64,
    pub rank: usize,
    pub parent_context: Option<ParentContext>,
}

pub struct ParentContext { pub kind: SymbolKind, pub name: String, pub line: usize }
```

`RankedBlock` flattens `CodeBlock` at serialization (no nested `block` key in JSON/TOON output).
Score breakdown fields (`bm25_score`, `coverage_boost`, `node_type_boost`) are additive inputs
to `final_score`; see `query-and-ranking.md` for the ranking algorithm itself, not covered here.

## SearchOptions / SearchLimits

```rust
pub struct SearchOptions {
    pub query: String,
    pub path: Utf8PathBuf,
    pub allow_tests: bool,
    pub no_gitignore: bool,
    pub limits: SearchLimits,
    pub exact: bool,
    pub language: Option<Language>,
}
// Default: query="", path="", allow_tests=false, no_gitignore=false,
//          limits=SearchLimits::default(), exact=false, language=None

pub struct SearchLimits {
    pub max_results: Option<usize>,   // default Some(50)
    pub max_bytes: usize,             // default 2_097_152
    pub max_tokens: Option<usize>,    // default Some(20_000)
    pub max_candidates: usize,        // default 1_000
}
```

Per the edge-case audit: `max_bytes` is the only one of these clamped to a hard ceiling;
`max_results`, `max_tokens`, and `max_candidates` are caller-controlled with no enforced cap
(07-edge-cases-and-failure-modes.md, overview finding on opt-out resource limits). Don't assume
passing `None`/large values is bounded by the type — it isn't.

### `Language` vs `LanguageId` — do not conflate

`SearchOptions.language` is `Option<Language>`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Language { TypeScript, JavaScript, Rust, Python }
```

This is the small, **public-API** enum — 4 variants, also the CLI `--language` flag's
`ValueEnum`. It is a different type from `LanguageId` (`01-core-architecture.md`, the
`LanguageAnalyzer`/`AnalyzerRegistry` section), the larger **internal analyzer-dispatch**
enum — `TypeScript, Rust, Python, Go, Java, C, Cpp, Bash, CSharp, Ruby, Php, Kotlin, Scala`
(12 variants, includes fallback-tier-only languages with no `LangData` variant at all). Full
treatment of `LanguageId` belongs to `language-analyzer-contract.md`, not here — this note
exists only so the two aren't reached for interchangeably.

## Diagnostic / DiagnosticKind

```rust
pub struct Diagnostic { pub kind: DiagnosticKind, pub path: Option<Utf8PathBuf>, pub message: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticKind { Degraded, Skipped, Warning }
```

Every `*Result`/`*Response` type below carries a `diagnostics: Vec<Diagnostic>` field. Per the
edge-case audit, **population of that field is inconsistently specified** — do not assume every
degradation path pushes a `Diagnostic`. Confirmed gaps: `dependents` drops unanalyzable files
with only a `tracing::warn!`, never a `Diagnostic` (07-edge-cases..., Part 1 finding #1); most
Part 2 commands declare the field but never describe it being populated (Part 2 finding #3).
When implementing a command that can degrade, check that command's own spec text for whether
`diagnostics` is actually supposed to be pushed to — don't infer it from the field's existence.

## LangData + per-language variants

```rust
#[serde(tag = "language", content = "data")]
#[non_exhaustive]
pub enum LangData {
    #[serde(rename = "typescript")] Ts(TsData),
    #[serde(rename = "rust")]       Rust(RustData),
    #[serde(rename = "python")]     Python(PythonData),
    #[serde(rename = "go")]         Go(GoData),
    #[serde(rename = "java")]       Java(JavaData),
}

pub struct TsData {
    #[serde(default)] pub jsx_elements: Vec<JsxElementEntry>,
    #[serde(default)] pub type_only_imports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub unresolved_aliases: Vec<String>,
}
pub struct RustData {}    // empty placeholder
pub struct PythonData {}  // empty placeholder
pub struct GoData {}      // empty placeholder
pub struct JavaData {}    // empty placeholder
```

`LangData` covers only 5 languages (a strict subset of `LanguageId`'s 12 — see above); `Go`/`Java`
replaced an earlier `CSharp` variant on this enum specifically (roadmap dropped C#, added Go/Java)
per `05-research-and-decisions.md` §6 — `LanguageId` still lists `CSharp` because it's a
fallback-tier analyzer target, not a `LangData`-producing one. `RustData`/`PythonData`/`GoData`/`JavaData`
are empty today; only `TsData` is populated. Access via `LangData::ts() -> Option<&TsData>` and
`LangData.jsx_elements() -> &[JsxElementEntry]` (empty slice for non-TS).

## Dependency types

```rust
pub struct DependencyRecord { pub line: usize, #[serde(default)] pub bindings: Vec<DependencyBinding>, pub target: DependencyTarget }
pub struct DependencyBinding { pub imported: String, pub local: String, pub kind: BindingKind }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingKind { Named, Default, Namespace, Glob, NamespaceWide }

#[serde(tag = "kind")]
#[non_exhaustive]
pub enum DependencyTarget {
    File { specifier: String, resolved: Option<Utf8PathBuf>, is_relative: bool },
    RustPath { segments: Vec<String>, anchor: RustPathAnchor, resolved: Option<Utf8PathBuf> },
    Namespace { segments: Vec<String>, is_static: bool, alias: Option<String> },
}

#[non_exhaustive]
pub enum RustPathAnchor { Crate, Super, #[serde(rename = "self")] Selff, Extern(String) }
```

`DependencyTarget::Namespace` is spec residue from the dropped C# roadmap item — present in the
type, cosmetic/unused in practice, not a functional gap (07-edge-cases-and-failure-modes.md,
Part 3 finding #20). `RustPathAnchor::Selff` (double-f) is the actual variant name — `self` is a
reserved word, hence `#[serde(rename = "self")]` on a differently-spelled Rust identifier.

## Minor types (one line each)

- `SymbolKind` — universal (`Function, Method, Constructor, Class, Struct, Enum, Interface, TypeAlias, Property, Field, Variable, Module`) + Rust-only (`Impl, Macro`) + `Other` catch-all. `Copy`, camelCase serde.
- `ExportRecord { name, line, re_export }` — one entry per exported symbol; `re_export: bool` distinguishes `export { x } from './y'` from a local export.
- `JsxAttribute { name, string_value: Option<String>, is_expression, is_spread }` — TS/TSX only, lives under `TsData` via `JsxElementEntry`.
- `JsxElementEntry { name, is_html, line, attributes: Vec<JsxAttribute> }` — `is_html` distinguishes `<div>` from `<MyComponent>`.
- `SearchResponse { results: Vec<RankedBlock>, total_blocks_before_truncation, truncated, truncation_marker: Option<String>, total_bytes, total_tokens, diagnostics }` — top-level `search` command output.
- `SymbolsResult { files: BTreeMap<Utf8PathBuf, Vec<SymbolEntry>>, total_symbol_count, truncation_marker, diagnostics }` — `BTreeMap` for deterministic file ordering.
- `DependentsResult { file, dependents: Vec<Utf8PathBuf>, imports: Vec<Utf8PathBuf>, total_dependent_count, total_import_count, truncation_marker, diagnostics }`.
- `LineHit { line_number, text }` — not `Serialize`; internal grep-hit representation only.
- `TsconfigMode { Auto, Manual(Utf8PathBuf), Skip }` — workspace tsconfig resolution strategy.
- `WorkspaceOptions { root, tsconfig }` — builder methods `new(root)`, `with_tsconfig(mode)` (consuming, `#[must_use]`).
- `ExtractRequest { file, line_start: Option<usize>, line_end: Option<usize> }` — both bounds `None` means whole file.

## Naming contract

Any future spec text or code that introduces `impl_owner` (instead of `owner`) or
`Visibility::Super` (as a fifth variant) is **wrong by construction** — not a stylistic
variance, a would-not-compile contradiction with the canonical types on this page.

- `impl_owner` — `07-edge-cases-and-failure-modes.md`, Part 4 finding #1. `04-analysis-fidelity.md`
  uses this name throughout (header, code identifiers, test description); the real field, per
  both `01-core-architecture.md` §4 and `03-multi-language-platform.md`, is `owner`.
- `Visibility::Super` — `07-edge-cases-and-failure-modes.md`, Part 4 finding #2. The real
  `Visibility` enum has exactly four variants; `pub(super)` maps to `Visibility::Module`,
  confirmed by `02-inspection-and-analysis.md`'s pre-existing `parse_visibility`.

Both were independently re-verified against source text during the audit and confirmed as
genuine defects in `04-analysis-fidelity.md`, not nitpicks — treat `04-analysis-fidelity.md` as
the document that needs to be fixed to match this page, not the other way around.
