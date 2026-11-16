# Core Architecture (v0.1.0)

Naming, branding, and the full v0.1.0 baseline: types, error handling, query language, ranking, caching, the `LanguageAnalyzer`/`WorkspaceEnricher` two-level design, CLI, and the reference TypeScript/JavaScript analyzer. Read this first — everything else in `docs/spec/` builds on it.

Part of the [monokl spec](./README.md). See [ARCHITECTURE.md](../../ARCHITECTURE.md) for the high-level design in prose before diving into this reconstruction-grade detail.

---

# Naming & Branding

## Name: Monokl

**Monokl** (pronounced "MON-oh-kl") is an invented word derived from _monocle_ — a single precision lens used for close examination. The name reflects the tool's purpose: examining code structure with precision and clarity.

- **Crate name:** `monokl` (formerly `loupe`)
- **Package name:** `monokl`
- **CLI binary:** `monokl`
- **Short command alias:** `mnkl`
- **npm wrapper:** `@orin-dx/monokl`
- **Cache directory:** `.monokl/`

### Rationale

The name was chosen after an exhaustive search for a name that:

- Evokes precision optical examination (monocle, loupe family)
- Has no trademark conflicts (USPTO, EUIPO, crates.io, npm)
- Has no negative linguistic associations in major languages
- Is short, pronounceable across languages, and memorable
- Is ownable and registerable as a brand
- Carries a Rust/systems-tool feel

The `k` substitution (monocle → monokl) makes the name distinctive and technically flavored, consistent with the Rust ecosystem aesthetic (cf. `tokei`, `zoxide`, `hyperfine`). In several Slavic languages (Russian, Czech, Slovak, Polish), "monokl" is the standard word for monocle — a bonus rather than a conflict.

### Trademark & Namespace Status

- ✅ No USPTO trademark conflict found
- ✅ No EUIPO trademark conflict found
- ✅ `crates.io/crates/monokl` — unclaimed (reserve immediately on first publish)
- ✅ `npmjs.com/package/monokl` — verify and claim
- ✅ No industry developer tool conflicts found
- ⚠️ File USPTO Class 9 + Class 42 trademark application before public launch

---

# Section 1: Cargo.toml — complete dependency manifest

```toml
[package]
name        = "monokl"
version     = "0.1.0"
edition.workspace     = true
rust-version.workspace = true
license.workspace     = true
repository.workspace  = true
authors.workspace     = true
categories.workspace  = true
keywords    = ["monokl", "search", "ast", "semantic", "typescript"]
description = "AST-aware semantic code search for TypeScript/JavaScript and Rust."
publish     = ["orin-cargo"]

[lib]
name       = "monokl"
path       = "src/lib.rs"

[[bin]]
name              = "monokl"
path              = "src/main.rs"
required-features = ["cli"]

[features]
default = ["lang-ts", "cli"]

# TypeScript/JavaScript analysis via OXC parser + AST + resolver.
lang-ts = [
    "dep:oxc_parser",
    "dep:oxc_ast",
    "dep:oxc_span",
    "dep:oxc_allocator",
    "dep:oxc_resolver",
]

# CLI binary — clap + miette diagnostics.
cli = ["dep:clap", "miette"]

# Miette Diagnostic derive on MonoklError — forwarded to io-errors for full chain rendering.
miette = ["dep:miette", "io-errors/miette"]


[dependencies]
io-errors = { path = "../io-errors", version = "0.1", registry = "orin-cargo", default-features = false }

blake3        = { workspace = true }
camino        = { workspace = true }
dashmap       = { workspace = true }
grep-regex    = { workspace = true }
grep-searcher = { workspace = true }
ignore        = { workspace = true }
rayon         = { workspace = true }
regex         = { workspace = true }
rustc-hash    = { workspace = true }
serde         = { workspace = true }
serde_json    = { workspace = true }
thiserror     = { workspace = true }
tiktoken-rs   = { workspace = true }
tracing       = { workspace = true }

# lang-ts feature-gated OXC deps.
oxc_parser    = { workspace = true, optional = true }
oxc_ast       = { workspace = true, optional = true }
oxc_span      = { workspace = true, optional = true }
oxc_allocator = { workspace = true, optional = true }
# Versioned independently from OXC 0.128 suite — own 11.x line.
oxc_resolver  = { workspace = true, optional = true }

# CLI feature-gated deps.
clap   = { workspace = true, optional = true }
miette = { workspace = true, optional = true }


[dev-dependencies]
insta             = { workspace = true }
pretty_assertions = { workspace = true }
proptest          = { workspace = true }
serde_json        = { workspace = true }
tempfile          = { workspace = true }
criterion         = { workspace = true }

[[bench]]
name    = "pipeline"
harness = false

[lints]
workspace = true
```

The `moon.yml`:

```yaml
$schema: 'https://moonrepo.dev/schemas/project.json'
id: 'monokl'
layer: 'library'
language: 'rust'
toolchains:
  rust: {}
tags:
  - 'language/rust'
  - 'tool'

project:
  name: 'monokl'
  description: 'AST-aware semantic code search for TypeScript/JavaScript and Rust. Distributed via @orin-dx/monokl npm package.'
```

# Section 2: lib.rs — module declarations and feature gates

```rust
// Test modules use `.unwrap()` / `.expect()` liberally — conventional in tests because
// a failing assertion should panic, and the workspace-level `deny unwrap_used` / `expect_used`
// lint is intended for production code only. Scope the allow to `#[cfg(test)]` so prod code
// stays covered.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod analysis;
pub mod budget;
#[cfg(feature = "cli")]
pub mod cli;
pub mod dedup;
#[cfg(feature = "lang-ts")]
pub mod enrich;
pub mod error;
#[cfg(feature = "lang-ts")]
pub mod indices;
#[cfg(feature = "cli")]
pub mod output;
#[cfg(feature = "lang-ts")]
pub mod pipeline;
pub mod query;
pub mod rank;
pub mod text_search;
pub mod tokens;
pub mod types;

#[cfg(test)]
mod tests_types;
```

Modules gated on `lang-ts`: `enrich`, `indices`, `pipeline`. Modules gated on `cli`: `cli`, `output`. Everything else is always compiled.

# Section 3: error.rs — MonoklError enum verbatim

```rust
use camino::Utf8PathBuf;
use io_errors::FileIoError;

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum MonoklError {
    #[error(transparent)]
    Io(#[from] FileIoError),

    #[error("walker error at {path}")]
    Walk {
        path: Utf8PathBuf,
        #[source]
        source: ignore::Error,
    },

    #[error("regex compilation failed")]
    RegexBuild(#[from] grep_regex::Error),

    #[error("non-UTF-8 path encountered: {path:?}")]
    NonUtf8Path { path: std::path::PathBuf },

    #[error("query has too many terms: {count} > {limit}")]
    TooManyTerms { count: usize, limit: usize },

    #[error("tokenizer init failed")]
    TokenizerInit,

    #[error("json parsing or serialization failed")]
    Json(#[from] serde_json::Error),

    #[error("path outside workspace root: {path}")]
    PathOutsideRoot { path: Utf8PathBuf },

    #[error("disk cache is stale (version or config hash mismatch)")]
    StaleDiskCache,

    #[error("invalid git ref {ref_:?}: {reason}")]
    InvalidGitRef { ref_: String, reason: &'static str },

    #[error("git {operation} failed: {message}")]
    Git { operation: &'static str, message: String },

    #[error("refusing to read symlink: {path}")]
    SymlinkRejected { path: Utf8PathBuf },

    #[error("file too large: {path} is {size} bytes, cap is {cap}")]
    FileTooLarge { path: Utf8PathBuf, size: u64, cap: u64 },

    #[error("internal lock poisoned in {context} — a prior panic left shared state possibly \
             inconsistent; this is a bug, please report")]
    LockPoisoned { context: &'static str },

    #[error("ambiguous dependency resolution requires at least one candidate")]
    EmptyAmbiguousPaths,
}

pub type Result<T> = std::result::Result<T, MonoklError>;
```

> **Post-research correction.** `InvalidGitRef`/`Git`/`SymlinkRejected`/`FileTooLarge` are added here — Part 3's `git_scope.rs` and `io_safety.rs` construct all four, but an earlier draft never declared them on the enum, a second genuine compile-breaking gap caught alongside §11's. `EmptyAmbiguousPaths` supports the validated dependency-resolution constructor added in §4. Two variants from that earlier draft are removed: `Parse { path, details: String }` and `Query { input, message: String }` are never constructed anywhere in this document — parse failures are surfaced via `FileAnalysis::had_parse_errors` + `tracing::warn!` (§17), and query failures only ever raise `TooManyTerms`. `#[non_exhaustive]` means downstream `match` arms must already use a wildcard, so removing genuinely dead variants is a clean cut, not a breaking one.

# Section 4: types.rs — all public types verbatim

```rust
use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::MonoklError;

// ── Symbol kind ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    // Universal — all languages
    Function,
    Method,
    Constructor,
    Class,
    Struct,
    Enum,
    Interface,
    TypeAlias,
    Property,
    Field,
    Variable,
    Module,
    // Rust-specific
    Impl,
    Macro,
    // Other
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    Public,
    Crate,
    Module,
    Private,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRecord {
    pub line: usize,
    #[serde(default)]
    pub bindings: Vec<DependencyBinding>,
    pub target: DependencyTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyBinding {
    pub imported: String,
    pub local: String,
    pub kind: BindingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BindingKind {
    Named,
    Default,
    Namespace,
    Glob,
    NamespaceWide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[non_exhaustive]
pub enum DependencyTarget {
    File {
        specifier: String,
        resolution: DependencyResolution,
        is_relative: bool,
    },
    RustPath {
        segments: Vec<String>,
        anchor: RustPathAnchor,
        resolution: DependencyResolution,
    },
    Namespace {
        segments: Vec<String>,
        is_static: bool,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmbiguousPaths {
    first: Utf8PathBuf,
    additional: BTreeSet<Utf8PathBuf>,
}

impl AmbiguousPaths {
    pub fn try_new(candidates: Vec<Utf8PathBuf>) -> std::result::Result<Self, MonoklError> {
        let mut candidates: BTreeSet<_> = candidates.into_iter().collect();
        let Some(first) = candidates.iter().next().cloned() else {
            return Err(MonoklError::EmptyAmbiguousPaths);
        };
        candidates.remove(&first);
        Ok(Self { first, additional: candidates })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Utf8Path> {
        std::iter::once(self.first.as_path())
            .chain(self.additional.iter().map(Utf8PathBuf::as_path))
    }
}

impl<'de> Deserialize<'de> for AmbiguousPaths {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            first: Utf8PathBuf,
            additional: BTreeSet<Utf8PathBuf>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let mut candidates: Vec<_> = wire.additional.into_iter().collect();
        candidates.push(wire.first);
        Self::try_new(candidates).map_err(serde::de::Error::custom)
    }
}

/// Resolution is lossless: callers must not silently select the first candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
#[non_exhaustive]
pub enum DependencyResolution {
    Resolved { path: Utf8PathBuf },
    Ambiguous { candidates: AmbiguousPaths, diagnostic: Diagnostic },
    Unresolved { diagnostic: Diagnostic },
    External,
}

impl DependencyResolution {
    pub fn resolved_path(&self) -> Option<&Utf8Path> {
        match self {
            Self::Resolved { path } => Some(path.as_path()),
            Self::Ambiguous { .. } | Self::Unresolved { .. } | Self::External => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RustPathAnchor {
    Crate,
    Super,
    #[serde(rename = "self")]
    Selff,
    Extern(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    pub name: String,
    pub line: usize,
    pub re_export: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsxAttribute {
    pub name: String,
    pub string_value: Option<String>,
    pub is_expression: bool,
    pub is_spread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsxElementEntry {
    pub name: String,
    pub is_html: bool,
    pub line: usize,
    pub attributes: Vec<JsxAttribute>,
}

> **Post-research correction.** `Go`/`Java` replace an earlier `CSharp` variant — the language
> roadmap dropped C# and added Go and Java instead (see
> [`05-research-and-decisions.md` §6](./05-research-and-decisions.md)). `GoData`/`JavaData` are
> empty placeholders today, same as `PythonData` — populated once those analyzers land.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "language", content = "data")]
#[non_exhaustive]
pub enum LangData {
    #[serde(rename = "typescript")]
    Ts(TsData),
    #[serde(rename = "rust")]
    Rust(RustData),
    #[serde(rename = "python")]
    Python(PythonData),
    #[serde(rename = "go")]
    Go(GoData),
    #[serde(rename = "java")]
    Java(JavaData),
}

impl LangData {
    pub fn ts(&self) -> Option<&TsData> {
        if let LangData::Ts(ts) = self { Some(ts) } else { None }
    }
    pub fn jsx_elements(&self) -> &[JsxElementEntry] {
        self.ts().map_or(&[], |ts| ts.jsx_elements.as_slice())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TsData {
    #[serde(default)]
    pub jsx_elements: Vec<JsxElementEntry>,
    #[serde(default)]
    pub type_only_imports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustData {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonData {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoData {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaData {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeBlock {
    pub file: Utf8PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub node_kind: SymbolKind,
    pub code: String,
    pub symbol_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_lines: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedBlock {
    #[serde(flatten)]
    pub block: CodeBlock,
    pub bm25_score: f64,
    pub coverage_boost: f64,
    pub node_type_boost: f64,
    pub final_score: f64,
    pub rank: usize,
    pub parent_context: Option<ParentContext>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentContext {
    pub kind: SymbolKind,
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub path: Option<Utf8PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticKind {
    Degraded,
    Skipped,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub query: String,
    pub path: Utf8PathBuf,
    pub allow_tests: bool,
    pub no_gitignore: bool,
    pub limits: SearchLimits,
    pub exact: bool,
    pub language: Option<Language>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query: String::new(),
            path: Utf8PathBuf::new(),
            allow_tests: false,
            no_gitignore: false,
            limits: SearchLimits::default(),
            exact: false,
            language: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchLimits {
    pub max_results: Option<usize>,
    pub max_bytes: usize,
    pub max_tokens: Option<usize>,
    pub max_candidates: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_results: Some(50),
            max_bytes: 2_097_152,
            max_tokens: Some(20_000),
            max_candidates: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[cfg_attr(feature = "cli", value(name = "typescript"))]
    TypeScript,
    #[cfg_attr(feature = "cli", value(name = "javascript"))]
    JavaScript,
    #[cfg_attr(feature = "cli", value(name = "rust"))]
    Rust,
    #[cfg_attr(feature = "cli", value(name = "python"))]
    Python,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<RankedBlock>,
    pub total_blocks_before_truncation: usize,
    pub truncated: bool,
    pub truncation_marker: Option<String>,
    pub total_bytes: usize,
    pub total_tokens: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolsResult {
    pub files: BTreeMap<Utf8PathBuf, Vec<SymbolEntry>>,
    pub total_symbol_count: usize,
    pub truncation_marker: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependentsResult {
    pub file: Utf8PathBuf,
    pub dependents: Vec<Utf8PathBuf>,
    pub imports: Vec<Utf8PathBuf>,
    pub total_dependent_count: usize,
    pub total_import_count: usize,
    pub truncation_marker: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct LineHit {
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum TsconfigMode {
    Auto,
    Manual(Utf8PathBuf),
    Skip,
}

#[derive(Debug, Clone)]
pub struct WorkspaceOptions {
    pub root: Utf8PathBuf,
    pub tsconfig: TsconfigMode,
}

impl WorkspaceOptions {
    pub fn new(root: impl Into<Utf8PathBuf>) -> Self {
        Self {
            root: root.into(),
            tsconfig: TsconfigMode::Auto,
        }
    }
    #[must_use]
    pub fn with_tsconfig(mut self, mode: TsconfigMode) -> Self {
        self.tsconfig = mode;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRequest {
    pub file: Utf8PathBuf,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
}
```

Serde fixtures lock `DependencyResolution`'s tagged wire shape and behavioral
fixtures cover all four outcomes, including the non-empty/sorted/deduplicated
ambiguous-candidate invariant.

# Section 5: query/ — lexer, parser, plan verbatim

## query/mod.rs

```rust
pub mod lexer;
pub mod parser;
pub mod plan;

pub use parser::{Modifier, ParsedTerm, parse};
pub use plan::QueryPlan;
```

## query/lexer.rs

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Plus,
    Minus,
    RegexPrefix,
    QuotedString(String),
    Word(String),
    Eof,
}

pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_ascii_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn read_word(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.peek() {
                None => break,
                Some(ch) if ch.is_ascii_whitespace() => break,
                Some(ch) => {
                    s.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        s
    }

    fn read_quoted(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.advance() {
                None | Some('"') => break,
                Some('\\') => {
                    match self.peek() {
                        Some('"') => {
                            self.pos += 1;
                            s.push('"');
                        }
                        _ => s.push('\\'),
                    }
                }
                Some(ch) => s.push(ch),
            }
        }
        s
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        match self.peek() {
            None => Token::Eof,
            Some('+') => { self.pos += 1; Token::Plus }
            Some('-') => { self.pos += 1; Token::Minus }
            Some('"') => { self.pos += 1; Token::QuotedString(self.read_quoted()) }
            Some(_) => {
                if self.remaining().starts_with("regex:") {
                    self.pos += "regex:".len();
                    Token::RegexPrefix
                } else {
                    Token::Word(self.read_word())
                }
            }
        }
    }
}
```

Rules: `+`/`-` consumed as Token::Plus/Token::Minus only when they begin a term (`read_word` greedily consumes everything except whitespace). `regex:` (case-sensitive) emits RegexPrefix, the next token is the pattern. `\"` is the only recognized escape in quoted strings.

## query/parser.rs

```rust
use super::lexer::{Lexer, Token};
use crate::error::{MonoklError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modifier {
    Required,
    Excluded,
    Optional,
}

#[derive(Debug, Clone)]
pub struct ParsedTerm {
    pub modifier: Modifier,
    pub pattern: String,
    pub is_regex: bool,
}

pub fn parse(input: &str) -> Result<Vec<ParsedTerm>> {
    const LIMIT: usize = 64;
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut lexer = Lexer::new(input);
    let mut terms: Vec<ParsedTerm> = Vec::new();
    loop {
        let tok = lexer.next_token();
        match tok {
            Token::Eof => break,
            Token::Plus => {
                let content = lexer.next_token();
                if let Some(term) = content_to_term(content, Modifier::Required) {
                    terms.push(term);
                }
            }
            Token::Minus => {
                let content = lexer.next_token();
                if let Some(term) = content_to_term(content, Modifier::Excluded) {
                    terms.push(term);
                }
            }
            Token::RegexPrefix => {
                let pattern_tok = lexer.next_token();
                match pattern_tok {
                    Token::Word(raw) | Token::QuotedString(raw) => {
                        terms.push(ParsedTerm {
                            modifier: Modifier::Optional,
                            pattern: raw,
                            is_regex: true,
                        });
                    }
                    _ => {}
                }
            }
            Token::Word(w) => {
                terms.push(ParsedTerm {
                    modifier: Modifier::Optional,
                    pattern: regex::escape(&w),
                    is_regex: false,
                });
            }
            Token::QuotedString(s) => {
                terms.push(ParsedTerm {
                    modifier: Modifier::Optional,
                    pattern: regex::escape(&s),
                    is_regex: false,
                });
            }
        }
    }
    if terms.len() > LIMIT {
        return Err(MonoklError::TooManyTerms {
            count: terms.len(),
            limit: LIMIT,
        });
    }
    Ok(terms)
}

fn content_to_term(tok: Token, modifier: Modifier) -> Option<ParsedTerm> {
    match tok {
        Token::Word(w) => Some(ParsedTerm {
            modifier,
            pattern: regex::escape(&w),
            is_regex: false,
        }),
        Token::QuotedString(s) => Some(ParsedTerm {
            modifier,
            pattern: regex::escape(&s),
            is_regex: false,
        }),
        Token::RegexPrefix => None, // `+regex:foo` documented as silently losing modifier
        _ => None,
    }
}
```

Limit: `const LIMIT: usize = 64`. Whitespace-only input returns an empty vec without error.

## query/plan.rs

```rust
use super::parser::{Modifier, ParsedTerm};

#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub terms: Vec<ParsedTerm>,
    pub required: Vec<usize>,
    pub excluded: Vec<usize>,
    pub scored: Vec<usize>,
}

impl QueryPlan {
    pub fn from_terms(terms: Vec<ParsedTerm>) -> Self {
        let required: Vec<usize> = terms.iter().enumerate()
            .filter(|(_, t)| t.modifier == Modifier::Required)
            .map(|(i, _)| i).collect();
        let excluded: Vec<usize> = terms.iter().enumerate()
            .filter(|(_, t)| t.modifier == Modifier::Excluded)
            .map(|(i, _)| i).collect();
        let scored: Vec<usize> = terms.iter().enumerate()
            .filter(|(_, t)| t.modifier == Modifier::Optional)
            .map(|(i, _)| i).collect();
        Self { terms, required, excluded, scored }
    }

    pub fn is_empty(&self) -> bool { self.terms.is_empty() }

    pub fn search_patterns(&self) -> Vec<&str> {
        self.required.iter().chain(self.scored.iter())
            .map(|&i| self.terms[i].pattern.as_str())
            .collect()
    }
}
```

`search_patterns()` returns required followed by scored — never excluded (those become a post-match filter).

# Section 6: tokens.rs verbatim

```rust
static STOP_WORDS: &[&str] = &[
    "the","and","for","are","but","not","you","all","can","had","her","was","one","our",
    "out","day","get","has","him","his","how","its","may","new","now","old","see","two",
    "use","way","who","did","let","put","too","any",
    // common code stop words
    "fn","var","const","mut","pub","use","mod","impl","type","enum","struct","trait",
    "return","import","export","from","default","class","extends","interface",
    // added post-research: near-universal Rust tokens omitted from the original list
    // (found via a dedicated tokenizer audit against monokl's own splitting rules)
    "crate","self","super","dyn","async","await","unsafe",
];

pub fn tokenize(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty() { return result; }
    let mut current_word = String::new();
    for idx in 0..chars.len() {
        let ch = chars[idx];
        if ch == '_' || ch == '-' || !ch.is_ascii_alphanumeric() {
            if !current_word.is_empty() {
                result.push(current_word.clone());
                current_word.clear();
            }
            continue;
        }
        let should_break = if idx > 0 {
            let prev_ch = chars[idx - 1];
            if prev_ch.is_ascii_lowercase() && ch.is_ascii_uppercase() {
                true
            } else if prev_ch.is_ascii_uppercase() && ch.is_ascii_uppercase() {
                idx + 1 < chars.len() && chars[idx + 1].is_ascii_lowercase()
            } else if prev_ch.is_ascii_uppercase() && ch.is_ascii_lowercase() {
                false
            } else if prev_ch.is_ascii_alphabetic() && ch.is_ascii_digit() {
                false
            } else if prev_ch.is_ascii_digit() && ch.is_ascii_alphabetic() {
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_break && !current_word.is_empty() {
            result.push(current_word.clone());
            current_word.clear();
        }
        current_word.push(ch.to_ascii_lowercase());
    }
    if !current_word.is_empty() { result.push(current_word); }
    result.into_iter()
        .filter(|token| token.len() >= 2 && !STOP_WORDS.contains(&token.as_str()))
        .collect()
}

pub fn tokenize_source(source: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_fragment = String::new();
    for ch in source.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current_fragment.push(ch);
        } else if !current_fragment.is_empty() {
            let tokens = tokenize(&current_fragment);
            result.extend(tokens);
            current_fragment.clear();
        }
    }
    if !current_fragment.is_empty() {
        let tokens = tokenize(&current_fragment);
        result.extend(tokens);
    }
    result
}
```

Split transitions: lower→UPPER breaks; UPPER→UPPER breaks only if next char is lowercase (handles "XMLParser" → "xml","parser"); UPPER→lower never breaks; letter→digit doesn't break (keeps "v8"); digit→letter breaks. Filter: token.len() >= 2 AND not in STOP_WORDS. `tokenize_source` preserves duplicates for BM25 TF.

# Section 7: text_search.rs verbatim

```rust
use std::collections::BTreeMap;
use camino::{Utf8Path, Utf8PathBuf};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{SearcherBuilder, sinks::UTF8};
use ignore::WalkBuilder;
use crate::error::Result;
use crate::types::LineHit;

pub fn search_files(
    root: &Utf8Path,
    patterns: &[&str],
    allow_tests: bool,
    no_gitignore: bool,
    max_candidates: usize,
) -> Result<BTreeMap<Utf8PathBuf, Vec<LineHit>>> {
    let combined = patterns.join("|");
    let matcher = RegexMatcherBuilder::new().case_insensitive(true).build(&combined)?;
    let walk = WalkBuilder::new(root)
        .standard_filters(!no_gitignore)
        .build();
    let mut results: BTreeMap<Utf8PathBuf, Vec<LineHit>> = BTreeMap::new();
    let mut candidate_count = 0usize;
    for entry in walk {
        let entry = entry.map_err(|e| crate::error::MonoklError::Walk {
            path: root.to_owned(),
            source: e,
        })?;
        if !entry.file_type().is_some_and(|t| t.is_file()) { continue; }
        let path = match Utf8PathBuf::from_path_buf(entry.into_path()) {
            Ok(p) => p,
            Err(_non_utf8) => continue,
        };
        if !allow_tests && is_test_path(&path) { continue; }
        if candidate_count >= max_candidates { break; }
        let mut hits: Vec<LineHit> = Vec::new();
        let mut searcher = SearcherBuilder::new().build();
        if let Err(e) = searcher.search_path(
            &matcher,
            path.as_std_path(),
            UTF8(|line_number, line| {
                #[allow(clippy::cast_possible_truncation)]
                hits.push(LineHit {
                    line_number: line_number as usize,
                    text: line.trim_end_matches('\n').to_owned(),
                });
                Ok(true)
            }),
        ) {
            tracing::warn!("search_path error on {path}: {e}");
            continue;
        }
        if !hits.is_empty() {
            candidate_count += 1;
            results.insert(path, hits);
        }
    }
    Ok(results)
}

fn is_test_path(path: &Utf8Path) -> bool {
    let s = path.as_str();
    s.contains("/__tests__/")
        || s.contains("/test/")
        || s.contains("/tests/")
        || s.contains("/spec/")
        || s.ends_with(".test.ts")
        || s.ends_with(".test.tsx")
        || s.ends_with(".test.js")
        || s.ends_with(".spec.ts")
        || s.ends_with(".spec.tsx")
        || s.ends_with(".spec.js")
}
```

`standard_filters(!no_gitignore)` — true by default (respects .gitignore/.ignore). Patterns joined with `|`. Note: trailing newline trimmed from line text.

# Section 8: rank/ — bm25, boost, tokenize_doc, mod verbatim

## rank/mod.rs

```rust
pub mod bm25;
pub mod boost;
pub mod tokenize_doc;

pub use bm25::{round6, score};
pub use boost::{coverage_boost, node_type_boost};
pub use tokenize_doc::tokenize_block;

use crate::types::{CodeBlock, RankedBlock};

#[allow(clippy::cast_precision_loss)]
pub fn rank_blocks<S: std::hash::BuildHasher>(
    blocks: Vec<(CodeBlock, Vec<String>)>,
    query_terms: &[&str],
    df: &std::collections::HashMap<String, usize, S>,
) -> Vec<RankedBlock> {
    if blocks.is_empty() { return Vec::new(); }
    let total_tokens: usize = blocks.iter().map(|(_, toks)| toks.len()).sum();
    let avg_doc_len = total_tokens as f64 / blocks.len() as f64;
    let corpus_size = blocks.len();
    let mut ranked: Vec<RankedBlock> = blocks.into_iter().map(|(block, doc_tokens)| {
        let bm25 = bm25::score(&doc_tokens, query_terms, avg_doc_len, corpus_size, df);
        let cov = boost::coverage_boost(&block);
        let ntype = boost::node_type_boost(block.node_kind);
        let final_score = round6(bm25 * ntype + cov);
        RankedBlock {
            block,
            bm25_score: bm25,
            coverage_boost: cov,
            node_type_boost: ntype,
            final_score,
            rank: 0,
            parent_context: None,
        }
    }).collect();
    ranked.sort_by(|a, b| {
        b.final_score.partial_cmp(&a.final_score).unwrap_or(std::cmp::Ordering::Equal)
    });
    for (i, block) in ranked.iter_mut().enumerate() { block.rank = i + 1; }
    ranked
}
```

Final score formula: `round6(bm25 * node_type_boost + coverage_boost)`. Sort descending by `final_score`, 1-based rank.

## rank/bm25.rs

```rust
const K1: f64 = 1.5;
const B: f64 = 0.5;
const DP: f64 = 1_000_000.0;

pub fn round6(x: f64) -> f64 {
    (x * DP).round() / DP
}

#[allow(clippy::cast_precision_loss)]
pub fn score<S: std::hash::BuildHasher>(
    doc_tokens: &[String],
    query_terms: &[&str],
    avg_doc_len: f64,
    corpus_size: usize,
    df: &std::collections::HashMap<String, usize, S>,
) -> f64 {
    if query_terms.is_empty() || doc_tokens.is_empty() { return 0.0; }
    let doc_len = doc_tokens.len() as f64;
    let n = corpus_size as f64;
    let mut total_score = 0.0;
    for term in query_terms {
        let df_t = df.get(*term).copied().unwrap_or(0) as f64;
        let tf = doc_tokens.iter().filter(|t| t == term).count() as f64;
        if tf == 0.0 { continue; }
        let idf = ((n - df_t + 0.5) / (df_t + 0.5) + 1.0).ln();
        let norm_factor = K1 * (1.0 - B + B * (doc_len / avg_doc_len));
        let tf_norm = (tf * (K1 + 1.0)) / (tf + norm_factor);
        total_score += idf * tf_norm;
    }
    round6(total_score)
}
```

Constants: `K1=1.5`, `B=0.5`, `DP=1_000_000.0` (6 decimal places). Formula: `IDF = ln((N - df + 0.5)/(df + 0.5) + 1)`; `norm_factor = K1 * (1 - B + B * doc_len/avg_doc_len)`; `tf_norm = (tf*(K1+1))/(tf + norm_factor)`; final BM25 = sum over terms, then round6.

## rank/boost.rs

```rust
use crate::types::{CodeBlock, SymbolKind};

const DP: f64 = 1_000_000.0;

fn round6(x: f64) -> f64 { (x * DP).round() / DP }

#[allow(clippy::cast_precision_loss)]
pub fn coverage_boost(block: &CodeBlock) -> f64 {
    if block.matched_lines.is_empty() { return 0.0; }
    let block_lines: std::collections::HashSet<usize> = (block.line_start..=block.line_end).collect();
    let matched_in_block = block.matched_lines.iter().filter(|l| block_lines.contains(l)).count();
    round6(matched_in_block as f64 / block.matched_lines.len() as f64)
}

pub fn node_type_boost(kind: SymbolKind) -> f64 {
    match kind {
        SymbolKind::Function => 1.5,
        SymbolKind::Class | SymbolKind::Method | SymbolKind::Constructor => 1.4,
        SymbolKind::Interface | SymbolKind::Struct => 1.3,
        SymbolKind::TypeAlias | SymbolKind::Enum => 1.2,
        SymbolKind::Variable | SymbolKind::Impl => 1.1,
        SymbolKind::Property | SymbolKind::Field | SymbolKind::Macro => 1.0,
        SymbolKind::Module => 0.9,
        SymbolKind::Other => 0.7,
    }
}
```

Exact node_type_boost values: Function=1.5; Class/Method/Constructor=1.4; Interface/Struct=1.3; TypeAlias/Enum=1.2; Variable/Impl=1.1; Property/Field/Macro=1.0; Module=0.9; Other=0.7. Coverage boost is `matched_in_block / len(matched_lines)` rounded to 6dp; 0 if matched_lines empty.

## rank/tokenize_doc.rs

```rust
pub fn tokenize_block(block: &crate::types::CodeBlock) -> Vec<String> {
    let mut tokens = crate::tokens::tokenize_source(&block.code);
    if let Some(sig) = &block.symbol_signature {
        tokens.extend(crate::tokens::tokenize_source(sig));
    }
    tokens
}
```

# Section 9: dedup.rs verbatim

```rust
use crate::types::{CodeBlock, RankedBlock};

pub fn dedup_blocks(blocks: Vec<RankedBlock>) -> Vec<RankedBlock> {
    let mut result: Vec<RankedBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let is_duplicate = result.iter().any(|kept| {
            kept.block.file == block.block.file && overlaps_significantly(&kept.block, &block.block)
        });
        if !is_duplicate {
            result.push(block);
        }
    }
    for (i, b) in result.iter_mut().enumerate() {
        b.rank = i + 1;
    }
    result
}

fn overlaps_significantly(a: &CodeBlock, b: &CodeBlock) -> bool {
    let overlap_start = a.line_start.max(b.line_start);
    let overlap_end = a.line_end.min(b.line_end);
    if overlap_start > overlap_end { return false; }
    let overlap_lines = overlap_end - overlap_start + 1;
    // `saturating_sub` guards against a CodeBlock with line_end < line_start — nothing enforces
    // that invariant at the type level, so an underflow panic is a real possibility, not a
    // theoretical one, for a hand-constructed or future-analyzer-produced CodeBlock.
    let smaller_len = (a.line_end.saturating_sub(a.line_start) + 1)
        .min(b.line_end.saturating_sub(b.line_start) + 1);
    overlap_lines * 2 > smaller_len
}
```

Dedup rule: same file AND `overlap_lines * 2 > smaller_block_length` (i.e., >50% overlap of the smaller block). Input is assumed sorted desc by `final_score`; first kept wins. Ranks reassigned 1-based after dedup.

# Section 10: budget.rs verbatim

```rust
use std::sync::OnceLock;
use tiktoken_rs::CoreBPE;
use crate::error::{MonoklError, Result};
use crate::types::{Diagnostic, RankedBlock, SearchLimits, SearchResponse};

static TOKENIZER: OnceLock<CoreBPE> = OnceLock::new();

fn tokenizer() -> Result<&'static CoreBPE> {
    if let Some(bpe) = TOKENIZER.get() { return Ok(bpe); }
    let bpe = tiktoken_rs::o200k_base().map_err(|_| MonoklError::TokenizerInit)?;
    Ok(TOKENIZER.get_or_init(|| bpe))
}

pub fn count_tokens(s: &str) -> Result<usize> {
    Ok(tokenizer()?.encode_with_special_tokens(s).len())
}

pub fn apply_budget(
    blocks: Vec<RankedBlock>,
    limits: &SearchLimits,
    diagnostics: Vec<Diagnostic>,
) -> Result<SearchResponse> {
    let total_before = blocks.len();
    let bpe = tokenizer()?;
    let max_results = limits.max_results.unwrap_or(50);
    let max_bytes = limits.max_bytes.min(2_097_152);
    let max_tokens = limits.max_tokens.unwrap_or(20_000);
    let mut results = Vec::new();
    let mut total_bytes = 0usize;
    let mut total_tokens = 0usize;
    let mut truncated = false;
    for block in blocks {
        if results.len() >= max_results { truncated = true; break; }
        let block_bytes = block.block.code.len();
        if total_bytes + block_bytes > max_bytes { truncated = true; break; }
        let block_tokens = bpe.encode_with_special_tokens(&block.block.code).len();
        if total_tokens + block_tokens > max_tokens { truncated = true; break; }
        total_bytes += block_bytes;
        total_tokens += block_tokens;
        results.push(block);
    }
    let truncation_marker = if truncated {
        let omitted = total_before - results.len();
        Some(format!("[+{omitted} more results omitted; refine with +term or -term]"))
    } else { None };
    Ok(SearchResponse {
        results,
        total_blocks_before_truncation: total_before,
        truncated,
        truncation_marker,
        total_bytes,
        total_tokens,
        diagnostics,
    })
}
```

Defaults applied here when `Option` is `None`: max_results=50, max_tokens=20_000. Hard ceiling: `max_bytes.min(2_097_152)`. Truncation marker format: `"[+N more results omitted; refine with +term or -term]"`.

# Section 11: analysis/lang.rs — LanguageAnalyzer trait verbatim

> **Post-research correction.** The trait below is shown in its final, complete shape — including `language_id`/`capabilities`/`analyze_with_profile`, which are only exercised starting in Part 3's multi-language rework (§3-4). An earlier draft of this spec showed only `supports`/`analyze` here and left `AnalyzerRegistry`/`query_support.rs` (Part 3) calling methods that didn't exist on the trait — a genuine compile-breaking gap caught in a post-hoc Rust-idiom audit. Rust traits are declared once, not incrementally per "part," so the full shape belongs here; `impl LanguageAnalyzer for TsAnalyzer` grows its concrete `language_id`/`capabilities`/`analyze_with_profile` methods in Part 3 §3 alongside the registry that first calls them — v0.1.0 code only ever exercises `supports`/`analyze` (inherited from the default impl below) and never observes the rest.

```rust
use std::sync::Arc;
use camino::Utf8Path;
use crate::error::Result;
use super::file_analysis::FileAnalysis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    TypeScript,
    Rust,
    Python,
    Go,
    Java,
    /// Generic tree-sitter fallback tier (05-research-and-decisions.md §10), `Heuristic` unless
    /// otherwise noted; `Cpp` is `Heuristic` only pending the optional Clang tier in §12.
    C,
    Cpp,
    Bash,
    CSharp,
    Ruby,
    Php,
    /// Fallback-tier `Heuristic` presence only (§10) — each has a hard grammar-accuracy ceiling
    /// that rules out `Structural` via tree-sitter alone; the optional JVM-subprocess tier in
    /// §12 is the real path to `Structural` for both.
    Kotlin,
    Scala,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityPrecision {
    Unsupported,
    Heuristic,
    Structural,
    Exact,
}

#[derive(Debug, Clone, Copy)]
pub struct AnalyzerCapabilities {
    pub blocks: bool,
    pub classified_imports: bool,
    pub resolved_import_graph: CapabilityPrecision,
    pub refs: CapabilityPrecision,
    pub definition: CapabilityPrecision,
    pub inspect_detail: CapabilityPrecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisProfile {
    /// Only `dependencies` are needed — skips symbol/export/block extraction entirely.
    Dependencies,
    /// Symbols, dependencies, exports — skips the language-specific inspect-entry pass.
    Structural,
    /// Everything, including the per-language `InspectEntry` classification.
    Full,
}

pub trait LanguageAnalyzer: Send + Sync {
    /// Every language this analyzer instance handles. Single-language analyzers (`TsAnalyzer`,
    /// `RustAnalyzer`) return a one-element slice; the generic fallback-tier analyzer
    /// (05-research-and-decisions.md §10) returns several.
    fn languages(&self) -> &[LanguageId];
    fn supports(&self, path: &Utf8Path) -> bool;
    /// Which specific language `path` maps to, for analyzers that own more than one. Returns
    /// `None` only if `supports(path)` would also be `false`. Single-language analyzers implement
    /// this as `self.supports(path).then_some(<their one LanguageId>)`; the fallback-tier
    /// analyzer resolves it via its own internal extension table (§10).
    fn language_for(&self, path: &Utf8Path) -> Option<LanguageId>;
    /// Capabilities for one specific language this analyzer handles — callers must only pass a
    /// value `language_for`/`languages` actually returned. Per-language rather than per-analyzer
    /// because a single fallback-tier instance can have genuinely different ceilings per language
    /// (e.g. C reaches `Structural`, C++ is capped at `Heuristic` pending §12's Clang tier).
    fn capabilities(&self, language: LanguageId) -> AnalyzerCapabilities;

    fn analyze_with_profile(
        &self,
        path: &Utf8Path,
        source: Box<dyn FnOnce() -> Result<String>>,
        profile: AnalysisProfile,
    ) -> Result<Arc<FileAnalysis>>;

    /// Convenience wrapper — full-fidelity analysis, the only profile v0.1.0 code ever requests.
    fn analyze(&self, path: &Utf8Path, source: Box<dyn FnOnce() -> Result<String>>) -> Result<Arc<FileAnalysis>> {
        self.analyze_with_profile(path, source, AnalysisProfile::Full)
    }
}
```

# Section 12: analysis/content_hash.rs verbatim

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn of(content: &[u8]) -> Self {
        Self(blake3::hash(content).to_hex().to_string())
    }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

# Section 13: analysis/file_analysis.rs verbatim

```rust
use camino::Utf8PathBuf;
use crate::types::{CodeBlock, DependencyRecord, ExportRecord, JsxElementEntry, LangData, SymbolEntry};
use super::content_hash::ContentHash;

#[derive(Debug, Clone)]
pub struct FileAnalysis {
    pub source_path: Utf8PathBuf,
    pub content_hash: ContentHash,
    pub had_parse_errors: bool,
    pub symbols: Vec<SymbolEntry>,
    pub dependencies: Vec<DependencyRecord>,
    pub exports: Vec<ExportRecord>,
    pub blocks: Vec<CodeBlock>,
    pub lang: LangData,
}

impl FileAnalysis {
    pub fn language(&self) -> &str {
        match &self.lang {
            LangData::Ts(_) => "typescript",
            LangData::Rust(_) => "rust",
            LangData::Python(_) => "python",
            LangData::Go(_) => "go",
            LangData::Java(_) => "java",
        }
    }
    pub fn jsx_elements(&self) -> &[JsxElementEntry] { self.lang.jsx_elements() }
    pub fn is_typescript(&self) -> bool { matches!(self.lang, LangData::Ts(_)) }
}
```

# Section 14: analysis/node_kind.rs verbatim

```rust
#[cfg(feature = "lang-ts")]
pub(crate) use ts::node_kind_for_statement;
#[cfg(feature = "lang-ts")]
pub(crate) use ts::node_kind_for_declaration;

#[cfg(feature = "lang-ts")]
mod ts {
    use oxc_ast::ast::{Declaration, Statement};
    use crate::types::SymbolKind;

    pub(crate) fn node_kind_for_statement(stmt: &Statement<'_>) -> Option<SymbolKind> {
        match stmt {
            Statement::FunctionDeclaration(_) => Some(SymbolKind::Function),
            Statement::ClassDeclaration(_) => Some(SymbolKind::Class),
            Statement::TSInterfaceDeclaration(_) => Some(SymbolKind::Interface),
            Statement::TSTypeAliasDeclaration(_) => Some(SymbolKind::TypeAlias),
            Statement::TSEnumDeclaration(_) => Some(SymbolKind::Enum),
            Statement::VariableDeclaration(_) => Some(SymbolKind::Variable),
            Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportDefaultDeclaration(_) => Some(SymbolKind::Other),
            _ => None,
        }
    }

    pub(crate) fn node_kind_for_declaration(decl: &Declaration<'_>) -> Option<SymbolKind> {
        match decl {
            Declaration::FunctionDeclaration(_) => Some(SymbolKind::Function),
            Declaration::ClassDeclaration(_) => Some(SymbolKind::Class),
            Declaration::TSInterfaceDeclaration(_) => Some(SymbolKind::Interface),
            Declaration::TSTypeAliasDeclaration(_) => Some(SymbolKind::TypeAlias),
            Declaration::TSEnumDeclaration(_) => Some(SymbolKind::Enum),
            Declaration::VariableDeclaration(_) => Some(SymbolKind::Variable),
            _ => None,
        }
    }
}
```

# Section 15: analysis/cache.rs verbatim

```rust
use std::sync::{Arc, OnceLock};
use camino::Utf8Path;
use dashmap::DashMap;
use super::file_analysis::FileAnalysis;

type InMemoryCache = DashMap<String, Arc<FileAnalysis>>;

fn cache() -> &'static InMemoryCache {
    static CACHE: OnceLock<InMemoryCache> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

pub(crate) fn lookup(path: &Utf8Path) -> Option<Arc<FileAnalysis>> {
    cache().get(path.as_str()).map(|entry_ref| Arc::clone(entry_ref.value()))
}

pub(crate) fn insert(path: &Utf8Path, a: Arc<FileAnalysis>) {
    cache().insert(path.to_string(), a);
}
```

Process-global `DashMap<String, Arc<FileAnalysis>>` keyed by path-string. Insert unconditionally replaces.

# Section 16: analysis/persist.rs verbatim

```rust
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use io_errors::FileIoError;
use tracing::warn;
use crate::error::Result;
use crate::types::{CodeBlock, DependencyRecord, ExportRecord, SymbolEntry, TsData};
use super::content_hash::ContentHash;

const CACHE_VERSION: u32 = 2;
const MAX_CACHE_BYTES: usize = 100 * 1024 * 1024; // 100 MB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedFileAnalysis {
    pub content_hash: ContentHash,
    pub mtime_ns: u64,
    pub size_bytes: u64,
    pub symbols: Vec<SymbolEntry>,
    #[serde(default)]
    pub dependencies: Vec<DependencyRecord>,
    pub exports: Vec<ExportRecord>,
    #[serde(default)]
    pub ts_data: Option<TsData>,
    #[serde(default)]
    pub blocks: Vec<CodeBlock>,
    pub had_parse_errors: bool,
    #[serde(default)]
    pub last_accessed_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheFile {
    pub version: u32,
    pub config_hash: String,
    pub entries: BTreeMap<Utf8PathBuf, PersistedFileAnalysis>,
}

struct CacheState {
    cache_file: Option<CacheFile>,
    write_queue: Vec<(Utf8PathBuf, PersistedFileAnalysis)>,
}

impl CacheState {
    fn new() -> Self { Self { cache_file: None, write_queue: Vec::new() } }
}

fn cache_state() -> &'static Mutex<CacheState> {
    static STATE: OnceLock<Mutex<CacheState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(CacheState::new()))
}

fn lock_state() -> Result<MutexGuard<'static, CacheState>> {
    // A poisoned mutex here means some thread panicked while holding it — with `write_queue`
    // possibly mid-mutation (e.g. drained by `flush()` but not yet re-committed). Silently
    // recovering via `PoisonError::into_inner` would launder that torn state back into use and
    // could persist a lost-write bug to disk without any signal. Propagate instead.
    cache_state().lock().map_err(|_| MonoklError::LockPoisoned { context: "analysis::persist::CacheState" })
}

fn now_ns() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

pub(crate) fn init(root: &Utf8Path, config_hash: &str) -> Result<()> {
    let mut state = lock_state()?;
    if state.cache_file.is_none() {
        state.cache_file = Some(match load_cache(root, config_hash) {
            Ok(c) => c,
            Err(crate::error::MonoklError::StaleDiskCache) => {
                warn!("on-disk cache is stale (version or config_hash mismatch); starting fresh");
                CacheFile {
                    version: CACHE_VERSION,
                    config_hash: config_hash.to_string(),
                    entries: BTreeMap::new(),
                }
            }
            Err(e) => return Err(e),
        });
    }
    Ok(())
}

fn load_cache(root: &Utf8Path, config_hash: &str) -> Result<CacheFile> {
    if root.is_relative() {
        return Err(crate::error::MonoklError::Io(FileIoError::read(
            root.to_owned(),
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "workspace root must be an absolute path"),
        )));
    }
    let cache_path = root.join(".monokl/cache.json");
    if !cache_path.exists() {
        return Ok(CacheFile {
            version: CACHE_VERSION,
            config_hash: config_hash.to_string(),
            entries: BTreeMap::new(),
        });
    }
    let content = std::fs::read_to_string(&cache_path).map_err(|e| FileIoError::read(&cache_path, e))?;
    let cache: CacheFile = serde_json::from_str(&content)?;
    if cache.version != CACHE_VERSION || cache.config_hash != config_hash {
        return Err(crate::error::MonoklError::StaleDiskCache);
    }
    Ok(cache)
}

pub(crate) fn lookup(path: &Utf8Path, mtime_ns: u64, size_bytes: u64) -> Result<Option<PersistedFileAnalysis>> {
    let mut state = lock_state()?;
    if let Some(cache) = &mut state.cache_file {
        if let Some(entry) = cache.entries.get_mut(path) {
            if entry.mtime_ns == mtime_ns && entry.size_bytes == size_bytes {
                entry.last_accessed_ns = now_ns();
                return Ok(Some(entry.clone()));
            }
        }
    }
    Ok(None)
}

pub(crate) fn lookup_by_hash(path: &Utf8Path, hash: &ContentHash) -> Result<Option<PersistedFileAnalysis>> {
    let mut state = lock_state()?;
    if let Some(cache) = &mut state.cache_file {
        if let Some(entry) = cache.entries.get_mut(path) {
            if &entry.content_hash == hash {
                entry.last_accessed_ns = now_ns();
                return Ok(Some(entry.clone()));
            }
        }
    }
    Ok(None)
}

pub(crate) fn refresh_mtime(path: &Utf8Path, mtime_ns: u64, size_bytes: u64) -> Result<()> {
    let mut state = lock_state()?;
    if let Some(cache) = &mut state.cache_file {
        if let Some(entry) = cache.entries.get_mut(path) {
            entry.mtime_ns = mtime_ns;
            entry.size_bytes = size_bytes;
        }
    }
    Ok(())
}

pub(crate) fn queue_write(path: &Utf8Path, entry: PersistedFileAnalysis) {
    if let Ok(mut state) = cache_state().lock() {
        state.write_queue.push((path.to_owned(), entry));
    }
}

pub(crate) fn flush(root: &Utf8Path, config_hash: &str) -> Result<()> {
    let mut state = lock_state()?;
    let mut cache = if let Some(ref existing) = state.cache_file {
        existing.clone()
    } else {
        load_cache(root, config_hash)?
    };
    for (path, entry) in state.write_queue.drain(..) {
        cache.entries.insert(path, entry);
    }
    cache.version = CACHE_VERSION;
    cache.config_hash = config_hash.to_string();
    let json_str = {
        let serialized = serde_json::to_string_pretty(&cache)?;
        if serialized.len() > MAX_CACHE_BYTES {
            evict_lru(&mut cache, MAX_CACHE_BYTES)?
        } else {
            serialized
        }
    };
    let cache_dir = root.join(".monokl");
    std::fs::create_dir_all(&cache_dir).map_err(|e| FileIoError::create_dir(&cache_dir, e))?;
    let cache_path = root.join(".monokl/cache.json");
    let temp_path = root.join(format!(".monokl/cache.json.{}.tmp", std::process::id()));
    std::fs::write(&temp_path, &json_str).map_err(|e| FileIoError::write(&temp_path, e))?;
    std::fs::rename(&temp_path, &cache_path).map_err(|e| FileIoError::write(&cache_path, e))?;
    state.cache_file = Some(cache);
    Ok(())
}

fn evict_lru(cache: &mut CacheFile, max_bytes: usize) -> Result<String> {
    let mut ordered: Vec<(Utf8PathBuf, u64)> = cache.entries.iter()
        .map(|(p, e)| (p.clone(), e.last_accessed_ns)).collect();
    ordered.sort_by_key(|(_, ts)| *ts);
    let chunk = (ordered.len() / 10).max(1);
    let mut in_chunk: usize = 0;
    for (path, _) in &ordered {
        if in_chunk > 0 && in_chunk % chunk == 0 {
            let json = serde_json::to_string_pretty(cache)?;
            if json.len() <= max_bytes {
                return Ok(json);
            }
        }
        cache.entries.remove(path);
        in_chunk += 1;
    }
    serde_json::to_string_pretty(cache).map_err(Into::into)
}
```

Key constants: `CACHE_VERSION = 2`; `MAX_CACHE_BYTES = 100 * 1024 * 1024` (100 MB). Cache file path: `<root>/.monokl/cache.json`. Atomic write: `.monokl/cache.json.<pid>.tmp` then rename (per-process temp prevents nextest races). LRU eviction in batches of `(len/10).max(1)` with re-check after each full chunk. Cache is rejected when `version != CACHE_VERSION || config_hash != ...` → `MonoklError::StaleDiskCache`, treated as cold start in `init()`.

# Section 17: analysis/ts_analyzer.rs verbatim (most important)

```rust
use std::sync::Arc;
use camino::{Utf8Path, Utf8PathBuf};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier,
    JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXElementName,
    JSXMemberExpressionObject, ModuleExportName, Statement,
};
use oxc_parser::Parser;
use oxc_resolver::{ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences};
use oxc_span::{GetSpan, SourceType};
use io_errors::FileIoError;
use tracing::warn;
use crate::error::{MonoklError, Result};
use crate::types::{
    BindingKind, CodeBlock, DependencyBinding, DependencyRecord, DependencyResolution, DependencyTarget,
    Diagnostic, DiagnosticKind, ExportRecord, JsxAttribute, JsxElementEntry, LangData, SymbolEntry,
    SymbolKind, TsData, TsconfigMode, WorkspaceOptions,
};
use super::cache;
use super::content_hash::ContentHash;
use super::file_analysis::FileAnalysis;
use super::lang::LanguageAnalyzer;
use super::node_kind::{node_kind_for_declaration, node_kind_for_statement};
use super::persist;

pub struct TsAnalyzer {
    resolver: Resolver,
    workspace_root: Utf8PathBuf,
}

impl TsAnalyzer {
    // The resolver constructor is infallible today, but normalizing the workspace boundary is not.
    pub fn new(workspace_root: &Utf8Path, tsconfig: Option<&Utf8Path>) -> Result<Self> {
        let raw_root = std::fs::canonicalize(workspace_root.as_std_path())
            .map_err(|e| MonoklError::Io(FileIoError::read(workspace_root, e)))?;
        let workspace_root = Utf8PathBuf::from_path_buf(raw_root)
            .map_err(|path| MonoklError::NonUtf8Path { path })?;
        let tsconfig_opt = tsconfig.map(|p| {
            TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: p.as_std_path().to_owned(),
                references: TsconfigReferences::Auto,
            })
        });
        let resolver = Resolver::new(ResolveOptions {
            tsconfig: tsconfig_opt,
            extensions: vec![
                ".ts".into(), ".tsx".into(), ".mts".into(), ".cts".into(),
                ".js".into(), ".jsx".into(), ".mjs".into(), ".cjs".into(),
                ".json".into(),
            ],
            condition_names: vec!["import".into(), "require".into(), "node".into(), "default".into()],
            ..ResolveOptions::default()
        });
        Ok(Self { resolver, workspace_root })
    }

    pub fn from_workspace_opts(opts: &WorkspaceOptions) -> Result<Self> {
        let tsconfig_path = match &opts.tsconfig {
            TsconfigMode::Auto => find_tsconfig_from_root(&opts.root),
            TsconfigMode::Manual(p) => Some(p.clone()),
            TsconfigMode::Skip => None,
        };
        Self::new(&opts.root, tsconfig_path.as_deref())
    }

    pub fn config_hash_for(opts: &WorkspaceOptions) -> String {
        const OXC_VERSION: &str = "0.128";
        let tsconfig_fragment = match &opts.tsconfig {
            TsconfigMode::Auto => find_tsconfig_from_root(&opts.root),
            TsconfigMode::Manual(p) => Some(p.clone()),
            TsconfigMode::Skip => None,
        }
        .and_then(|p| std::fs::read(&p).ok())
        .map_or_else(|| "no-tsconfig".to_owned(), |bytes| format!("{}", blake3::hash(&bytes)));
        format!("monokl-{}-oxc-{}-lang-ts-{}", env!("CARGO_PKG_VERSION"), OXC_VERSION, tsconfig_fragment)
    }

    fn resolve_import(&self, importer: &Utf8Path, specifier: &str) -> DependencyResolution {
        let Some(dir) = importer.parent() else {
            return DependencyResolution::Unresolved {
                diagnostic: Diagnostic {
                    kind: DiagnosticKind::Degraded,
                    path: Some(importer.to_owned()),
                    message: "importer has no parent directory".to_owned(),
                },
            };
        };
        match self.resolver.resolve(dir, specifier) {
            Ok(resolution) => match Utf8PathBuf::from_path_buf(resolution.full_path().clone()) {
                Ok(path) if path.starts_with(&self.workspace_root) => {
                    DependencyResolution::Resolved { path }
                }
                Ok(_) => DependencyResolution::External,
                Err(_) => DependencyResolution::Unresolved {
                    diagnostic: Diagnostic {
                        kind: DiagnosticKind::Degraded,
                        path: Some(importer.to_owned()),
                        message: format!("resolved import path is not UTF-8: {specifier}"),
                    },
                },
            },
            Err(error) => DependencyResolution::Unresolved {
                diagnostic: Diagnostic {
                    kind: DiagnosticKind::Degraded,
                    path: Some(importer.to_owned()),
                    message: format!("unable to resolve workspace import {specifier}: {error}"),
                },
            },
        }
    }

    pub(crate) fn looks_like_workspace_alias(specifier: &str) -> bool {
        if specifier.starts_with('.') || specifier.starts_with('@') {
            return false;
        }
        specifier.contains('/')
    }
}

impl LanguageAnalyzer for TsAnalyzer {
    fn supports(&self, path: &Utf8Path) -> bool {
        if path.as_str().ends_with(".d.ts") { return false; }
        matches!(path.extension(), Some("ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"))
    }

    fn analyze(&self, path: &Utf8Path, source: Box<dyn FnOnce() -> Result<String>>) -> Result<Arc<FileAnalysis>> {
        // Tier 1: mtime/size fast path
        let meta = std::fs::metadata(path).map_err(|e| FileIoError::read(path, e))?;
        let mtime_ns = meta.modified().map_err(|e| FileIoError::read(path, e))?
            .duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        let size_bytes = meta.len();
        if let Some(persisted) = persist::lookup(path, mtime_ns, size_bytes)? {
            let analysis = Arc::new(analysis_from_persisted(path, persisted));
            cache::insert(path, Arc::clone(&analysis));
            return Ok(analysis);
        }
        // Tier 2: read source + check in-memory cache
        let src = source()?;
        let content_hash = ContentHash::of(src.as_bytes());
        if let Some(cached) = cache::lookup(path) {
            if cached.content_hash == content_hash { return Ok(cached); }
        }
        // Tier 3: content-hash disk cache (mtime changed, content unchanged)
        if let Some(persisted) = persist::lookup_by_hash(path, &content_hash)? {
            persist::refresh_mtime(path, mtime_ns, size_bytes)?;
            let analysis = Arc::new(analysis_from_persisted(path, persisted));
            cache::insert(path, Arc::clone(&analysis));
            return Ok(analysis);
        }
        // Tier 4: full parse
        let analysis = Arc::new(parse_full(self, path, &src, content_hash, mtime_ns, size_bytes));
        cache::insert(path, Arc::clone(&analysis));
        Ok(analysis)
    }
}
```

The full `parse_full` orchestrates extraction:

```rust
fn parse_full(
    analyzer: &TsAnalyzer,
    path: &Utf8Path,
    source: &str,
    content_hash: ContentHash,
    mtime_ns: u64,
    size_bytes: u64,
) -> FileAnalysis {
    let allocator = Allocator::default();
    let source_type = source_type_for(path);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let had_parse_errors = !parsed.errors.is_empty();
    if had_parse_errors {
        let details = parsed.errors.iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>().join("; ");
        warn!(path = %path, errors = %details, "parse errors encountered during analysis");
    }
    let mut symbols: Vec<SymbolEntry> = Vec::new();
    let mut dependencies: Vec<DependencyRecord> = Vec::new();
    let mut exports: Vec<ExportRecord> = Vec::new();
    let mut jsx_elements: Vec<JsxElementEntry> = Vec::new();
    let mut type_only_imports: Vec<String> = Vec::new();
    let mut blocks: Vec<CodeBlock> = Vec::new();
    for stmt in &parsed.program.body {
        extract_stmt(analyzer, path, source, stmt,
                    &mut symbols, &mut dependencies, &mut exports, &mut type_only_imports);
        extract_jsx_from_stmt(source, stmt, &mut jsx_elements);
        extract_block(path, source, stmt, &mut blocks);
    }
    drop(parsed);
    drop(allocator);
    let unresolved_aliases: Vec<String> = dependencies.iter()
        .filter_map(|dep| match &dep.target {
            crate::types::DependencyTarget::File {
                specifier,
                resolution: DependencyResolution::Unresolved { .. },
                ..
            }
                if TsAnalyzer::looks_like_workspace_alias(specifier)
                => Some(specifier.clone()),
            _ => None,
        }).collect();
    let ts_data = TsData {
        jsx_elements: jsx_elements.clone(),
        type_only_imports: type_only_imports.clone(),
        unresolved_aliases,
    };
    let persisted = persist::PersistedFileAnalysis {
        content_hash: content_hash.clone(),
        mtime_ns, size_bytes,
        symbols: symbols.clone(),
        dependencies: dependencies.clone(),
        exports: exports.clone(),
        ts_data: Some(ts_data.clone()),
        blocks: blocks.clone(),
        had_parse_errors,
        last_accessed_ns: 0,
    };
    persist::queue_write(path, persisted);
    FileAnalysis {
        source_path: path.to_owned(),
        content_hash,
        had_parse_errors,
        symbols, dependencies, exports, blocks,
        lang: LangData::Ts(ts_data),
    }
}
```

`extract_stmt` handles all top-level statement kinds. For each `Statement::ImportDeclaration`: pull `import_decl.source.value`, compute line from `import_decl.span.start`. If `import_decl.import_kind.is_type()` push specifier to `type_only_imports` and return. Otherwise call `analyzer.resolve_import(path, specifier)`, set `is_relative = specifier.starts_with('.')`, iterate `import_decl.specifiers` mapping:

- `ImportSpecifier(named)` → `DependencyBinding { imported: module_export_name_to_string(&named.imported), local: named.local.name.as_str().to_string(), kind: BindingKind::Named }`
- `ImportDefaultSpecifier(def)` → `DependencyBinding { imported: local.clone(), local, kind: BindingKind::Default }`
- `ImportNamespaceSpecifier(ns)` → `DependencyBinding { imported: "*".to_string(), local, kind: BindingKind::Namespace }`

Push `DependencyRecord { line, bindings, target: DependencyTarget::File { specifier, resolution: analyzer.resolve_import(path, specifier), is_relative } }`. Resolver failures are `Unresolved { diagnostic }`; a successfully resolved target outside the indexed workspace is `External`; a successful in-workspace UTF-8 path is `Resolved`. The current resolver produces no ambiguous set, but the stored shape retains validated `AmbiguousPaths` for analyzers that do.

For each declaration emit a SymbolEntry as in the source. Note: bare `FunctionDeclaration` uses `first_line_signature(code_slice)` for `signature`; bare `ClassDeclaration`/`TSInterfaceDeclaration`/`TSTypeAliasDeclaration`/`TSEnumDeclaration`/`VariableDeclaration` use `signature: None`.

For `Statement::ExportNamedDeclaration(export_named)`:

- `is_re_export = export_named.source.is_some()`
- For each `specifier` in `export_named.specifiers`: push `ExportRecord { name: module_export_name_to_string(&specifier.exported), line, re_export: is_re_export }`
- If `export_named.declaration.is_some()`: call `extract_export_decl(source, decl, symbols, exports)` which emits both a `SymbolEntry` and a non-re-export `ExportRecord`.

For `Statement::ExportDefaultDeclaration`: push `ExportRecord { name: "default", line, re_export: false }`. If declaration is `FunctionDeclaration` with an id → push `SymbolKind::Function` (with `first_line_signature`); if `ClassDeclaration` with an id → push `SymbolKind::Class`; otherwise no symbol.

`extract_jsx_from_stmt`: walks `ExpressionStatement.expression`, `ReturnStatement.argument`, `VariableDeclaration.declarations[].init`, `ExportNamedDeclaration.declaration` (via `extract_jsx_from_decl`), `ExportDefaultDeclaration` (when inner is `FunctionDeclaration` with body), and `FunctionDeclaration.body`. `collect_jsx_from_expr` recurses through `JSXElement`, `ArrowFunctionExpression.body.statements`, `ConditionalExpression.{consequent,alternate}`, `LogicalExpression.{left,right}`, `ParenthesizedExpression.expression`, `SequenceExpression.expressions`.

For each `JSXElement`: compute `name = jsx_element_name_to_string(&opening.name)`, `is_html = name.chars().next().is_some_and(char::is_lowercase)`, `line = byte_offset_to_line(source, opening.span.start)`. For each `JSXAttributeItem`:

- `Attribute(attr)`: `attr_name` = identifier-name or `"{ns}:{name}"`; value mapping: `StringLiteral` → `(Some(value), false)`, `ExpressionContainer|Element|Fragment` → `(None, true)`, `None` → `(None, false)`. Push `JsxAttribute { name, string_value, is_expression, is_spread: false }`.
- `SpreadAttribute(_)` → push `JsxAttribute { name: "...", string_value: None, is_expression: false, is_spread: true }`.

Push `JsxElementEntry { name, is_html, line, attributes }`. Then iterate `jsx_elem.children` — for any `JSXChild::Element(child)` add a flat second-level entry (not recursing further into the child's own children — explicit limitation in source).

`jsx_element_name_to_string`: `Identifier|IdentifierReference` → name; `NamespacedName` → `"{ns}:{name}"`; `MemberExpression` → `jsx_member_expr_to_string`; `ThisExpression` → `"this"`. `jsx_member_expr_to_string`: recursive `"{obj}.{prop}"`.

`extract_block`: skips `Statement::ImportDeclaration` entirely. Computes `span = stmt.span()`, `start = span.start as usize`, `end = (span.end as usize).min(source.len())`; if `start >= end` return; `code = source[start..end].to_string()`; if `code.trim().is_empty()` return. Determine `node_kind`: for `ExportNamedDeclaration` use inner declaration's kind via `node_kind_for_declaration` (fallback `Other`); for `ExportDefaultDeclaration` use `Other`; otherwise `node_kind_for_statement(stmt)` or return on `None`. `symbol_signature` = `first_line_signature(&code)` only for `SymbolKind::Function`. Push `CodeBlock { file: path, line_start, line_end, node_kind, code, symbol_signature, matched_lines: Vec::new(), matched_keywords: Vec::new() }`.

Helpers:

```rust
fn analysis_from_persisted(path: &Utf8Path, entry: persist::PersistedFileAnalysis) -> FileAnalysis {
    let ts_data = entry.ts_data.unwrap_or_default();
    FileAnalysis {
        source_path: path.to_owned(),
        content_hash: entry.content_hash,
        had_parse_errors: entry.had_parse_errors,
        symbols: entry.symbols,
        dependencies: entry.dependencies,
        exports: entry.exports,
        blocks: entry.blocks,
        lang: LangData::Ts(ts_data),
    }
}

fn module_export_name_to_string(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.as_str().to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.as_str().to_string(),
        ModuleExportName::StringLiteral(lit) => lit.value.as_str().to_string(),
    }
}

fn source_type_for(path: &Utf8Path) -> SourceType {
    match path.extension() {
        Some("tsx" | "jsx") => SourceType::tsx(),
        Some("mjs" | "cjs") => SourceType::mjs(),
        _ => SourceType::ts(),
    }
}

fn byte_offset_to_line(source: &str, offset: u32) -> usize {
    let clamped = (offset as usize).min(source.len());
    source[..clamped].bytes().filter(|&b| b == b'\n').count() + 1
}

fn find_tsconfig_from_root(root: &Utf8Path) -> Option<Utf8PathBuf> {
    let mut dir: &Utf8Path = root;
    loop {
        let candidate = dir.join("tsconfig.json");
        if candidate.exists() { return Some(candidate); }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
}

fn first_line_signature(code: &str) -> Option<String> {
    let first_line = code.lines().next()?;
    let sig = first_line.split_once('{').map_or(first_line, |(before, _)| before).trim();
    if sig.is_empty() { None } else { Some(sig.to_string()) }
}
```

`config_hash_for` format: `"monokl-{CARGO_PKG_VERSION}-oxc-0.128-lang-ts-{tsconfig_blake3_or_'no-tsconfig'}"`.

The 4-tier cache lookup in `analyze` is the canonical pattern: stat → persist::lookup (mtime+size) → source() + content_hash → in-memory cache (verify hash) → persist::lookup_by_hash + refresh_mtime → full parse → cache::insert.

Resolver extensions ordering: `.ts .tsx .mts .cts .js .jsx .mjs .cjs .json` — TS sources before compiled JS so workspace shadows compiled output. Condition names: `import require node default`.

# Section 18: enrich/ — WorkspaceEnricher trait and implementations verbatim

## enrich/mod.rs

```rust
use crate::analysis::file_analysis::FileAnalysis;
use crate::indices::{FileIdx, WorkspaceIndex};

pub(crate) trait WorkspaceEnricher: Send {
    fn update(&mut self, idx: FileIdx, analysis: &FileAnalysis);
    fn finalize(self: Box<Self>, index: &mut WorkspaceIndex);
}

pub mod import_graph;
pub mod symbol_index;
```

## enrich/import_graph.rs

```rust
use camino::Utf8PathBuf;
use crate::analysis::file_analysis::FileAnalysis;
use crate::indices::{FileIdx, WorkspaceIndex};
use crate::types::{DependencyRecord, DependencyResolution, DependencyTarget};
use super::WorkspaceEnricher;

pub struct ImportGraphEnricher {
    deferred: Vec<(FileIdx, Utf8PathBuf)>,
}

impl ImportGraphEnricher {
    pub fn new() -> Self { Self { deferred: Vec::new() } }
}

impl Default for ImportGraphEnricher {
    fn default() -> Self { Self::new() }
}

fn target_resolved(dep: &DependencyRecord) -> Option<&Utf8PathBuf> {
    match &dep.target {
        DependencyTarget::File { resolution, .. } | DependencyTarget::RustPath { resolution, .. } => {
            match resolution {
                DependencyResolution::Resolved { path } => Some(path),
                DependencyResolution::Ambiguous { .. }
                | DependencyResolution::Unresolved { .. }
                | DependencyResolution::External => None,
            }
        }
        DependencyTarget::Namespace { .. } => None,
    }
}

impl WorkspaceEnricher for ImportGraphEnricher {
    fn update(&mut self, idx: FileIdx, analysis: &FileAnalysis) {
        self.deferred.extend(
            analysis.dependencies.iter()
                .filter_map(|dep| target_resolved(dep).map(|resolved| (idx, resolved.clone()))),
        );
    }
    fn finalize(self: Box<Self>, index: &mut WorkspaceIndex) {
        for (importer_idx, resolved_path) in self.deferred {
            if let Some(&dep_idx) = index.file_index.get(&resolved_path) {
                index.import_graph.add_edge(importer_idx, dep_idx);
            }
        }
    }
}
```

## enrich/symbol_index.rs

```rust
use crate::analysis::file_analysis::FileAnalysis;
use crate::indices::{FileIdx, WorkspaceIndex};
use super::WorkspaceEnricher;

pub struct SymbolIndexEnricher {
    entries: Vec<(FileIdx, String, usize)>,
}

impl SymbolIndexEnricher {
    pub fn new() -> Self { Self { entries: Vec::new() } }
}

impl Default for SymbolIndexEnricher {
    fn default() -> Self { Self::new() }
}

impl WorkspaceEnricher for SymbolIndexEnricher {
    fn update(&mut self, idx: FileIdx, analysis: &FileAnalysis) {
        self.entries.extend(
            analysis.symbols.iter().map(|sym| (idx, sym.name.clone(), sym.line)),
        );
    }
    fn finalize(self: Box<Self>, index: &mut WorkspaceIndex) {
        for (idx, name, line) in self.entries {
            index.symbol_index.insert(&name, idx, line);
        }
    }
}
```

# Section 19: indices/ — FileIdx, ImportGraph, SymbolIndex, WorkspaceIndex verbatim

## indices/mod.rs

```rust
pub(crate) mod file_idx;
pub mod import_graph;
pub mod symbol_index;
pub mod workspace;

pub(crate) use file_idx::FileIdx;
pub use import_graph::ImportGraph;
pub use symbol_index::SymbolIndex;
pub use workspace::WorkspaceIndex;
```

## indices/file_idx.rs

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FileIdx(pub(crate) u32);
```

No `Serialize`. `pub(crate)` only.

## indices/import_graph.rs

```rust
use rustc_hash::FxHashMap;
use super::file_idx::FileIdx;

pub struct ImportGraph {
    forward: FxHashMap<FileIdx, Vec<FileIdx>>,
    reverse: FxHashMap<FileIdx, Vec<FileIdx>>,
}

impl ImportGraph {
    pub(crate) fn new() -> Self {
        Self {
            forward: FxHashMap::default(),
            reverse: FxHashMap::default(),
        }
    }
    pub(crate) fn add_edge(&mut self, importer: FileIdx, target: FileIdx) {
        self.forward.entry(importer).or_default().push(target);
        self.reverse.entry(target).or_default().push(importer);
    }
    pub(crate) fn dependents(&self, idx: FileIdx) -> &[FileIdx] {
        self.reverse.get(&idx).map_or(&[], Vec::as_slice)
    }
    pub(crate) fn imports(&self, idx: FileIdx) -> &[FileIdx] {
        self.forward.get(&idx).map_or(&[], Vec::as_slice)
    }
}
```

## indices/symbol_index.rs

```rust
use rustc_hash::FxHashMap;
use super::file_idx::FileIdx;

pub struct SymbolIndex {
    index: FxHashMap<String, Vec<(FileIdx, usize)>>,
}

impl SymbolIndex {
    pub(crate) fn new() -> Self { Self { index: FxHashMap::default() } }
    pub(crate) fn insert(&mut self, name: &str, file: FileIdx, line: usize) {
        self.index.entry(name.to_owned()).or_default().push((file, line));
    }
    #[allow(dead_code)]
    pub(crate) fn lookup(&self, name: &str) -> &[(FileIdx, usize)] {
        self.index.get(name).map_or(&[], Vec::as_slice)
    }
}
```

## indices/workspace.rs

```rust
use std::sync::Arc;
use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use crate::analysis::file_analysis::FileAnalysis;
use crate::analysis::lang::LanguageAnalyzer;
use crate::enrich::WorkspaceEnricher;
use crate::error::Result;
use super::file_idx::FileIdx;
use super::import_graph::ImportGraph;
use super::symbol_index::SymbolIndex;

pub struct WorkspaceIndex {
    pub(crate) files: Vec<Utf8PathBuf>,
    pub(crate) file_index: FxHashMap<Utf8PathBuf, FileIdx>,
    pub(crate) analyses: Vec<Arc<FileAnalysis>>,
    pub import_graph: ImportGraph,
    pub symbol_index: SymbolIndex,
}

impl WorkspaceIndex {
    pub(crate) fn build(
        root: &Utf8Path,
        analyzer: &dyn LanguageAnalyzer,
        enrichers: Vec<Box<dyn WorkspaceEnricher>>,
        config_hash: &str,
    ) -> Result<Self> {
        crate::analysis::persist::init(root, config_hash)?;
        // Phase 1: walk + parallel analyze
        let walk_entries: Vec<Utf8PathBuf> = {
            let mut entries = Vec::new();
            for result in ignore::WalkBuilder::new(root).standard_filters(true).build() {
                let entry = result.map_err(|e| crate::error::MonoklError::Walk {
                    path: root.to_owned(),
                    source: e,
                })?;
                if !entry.file_type().is_some_and(|t| t.is_file()) { continue; }
                if let Ok(p) = Utf8PathBuf::from_path_buf(entry.into_path()) {
                    if analyzer.supports(&p) { entries.push(p); }
                }
            }
            entries
        };
        let mut analyses: Vec<Arc<FileAnalysis>> = walk_entries.par_iter().filter_map(|path| {
            // `path: &Utf8PathBuf` from `par_iter()` already derefs for `analyzer.analyze(path, ..)`
            // and for the `tracing::warn!` Display below — only the closure that escapes into
            // `Box::new(move || ..)` needs its own owned copy. The extra `path.clone()` this spec
            // originally showed here cloned a `Utf8PathBuf` on every file in a parallel hot path
            // for no reason.
            let path_for_closure = path.clone();
            match analyzer.analyze(path, Box::new(move || {
                std::fs::read_to_string(&path_for_closure)
                    .map_err(|e| crate::error::MonoklError::Io(io_errors::FileIoError::read(&path_for_closure, e)))
            })) {
                Ok(a) => Some(a),
                Err(e) => { tracing::warn!("analyze failed for {path}: {e}"); None }
            }
        }).collect();
        analyses.sort_by(|a, b| a.source_path.cmp(&b.source_path));
        let mut files: Vec<Utf8PathBuf> = Vec::with_capacity(analyses.len());
        let mut file_index: FxHashMap<Utf8PathBuf, FileIdx> = FxHashMap::default();
        for (i, analysis) in analyses.iter().enumerate() {
            let idx = FileIdx(i as u32);
            files.push(analysis.source_path.clone());
            file_index.insert(analysis.source_path.clone(), idx);
        }
        // Phase 2: stream update() through enrichers
        let mut enrichers = enrichers;
        for analysis in &analyses {
            if let Some(&idx) = file_index.get(&analysis.source_path) {
                for enricher in &mut enrichers {
                    enricher.update(idx, analysis);
                }
            }
        }
        // Phase 3: finalize enrichers
        let mut index = WorkspaceIndex {
            files, file_index, analyses,
            import_graph: ImportGraph::new(),
            symbol_index: SymbolIndex::new(),
        };
        for enricher in enrichers { enricher.finalize(&mut index); }
        crate::analysis::persist::flush(root, config_hash)?;
        Ok(index)
    }

    pub(crate) fn path_to_idx(&self, path: &Utf8Path) -> Option<FileIdx> {
        self.file_index.get(path).copied()
    }
    pub(crate) fn idx_to_path(&self, idx: FileIdx) -> Option<&Utf8Path> {
        self.files.get(idx.0 as usize).map(Utf8PathBuf::as_path)
    }

    pub(crate) fn analysis(&self, path: &Utf8Path) -> Option<Arc<FileAnalysis>> {
        let idx = self.path_to_idx(path)?;
        self.analyses.get(idx.0 as usize).cloned()
    }

    pub(crate) fn analyses(&self) -> impl Iterator<Item = (&Utf8Path, &Arc<FileAnalysis>)> {
        self.files.iter().map(Utf8PathBuf::as_path).zip(self.analyses.iter())
    }

    pub fn build_standard(opts: &crate::types::WorkspaceOptions) -> Result<Self> {
        use crate::analysis::TsAnalyzer;
        use crate::enrich::import_graph::ImportGraphEnricher;
        use crate::enrich::symbol_index::SymbolIndexEnricher;
        let analyzer = TsAnalyzer::from_workspace_opts(opts)?;
        let config_hash = TsAnalyzer::config_hash_for(opts);
        let enrichers: Vec<Box<dyn WorkspaceEnricher>> = vec![
            Box::new(ImportGraphEnricher::new()),
            Box::new(SymbolIndexEnricher::new()),
        ];
        Self::build(&opts.root, &analyzer, enrichers, &config_hash)
    }

    pub fn dependents_of(&self, file: &Utf8Path) -> Vec<&Utf8Path> {
        let Some(idx) = self.path_to_idx(file) else { return Vec::new(); };
        self.import_graph.dependents(idx).iter()
            .filter_map(|&dep_idx| self.idx_to_path(dep_idx)).collect()
    }
    pub fn imports_of(&self, file: &Utf8Path) -> Vec<&Utf8Path> {
        let Some(idx) = self.path_to_idx(file) else { return Vec::new(); };
        self.import_graph.imports(idx).iter()
            .filter_map(|&imp_idx| self.idx_to_path(imp_idx)).collect()
    }
    pub fn symbol_locations(&self, name: &str) -> Vec<(&Utf8Path, usize)> {
        self.symbol_index.lookup(name).iter()
            .filter_map(|(idx, line)| self.idx_to_path(*idx).map(|p| (p, *line))).collect()
    }
    pub fn files(&self) -> &[Utf8PathBuf] { &self.files }
}
```

Three-phase build: (1) walk via `ignore::WalkBuilder` with `standard_filters(true)`, filter by `analyzer.supports()`, run `analyzer.analyze()` in parallel via `rayon::par_iter()`, sort by `source_path` for deterministic `FileIdx`. (2) Stream `enricher.update(idx, analysis)` for each file × each enricher. (3) Build empty `WorkspaceIndex`, then `enricher.finalize(&mut index)` for each. Finally `persist::flush(root, config_hash)`.

# Section 20: pipeline.rs verbatim

The full `pipeline.rs` is reproduced in the reading transcript above. Headline functions:

```rust
#[allow(clippy::too_many_lines, clippy::items_after_statements)]
pub fn search(opts: &SearchOptions) -> Result<SearchResponse> { ... }

pub fn extract(req: &ExtractRequest) -> Result<Vec<CodeBlock>> { ... }

pub fn symbols(files: &[Utf8PathBuf], _lite: bool) -> Result<SymbolsResult> { ... }

pub fn dependents(file: &Utf8Path, root: &Utf8Path) -> Result<DependentsResult> { ... }
```

**`search` pipeline (8 stages):**

1. Language filter: if `opts.language` is some and not `TypeScript|JavaScript`, return empty `apply_budget` with a `Skipped` diagnostic `"language {lang:?} not supported in v1; only TypeScript/JavaScript"`.
2. Query parse: if `opts.exact && opts.query.trim().is_empty()` → empty response; if exact → single `ParsedTerm { modifier: Optional, pattern: regex::escape(&opts.query), is_regex: false }`. Else call `parse(&opts.query)?`; empty parsed → empty response. Build `QueryPlan::from_terms(terms)`. If `plan.search_patterns().is_empty()` → empty response.
3. Canonicalize `opts.path` (returns `MonoklError::Io(FileIoError::read(...))` or `MonoklError::NonUtf8Path`).
4. Stage 2+3: `text_search::search_files(&root, &patterns, allow_tests, no_gitignore, max_candidates)?` → `BTreeMap<path, Vec<LineHit>>`.
5. Build `term_regexes` (one `regex::Regex` per term, case-insensitive, silently dropping regex compile failures).
6. Stage 4 (boolean eval): excluded terms → second `text_search::search_files` pass to gather `excluded_files: HashSet<Utf8PathBuf>`. Then filter raw_hits: drop files in `excluded_files`; for required terms, verify at least one hit line matches each required term's regex.
7. Stage 5 (block retrieval): build `TsAnalyzer::from_workspace_opts(&WorkspaceOptions::new(&root))`. For each surviving (path, file_hits): skip if `!analyzer.supports(path)`. Read source (push `Skipped` diagnostic on IO error). Call `analyzer.analyze(path, Box::new(move || Ok(src)))`. If parse error → push `Degraded` diagnostic. For each block: keep only those whose `[line_start, line_end]` contains any matched line; sort hit lines; annotate `matched_lines` and `matched_keywords` (terms whose regex matches any of the block's hit-line texts). Call `rank::tokenize_block(&annotated)` → push `(annotated, tokens)`.
8. Stage 6 (BM25): build `df: FxHashMap<String, usize>` counting unique-per-block term occurrences. Get `bm25_query_terms` (all non-Excluded patterns). Call `rank::rank_blocks(all_blocks, &bm25_query_terms, &df)`.
9. Stage 7: `dedup::dedup_blocks(ranked)`.
10. Stage 8: `budget::apply_budget(deduped, &opts.limits, diagnostics)`.

**`extract`:** derive root from `req.file.parent()` or `"."`; create `TsAnalyzer::from_workspace_opts`; if not supported return `Ok(Vec::new())`; call `analyzer.analyze` with read closure; filter blocks by overlap: `(None,None)` → all; `(Some(s),None)` → `block.line_start <= s && block.line_end >= s`; `(None,Some(e))` → `block.line_start <= e`; `(Some(s),Some(e))` → `block.line_start <= e && block.line_end >= s`. Return cloned `Vec<CodeBlock>`.

**`symbols`:** derive root from `files.first().and_then(|f| f.parent()).unwrap_or(Utf8Path::new("."))`. Constants: `per_file_cap = 50`, `total_cap = 500`. For each file: skip unsupported; analyze; on parse errors push `Degraded` diagnostic `"{N} symbols from partial parse"`. Per-file cap via `symbols.truncate(50)`. Total cap: if adding would exceed 500, truncate to `total_cap - total_symbol_count`, insert partial, set `truncation_marker = "[symbol cap 500 reached; pass fewer files or increase limit]"`, break.

**`dependents`:** canonicalize both file and root (errors → `MonoklError::Io(FileIoError::read)` or `NonUtf8Path`); if `!abs_file.starts_with(&abs_root)` → `MonoklError::PathOutsideRoot`. Cap = 200. Auto tsconfig diagnostic: if `tsconfig.json` not at root nor parent → push `Warning` diagnostic with message containing "tsconfig.json". Build standard workspace index via `TsAnalyzer::from_workspace_opts` + `ImportGraphEnricher::new()` + `SymbolIndexEnricher::new()` + `WorkspaceIndex::build`. If file not in index → return empty `DependentsResult`. Collect raw_dependents/raw_imports via `import_graph.dependents()/imports()`. truncation_marker (set when either > cap) format: `"[results capped at 200; {tot_dep} dependents, {tot_imp} imports total]"`. Return with `take(200)`.

# Section 21: cli.rs and output.rs verbatim

## cli.rs

```rust
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use crate::types::Language;

#[derive(Parser)]
#[command(name = "monokl", about = "AST-aware semantic code search for TypeScript/JavaScript and Rust codebases.", version)]
pub struct Cli {
    #[arg(long, global = true, default_value_t = false)]
    pub pretty: bool,
    #[command(subcommand)]
    pub cmd: Subcmd,
}

#[derive(Subcommand)]
pub enum Subcmd {
    Search {
        query: String,
        #[arg(long, default_value = ".")]
        path: Utf8PathBuf,
        #[arg(long)]
        max_results: Option<usize>,
        #[arg(long)]
        max_tokens: Option<usize>,
        #[arg(long)]
        max_bytes: Option<usize>,
        #[arg(long)]
        max_candidates: Option<usize>,
        #[arg(long)]
        allow_tests: bool,
        #[arg(long)]
        no_gitignore: bool,
        #[arg(long)]
        exact: bool,
        #[arg(long, value_enum)]
        language: Option<Language>,
    },
    Symbols {
        #[arg(required = true)]
        files: Vec<Utf8PathBuf>,
        #[arg(long)]
        lite: bool,
    },
    Extract {
        file: Utf8PathBuf,
        #[arg(long)]
        line_start: Option<usize>,
        #[arg(long)]
        line_end: Option<usize>,
    },
    Dependents {
        file: Utf8PathBuf,
        #[arg(long)]
        root: Utf8PathBuf,
    },
    CountTokens {
        files: Vec<Utf8PathBuf>,
        #[arg(long)]
        stdin: bool,
    },
}
```

## output.rs

```rust
use std::io::IsTerminal as _;
use crate::error::{MonoklError, Result};

#[allow(clippy::print_stdout)]
pub fn render_json<T: serde::Serialize>(value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{json}");
    Ok(())
}

#[allow(clippy::print_stdout)]
pub fn render_json_compact<T: serde::Serialize>(value: &T) -> Result<()> {
    let json = serde_json::to_string(value)?;
    println!("{json}");
    Ok(())
}

#[allow(clippy::print_stdout)]
pub fn render_output<T: serde::Serialize>(value: &T, pretty: bool) -> Result<()> {
    if pretty || std::io::stdout().is_terminal() {
        render_json(value)
    } else {
        render_json_compact(value)
    }
}

#[allow(clippy::print_stderr)]
pub fn fatal(message: &str) -> ! {
    eprintln!("monokl: error: {message}");
    std::process::exit(1);
}

pub fn render_error(err: &MonoklError) -> ! {
    #[allow(clippy::print_stdout)]
    { println!("{{\"error\": \"{err}\"}}"); }
    std::process::exit(1);
}
```

# Section 22: main.rs verbatim

```rust
#[cfg(feature = "cli")]
#[allow(clippy::too_many_lines)]
fn main() -> miette::Result<()> {
    use clap::Parser as _;
    use miette::{Context as _, IntoDiagnostic as _};

    #[allow(clippy::print_stdout, clippy::print_stderr)]
    fn run() -> miette::Result<()> {
        #[derive(serde::Serialize)]
        struct TokenCountEntry { path: String, tokens: usize }

        #[derive(serde::Serialize)]
        struct TokenCountResult { files: Vec<TokenCountEntry>, total_tokens: usize }

        let cli = monokl::cli::Cli::parse();
        let pretty = cli.pretty;

        match cli.cmd {
            monokl::cli::Subcmd::Search { query, path, max_results, max_tokens, max_bytes, max_candidates, allow_tests, no_gitignore, exact, language } => {
                let limits = monokl::types::SearchLimits {
                    max_results,
                    max_bytes: max_bytes.unwrap_or(2_097_152).min(2_097_152),
                    max_tokens,
                    max_candidates: max_candidates.unwrap_or(1_000),
                };
                let opts = monokl::types::SearchOptions {
                    query, path, allow_tests, no_gitignore, limits, exact, language,
                };
                let response = monokl::pipeline::search(&opts).into_diagnostic().wrap_err("search failed")?;
                monokl::output::render_output(&response, pretty).into_diagnostic().wrap_err("rendering output")?;
            }
            monokl::cli::Subcmd::Symbols { files, lite } => {
                let result = monokl::pipeline::symbols(&files, lite).into_diagnostic().wrap_err("symbols failed")?;
                monokl::output::render_output(&result, pretty).into_diagnostic().wrap_err("rendering output")?;
            }
            monokl::cli::Subcmd::Extract { file, line_start, line_end } => {
                let req = monokl::types::ExtractRequest { file, line_start, line_end };
                let blocks = monokl::pipeline::extract(&req).into_diagnostic().wrap_err("extract failed")?;
                monokl::output::render_output(&blocks, pretty).into_diagnostic().wrap_err("rendering output")?;
            }
            monokl::cli::Subcmd::Dependents { file, root } => {
                let result = monokl::pipeline::dependents(&file, &root).into_diagnostic().wrap_err("dependents failed")?;
                monokl::output::render_output(&result, pretty).into_diagnostic().wrap_err("rendering output")?;
            }
            monokl::cli::Subcmd::CountTokens { files, stdin } => {
                let mut entries: Vec<TokenCountEntry> = Vec::new();
                let mut total_tokens: usize = 0;
                if stdin {
                    use std::io::Read as _;
                    let mut src = String::new();
                    std::io::stdin().read_to_string(&mut src).into_diagnostic().wrap_err("reading stdin")?;
                    let tokens = monokl::budget::count_tokens(&src).into_diagnostic().wrap_err("counting tokens")?;
                    total_tokens += tokens;
                    entries.push(TokenCountEntry { path: "<stdin>".to_owned(), tokens });
                } else {
                    for path in &files {
                        let src = std::fs::read_to_string(path).into_diagnostic().wrap_err_with(|| format!("reading {path}"))?;
                        let tokens = monokl::budget::count_tokens(&src).into_diagnostic().wrap_err_with(|| format!("counting tokens in {path}"))?;
                        total_tokens += tokens;
                        entries.push(TokenCountEntry { path: path.to_string(), tokens });
                    }
                }
                let result = TokenCountResult { files: entries, total_tokens };
                monokl::output::render_output(&result, pretty).into_diagnostic().wrap_err("rendering output")?;
            }
        }
        Ok(())
    }
    run()
}

#[cfg(not(feature = "cli"))]
#[allow(clippy::print_stderr)]
fn main() {
    eprintln!("monokl: CLI feature not enabled. Build with --features cli");
    std::process::exit(1);
}
```

Notable: max_bytes is `max_bytes.unwrap_or(2_097_152).min(2_097_152)` (always clamped to 2MB hard ceiling); max_candidates defaults to 1_000. The `<stdin>` literal path string is used for stdin entries. The inline `TokenCountEntry`/`TokenCountResult` use `path`/`tokens`/`total_tokens` field names (note: `total_tokens` is snake_case here, unlike the other camelCase JSON in the system).

# Section 23: Integration tests — full test bodies

All six integration test files are fully reproduced verbatim above in the reading transcript. Summary of test-function names by file:

**tests/integration_search.rs** — fixture-based search tests. Helper: `fixture_root()` returns `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/small-ts")`. `search_opts(query)` builds options with `allow_tests: true`, `no_gitignore: true`, `max_results: Some(20)`, `max_bytes: 2_097_152`, `max_tokens: Some(10_000)`, `max_candidates: 100`. Tests:

- `search_finds_function_by_name` — query `"validatePrice"`, asserts non-empty and code contains "validatePrice".
- `search_finds_multiple_symbols` — query `"formatCurrency"`, asserts at least format.ts is in results.
- `search_source_files_found_when_allow_tests_true` — format.ts in results for formatCurrency.
- `search_result_has_correct_structure` — for each ranked block: rank > 0, final_score >= 0, non-empty code, line_start <= line_end.
- `search_respects_max_results_limit` — max_results=2, results.len() <= 2.
- `search_empty_query_returns_empty` — empty query, empty results.
- `search_excluded_term_filters_results` — `"useCart -addItem"` excludes useCart.ts.
- `search_finds_interface_in_results` — query `"ValidationResult"`.

**tests/integration_symbols.rs**:

- `symbols_finds_exported_functions` — format.ts + validation.ts contain `formatCurrency`, `truncateString`, `validatePrice`.
- `symbols_finds_interface_and_type_alias` — validation.ts has `ValidationResult` (Interface), `PriceValidator` (TypeAlias).
- `symbols_total_count_is_accurate` — `total_symbol_count == sum(file.values().map(Vec::len))`.
- `symbols_lite_mode_returns_same_names` — `symbols(files, false)` and `symbols(files, true)` yield identical name sets.
- `symbols_tsx_file_finds_component` — Button.tsx contains `Button` or `ButtonProps`.

**tests/integration_extract.rs**:

- `extract_full_file_returns_blocks` — non-empty, contains formatCurrency.
- `extract_line_range_filters_to_overlap` — lines 1..=4 overlap with returned blocks.
- `extract_nonexistent_file_returns_error` — returns Err.
- `extract_non_ts_file_returns_empty` — Cargo.toml returns empty vec without error.
- `extract_tsx_file_returns_blocks` — Button.tsx returns blocks containing "Button".

**tests/integration_dependents.rs**:

- `dependents_finds_direct_importers` — validation.ts is in dependents of format.ts.
- `dependents_leaf_file_has_no_dependents` — src/index.ts has 0 dependents in fixture.
- `dependents_imports_list_is_populated` — validation.ts has `total_import_count > 0`.
- `dependents_path_outside_root_returns_error` — `matches!(err, MonoklError::PathOutsideRoot { .. })`.
- `dependents_button_imports_validation` — Button.tsx imports validation.ts.

**tests/integration_cli_output.rs** (gated `#![cfg(feature = "cli")]`):

- `search_exits_zero_and_emits_compact_json` — stdout JSON has `results`/`truncated`/`totalTokens`.
- `search_success_produces_no_stderr` — stderr empty on success.
- `symbols_exits_zero_and_emits_compact_json` — JSON has `files`/`totalSymbolCount`.
- `symbols_output_contains_expected_symbol_names` — stdout contains "formatCurrency" and "truncateString".
- `count_tokens_exits_zero_and_emits_compact_json` — JSON has `files` (array) and `total_tokens` (>0).
- `count_tokens_file_entries_have_path_and_tokens` — every entry has string `path`, number `tokens`.
- `search_nonexistent_path_exits_nonzero` — `output.status.success() == false`.
- `symbols_output_is_compact_not_pretty_printed` — stdout doesn't start with `"{\n"`; trimmed has no embedded `\n`.
- `search_output_is_compact_not_pretty_printed` — same invariant for search.

**tests/integration_output_shape.rs** (gated `cli`):

- `snapshot_search_output_top_level_keys`
- `snapshot_search_result_entry_keys`
- `snapshot_symbols_output_top_level_keys`
- `snapshot_symbols_entry_keys`
- `snapshot_count_tokens_output_top_level_keys`
- `snapshot_count_tokens_file_entry_keys`

# Section 24: Snapshot files — exact content

## search_top_level_keys

```
[
    "diagnostics",
    "results",
    "totalBlocksBeforeTruncation",
    "totalBytes",
    "totalTokens",
    "truncated",
    "truncationMarker",
]
```

## search_result_entry_keys

```
[
    "bm25Score",
    "code",
    "coverageBoost",
    "file",
    "finalScore",
    "lineEnd",
    "lineStart",
    "matchedKeywords",
    "matchedLines",
    "nodeKind",
    "nodeTypeBoost",
    "parentContext",
    "rank",
    "symbolSignature",
]
```

## symbols_top_level_keys

```
[
    "diagnostics",
    "files",
    "totalSymbolCount",
    "truncationMarker",
]
```

## symbols_entry_keys

```
[
    "kind",
    "line",
    "name",
    "signature",
]
```

## count_tokens_top_level_keys

```
[
    "files",
    "total_tokens",
]
```

(Note: snake_case `total_tokens` because TokenCountResult is defined inline in main.rs without `#[serde(rename_all)]`.)

## count_tokens_file_entry_keys

```
[
    "path",
    "tokens",
]
```

## src/snapshots — types round-trip (excerpts)

**search_options_defaults.snap**:

```yaml
query: ""
path: ""
allowTests: false
noGitignore: false
limits:
  maxResults: 50
  maxBytes: 2097152
  maxTokens: 20000
  maxCandidates: 1000
exact: false
language: ~
```

**search_options_full.snap**:

```yaml
query: handleClick
path: src/components
allowTests: true
noGitignore: false
limits:
  maxResults: 25
  maxBytes: 1000000
  maxTokens: 10000
  maxCandidates: 500
exact: true
language: typescript
```

**search_limits_defaults.snap**:

```yaml
maxResults: 50
maxBytes: 2097152
maxTokens: 20000
maxCandidates: 1000
```

**search_limits_custom.snap**:

```yaml
maxResults: 100
maxBytes: 5000000
maxTokens: 40000
maxCandidates: 500
```

**ranked_block_round_trip.snap**:

```yaml
file: src/components/Card.tsx
lineStart: 5
lineEnd: 25
nodeKind: function
code: "export function Card() { return <div />; }"
symbolSignature: () => JSX.Element
matchedLines:
  - 10
matchedKeywords:
  - Card
bm25Score: 8.5
coverageBoost: 1.2
nodeTypeBoost: 0.9
finalScore: 9.18
rank: 1
parentContext:
  kind: module
  name: Card module
  line: 1
```

**symbol_entry_round_trip.snap**:

```yaml
name: MyComponent
kind: function
line: 42
signature: "(props: Props) => JSX.Element"
```

**code_block_round_trip.snap**:

```yaml
file: src/components/Button.tsx
lineStart: 10
lineEnd: 35
nodeKind: function
code: "export function Button(props: Props) {\n  return <button {...props} />;\n}"
symbolSignature: "(props: Props) => JSX.Element"
matchedLines:
  - 15
  - 20
  - 25
matchedKeywords:
  - Button
  - props
```

**dependents_result_full.snap**:

```yaml
file: src/utils.ts
dependents:
  - src/Button.tsx
  - src/Card.tsx
imports:
  - src/types.ts
  - node_modules/lodash
totalDependentCount: 2
totalImportCount: 2
truncationMarker: ~
diagnostics: []
```

**diagnostic_kind_degraded.snap**:

```yaml
kind: degraded
path: src/broken.ts
message: Parser degraded mode
```

# Section 25: Test fixtures — exact TypeScript source

## tests/fixtures/small-ts/src/index.ts

```ts
export { Button } from './components/Button';
export { useCart } from './hooks/useCart';
export { formatCurrency, truncateString } from './utils/format';
export { validatePrice } from './utils/validation';
export type { ButtonProps, CartItem, ValidationResult, PriceValidator } from './utils/validation';
```

## tests/fixtures/small-ts/src/components/Button.tsx

```tsx
import React from 'react';
import { validatePrice } from '../utils/validation';

export interface ButtonProps {
  label: string;
  price?: number;
  onClick?: () => void;
  disabled?: boolean;
}

export function Button({ label, price, onClick, disabled }: ButtonProps) {
  const priceValid = price === undefined || validatePrice(price).valid;
  return (
    <button onClick={onClick} disabled={disabled || !priceValid}>
      {label}
    </button>
  );
}

export default Button;
```

## tests/fixtures/small-ts/src/hooks/useCart.ts

```ts
import { useState } from 'react';
import { validatePrice } from '../utils/validation';
import type { ValidationResult } from '../utils/validation';

export interface CartItem {
  id: string;
  name: string;
  price: number;
}

export function useCart() {
  const [items, setItems] = useState<CartItem[]>([]);

  const addItem = (item: CartItem): ValidationResult => {
    const result = validatePrice(item.price);
    if (result.valid) setItems(prev => [...prev, item]);
    return result;
  };

  const removeItem = (id: string): void => {
    setItems(prev => prev.filter(i => i.id !== id));
  };

  return { items, addItem, removeItem };
}
```

## tests/fixtures/small-ts/src/utils/format.ts

```ts
export function formatCurrency(amount: number, currency: string = 'USD'): string {
  return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);
}

export function truncateString(str: string, maxLength: number): string {
  if (str.length <= maxLength) return str;
  return str.slice(0, maxLength - 3) + '...';
}
```

## tests/fixtures/small-ts/src/utils/validation.ts

```ts
import { formatCurrency } from './format';

export interface ValidationResult {
  valid: boolean;
  message?: string;
}

export function validatePrice(price: number): ValidationResult {
  if (price < 0) return { valid: false, message: 'Price cannot be negative' };
  if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };
  return { valid: true };
}

export type PriceValidator = (price: number) => ValidationResult;
```

# Section 26: napi-support boundary.rs verbatim

```rust
use std::any::Any;
use std::error::Error;
use std::fmt::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub fn run_napi<T, F, E>(f: F) -> napi::Result<T>
where
    F: FnOnce() -> Result<T, E>,
    E: Error,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(flatten_error(&e)),
        Err(payload) => Err(panic_payload_to_napi_error(&payload)),
    }
}

pub async fn run_napi_async<T, F, Fut, E>(f: F) -> napi::Result<T>
where
    F: FnOnce() -> Fut,
    Fut: core::future::Future<Output = Result<T, E>>,
    E: Error,
{
    match f().await {
        Ok(value) => Ok(value),
        Err(e) => Err(flatten_error(&e)),
    }
}

fn flatten_error(err: &dyn Error) -> napi::Error {
    let mut msg = format!("{err}");
    let mut source = err.source();
    while let Some(s) = source {
        let _ = write!(msg, "\n  caused by: {s}");
        source = s.source();
    }
    napi::Error::from_reason(msg)
}

// `&(dyn Any + Send)`, not `&Box<dyn Any + Send>` (clippy::borrowed_box) — the call site's
// `&payload` still coerces fine via Box's Deref impl, no change needed there.
fn panic_payload_to_napi_error(payload: &(dyn Any + Send)) -> napi::Error {
    let detail = payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(non-string panic payload)");
    napi::Error::from_reason(format!(
        "Rust panic caught at napi boundary: {detail} - this is a bug, please report"
    ))
}
```

Async variant deliberately omits `catch_unwind` — relies on task-runtime boundary instead.

# Section 27: io-errors file_io.rs verbatim

```rust
use camino::{Utf8Path, Utf8PathBuf};
use std::io;

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum FileIoError {
    #[error("failed to read {path}")]
    Read {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create directory {path}")]
    CreateDir {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove {path}")]
    Remove {
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
}

impl FileIoError {
    #[must_use]
    pub fn read(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self { Self::Read { path: path.into(), source } }
    #[must_use]
    pub fn write(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self { Self::Write { path: path.into(), source } }
    #[must_use]
    pub fn create_dir(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self { Self::CreateDir { path: path.into(), source } }
    #[must_use]
    pub fn remove(path: impl Into<Utf8PathBuf>, source: io::Error) -> Self { Self::Remove { path: path.into(), source } }

    #[must_use]
    pub fn path(&self) -> &Utf8Path {
        match self {
            Self::Read { path, .. }
            | Self::Write { path, .. }
            | Self::CreateDir { path, .. }
            | Self::Remove { path, .. } => path.as_path(),
        }
    }
    #[must_use]
    pub fn io_source(&self) -> &io::Error {
        match self {
            Self::Read { source, .. }
            | Self::Write { source, .. }
            | Self::CreateDir { source, .. }
            | Self::Remove { source, .. } => source,
        }
    }
    #[must_use]
    pub fn io_kind(&self) -> io::ErrorKind { self.io_source().kind() }
    #[must_use]
    pub fn is_not_found(&self) -> bool { self.io_kind() == io::ErrorKind::NotFound }
}
```

# Section 28: Workspace Cargo.toml — lint profile

```toml
[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
unreachable = "deny"
unimplemented = "deny"
dbg_macro = "deny"
todo = "deny"
print_stdout = "warn"
print_stderr = "warn"
await_holding_lock = "deny"
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"

[workspace.lints.rust]
unsafe_code = "deny"

[profile.release]
lto = true
codegen-units = 1
strip = true
```

> **Post-research addition.** `unreachable`/`unimplemented` join the deny list — the original list denied `panic` but not these two, which let a real `unreachable!("handled above")` panic path (in `git_scope.rs`'s `scope_files_with_index`, now fixed to not need it) slip past a policy meant to keep production code panic-free.

Workspace package fields:

```toml
[workspace.package]
version = "0.0.0"
edition = "2024"
rust-version = "1.85"
license = "MIT"
repository = "https://github.com/orin-dx/monokl"
authors = ["Orin DX Contributors"]
keywords = ["orin-dx", "developer-tools"]
categories = ["development-tools"]
publish = false
```

Workspace `resolver = "3"`, `members = ["crates/*"]`.

Relevant workspace dependencies for monokl (versions):

- `thiserror = "2"`, `serde = { version = "1", features = ["derive"] }`, `serde_json = { version = "1", features = ["preserve_order"] }`
- `camino = { version = "1", features = ["serde1"] }`, `rustc-hash = "2"`, `ignore = "0.4"`
- `oxc_parser = "0.128"`, `oxc_ast = "0.128"`, `oxc_span = "0.128"`, `oxc_allocator = "0.128"`, `oxc_resolver = "11"`
- `rayon = "1"`, `regex = "1"`, `blake3 = "1"`, `dashmap = "5"`, `tiktoken-rs = "0.6"`
- `grep-regex = "0.1"`, `grep-searcher = "0.1"`
- `clap = { version = "4", features = ["derive", "env"] }`, `miette = { version = "7", features = ["fancy"] }`
- `tracing = "0.1"`, `insta = { version = "1.47", features = ["yaml", "json"] }`
- `pretty_assertions = "1"`, `proptest = "1"`, `tempfile = "3"`, `criterion = { version = "0.7", features = ["html_reports"] }`

# Section 29: Engineering spec and architecture docs — key decisions

Summarized from `engineering-docs/projects/monokl-search/README.md`, `ai-docs/monokl/architecture.md`, `ai-docs/monokl/decisions/lsp-architecture.md`, and `ai-docs/monokl/planned/remaining-work.md`:

**Why not Probe (the reference pipeline):** binary-only (subprocess spawn ~30–50ms cold per call), tree-sitter (the workspace is OXC-standardized per ADR 0002), no persistent index (re-walks per invocation — unusable at 50k+ files), no io-errors/camino/napi-boundary alignment.

**What monokl keeps from Probe:** Pipeline shape (text prefilter → BM25 → dedup → budget) with substitutions: tree-sitter→OXC v0.128 + oxc_semantic; per-invocation walk→`FileAnalysis` cache + `WorkspaceIndex`; in-memory trigram→`grep-searcher` (ripgrep crates); PathBuf→Utf8PathBuf; anyhow→thiserror + FileIoError; tiktoken p50k_base→**o200k_base** (Claude/Sonnet).

**Eight design tenets:**

1. Parse once, use everywhere — single FileAnalysis cache feeds every tool.
2. Aggregate views, not re-scans — ImportGraph/SymbolIndex folded from cached FileAnalysis.
3. Token budget is non-negotiable — `max_bytes` 2MB hard ceiling, truncation marker required.
4. Determinism — BTreeMap for serialized output; tie-break on (file, line_start); BM25 rounded to 6dp for cross-platform f64 stability.
5. OXC arena lifetime containment (ADR 0002) — no `AstNode<'a>` escapes Allocator scope; all FileAnalysis fields are owned.
6. Graceful degradation — broken file flips `had_parse_errors=true`, contributes recoverable data.
7. Tier separation — Level 1 (per-file `LanguageAnalyzer`) co-produces; Level 2 (`WorkspaceEnricher`) folds.
8. ContentHash is authoritative; mtime+size is the cheap fast-path dirty bit.

**Non-goals:** full Salsa; C#/Roslyn in v1; tree-sitter; ts.LanguageService (dropped in TS 7.0); @typescript/api (Corsa IPC unstable until TS 7.1+); persistent on-disk search index (tantivy-style); agent loop; stemming; in-memory trigram index (removed in v4 — was 600MB–1.2GB).

**Two-level architecture:**

- Level 1 `LanguageAnalyzer` (one parse → FileAnalysis with symbols/dependencies/exports/blocks/JSX).
- Level 2 `WorkspaceEnricher::update(idx, &FileAnalysis)` accumulates state; `finalize(self: Box<Self>, &mut WorkspaceIndex)` writes to the now-fully-populated index. Implementations: `ImportGraphEnricher`, `SymbolIndexEnricher`.

**Two-tier cache invalidation:**

1. `stat()` → if mtime+size matches → return cached (no file read).
2. Else read file → blake3 ContentHash → if hash matches → refresh mtime/size, return cached.
3. Else re-parse → cache.

**Three-phase WorkspaceIndex::build:**

1. Walk + parallel analyze via Rayon `par_iter()`, sort by `source_path` for stable `FileIdx`.
2. Stream `enricher.update(idx, analysis)` for every (file, enricher) pair.
3. `enricher.finalize(&mut index)` for each — at this point `file_index` is fully populated so `ImportGraphEnricher` can resolve `Utf8PathBuf → FileIdx`.
4. Single atomic `persist::flush(root, config_hash)` at end (temp file + rename).

**FileIdx invariants:** `pub(crate) struct FileIdx(pub(crate) u32)` — no Serialize derive (compile-time guard against leaking across rebuild boundaries); `u32` saves 4 bytes per edge vs `usize` (~16MB at 50k files).

**Disk cache:** `<root>/.monokl/cache.json`; version=2; `config_hash` = `monokl-{CARGO_PKG_VERSION}-oxc-0.128-lang-ts-{tsconfig_blake3_or_'no-tsconfig'}` so monokl upgrade + tsconfig change both invalidate cache; size cap 100MB with LRU eviction in chunks of `(len/10).max(1)`.

**Search pipeline stages (8):** Query Parse → Text Search (`grep-searcher` Aho-Corasick multi-pattern in one pass) → Boolean Eval (`+required` AND filter on hit lines; `-excluded` requires second pass over candidate set to check whole file) → Block Retrieval (analyze cache + matched-line filter) → BM25 Rank (K1=1.5, B=0.5, 6dp rounding) → Dedup (>50% line overlap within same file) → Token Budget (o200k_base BPE, 2MB hard ceiling) → SearchResponse.

**BM25 tuning rationale:** K1=1.5, B=0.5 (Probe's code-search tuning). 6-decimal-place rounding before sort eliminates f64 FMA divergence between x86-64 CI and ARM64 dev machines.

**Node-type boost rationale:** Functions are the most common search target (rank 1.5); class/method/constructor next (1.4); types (1.2–1.3); module 0.9; Other 0.7. Boost multiplies the BM25 score; coverage boost is additive.

**LSP architecture (three-layer):** `LspSession` (wire protocol: `Content-Length: N\r\n\r\n` framing, NOT newline-delimited JSON; full `initialize`→`InitializeResult`→`initialized` handshake; circuit breaker — 3 crashes in 60s → 'tripped', 500ms backoff). `LanguageServerConfig` (per-language factory: `resolveServerPath`, `spawnArgs`, `rootUri`). `LspManager` (workspace router, lazy session start by extension).

**LSP binary resolution:** Never PATH for npm-distributed servers — use `require.resolve('typescript-language-server/lib/cli.mjs')` and spawn node. PATH acceptable for `rustup`/`dotnet` (controlled toolchain managers). C# (`CSharpServerConfig`) deferred to ADR 0008 (proto-dotnet/orbit).

**Language roadmap:** v1 = TypeScript/JavaScript (current — OXC + typescript-language-server). v1.1 = Rust (ra-ap-syntax + rust-analyzer) and Python (tree-sitter-python + pyright). v1.2 = C# (tree-sitter-c-sharp + OmniSharp/csharp-ls). Each slots into `LanguageAnalyzer` without changing the public NAPI API.

**Latency targets at 50k files:** symbols(1 file) cold <50ms / warm <5ms / lite <5ms; dependents cold <15s / warm <30ms / lite <50ms; search(3-term) cold <15s / warm <300ms; WorkspaceIndex::build <15s cold; disk cache flush <800ms.

**Per-file/total caps:**

- symbols: 50 per file, 500 total (truncation marker `"[symbol cap 500 reached; pass fewer files or increase limit]"`).
- dependents/imports: 200 each (marker `"[results capped at 200; {N} dependents, {M} imports total]"`).
- search candidates: default 1000 (`SearchLimits::max_candidates`).
- max_results default 50; max_tokens default 20000; max_bytes 2MB hard ceiling.

**Query language:** Whitespace-separated terms; `+term` required, `-term` excluded, bare `term` optional (scored), `"exact phrase"` literal (escaped), `regex:pattern` raw regex. Max 64 terms (`MonoklError::TooManyTerms`). Default behavior tokenizes camelCase/snake_case; `--exact` skips tokenization and passes the full query as one escaped literal.

**Test-path heuristic:** `is_test_path` excludes `/__tests__/`, `/test/`, `/tests/`, `/spec/` directories and `.test.ts`, `.test.tsx`, `.test.js`, `.spec.ts`, `.spec.tsx`, `.spec.js` extensions (unless `allow_tests=true`). Note: `.stories.tsx` is currently treated as production code.

**Known v1 gaps already resolved (per remaining-work.md):** `exact` mode wired in pipeline (P0 #1 done); `lsp-client.ts` rewritten with Content-Length framing + LspSession/TypeScriptServerConfig/LspManager (P0 #2 done); pipeline modules properly gated on `#[cfg(feature = "lang-ts")]` (P0 #3 done); per-process temp file in `persist::flush()` (prevents nextest race); allen-tools/liskov-tools/hopper-tools CLIs (P1 #4-6 done); `count-tokens` subcommand (P1 #7 done).

**Pending P2 work:** real benchmarks in `benches/pipeline.rs` (currently all 4 criterion groups are empty stubs `pipeline_cold`, `pipeline_warm_cache`, `dependents_warm`, `lite_symbols`); changesets + workspace build verification; downstream consumer-app fixtures for foundations-tools.

**Critical patterns to follow:**

- `crates/auto-barrel/src/analyzer.rs` for `OnceLock<RwLock<FxHashMap>>` cache.
- `crates/auto-barrel/src/scanner.rs` for `WalkBuilder` config.
- `crates/napi-support/src/boundary.rs` for `run_napi` panic-payload extraction.
- `crates/pulse-core/src/cache.rs` for atomic-write + 100MB cap + `config_hash` convention.
- `crates/pulse-core/src/types.rs` for `ContentHash` newtype to mirror.

Files relevant to this specification (all absolute, in the project tree at `/home/dev/Projects/monokl/`):

- `crates/monokl/src/{lib.rs,main.rs,error.rs,types.rs,tokens.rs,text_search.rs,dedup.rs,budget.rs,output.rs,cli.rs,pipeline.rs,tests_types.rs}`
- `crates/monokl/src/query/{mod.rs,lexer.rs,parser.rs,plan.rs}`
- `crates/monokl/src/rank/{mod.rs,bm25.rs,boost.rs,tokenize_doc.rs}`
- `crates/monokl/src/analysis/{mod.rs,cache.rs,content_hash.rs,file_analysis.rs,lang.rs,node_kind.rs,persist.rs,ts_analyzer.rs}`
- `crates/monokl/src/enrich/{mod.rs,import_graph.rs,symbol_index.rs}`
- `crates/monokl/src/indices/{mod.rs,file_idx.rs,import_graph.rs,symbol_index.rs,workspace.rs}`
- `crates/monokl/{Cargo.toml,moon.yml}`
- `crates/monokl/tests/integration_*.rs` (6 files) + `tests/snapshots/integration_output_shape__*.snap` (6 files)
- `crates/monokl/src/snapshots/monokl__tests_types__tests__*.snap` (round-trip type snapshots)
- `crates/monokl/tests/fixtures/small-ts/src/` (index.ts, components/Button.tsx, hooks/useCart.ts, utils/format.ts, utils/validation.ts)
- `crates/monokl/benches/pipeline.rs`
- `crates/napi-support/src/boundary.rs`
- `crates/io-errors/src/file_io.rs`
- `Cargo.toml` (workspace)
- `engineering-docs/projects/monokl-search/README.md`
- `ai-docs/monokl/{architecture.md,decisions/lsp-architecture.md,planned/remaining-work.md}`

result: Reconstruction-grade specification for the monokl crate produced inline, covering all 29 requested sections with verbatim code blocks for every module, exact serde key names, exact constants (CACHE_VERSION=2, K1=1.5, B=0.5, DP=1_000_000, 2_097_152 max_bytes ceiling, 50/500/200 caps, 64 query-term limit), all 6 integration-snapshot key sets, every round-trip YAML snapshot, all 5 fixture sources, and the architectural decisions from the four supporting docs.

---
