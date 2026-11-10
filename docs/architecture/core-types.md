# Core Types — Semantic Model

Source: `docs/spec/01-core-architecture.md` §4 ("types.rs — all public types verbatim"),
cross-referenced with `docs/spec/03-multi-language-platform.md`'s `SymbolEntry` field
additions (§ "`SymbolEntry` — gained `owner`, `trait_impl`, `visibility`, `kind_detail`").
`crates/monokl/src/types.rs` exists and implements part of this. This file states what `types.rs` must actually
contain, which in two places (below) differs from what `docs/spec/04-analysis-fidelity.md`
currently says.

None of these types derives `Serialize` or `Deserialize`: wire shapes belong to the output layer. A type with no
caller is not defined (see the decision-log entry "Removed public types nothing builds or consumes" in
`05-research-and-decisions.md`), so the spec's `SearchOptions`, `SearchLimits`, `SymbolsResult`, `DependentsResult`,
`TsconfigMode`, `WorkspaceOptions` and `ExtractRequest` are not in `types.rs` until a command uses them.

## SymbolEntry

```rust
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub signature: Option<String>,
    pub owner: Option<String>,
    pub trait_impl: Option<String>,
    pub visibility: Option<Visibility>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub matched_lines: Vec<usize>,
    pub matched_keywords: Vec<String>,
    pub parent: Option<ParentContext>,
}

pub struct RankedBlock {
    pub block: CodeBlock,
    pub bm25_score: f64,
    pub coverage_boost: f64,
    pub node_type_boost: f64,
    pub final_score: f64,
    pub rank: usize,
    pub parent_context: Option<ParentContext>,
}

pub struct ParentContext { pub kind: SymbolKind, pub name: String, pub line: usize }
```

`RankedBlock` nests its `CodeBlock`; the output layer picks the columns it prints.
`CodeBlock.parent` is the enclosing class or object literal of a member block and `None` for a top-level block; the analyzer sets it.
Ranking copies it into `RankedBlock.parent_context`, and the agent output prints it as the `parent` column (`<kind> <name>`).
Score breakdown fields (`bm25_score`, `coverage_boost`, `node_type_boost`) are additive inputs
to `final_score`; see `query-and-ranking.md` for the ranking algorithm itself, not covered here.

## `Language` vs `LanguageId` — do not conflate

The CLI's `--language` flag is `Option<Language>`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Language { TypeScript, JavaScript, Rust, Python }
```

This is the small, **public-API** enum — 4 variants, also the CLI `--language` flag's
`ValueEnum`. It is a different type from `LanguageId` (`01-core-architecture.md`, the
`LanguageAnalyzer`/`AnalyzerRegistry` section), the larger **internal analyzer-dispatch**
enum (`crates/monokl/src/analysis/lang.rs`; it includes fallback-tier-only languages with no `LangData` variant at all). Full
treatment of `LanguageId` belongs to `language-analyzer-contract.md`, not here — this note
exists only so the two aren't reached for interchangeably.

## Diagnostic / DiagnosticKind

```rust
pub struct Diagnostic { pub kind: DiagnosticKind, pub path: Option<Utf8PathBuf>, pub message: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind { Degraded, Skipped, Warning }
```

Every result type (today only `SearchResponse`) carries a `diagnostics: Vec<Diagnostic>` field. Per the
edge-case audit, **population of that field is inconsistently specified** — do not assume every
degradation path pushes a `Diagnostic`. Confirmed gaps: `dependents` drops unanalyzable files
with only a `tracing::warn!`, never a `Diagnostic` (07-edge-cases..., Part 1 finding #1); most
Part 2 commands declare the field but never describe it being populated (Part 2 finding #3).
When implementing a command that can degrade, check that command's own spec text for whether
`diagnostics` is actually supposed to be pushed to — don't infer it from the field's existence.

## LangData

```rust
#[non_exhaustive]
pub enum LangData {
    Ts(TsData),
}

pub struct TsData {
    pub jsx_elements: Vec<JsxElementEntry>,
    pub type_only_imports: Vec<String>,
    pub unresolved_aliases: Vec<String>,
}
```

`LangData` has one variant, `Ts`. The spec's `Rust`, `Python`, `Go` and `Java` variants (and their empty `*Data`
structs) return with the analyzer that fills them. `LanguageId` (see above) is the larger analyzer-dispatch enum.
Access via `LangData::ts() -> &TsData` and `LangData::jsx_elements() -> &[JsxElementEntry]`.

## Dependency types

```rust
pub struct DependencyRecord { pub line: usize, pub bindings: Vec<DependencyBinding>, pub target: DependencyTarget }
pub struct DependencyBinding { pub imported: String, pub local: String, pub kind: BindingKind }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind { Named, Default, Namespace, Glob }

#[non_exhaustive]
pub enum DependencyTarget {
    File { specifier: String, resolved: Option<Utf8PathBuf>, is_relative: bool },
}
```

The spec's `DependencyTarget::RustPath`, `DependencyTarget::Namespace`, `RustPathAnchor` and
`BindingKind::NamespaceWide` are not in the code: no analyzer constructs them. They return with the analyzer that
does.

## Minor types (one line each)

- `SymbolKind` — universal (`Function, Method, Constructor, Class, Struct, Enum, Interface, TypeAlias, Property, Field, Variable, Module`) + Rust-only (`Impl, Macro`) + `Other` catch-all. `Copy`.
- `ExportRecord { name, line, re_export }` — one entry per exported symbol; `re_export: bool` distinguishes `export { x } from './y'` from a local export.
- `JsxAttribute { name, string_value: Option<String>, is_expression, is_spread }` — TS/TSX only, lives under `TsData` via `JsxElementEntry`.
- `JsxElementEntry { name, is_html, line, attributes: Vec<JsxAttribute> }` — `is_html` distinguishes `<div>` from `<MyComponent>`.
- `SearchResponse { results: Vec<RankedBlock>, total_blocks_before_truncation, truncated, total_bytes, total_tokens, diagnostics }` — top-level `search` command output. Counts only: the output layer words the truncation hint.
- `LineHit { line_number, text }` — internal grep-hit representation only.

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
