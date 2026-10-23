# Multi-Language Platform (v0.4.0 → v0.7.0)

The `AnalyzerRegistry`, the real Rust analyzer (`ra_ap_syntax`), pipeline modularization, PR-aware git scopes, the capability/precision model that lets unsupported languages degrade gracefully instead of erroring, and the human-readable presentation layer.

Part of the [monokl spec](./README.md). Builds on [01-core-architecture.md](./01-core-architecture.md) and [02-inspection-and-analysis.md](./02-inspection-and-analysis.md).

---

# Part 3: Phase 2+ Additions (0.4.0 → 0.7.0)

> Added since Part 2. Multi-language architecture, analyzer registry, Rust parity via ra-ap-syntax, pipeline modularization, PR-aware scopes, capability-driven query, agent tool contracts.

## 1. Overview of Phase 2+ changes

Monokl Phase 2+ spans versions 0.4.0 → 0.7.0 of the `monokl` crate. It is approximately 8 PRs of work that transforms monokl from a single-language (TypeScript/JavaScript) AST search tool into a multi-language, capability-driven, mixed-workspace code intelligence platform.

Major themes:

1. **Multi-language architecture** (0.5.0). A new `AnalyzerRegistry` dispatches per-file to language-specific `LanguageAnalyzer` implementations. First-class Rust support via `ra-ap-syntax` (rust-analyzer's syntax library). The CLI/library can now ship `--no-default-features --features lang-rs,cli` (Rust-only) as well as the full TS+Rust default.
2. **Library contract & focused agent commands** (0.4.0). New `deps`, `deps-from`, `classify`, `migration-surface` commands. Field projection (`--only`/`--fields`) across `inspect`, `search`, `patterns`, `refs`, with `--format json|minimal|schema`. Inspect kind-filtering moved into the `InspectOptions` library contract.
3. **Pipeline modularization**. `pipeline.rs` split into `pipeline/{search, symbols, deps, graph, inspecting, session}.rs`. Workspace `WorkspaceSession` cached across calls for analyzer registry + standard index reuse.
4. **PR-aware review scopes** (0.6.0). New `git_scope` module backed by `gix`. `QueryScope::{FullRepo, ChangedFiles, ChangedLines, ImpactedNeighbors}` for `search`/`symbols`/`refs`/`definition`/`dependents`. PR semantics use `merge-base(base, head)..head`; `diff` keeps explicit snapshot semantics. Changed-line filtering and one-hop import-graph neighbor expansion.
5. **Human presentation layer** (0.6.0). `--format`, `--profile`, `--detail`, `--color` global flags. Text renderers for `search`, `symbols`, `refs`, `definition`, `dependents`, `inspect`, `deps`, `deps-from`, `classify`, `migration-surface`, `diff`, `explain`, `coverage`. Mermaid rendering for small `dependents` graphs. JSON remains the canonical agent contract.
6. **Capability-driven query behavior** (0.7.0). `search`, `inspect`, `dependents`, `refs`, `definition` now use shared analyzer capability policy. Unsupported surfaces emit explicit `Skipped` diagnostics; approximate-precision analyzers emit consistent warnings. `symbols`' classified-imports omission is keyed off `AnalyzerCapabilities::classified_imports` rather than a TS check.
7. **Cache architecture rework**. `DashMap`-only hot path in `analysis/cache.rs` (no more `Mutex<CacheMeta>`/`fs2` advisory locks at the in-memory layer). Disk persistence keys on content hash + mtime + size in `analysis/persist.rs` (consumed by `RustAnalyzer`).
8. **Read-safety floor** (`io_safety.rs`). Symlink rejection and 50 MB file size cap on user-facing entry points to prevent path-traversal and allocator-bomb surprises.

---

## 2. Updated `Cargo.toml` — new dependencies

```toml
[package]
name        = "monokl"
version     = "0.7.0"
edition.workspace     = true
rust-version.workspace = true
license.workspace     = true
repository.workspace  = true
authors.workspace     = true
categories.workspace  = true
keywords    = ["monokl", "search", "ast", "semantic", "typescript"]
description = "AST-aware semantic code search for TypeScript/JavaScript and Rust."
readme      = "README.md"
publish     = ["orin-cargo"]

[lib]
name       = "monokl"
path       = "src/lib.rs"

[[bin]]
name              = "monokl"
path              = "src/main.rs"
required-features = ["cli"]

[features]
default = ["lang-ts", "lang-rs", "cli"]

# TypeScript/JavaScript analysis via OXC parser + AST + resolver.
lang-ts = [
    "dep:oxc_parser",
    "dep:oxc_ast",
    "dep:oxc_span",
    "dep:oxc_allocator",
    "dep:oxc_resolver",
]

# Rust analysis via rust-analyzer's syntax library (ra-ap-syntax).
lang-rs = ["dep:ra_ap_syntax"]

# CLI binary — clap + miette diagnostics + terminal rendering helpers.
cli = ["dep:clap", "miette", "dep:owo-colors"]

# Miette Diagnostic derive on MonoklError — forwarded to io-errors for full chain rendering.
miette = ["dep:miette", "io-errors/miette"]
```

New runtime dependencies introduced relative to Phase 1:

- `dashmap` — concurrent hot-path cache.
- `fs2` — advisory lock file for disk cache writes (consumed by `analysis/persist.rs`).
- `gix` — git plumbing for PR-aware scopes (replaces `git2`/subprocess plans).
- `rustc-hash` — `FxHashMap` for analyzer-internal maps (Rust workspace package roots, impl-method buckets, etc.).
- `tempfile` — used by tests and by the persist write-queue staging file.
- `ra_ap_syntax` (feature `lang-rs`) — rust-analyzer's syntax library.
- `owo-colors` (feature `cli`) — ANSI colorizing for human renderers.

OXC stays at the 0.128 suite but `oxc_resolver` is versioned independently on its own 11.x line.

---

## 2a. `analysis/cache_tiers.rs` — the 4-tier lookup, shared

> **Post-research extraction.** v0.1.0's `TsAnalyzer::analyze` and this part's `RustAnalyzer::analyze_cached_or_parse` (§4) originally duplicated the same stat → persist-lookup → content-hash → persist-lookup-by-hash → parse skeleton almost verbatim, differing only in the parse step and (for Rust) a profile-validity check. Extracted here once every analyzer beyond the first needs it — Python/Go/Java will reuse this rather than re-deriving it a third, fourth, and fifth time.

```rust
use std::sync::Arc;
use camino::Utf8Path;
use crate::error::Result;
use super::content_hash::ContentHash;
use super::file_analysis::FileAnalysis;
use super::lang::AnalysisProfile;
use super::{cache, persist};

/// Shared 4-tier cache lookup for `LanguageAnalyzer::analyze_with_profile` implementations.
///
/// - `from_persisted` rebuilds a `FileAnalysis` from disk-cached data for the requested
///   `profile`, or returns `None` if the persisted record doesn't satisfy it (e.g. a
///   `Dependencies`-only record can't answer a `Full` request).
/// - `cached_is_valid` checks whether an in-memory cache hit satisfies the requested profile.
/// - `parse` is the tier-4 fallback: a full parse, given the already-read source.
///
/// Cache writes (`cache::insert`) only happen for `AnalysisProfile::Full` results, so a cheap
/// `Dependencies`/`Structural` request never overwrites a fuller cached analysis with a thinner
/// one.
pub(crate) fn lookup_or_parse(
    path: &Utf8Path,
    source: Box<dyn FnOnce() -> Result<String>>,
    profile: AnalysisProfile,
    from_persisted: impl Fn(persist::PersistedFileAnalysis, AnalysisProfile) -> Option<FileAnalysis>,
    cached_is_valid: impl Fn(&FileAnalysis, AnalysisProfile) -> bool,
    parse: impl FnOnce(&str, ContentHash, u64, u64, AnalysisProfile) -> FileAnalysis,
) -> Result<Arc<FileAnalysis>> {
    let meta = std::fs::metadata(path).map_err(|e| io_errors::FileIoError::read(path, e))?;
    #[allow(clippy::cast_possible_truncation)]
    let mtime_ns = meta.modified().map_err(|e| io_errors::FileIoError::read(path, e))?
        .duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let size_bytes = meta.len();

    // Tier 1: mtime/size fast path
    if let Some(persisted) = persist::lookup(path, mtime_ns, size_bytes)? {
        if let Some(analysis) = from_persisted(persisted, profile) {
            let analysis = Arc::new(analysis);
            if profile == AnalysisProfile::Full { cache::insert(path, Arc::clone(&analysis)); }
            return Ok(analysis);
        }
    }

    // Tier 2: read source + check in-memory cache
    let source_text = source()?;
    let content_hash = ContentHash::of(source_text.as_bytes());
    if let Some(cached) = cache::lookup(path) {
        if cached.content_hash == content_hash && cached_is_valid(&cached, profile) {
            return Ok(cached);
        }
    }

    // Tier 3: content-hash disk cache (mtime changed, content unchanged)
    if let Some(persisted) = persist::lookup_by_hash(path, &content_hash)? {
        if let Some(analysis) = from_persisted(persisted, profile) {
            persist::refresh_mtime(path, mtime_ns, size_bytes)?;
            let analysis = Arc::new(analysis);
            if profile == AnalysisProfile::Full { cache::insert(path, Arc::clone(&analysis)); }
            return Ok(analysis);
        }
    }

    // Tier 4: full parse
    let analysis = Arc::new(parse(&source_text, content_hash, mtime_ns, size_bytes, profile));
    if profile == AnalysisProfile::Full { cache::insert(path, Arc::clone(&analysis)); }
    Ok(analysis)
}
```

## 2b. `TsAnalyzer`'s completed trait impl

Before the registry below can hold `Arc<dyn LanguageAnalyzer>`, `TsAnalyzer` needs the `languages`/`language_for`/`capabilities` methods §11's trait now declares. `analyze` (§17) becomes `analyze_with_profile`, rebuilt on top of §2a's shared helper — v0.1.0's cache logic is unchanged, TS just always answers `Some`/`true` since it does a full parse regardless of profile:

```rust
impl LanguageAnalyzer for TsAnalyzer {
    fn languages(&self) -> &[LanguageId] {
        &[LanguageId::TypeScript]
    }

    fn supports(&self, path: &Utf8Path) -> bool {
        if path.as_str().ends_with(".d.ts") { return false; }
        matches!(path.extension(), Some("ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"))
    }

    fn language_for(&self, path: &Utf8Path) -> Option<LanguageId> {
        self.supports(path).then_some(LanguageId::TypeScript)
    }

    fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
        AnalyzerCapabilities {
            blocks: true,
            classified_imports: true,
            resolved_import_graph: CapabilityPrecision::Exact,
            refs: CapabilityPrecision::Structural,
            definition: CapabilityPrecision::Structural,
            inspect_detail: CapabilityPrecision::Structural,
        }
    }

    fn analyze_with_profile(
        &self,
        path: &Utf8Path,
        source: Box<dyn FnOnce() -> Result<String>>,
        profile: AnalysisProfile,
    ) -> Result<Arc<FileAnalysis>> {
        cache_tiers::lookup_or_parse(
            path,
            source,
            profile,
            |persisted, _profile| Some(analysis_from_persisted(path, persisted)),
            |_cached, _profile| true,
            |src, hash, mtime_ns, size_bytes, _profile| {
                parse_full(self, path, src, hash, mtime_ns, size_bytes)
            },
        )
    }
}
```

TS/JS always does a full parse regardless of `profile`, since `parse_full` produces symbols/dependencies/exports/blocks in one OXC pass at no extra marginal cost — unlike Rust's AST-vs-regex-fallback split, where a `Dependencies`-only request genuinely skips work, TS has no cheaper partial path to offer, so `from_persisted`/`cached_is_valid` always answer "yes." `resolved_import_graph: CapabilityPrecision::Exact` reflects `oxc_resolver`'s real module resolution, not a heuristic — the only analyzer in this spec entitled to `Exact` on that field.

---

## 3. Analyzer Registry (`analysis/registry.rs`)

The `AnalyzerRegistry` is the architectural boundary for mixed-language workspaces. Callers ask the registry which analyzer owns a path instead of hardcoding `TsAnalyzer` plus ad hoc `.rs` exceptions.

```rust
//! Multi-language analyzer registry.
//!
//! This is the architectural boundary for mixed-language workspaces. Callers
//! ask the registry which analyzer owns a file instead of hardcoding
//! `TsAnalyzer` plus ad hoc `.rs` exceptions.

use std::sync::Arc;

use camino::Utf8Path;

use crate::error::Result;
use crate::types::WorkspaceOptions;

#[cfg(feature = "lang-ts")]
use super::TsAnalyzer;
use super::{LanguageAnalyzer, LanguageId, RustAnalyzer};

pub struct AnalyzerRegistry {
    analyzers: Vec<Arc<dyn LanguageAnalyzer>>,
    config_hash: String,
}

impl AnalyzerRegistry {
    pub fn from_workspace_opts(opts: &WorkspaceOptions) -> Result<Self> {
        let mut analyzers: Vec<Arc<dyn LanguageAnalyzer>> = Vec::new();
        let mut hash_parts: Vec<String> = Vec::new();

        #[cfg(feature = "lang-ts")]
        {
            analyzers.push(Arc::new(TsAnalyzer::from_workspace_opts(opts)?));
            hash_parts.push(TsAnalyzer::config_hash_for(opts));
        }

        analyzers.push(Arc::new(RustAnalyzer::from_workspace_opts(opts)));
        hash_parts.push("rust-analyzer-v1".to_owned());

        Ok(Self {
            analyzers,
            config_hash: hash_parts.join("::"),
        })
    }

    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    pub fn analyzers(&self) -> &[Arc<dyn LanguageAnalyzer>] {
        &self.analyzers
    }

    pub fn supports(&self, path: &Utf8Path) -> bool {
        self.analyzer_for(path).is_some()
    }

    pub fn analyzer_for(&self, path: &Utf8Path) -> Option<&dyn LanguageAnalyzer> {
        self.analyzers
            .iter()
            .find(|analyzer| analyzer.supports(path))
            .map(Arc::as_ref)
    }

    pub fn supports_language(&self, path: &Utf8Path, languages: &[LanguageId]) -> bool {
        self.analyzer_for(path)
            .and_then(|analyzer| analyzer.language_for(path))
            .is_some_and(|language| languages.contains(&language))
    }
}
```

Key properties:

- Analyzers register in order; the first matching `supports(path)` wins. TS is registered first when `lang-ts` is enabled, Rust always.
- Configuration hash is composed from `TsAnalyzer::config_hash_for(opts)` plus the literal `"rust-analyzer-v1"`, joined with `::`. The composite hash flows into `crate::analysis::persist::init` for cache invalidation.
- `supports_language` lets language-filtered scans (e.g. `search --language rust`) check both "the analyzer recognizes this file" and "it's one of the requested languages".
- `Arc<dyn LanguageAnalyzer>` so the registry is shareable across rayon worker threads.

---

## 4. Rust Analyzer (`analysis/rust_analyzer.rs`)

The `RustAnalyzer` is monokl's first non-TypeScript implementation of the shared `LanguageAnalyzer` trait. It uses `ra-ap-syntax` (when `lang-rs` is enabled) for AST-level extraction and falls back to regex scanning otherwise.

### Struct + lifecycle

```rust
pub struct RustAnalyzer {
    workspace_root: Option<Utf8PathBuf>,
    package_roots: FxHashMap<String, Utf8PathBuf>,
}

impl RustAnalyzer {
    pub fn new() -> Self {
        Self {
            workspace_root: None,
            package_roots: FxHashMap::default(),
        }
    }

    pub fn from_workspace_opts(opts: &WorkspaceOptions) -> Self {
        Self {
            workspace_root: Some(opts.root.clone()),
            package_roots: discover_workspace_packages(&opts.root),
        }
    }
}
```

`discover_workspace_packages` walks `opts.root` with `ignore::WalkBuilder::standard_filters(true)` and harvests `Cargo.toml` files, mapping crate name (normalized `-` → `_`) to its directory. This enables `use acme_lib::Bar;` to resolve into the workspace.

### Trait implementation + capabilities

```rust
impl LanguageAnalyzer for RustAnalyzer {
    fn languages(&self) -> &[LanguageId] {
        &[LanguageId::Rust]
    }

    fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
        AnalyzerCapabilities {
            blocks: true,
            classified_imports: false,
            resolved_import_graph: CapabilityPrecision::Structural,
            refs: CapabilityPrecision::Structural,
            definition: CapabilityPrecision::Structural,
            inspect_detail: CapabilityPrecision::Structural,
        }
    }

    fn supports(&self, path: &Utf8Path) -> bool {
        path.extension() == Some("rs")
    }

    fn language_for(&self, path: &Utf8Path) -> Option<LanguageId> {
        self.supports(path).then_some(LanguageId::Rust)
    }
    /* analyze / analyze_with_profile delegate to analyze_cached_or_parse */
}
```

`classified_imports: false` is critical — `symbols` uses this exact flag to know it must suppress the per-file `ClassifiedImports` block and emit an aggregated warning (see §11).

### Cached-or-parse flow

Built on §2a's shared `cache_tiers::lookup_or_parse` — Rust's contribution is a profile-validity check plain TS doesn't need (a `Dependencies`-only persisted/cached record can't answer a `Full` request, since it never ran the AST inspect-entry pass):

```rust
fn analyze_cached_or_parse(
    &self,
    path: &Utf8Path,
    source: Box<dyn FnOnce() -> Result<String>>,
    profile: AnalysisProfile,
) -> Result<Arc<FileAnalysis>> {
    cache_tiers::lookup_or_parse(
        path,
        source,
        profile,
        |persisted, profile| analysis_from_persisted(path, persisted, profile),
        |cached, profile| {
            profile != AnalysisProfile::Full
                || matches!(&cached.lang, LangData::Rust(rust) if rust.inspect_entry.is_some())
        },
        |src, hash, mtime_ns, size_bytes, profile| {
            parse_with_profile(self, path, src, hash, mtime_ns, size_bytes, profile)
        },
    )
}
```

Lookup precedence: persisted-by-mtime → in-memory by content hash → persisted-by-content-hash (with mtime refresh) → fresh parse. The in-memory cache is only written when `profile == Full` because partial-profile analyses (e.g. `Dependencies`) must not overwrite full ones.

### parse_with_profile

```rust
fn parse_with_profile(
    analyzer: &RustAnalyzer,
    path: &Utf8Path,
    source_text: &str,
    content_hash: ContentHash,
    mtime_ns: u64,
    size_bytes: u64,
    profile: AnalysisProfile,
) -> FileAnalysis {
    let line_count = source_text.lines().count();
    let dependencies = extract_dependencies(
        path,
        source_text,
        analyzer.workspace_root.as_deref(),
        &analyzer.package_roots,
    );
    #[cfg(feature = "lang-rs")]
    let syntax_analysis = matches!(profile, AnalysisProfile::Structural | AnalysisProfile::Full)
        .then(|| crate::rust_syntax::analyze_rust_source_syntax(path, source_text));

    let inspect_entry = if matches!(profile, AnalysisProfile::Full) {
        #[cfg(feature = "lang-rs")]
        {
            syntax_analysis.as_ref().map(|analysis| analysis.inspect_entry.clone())
        }
        #[cfg(not(feature = "lang-rs"))]
        {
            Some(build_inspect_entry(path))
        }
    } else {
        None
    };

    let blocks = if matches!(profile, AnalysisProfile::Full) {
        #[cfg(feature = "lang-rs")]
        {
            syntax_analysis.as_ref().map_or_else(Vec::new, |syntax| {
                extract_blocks_from_tree(path, source_text, &syntax.tree)
            })
        }
        #[cfg(not(feature = "lang-rs"))]
        {
            extract_blocks_fallback(path, source_text)
        }
    } else {
        Vec::new()
    };

    let (symbols, exports, had_parse_errors) = match profile {
        AnalysisProfile::Dependencies => (Vec::new(), Vec::new(), false),
        AnalysisProfile::Structural | AnalysisProfile::Full => {
            #[cfg(feature = "lang-rs")]
            {
                match syntax_analysis.as_ref() {
                    Some(syntax) => (
                        extract_symbols(source_text),
                        syntax.exports.clone(),
                        syntax.had_parse_errors,
                    ),
                    None => (extract_symbols(source_text), Vec::new(), false),
                }
            }
            #[cfg(not(feature = "lang-rs"))]
            {
                (extract_symbols(source_text), extract_exports(source_text), false)
            }
        }
    };

    let analysis = FileAnalysis {
        source_path: path.to_owned(),
        content_hash: content_hash.clone(),
        had_parse_errors,
        symbols,
        dependencies,
        exports,
        blocks,
        line_count,
        lang: LangData::Rust(RustData {
            inspect_entry: inspect_entry.map(Box::new),
        }),
    };

    if profile == AnalysisProfile::Full {
        let rust_inspect_entry = match &analysis.lang {
            LangData::Rust(rust) => rust.inspect_entry.as_deref().cloned(),
            _ => None,
        };
        let persisted = persist::PersistedFileAnalysis {
            content_hash,
            mtime_ns,
            size_bytes,
            symbols: analysis.symbols.clone(),
            dependencies: analysis.dependencies.clone(),
            exports: analysis.exports.clone(),
            ts_data: None,
            rust_inspect_entry,
            blocks: analysis.blocks.clone(),
            had_parse_errors: analysis.had_parse_errors,
            last_accessed_ns: 0,
            line_count: analysis.line_count,
        };
        persist::queue_write(path, persisted);
    }

    analysis
}
```

### Symbol extraction (line-based, profile-independent)

> **Superseded in Part 4 when `lang-rs` is enabled.** The function below is compiled only `#[cfg(not(feature = "lang-rs"))]`. When the parser feature is active, a parallel AST-based `extract_symbols(source, tree)` runs instead — see Part 4 §1 for the full replacement.

```rust
fn extract_symbols(source: &str) -> Vec<SymbolEntry> {
    let mut symbols = Vec::new();
    let mut depth = 0usize;

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if depth == 0 {
            if let Some((name, visibility)) = capture_symbol(trimmed, "struct") {
                symbols.push(make_symbol(name, SymbolKind::Struct, index + 1, visibility, "rust-struct"));
            } else if let Some((name, visibility)) = capture_symbol(trimmed, "enum") {
                symbols.push(make_symbol(name, SymbolKind::Enum, index + 1, visibility, "rust-enum"));
            } else if let Some((name, visibility)) = capture_symbol(trimmed, "trait") {
                symbols.push(make_symbol(name, SymbolKind::Interface, index + 1, visibility, "rust-trait"));
            } else if let Some((name, visibility)) = capture_symbol(trimmed, "fn") {
                symbols.push(make_symbol(name, SymbolKind::Function, index + 1, visibility, "rust-function"));
            } else if let Some((name, visibility)) = capture_symbol(trimmed, "type") {
                symbols.push(make_symbol(name, SymbolKind::TypeAlias, index + 1, visibility, "rust-type"));
            } else if let Some((name, visibility)) = capture_symbol(trimmed, "const") {
                symbols.push(make_symbol(name, SymbolKind::Variable, index + 1, visibility, "rust-const"));
            } else if let Some((name, visibility)) = capture_symbol(trimmed, "mod") {
                symbols.push(make_symbol(name, SymbolKind::Module, index + 1, visibility, "rust-module"));
            }
        }
        depth = update_brace_depth(depth, line);
    }

    symbols
}
```

The `kind_detail` strings are stable values feeding §17 node-kinds: `"rust-struct"`, `"rust-enum"`, `"rust-trait"`, `"rust-function"`, `"rust-type"`, `"rust-const"`, `"rust-module"`. Symbols are only collected at brace depth 0 (top-level items).

### Block extraction (AST when `lang-rs`)

```rust
#[cfg(feature = "lang-rs")]
fn extract_blocks_from_tree(path: &Utf8Path, source: &str, tree: &SourceFile) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    for item in tree.items() {
        extract_item_block(path, source, &item, &mut blocks);
    }
    blocks
}

#[cfg(feature = "lang-rs")]
fn extract_item_block(path: &Utf8Path, source: &str, item: &ast::Item, blocks: &mut Vec<CodeBlock>) {
    let (range, node_kind, signature) = match item {
        ast::Item::Fn(func) => (func.syntax().text_range(), SymbolKind::Function,
            func.name().map(|name| name.text().to_string())),
        ast::Item::Struct(strukt) => (strukt.syntax().text_range(), SymbolKind::Struct,
            strukt.name().map(|name| name.text().to_string())),
        ast::Item::Enum(enm) => (enm.syntax().text_range(), SymbolKind::Enum,
            enm.name().map(|name| name.text().to_string())),
        ast::Item::Trait(trait_item) => (trait_item.syntax().text_range(), SymbolKind::Interface,
            trait_item.name().map(|name| name.text().to_string())),
        ast::Item::Impl(imp) => {
            let name = imp.self_ty()
                .map_or_else(|| String::from("impl"), |ty| ty.syntax().text().to_string());
            (imp.syntax().text_range(), SymbolKind::Impl, Some(name))
        }
        ast::Item::Module(module) => (module.syntax().text_range(), SymbolKind::Module,
            module.name().map(|name| name.text().to_string())),
        ast::Item::Const(konst) => (konst.syntax().text_range(), SymbolKind::Variable,
            konst.name().map(|name| name.text().to_string())),
        ast::Item::TypeAlias(alias) => (alias.syntax().text_range(), SymbolKind::TypeAlias,
            alias.name().map(|name| name.text().to_string())),
        ast::Item::MacroRules(macro_rules) => (macro_rules.syntax().text_range(), SymbolKind::Macro,
            macro_rules.name().map(|name| name.text().to_string())),
        _ => return,
    };

    push_block(path, source, range.start().into(), range.end().into(), node_kind, signature, blocks);

    if let ast::Item::Impl(imp) = item {
        for assoc in imp.assoc_item_list().into_iter().flat_map(|list| list.assoc_items()) {
            if let ast::AssocItem::Fn(func) = assoc {
                let range = func.syntax().text_range();
                push_block(path, source, range.start().into(), range.end().into(),
                    SymbolKind::Method,
                    func.name().map(|name| name.text().to_string()),
                    blocks);
            }
        }
    }
}
```

This means a struct with `impl Button { fn click() }` produces three blocks: the struct, the `impl` itself, and the inner `fn click` (as `SymbolKind::Method`). This drives Rust support in `search` and `extract`.

### Dependency extraction + path resolution

The Rust analyzer's dependency model produces `DependencyTarget::RustPath { segments, anchor, resolution }`. The resolver is workspace-aware:

```rust
fn extract_dependencies(
    path: &Utf8Path,
    source: &str,
    workspace_root: Option<&Utf8Path>,
    package_roots: &FxHashMap<String, Utf8PathBuf>,
) -> Vec<DependencyRecord> {
    collect_use_statements(source)
        .into_iter()
        .flat_map(|(line, clause)| {
            expand_use_clause(&clause).into_iter().map(move |leaf| {
                let (segments, binding) = rust_binding(&leaf);
                let anchor = rust_anchor(&segments);
                let resolution = match resolve_rust_path(path, &segments, workspace_root, package_roots) {
                    Some(path) => DependencyResolution::Resolved { path },
                    None if matches!(&anchor, RustPathAnchor::Extern(_)) => DependencyResolution::External,
                    None => DependencyResolution::Unresolved {
                        diagnostic: Diagnostic {
                            kind: DiagnosticKind::Degraded,
                            path: Some(path.to_owned()),
                            message: format!("unable to resolve Rust path {}", segments.join("::")),
                        },
                    },
                };
                DependencyRecord {
                    line,
                    bindings: vec![binding],
                    target: DependencyTarget::RustPath {
                        anchor,
                        segments,
                        resolution,
                    },
                }
            })
        })
        .collect()
}
```

`expand_use_clause` recursively flattens `use a::{b, c::{d, e}}` into `["a::b", "a::c::d", "a::c::e"]`. `rust_binding` peels off `as Alias` (Named), `*` (Glob), or just the trailing identifier. `rust_anchor` classifies the first segment: `crate`/no-prefix → `RustPathAnchor::Crate`, `super` → `Super`, `self` → `Selff`, anything else → `Extern(name)`.

`resolve_rust_path` is the load-bearing piece. It:

1. Climbs `find_crate_root(file)` until it finds a `Cargo.toml`.
2. Derives `module_segments_for(file, src_root)` — e.g. `src/foo/bar.rs` → `["foo", "bar"]`.
3. For `crate::…`: starts from `[]` and applies the tail segments.
4. For `self::…`: starts from current_module.
5. For `super::…`: pops one segment per leading `super`.
6. For `extern_name::…`: first tries `resolve_module_path(&src_root, segments)` as a local sibling, then checks if `extern_name` equals the current crate name, and finally falls back to `package_roots.get(&normalized).join("src")` for cross-crate workspace resolution.
7. Calls `resolve_module_path` which tries `{prefix}.rs` and `{prefix}/mod.rs` for each prefix length from the longest to the shortest.
8. Falls back to `crate_entry_path(&src_root)` (`src/lib.rs` then `src/main.rs`) when only one segment remains.
9. Final resolved path is returned only if `resolved.starts_with(&workspace_root_canon)` — workspace-boundary containment.

### Fallback exports (when `lang-rs` not enabled)

```rust
#[cfg(not(feature = "lang-rs"))]
fn extract_exports(source: &str) -> Vec<ExportRecord> {
    let mut exports = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some((name, _)) = capture_symbol(trimmed, "struct")
            .filter(|_| trimmed.starts_with("pub "))
            .or_else(|| capture_symbol(trimmed, "enum").filter(|_| trimmed.starts_with("pub ")))
            .or_else(|| capture_symbol(trimmed, "trait").filter(|_| trimmed.starts_with("pub ")))
            .or_else(|| capture_symbol(trimmed, "fn").filter(|_| trimmed.starts_with("pub ")))
            .or_else(|| capture_symbol(trimmed, "type").filter(|_| trimmed.starts_with("pub ")))
            .or_else(|| capture_symbol(trimmed, "const").filter(|_| trimmed.starts_with("pub ")))
            .or_else(|| capture_symbol(trimmed, "mod").filter(|_| trimmed.starts_with("pub ")))
        {
            exports.push(ExportRecord { name, line: index + 1, re_export: false });
        }
        if let Some(rest) = trimmed.strip_prefix("pub use ") {
            for leaf in expand_use_clause(rest.trim_end_matches(';').trim()) {
                let (_, binding) = rust_binding(&leaf);
                exports.push(ExportRecord { name: binding.local, line: index + 1, re_export: true });
            }
        }
    }
    exports
}
```

### Use-clause parsing helpers

```rust
pub(crate) fn expand_use_clause(clause: &str) -> Vec<String> {
    fn walk(prefix: &str, clause: &str, out: &mut Vec<String>) {
        let clause = clause.trim();
        if clause.is_empty() { return; }
        if let Some((head, body)) = split_group(clause) {
            let next_prefix = join_prefix(prefix, head.trim_end_matches("::"));
            for part in split_top_level(body, ',') {
                walk(&next_prefix, part, out);
            }
            return;
        }
        out.push(join_prefix(prefix, clause));
    }
    let mut out = Vec::new();
    walk("", clause, &mut out);
    out
}

pub(crate) fn rust_binding(path: &str) -> (Vec<String>, DependencyBinding) {
    let leaf = path.trim();
    if leaf.ends_with("::*") {
        let path = leaf.trim_end_matches("::*").trim();
        let segments = path.split("::").map(ToOwned::to_owned).collect::<Vec<_>>();
        return (segments, DependencyBinding {
            imported: "*".to_owned(),
            local: "*".to_owned(),
            kind: BindingKind::Glob,
        });
    }

    let (path, local) = if let Some((path, alias)) = leaf.split_once(" as ") {
        (path.trim(), alias.trim().to_owned())
    } else {
        let local = leaf.rsplit("::").next().unwrap_or(leaf).trim().to_owned();
        (leaf, local)
    };

    let imported = path.rsplit("::").next().unwrap_or(path).trim().to_owned();
    let segments = path.split("::").map(ToOwned::to_owned).collect::<Vec<_>>();
    (segments, DependencyBinding {
        imported,
        local,
        kind: BindingKind::Named,
    })
}
```

`expand_use_clause` and `rust_binding` are `pub(crate)` because `rust_syntax.rs` reuses them to keep parsing semantics identical between the AST path and the regex/fallback path.

### Comparison vs. previous regex stub

Phase 1's `rust_inspection.rs` was a freestanding regex parser that built `InspectEntry` directly. It is now subordinate:

- Block extraction, symbol extraction, dependency extraction, and export extraction all live in `analysis/rust_analyzer.rs`, plumbed through the `LanguageAnalyzer` trait.
- The AST path (`rust_syntax.rs`) is the default; the old regex code path is `#[cfg(not(feature = "lang-rs"))]` only and survives so the `lang-rs`-disabled build still produces inspect entries.
- The new analyzer participates in the shared `FileAnalysis`/`LangData::Rust(RustData)` shape, so search/extract/symbols/refs/definition all see Rust blocks and dependencies through the same code path as TypeScript.

---

## 5. `rust_syntax.rs` — Rust AST utilities

This is the `ra-ap-syntax`-driven inspection module. It is gated `#![cfg(feature = "lang-rs")]`.

### Module entry types

```rust
pub(crate) struct RustSyntaxAnalysis {
    pub inspect_entry: InspectEntry,
    pub exports: Vec<ExportRecord>,
    pub had_parse_errors: bool,
    pub tree: SourceFile,
}

#[must_use]
pub fn inspect_rust_file_syntax(file: &Utf8Path) -> InspectEntry {
    let Ok(source) = std::fs::read_to_string(file) else {
        return InspectEntry::Unknown(UnknownEntry {
            file: file.to_owned(),
            symbols: vec![],
            deps: crate::types::ClassifiedImports::default(),
            loc: 0,
        });
    };
    analyze_rust_source_syntax(file, &source).inspect_entry
}

#[must_use]
pub(crate) fn analyze_rust_source_syntax(file: &Utf8Path, source: &str) -> RustSyntaxAnalysis {
    let parse = SourceFile::parse(source, Edition::Edition2024);
    let had_parse_errors = !parse.errors().is_empty();
    let tree = parse.tree();
    let loc = source.lines().count();
    RustSyntaxAnalysis {
        inspect_entry: classify(file, source, loc, &tree),
        exports: collect_exports(source, &tree),
        had_parse_errors,
        tree,
    }
}
```

The `tree` is returned to the caller so `RustAnalyzer::parse_with_profile` can reuse it for block extraction without re-parsing.

### Classification priority

```rust
pub(crate) fn classify(file: &Utf8Path, source: &str, loc: usize, tree: &SourceFile) -> InspectEntry {
    let mut traits: Vec<ast::Trait> = Vec::new();
    let mut structs: Vec<ast::Struct> = Vec::new();
    let mut enums: Vec<ast::Enum> = Vec::new();
    let mut impls: Vec<ast::Impl> = Vec::new();
    let mut uses: Vec<ast::Use> = Vec::new();
    let mut modules: Vec<ast::Module> = Vec::new();

    for item in tree.items() {
        match item {
            ast::Item::Trait(t) => traits.push(t),
            ast::Item::Struct(s) => structs.push(s),
            ast::Item::Enum(e) => enums.push(e),
            ast::Item::Impl(i) => impls.push(i),
            ast::Item::Use(u) => uses.push(u),
            ast::Item::Module(m) => modules.push(m),
            _ => {}
        }
    }

    let deps = classify_uses(&uses, source);

    // Priority 1: traits.
    if let Some(t) = traits.into_iter().next() {
        return InspectEntry::RustTrait(build_trait_entry(file, &t, deps, loc));
    }

    // Priority 2: structs — pick the one with the most methods, then by field count.
    let methods_by_owner = collect_impl_methods(&impls);
    let trait_impls_by_owner = collect_trait_impls(&impls);
    if let Some(s) = pick_main_struct(structs, &methods_by_owner) {
        return InspectEntry::RustStruct(build_struct_entry(file, &s, tree, &methods_by_owner,
            &trait_impls_by_owner, deps, loc));
    }

    // Priority 3: enums.
    if let Some(e) = enums.into_iter().next() {
        return InspectEntry::RustEnum(build_enum_entry(file, &e, deps, loc));
    }

    // Priority 4: module / re-export fallback.
    let mut re_exports = Vec::new();
    for u in &uses {
        if let Some(text) = pub_use_text(u) {
            re_exports.push(text);
        }
    }
    for m in &modules {
        if let Some(text) = pub_mod_text(m) {
            re_exports.push(text);
        }
    }

    InspectEntry::RustModule(RustModuleEntry {
        file: file.to_owned(),
        re_exports,
        deps,
        loc,
    })
}
```

Traits > most-prominent struct > first enum > module fallback — matching the regex stub's priority. Multi-struct files surface the struct with the most methods (`method_count * 100 + field_count` score).

### Export collection

```rust
pub(crate) fn collect_exports(source: &str, tree: &SourceFile) -> Vec<ExportRecord> {
    let mut exports = Vec::new();
    for item in tree.items() {
        match item {
            ast::Item::Struct(strukt) if is_exported_item(&strukt) => { /* … */ }
            ast::Item::Enum(enm) if is_exported_item(&enm) => { /* … */ }
            ast::Item::Trait(trait_item) if is_exported_item(&trait_item) => { /* … */ }
            ast::Item::Fn(func) if is_exported_item(&func) => { /* … */ }
            ast::Item::TypeAlias(alias) if is_exported_item(&alias) => { /* … */ }
            ast::Item::Const(konst) if is_exported_item(&konst) => { /* … */ }
            ast::Item::Module(module) if is_exported_item(&module) => { /* … */ }
            ast::Item::Use(use_item) => {
                let line = byte_offset_to_line(source, use_item.syntax().text_range().start().into());
                let text = use_item.syntax().text().to_string();
                if let Some(rest) = text.trim().strip_prefix("pub use ") {
                    for leaf in rust_analyzer::expand_use_clause(rest.trim_end_matches(';').trim()) {
                        let (_, binding) = rust_analyzer::rust_binding(&leaf);
                        exports.push(ExportRecord {
                            name: binding.local,
                            line,
                            re_export: true,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    exports
}

fn is_exported_item<T>(item: &T) -> bool where T: HasAttrs + HasVisibility {
    item.visibility().is_some() || has_napi_attr(item)
}

fn has_napi_attr<T>(item: &T) -> bool where T: HasAttrs {
    item.attrs().any(|attr| {
        let text = attr.syntax().text().to_string();
        let text = text.trim();
        text.strip_prefix("#[")
            .and_then(|rest| rest.split(['(', ']', ' ']).next())
            .is_some_and(|name| name == "napi")
    })
}
```

`#[napi]` / `#[napi(object)]` annotated items are exported even without explicit visibility — this matches the napi-rs convention where the macro produces extern-visible bindings regardless of Rust `pub`.

### Struct/method/derive/cfg helpers

`pick_main_struct` scores `method_count * 100 + field_count`. `extract_struct_fields` handles both record and tuple struct field lists. `build_struct_entry` assembles `RustStructEntry` from the AST, including `derives` (via `extract_derives`), `cfg_features` (via the AST-walking `extract_cfg_features`), `trait_impls` (from `collect_trait_impls`), `methods` (from `collect_impl_methods`), and `error_type` (from `extract_result_err_type` scanning method signatures for `Result<_, E>`).

`extract_cfg_features` walks every `Attr` descendant of the syntax tree and looks for `cfg(...)` attributes. Inside each it scans for `feature = "X"` substring patterns, deduplicating. This handles compound predicates like `#[cfg(any(feature = "X", feature = "Y"))]` and also nested feature gates on struct fields, not just top-level attributes.

### Trait / enum / use classification

`split_trait_methods` distinguishes required (no `body()`) from provided methods. `fn_to_method` formats the method signature as `"(params) -> ReturnType"`, captures `async`, and maps visibility via `node_visibility`.

`classify_uses` walks `ast::Use` items, calls `expand_use_paths(use_tree)` to flatten to fully-qualified paths, then buckets them:

- `std`/`core`/`alloc` → `std`
- `crate`/`super`/`self` → `local`
- known role crates (`tokio`, `axum`, `serde`, `thiserror`, `sqlx`, `proptest`, `criterion`, etc.) → specific role fields via `classify_external_crate`
- workspace crates (`is_workspace_crate`: matches a known-crates allowlist for the workspace) → `workspace`
- other → `external`

Unit-testing flag: `imports.testing.unit = source.contains("#[cfg(test)]") || source.contains("#[test]") || source.contains("#[tokio::test]")`.

---

## 6. `rust_inspection.rs` — updated

The regex-based stub is now an inline fallback module — its public surface is `inspect_rust_file(file)` returning `InspectEntry`, used when the `lang-rs` feature is disabled. It also serves as a test reference for what the AST path should produce.

The file gained a long documentation header explaining that regex patterns are validated at workspace build time by the bundled unit tests, and an `expect_used` allow for the `LazyLock::new(|| Regex::new(...).expect(...))` initializers. **Post-research correction**: scope that allow to each regex static individually, not `#![allow(clippy::expect_used)]` at the module level — a module-wide allow silently permits any other `.expect()` added later in the file, not just the ones whose panic-freedom is actually guaranteed by the build-time tests:

```rust
#[allow(clippy::expect_used)]
static RE_STRUCT_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)")
        .expect("RE_STRUCT_HEADER pattern is validated by unit tests at build time")
});
```

Applied identically to every other `LazyLock<Regex>` static in the file (`RE_ENUM_HEADER`, `RE_TRAIT_HEADER`, `RE_USE`, `RE_PUB_USE`, `RE_PUB_MOD`, `RE_IMPL_HEADER`, `RE_FN`, `RE_DERIVE`, `RE_CFG_FEATURE`).

Module-level docs now explicitly call out the limitation: "Without crate-graph resolution, a `use submodule::Item` looks identical to a `use externcrate::Item` from a single file's perspective. We treat any non-`{std,core,alloc,crate,super,self}` first segment as an external crate." This honesty about heuristic precision feeds the `CapabilityPrecision::Structural` flag in §11.

Mechanics retained from before:

- `strip_block_comments` — replaces `/* … */` content with spaces while preserving newlines so line numbers stay accurate.
- `RE_STRUCT_HEADER`, `RE_ENUM_HEADER`, `RE_TRAIT_HEADER`, `RE_USE`, `RE_PUB_USE`, `RE_PUB_MOD`, `RE_IMPL_HEADER`, `RE_FN`, `RE_DERIVE`, `RE_CFG_FEATURE` — `LazyLock<Regex>`-wrapped patterns.
- `extract_structs`, `extract_enums`, `extract_traits`, `extract_impl_methods`, `extract_trait_impls`, `extract_imports`, `extract_re_exports`, `extract_cfg_features`, `extract_result_err_type` — produce the same `Raw*`/`RustField`/`RustMethod`/`RustVariant`/`RustImports` shapes as the AST path.
- `find_balanced`, `split_top_level_commas`, `parse_visibility`, `strip_visibility_prefix`, `collapse_whitespace`, `line_is_commented`, `first_non_sig_char` — naive but effective string manipulation helpers.

What is new vs. Phase 1's standalone module:

- Module no longer owns `InspectEntry::Rust*` construction unconditionally — it is reached only via `RustAnalyzer::build_inspect_entry` when `lang-rs` is off.
- Re-exports of the helper functions used by `rust_syntax.rs` (`expand_use_clause`, `rust_binding`) live in the `analysis::rust_analyzer` module instead, so this file stops being the canonical source for use-clause parsing semantics.

---

## 7. Updated cache architecture (`analysis/cache.rs`)

The in-memory hot path is now a process-local `DashMap` only:

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

/// Look up a cache entry by path.
///
/// Returns a cloned `Arc` from the cache, or `None` if not present.
pub(crate) fn lookup(path: &Utf8Path) -> Option<Arc<FileAnalysis>> {
    cache()
        .get(path.as_str())
        .map(|entry_ref| Arc::clone(entry_ref.value()))
}

/// Insert or update the cache with a complete analysis.
///
/// Any existing entry for this path is unconditionally replaced — every
/// `FileAnalysis` is now fully populated, so there is no downgrade risk.
pub(crate) fn insert(path: &Utf8Path, a: Arc<FileAnalysis>) {
    cache().insert(path.to_string(), a);
}
```

Key change vs. Phase 1: the previous module wrapped a `Mutex<CacheMeta>` around the inner state and used `fs2::FileExt` advisory locks at the cache layer to coordinate cross-process disk writes. Phase 2+ splits responsibilities cleanly:

- **`analysis/cache.rs`** is now lock-free for reads (DashMap shard locks only), and is process-local.
- **`analysis/persist.rs`** owns disk persistence. It is the layer that uses `Mutex<CacheMeta>` for the per-config-hash metadata and `fs2` advisory locks for the on-disk write coordination (see `RustAnalyzer::analyze_cached_or_parse` calling `persist::lookup`, `persist::lookup_by_hash`, `persist::refresh_mtime`, `persist::queue_write`, and `persist::init`).

This split is the ADR-0016 boundary referenced in the call-out: hot reads of a current-process analysis don't have to traverse a mutex; cross-process invalidation lives in the disk persistence layer.

> **Post-research addition.** `fs2` advisory locking was asserted in prose above and in the dependency list (§2) but never actually shown — a real gap for a reconstruction-grade spec. It solves a different problem than `flush()`'s existing tempfile-then-rename (§1's `persist.rs`): the rename already guarantees no reader ever sees a _torn_ `cache.json`; it does nothing to stop a _lost update_, where two concurrent monokl processes each read-merge-write and the second `rename()` silently discards the first process's queued entries. The advisory lock below closes that gap by serializing the read-merge-write-rename span itself:
>
> ```rust
> use fs2::FileExt;
>
> fn with_cache_lock<T>(root: &Utf8Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
>     let lock_path = root.join(".monokl/cache.lock");
>     std::fs::create_dir_all(root.join(".monokl"))
>         .map_err(|e| io_errors::FileIoError::create_dir(root.join(".monokl"), e))?;
>     let lock_file = std::fs::OpenOptions::new()
>         .create(true)
>         .truncate(false)
>         .write(true)
>         .open(&lock_path)
>         .map_err(|e| io_errors::FileIoError::write(&lock_path, e))?;
>     // Exclusive — only one process may be inside the read-merge-write-rename span at a time.
>     // Held for the whole closure; released on drop (including on early return via `?`).
>     lock_file.lock_exclusive().map_err(|e| io_errors::FileIoError::write(&lock_path, e))?;
>     let result = f();
>     let _ = lock_file.unlock(); // best-effort — dropping `lock_file` releases it regardless
>     result
> }
> ```
>
> `flush()` (§1) wraps its existing body — stat, merge `write_queue`, serialize, tempfile-write, rename — in `with_cache_lock(root, || { .. })`. Pure reads (`lookup`, `lookup_by_hash`) take no lock at all: they only ever read the in-memory `state.cache_file` snapshot loaded at `init()`, never re-read `cache.json` from disk mid-process, so there's no torn/stale read to guard against on the read path. A stale lock left by a killed process resolves itself: `fs2`'s advisory lock is tied to the OS file descriptor, released automatically when that process exits, killed or not.

Cache key shape — the persisted entry type is `PersistedFileAnalysis` exposing:

```rust
pub struct PersistedFileAnalysis {
    pub content_hash: ContentHash,
    pub mtime_ns: u64,
    pub size_bytes: u64,
    pub symbols: Vec<SymbolEntry>,
    pub dependencies: Vec<DependencyRecord>,
    pub exports: Vec<ExportRecord>,
    pub ts_data: Option<TsData>,
    pub rust_inspect_entry: Option<InspectEntry>,
    pub blocks: Vec<CodeBlock>,
    pub had_parse_errors: bool,
    pub last_accessed_ns: u64,
    pub line_count: usize,
}
```

(Reconstructed from the `parse_with_profile` `PersistedFileAnalysis` construction in §4 — confirms the per-language `ts_data`/`rust_inspect_entry` slots, content-hash + mtime + size index, and `last_accessed_ns` field for cache eviction.)

The session layer in `pipeline/session.rs` then calls `crate::analysis::persist::init(&opts.root, &config_hash)?` to bootstrap the on-disk cache directory keyed on the registry's composite config hash.

---

## 8. Pipeline modularization

`pipeline.rs` is now an orchestrator that delegates to a `pipeline/` subdirectory of focused submodules. The `pipeline` module's children are:

```rust
mod deps;
mod graph;
mod inspecting;
mod search;
pub(crate) mod session;
mod symbols;
```

Public functions in `pipeline.rs` are thin wrappers that route to the appropriate submodule. The orchestrator surface:

```rust
pub fn search(opts: &SearchOptions) -> Result<SearchResponse> { search::run(opts) }
pub fn extract(req: &ExtractRequest) -> Result<ExtractResult> { inspecting::extract(req) }
pub fn symbols(files: &[Utf8PathBuf], lite: bool) -> Result<SymbolsResult> { symbols::run(files, lite) }
pub fn symbols_with_options(...) -> Result<SymbolsResult> { symbols::run_with_options(files, opts, lite) }
pub fn symbols_with_options_scoped(...) -> Result<SymbolsResult> { symbols::run_with_options_scoped(files, opts, lite, scope) }
pub fn dependents(file: &Utf8Path, root: &Utf8Path) -> Result<DependentsResult> { graph::dependents(file, root) }
pub fn dependents_scoped(file: &Utf8Path, root: &Utf8Path, scope: Option<&QueryScopeOptions>) -> Result<DependentsResult> { graph::dependents_scoped(file, root, scope) }
pub fn inspect(files: &[Utf8PathBuf], opts: &WorkspaceOptions) -> Result<InspectResult> {
    inspect_with_options(files, &InspectOptions::new(opts.clone()))
}
pub fn inspect_with_options(files: &[Utf8PathBuf], opts: &InspectOptions) -> Result<InspectResult> {
    let mut result = inspection::inspect_files(files, &opts.workspace)?;
    if let Some(kind) = opts.kind_filter.as_deref() {
        result.entries.retain(|entry| entry.kind() == kind);
        result.file_count = result.entries.len();
    }
    Ok(result)
}
pub fn absolute_workspace_root(candidate: Option<&Utf8Path>) -> Result<Utf8PathBuf> {
    session::absolute_workspace_root(candidate)
}
pub fn deps(paths: &[Utf8PathBuf], opts: &WorkspaceOptions) -> Result<DepsResult> { deps::deps(paths, opts) }
pub fn deps_from(package: &str, root: &Utf8Path, opts: &WorkspaceOptions) -> Result<DepsFromResult> { deps::deps_from(package, root, opts) }
pub fn classify(paths: &[Utf8PathBuf], opts: &WorkspaceOptions, legacy: &[String], target: &[String]) -> Result<ClassifyResult> { deps::classify(paths, opts, legacy, target) }
pub fn migration_surface(root: &Utf8Path, opts: &WorkspaceOptions, legacy: &[String], target: &[String]) -> Result<MigrationSurfaceResult> { deps::migration_surface(root, opts, legacy, target) }
#[cfg(feature = "lang-ts")]
pub fn patterns(dir: &Utf8Path, opts: &WorkspaceOptions) -> Result<PatternsResult> { inspecting::patterns(dir, opts) }
pub fn refs(symbol, opts, include_tests, max_refs, from_file) -> Result<refs::RefsResult> { refs::find_refs(symbol, opts, include_tests, max_refs, from_file) }
pub fn refs_scoped(symbol, opts, include_tests, max_refs, from_file, scope) -> Result<refs::RefsResult> { refs::find_refs_scoped(...) }
pub fn refs_with_kinds(...) / refs_with_kinds_scoped(...) -> Result<refs::RefsResult>
pub fn definition(symbol, opts, from_file) -> Result<definition::DefinitionResult> { definition::find_definition(symbol, opts, from_file) }
pub fn definition_scoped(symbol, opts, from_file, scope) -> Result<definition::DefinitionResult> { definition::find_definition_scoped(...) }
#[cfg(feature = "lang-ts")]
pub fn diff(base: &str, head: &str, root: &Utf8Path) -> Result<diff_mod::DiffResult> { diff_mod::diff(base, head, root) }
#[cfg(feature = "lang-ts")]
pub fn tokens(dir: &Utf8Path, opts: &WorkspaceOptions) -> Result<crate::tokens_analysis::TokensResult> { ... }
#[cfg(feature = "lang-ts")]
pub fn explain / coverage / data_flow / similar (thin wrappers)
pub fn source_files(paths, opts) -> Result<Vec<Utf8PathBuf>> { deps::source_files(paths, opts) }
pub fn candidate_source_files(paths, opts) -> Result<Vec<Utf8PathBuf>> { deps::candidate_source_files(paths, opts) }
```

### `pipeline/search.rs`

Owns the 8-stage search pipeline plus scope filtering, language filter validation, and the rayon parallel block extraction. Key flow:

1. Parse query (or escape-as-literal in `exact` mode); return empty with diagnostic if parsing yields zero terms.
2. Build `QueryPlan` from `terms`; partition `lang_filters` into supported (`ts/tsx/js/jsx/mjs/cjs/mts/cts/rs/rust`) and unsupported; collect search patterns; bail with diagnostic when no positive patterns.
3. Canonicalize workspace root, build the per-workspace session, derive `requested_languages` from CLI flag + lang filters, enumerate source files via `session.source_files`, filter by `path_filters` and `matches_lang_filter`.
4. Emit `Skipped` diagnostic for unsupported requested languages; if `has_block_capability` is false for the requested set, return empty with diagnostic.
5. Apply git scope: if `ImpactedNeighbors`, fetch `session.standard_index()` and call `git_scope::scope_files_with_index(files, &root, &opts.scope, Some(index.as_ref()))`; otherwise `git_scope::scope_files`. Surface scope diagnostics describing match counts.
6. Compile term regexes honoring `case_sensitive`; build `scan_patterns` = positive patterns ∪ excluded term patterns.
7. `text_search::search_files(&files, &scan_patterns, opts.limits.max_candidates, opts.case_sensitive)` gives `Vec<(path, Vec<LineHit>)>`.
8. Parallel `into_par_iter` over hits: resolve analyzer per file, skip files whose analyzer lacks `capabilities().blocks`, apply boolean exclusion/required-term filtering, call `analyzer.analyze`, annotate blocks with `matched_lines`/`matched_keywords` restricted to lines that pass `git_scope::line_matches_line_scope`, tokenize each block via `rank::tokenize_block`.
9. Aggregate per-language "block-extraction omitted" counts into a single diagnostic per language.
10. Build `df` over tokens, rank via `rank::rank_blocks(all_blocks, &bm25_query_terms, &df)`, dedup via `dedup::dedup_blocks`, apply budget via `budget::apply_budget`.

### `pipeline/symbols.rs`

Modular workflow:

- `run(files, lite)` — derives the workspace root from `files[0].parent()` and calls `run_with_options`.
- `run_with_options(files, opts, lite)` → `run_with_options_scoped(files, opts, lite, None)`.
- `run_with_options_scoped(files, opts, lite, scope)`:
  - Drops `lite` (kept for API back-compat).
  - Opens `WorkspaceSession`.
  - If `scope.mode == FullRepo`, uses the caller-provided file list; otherwise calls `session.source_files(...)` with `allow_tests: true, no_gitignore: false, languages: None` to give the scope layer the broadest candidate set.
  - Calls `apply_scope(&candidate_files, opts, &session, &scope, &mut diagnostics)` which itself calls `scoped_files` (which dispatches to `git_scope::scope_files_with_index` or `git_scope::scope_files`). Empty selection → return empty result with a "matched no supported files" diagnostic.
  - Surfaces a `scope_diagnostic_message` describing changed file/line/hunk counts when applicable.
  - `analyze_symbol_files` parallelizes per-file analysis (`AnalysisProfile::Structural`). Each outcome carries `symbols`, `classified` (only when `analyzer.capabilities(language).classified_imports` for that file's resolved language), `omitted_classified_imports`, and degraded diagnostics from `had_parse_errors`.
  - For `ChangedLines` scope: symbols are filtered by `git_scope::line_matches_line_scope(selection, file, symbol.line)`.
  - Per-file cap = 50, total cap = 500 — when total is hit, the in-progress file is truncated and `truncation_marker` is set, loop breaks.
  - At the end, emits an aggregated `Warning` diagnostic if any files had `omitted_classified_imports` (Rust files trigger this; ChangedLines scope also increments it because import classification is file-level).

### `pipeline/deps.rs`

Owns `deps`, `deps_from`, `classify`, `migration_surface`, plus the shared `source_files` / `candidate_source_files` helpers. Defaults for migration:

```rust
const DEFAULT_LEGACY_PREFIXES: &[&str] = &["@legacy-ui/core", "styled-components"];
const DEFAULT_TARGET_PREFIXES: &[&str] = &[
    "@design-system/core",
    "@design-system/foundation",
    "@design-system/web",
    "@design-system/ui",
];
```

`deps` runs inspect via `inspect_with_options` then projects each `InspectEntry` through `dependency_summary` (which converts `ClassifiedImports` to `FileDependencySummary` for TS entries and `RustImports` for Rust entries).

`deps_from` parallelizes per-file analysis with `AnalysisProfile::Dependencies` and filters `dep.target` matches via `dependency_target_matches` (handles `File`/`RustPath`/`Namespace` targets) and `package_matches` (exact match or prefix followed by `/`, `::`, or `.`). Builds `DepsFromEntry` lists with binding names from `dependency_binding_names` (formats `imported as local`, `*`, or just the local/imported name).

`classify` runs per-file `dependency_specifiers_for_file` (also `AnalysisProfile::Dependencies`), then matches specifiers against the legacy/target prefix sets. Unsupported files produce `ClassificationEntry { status: "unsupported", score: 0.0, … }`.

`migration_surface` is `classify` plus aggregation into `MigrationSurfaceSummary` with `adoption_pct = (target_files + mixed_files) / total`.

`unsupported_file_message` formats: `"no analyzer registered for {language} files in this build"` using `source_scan::candidate_language(file)` to identify the language.

### `pipeline/graph.rs`

Owns `dependents` and `dependents_scoped`. Build flow:

1. Canonicalize file and root; verify `abs_file.starts_with(abs_root)` else `PathOutsideRoot`.
2. Open session; build `standard_index`.
3. Look up the analyzer for `abs_file`. If no analyzer matches → `add_missing_analyzer_diagnostic("dependents", …)`, return empty.
4. Call `require_precision_capability` on `resolved_import_graph` with `PrecisionRequirement { operation: "dependents", minimum: CapabilityPrecision::Structural }`. Below-structural surfaces "is not supported" and returns empty; structural-but-not-exact emits a precision-warning diagnostic.
5. TypeScript-only: warn when no `tsconfig.json` is present at or above the workspace root.
6. Fetch raw dependents/imports from `index.import_graph`.
7. If scope is git-backed, narrow the result lists by `git_scope::scope_files_with_index(index.files.clone(), &abs_root, &scope, Some(index.as_ref()))`. For `ChangedLines`, attach a warning that "scope changed-lines falls back to changed-file behavior for dependents because the import graph is file-level."
8. Cap each list at 200, set `truncation_marker` when either was capped.

### `pipeline/inspecting.rs`

Owns `extract` and `patterns`. Extract flow:

```rust
pub(super) fn extract(req: &ExtractRequest) -> Result<ExtractResult> {
    let root = session::absolute_workspace_root(req.file.parent())?;
    let session = session::for_workspace(&WorkspaceOptions::new(root))?;
    let Some(analyzer) = session.registry().analyzer_for(&req.file) else {
        let mut diagnostics = Vec::new();
        add_missing_analyzer_diagnostic("extract", &req.file, &mut diagnostics);
        return Ok(ExtractResult { blocks: Vec::new(), diagnostics });
    };

    let file_for_closure = req.file.clone();
    let full = analyzer.analyze(&req.file, Box::new(move || {
        std::fs::read_to_string(file_for_closure.as_std_path())
            .map_err(|e| MonoklError::Io(FileIoError::read(&file_for_closure, e)))
    }))?;

    let mut diagnostics = Vec::new();
    if full.had_parse_errors { /* push Degraded diagnostic */ }

    let blocks = full.blocks.iter()
        .filter(|block| match (req.line_start, req.line_end) {
            (None, None) => true,
            (Some(start), None) => block.line_start <= start && block.line_end >= start,
            (None, Some(end)) => block.line_start <= end,
            (Some(start), Some(end)) => block.line_start <= end && block.line_end >= start,
        })
        .cloned()
        .collect();

    Ok(ExtractResult { blocks, diagnostics })
}
```

`patterns` (TS-only) walks `dir` for `LanguageId::TypeScript` files, runs `inspection::inspect_files_with_registry`, and aggregates counts into `PatternsResult`. Adds extended hook tracking (`useContext`, `useReducer`, `useMemo`, `useSuspense`), `hoc_count`/`lazy_count`/`zod_usage` from `is_hoc`/`is_lazy`/non-empty `zod_schemas`, plus toolchain accumulation (first-seen build tool, first-seen test runner, deduplicated types).

### `pipeline/session.rs`

The new `WorkspaceSession` cache. Workspace-keyed by `"{root}::{tsconfig}"` (where tsconfig is one of `"auto"`, `"skip"`, or `"manual:{path}"`):

```rust
pub(crate) struct WorkspaceSession {
    opts: WorkspaceOptions,
    registry: AnalyzerRegistry,
    config_hash: String,
    source_files: DashMap<SourceScanCacheKey, Arc<Vec<Utf8PathBuf>>>,
    standard_index: Mutex<Option<Arc<WorkspaceIndex>>>,
}

impl WorkspaceSession {
    pub(crate) fn registry(&self) -> &AnalyzerRegistry { &self.registry }

    pub(crate) fn source_files(&self, opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
        let key = SourceScanCacheKey {
            allow_tests: opts.allow_tests,
            no_gitignore: opts.no_gitignore,
            languages: normalize_languages(opts.languages.clone()),
        };
        if let Some(entry) = self.source_files.get(&key) {
            return Ok((**entry).clone());
        }
        let files = source_scan::collect_supported_files(&self.opts.root, &self.registry, opts)?;
        let files = Arc::new(files);
        self.source_files.insert(key, Arc::clone(&files));
        Ok((*files).clone())
    }

    pub(crate) fn standard_index(&self) -> Result<Arc<WorkspaceIndex>> {
        let mut guard = self.standard_index.lock()
            .map_err(|_| MonoklError::LockPoisoned { context: "pipeline::session::WorkspaceSession::standard_index" })?;
        if let Some(index) = guard.as_ref() {
            return Ok(Arc::clone(index));
        }
        let enrichers: Vec<Box<dyn WorkspaceEnricher>> = vec![
            Box::new(ImportGraphEnricher::new()),
            Box::new(SymbolIndexEnricher::new()),
        ];
        let index = Arc::new(WorkspaceIndex::build_with_profile(
            &self.opts.root,
            &self.registry,
            enrichers,
            &self.config_hash,
            crate::analysis::AnalysisProfile::Structural,
        )?);
        *guard = Some(Arc::clone(&index));
        Ok(index)
    }
}

pub(crate) fn for_workspace(opts: &WorkspaceOptions) -> Result<Arc<WorkspaceSession>> {
    let key = workspace_cache_key(opts);
    if let Some(entry) = session_cache().get(&key) {
        return Ok(Arc::clone(&entry));
    }
    let registry = AnalyzerRegistry::from_workspace_opts(opts)?;
    let config_hash = registry.config_hash().to_owned();
    crate::analysis::persist::init(&opts.root, &config_hash)?;
    let session = Arc::new(WorkspaceSession {
        opts: opts.clone(),
        registry,
        config_hash,
        source_files: DashMap::new(),
        standard_index: Mutex::new(None),
    });
    session_cache().insert(key, Arc::clone(&session));
    Ok(session)
}

pub(crate) fn absolute_workspace_root(candidate: Option<&Utf8Path>) -> Result<Utf8PathBuf> {
    if let Some(path) = candidate {
        return path.canonicalize_utf8().map_err(|e| MonoklError::Io(FileIoError::read(path, e)));
    }
    let cwd = std::env::current_dir().map_err(|e| MonoklError::Io(FileIoError::read(".", e)))?;
    Utf8PathBuf::from_path_buf(cwd).map_err(|path| MonoklError::NonUtf8Path { path })
}
```

`SourceScanCacheKey` includes `languages: Option<Vec<LanguageId>>` so cached source-file lists are keyed by language-filter set. `normalize_languages` sorts and dedupes for stable hashing.

The session cache lives in a `OnceLock<DashMap<String, Arc<WorkspaceSession>>>`.

---

## 9. `git_scope.rs` — PR-aware scopes

Shared git-backed query scoping utilities — the single owner of git ref validation, changed-file enumeration, blob lookup at a specific revision, and changed-file filtering for review-oriented scopes. Backed by `gix`.

### Core types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeSetMode {
    PrAware,
    #[cfg_attr(not(feature = "lang-ts"), allow(dead_code))]
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitFileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitChangedFile {
    pub path: Utf8PathBuf,
    pub kind: GitFileChangeKind,
}

#[derive(Debug, Clone)]
pub(crate) struct ChangeSet {
    mode: ChangeSetMode,
    requested_base: String,
    requested_head: String,
    effective_base: String,
    effective_head: String,
    pub files: Vec<GitChangedFile>,
    pub line_ranges: BTreeMap<Utf8PathBuf, Vec<LineRange>>,
    pub changed_line_count: usize,
    pub changed_hunk_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ScopeSelection {
    pub files: Vec<Utf8PathBuf>,
    pub changed_path_count: Option<usize>,
    pub changed_line_count: Option<usize>,
    pub changed_hunk_count: Option<usize>,
    pub matched_supported_file_count: Option<usize>,
    pub neighbor_file_count: Option<usize>,
    pub change_summary: Option<String>,
    pub line_ranges: BTreeMap<Utf8PathBuf, Vec<LineRange>>,
}
```

### Ref validation

```rust
pub(crate) fn validate_git_ref(r: &str) -> Result<()> {
    if r.is_empty() {
        return Err(MonoklError::InvalidGitRef { ref_: r.to_owned(), reason: "ref is empty" });
    }
    if r.starts_with('-') {
        return Err(MonoklError::InvalidGitRef {
            ref_: r.to_owned(),
            reason: "starts with '-' — refusing as it could be parsed as a git option",
        });
    }
    let ok = r.chars().all(|c| {
        matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '/' | '.' | '@' | '{' | '}' | '^' | '~' | ':' | '+' | '-')
    });
    if !ok {
        return Err(MonoklError::InvalidGitRef {
            ref_: r.to_owned(),
            reason: "contains characters outside the safe set",
        });
    }
    Ok(())
}
```

### PR semantics

```rust
fn effective_pr_range(repo: &gix::Repository, scope: &QueryScopeOptions) -> Result<(String, String)> {
    let (base, head) = requested_git_range(scope);
    let base_id = resolve_commit_id(repo, base)?;
    let head_id = resolve_commit_id(repo, head)?;
    let merge_base = repo
        .merge_base(base_id.detach(), head_id.detach())
        .map_err(|err| MonoklError::Git { operation: "merge-base", message: format!("{base} {head}: {err}") })?;

    Ok((merge_base.to_string(), head.to_owned()))
}

pub(crate) fn build_pr_change_set(root: &Utf8Path, scope: &QueryScopeOptions) -> Result<ChangeSet> {
    let (base, head) = requested_git_range(scope);
    build_change_set(
        root,
        base,
        head,
        ChangeSetMode::PrAware,
        scope.mode == crate::types::QueryScope::ChangedLines,
    )
}
```

`requested_git_range` defaults `base` to `"origin/main"` and `head` to `"HEAD"`. `ChangeSetMode::Snapshot` (used only by `diff`) keeps the literal base/head pair without applying merge-base substitution; `ChangeSetMode::PrAware` replaces base with `merge-base(base, head)`. This is the load-bearing distinction the 0.6.0 changelog calls out — `diff` stays explicit-snapshot.

### Change collection

```rust
fn collect_changes(root: &Utf8Path, base: &str, head: &str, include_line_ranges: bool) -> Result<CollectedChanges> {
    let repo = open_repo(root)?;
    let from = resolve_tree(&repo, base)?;
    let to = resolve_tree(&repo, head)?;
    let mut changes: Vec<GitChangedFile> = Vec::new();
    let mut line_ranges: BTreeMap<Utf8PathBuf, Vec<LineRange>> = BTreeMap::new();
    let mut changed_line_count = 0usize;
    let mut changed_hunk_count = 0usize;

    let mut change_platform = from.changes().map_err(|err| MonoklError::Git { /* … */ })?;
    let mut diff_cache = if include_line_ranges {
        Some(repo.diff_resource_cache_for_tree_diff().map_err(/* … */)?)
    } else { None };
    change_platform.options(|opts| {
        opts.track_rewrites(Some(gix::diff::Rewrites::default())).track_path();
    });
    change_platform.for_each_to_obtain_tree(&to, |change| {
        let Some(path) = relative_path_from_change(&change) else {
            return Ok::<ControlFlow<()>, MonoklError>(ControlFlow::Continue(()));
        };

        let kind = match change {
            gix::object::tree::diff::Change::Addition { .. } => GitFileChangeKind::Added,
            gix::object::tree::diff::Change::Deletion { .. } => GitFileChangeKind::Deleted,
            gix::object::tree::diff::Change::Modification { .. } => GitFileChangeKind::Modified,
            gix::object::tree::diff::Change::Rewrite { copy, .. } => {
                if copy {
                    GitFileChangeKind::Added
                } else {
                    match relative_source_path(&change) {
                        Some(from) => GitFileChangeKind::Renamed { from },
                        None => GitFileChangeKind::Modified,
                    }
                }
            }
        };

        if include_line_ranges {
            let ranges = if let Some(diff_cache) = diff_cache.as_mut() {
                line_ranges_for_change(&change, diff_cache)?
            } else { Vec::new() };
            if !ranges.is_empty() {
                changed_hunk_count += ranges.len();
                changed_line_count += ranges.iter()
                    .map(|range| range.end.saturating_sub(range.start) + 1).sum::<usize>();
                line_ranges.insert(path.clone(), ranges);
            }
        }

        changes.push(GitChangedFile { path, kind });
        Ok(ControlFlow::Continue(()))
    }).map_err(/* … */)?;
    Ok(CollectedChanges { files: changes, line_ranges, changed_line_count, changed_hunk_count })
}
```

`line_ranges_for_change` uses `gix::diff::blob::diff` with the platform's `InternalDiff` operation, harvesting `after: Range<u32>` head-side ranges and converting them to 1-based inclusive `LineRange { start, end }`. Binary and external-diff cases yield empty ranges.

### Scope dispatch

```rust
pub(crate) fn scope_files(files: Vec<Utf8PathBuf>, root: &Utf8Path, scope: &QueryScopeOptions) -> Result<ScopeSelection> {
    scope_files_with_index(files, root, scope, None)
}

pub(crate) fn scope_files_with_index(
    files: Vec<Utf8PathBuf>,
    root: &Utf8Path,
    scope: &QueryScopeOptions,
    index: Option<&WorkspaceIndex>,
) -> Result<ScopeSelection> {
    // `FullRepo` short-circuits before `prepare_scope_inputs` — it needs no git range, so
    // folding it into the match below (rather than an unreachable!() arm after an early return)
    // makes the impossible case structurally absent instead of asserted away at runtime.
    match scope.mode {
        crate::types::QueryScope::FullRepo => Ok(full_repo_selection(files)),
        crate::types::QueryScope::ChangedFiles => {
            let prepared = prepare_scope_inputs(files, root, scope)?;
            Ok(changed_files_selection(prepared))
        }
        crate::types::QueryScope::ChangedLines => {
            let prepared = prepare_scope_inputs(files, root, scope)?;
            Ok(changed_lines_selection(prepared))
        }
        crate::types::QueryScope::ImpactedNeighbors => {
            let prepared = prepare_scope_inputs(files, root, scope)?;
            Ok(impacted_neighbors_selection(prepared, index))
        }
    }
}
```

`prepare_scope_inputs` canonicalizes worktree root, builds a PR change set, canonicalizes changed paths and changed line ranges to absolute UTF-8 paths, and intersects with the caller-provided `files` list to produce `changed_supported_files`.

`impacted_neighbors_selection` walks `index.dependents_of(changed_file)` and `index.imports_of(changed_file)` for each changed supported file and unions canonical neighbor paths into the selection. `neighbor_file_count` reports how many new (non-changed) files were added through neighbor expansion.

### Line-scope helpers

```rust
pub(crate) fn line_ranges_for_file<'a>(selection: &'a ScopeSelection, file: &Utf8Path) -> Option<&'a [LineRange]> {
    selection.line_ranges.get(file).map(Vec::as_slice)
}

pub(crate) fn range_overlaps_line_scope(selection: &ScopeSelection, file: &Utf8Path, start: usize, end: usize) -> bool {
    line_ranges_for_file(selection, file)
        .is_none_or(|ranges| ranges.iter().any(|range| start <= range.end && end >= range.start))
}

pub(crate) fn line_matches_line_scope(selection: &ScopeSelection, file: &Utf8Path, line: usize) -> bool {
    range_overlaps_line_scope(selection, file, line, line)
}
```

`range_overlaps_line_scope` returns `true` when the file has no recorded line range (so non-changed-lines scopes don't accidentally suppress everything) — this is the bridge that lets search and symbols use the same line-filter helper across all scope modes.

### Blob-at-ref (TS-only)

```rust
#[cfg(feature = "lang-ts")]
pub(crate) fn blob_at_ref(root: &Utf8Path, git_ref: &str, path: &str) -> Result<String> {
    let repo = open_repo(root)?;
    blob_at_ref_in_repo(&repo, git_ref, path).map_err(|message| MonoklError::Git {
        operation: "show",
        message,
    })
}

#[cfg(feature = "lang-ts")]
pub(crate) fn blob_at_ref_in_repo(
    repo: &gix::Repository,
    git_ref: &str,
    path: &str,
) -> std::result::Result<String, String> {
    let tree = resolve_tree(repo, git_ref).map_err(|err| err.to_string())?;
    let entry = tree.lookup_entry_by_path(path)
        .map_err(|err| format!("{git_ref}:{path}: {err}"))?
        .ok_or_else(|| format!("{git_ref}:{path}: path not found in tree"))?;
    let object = repo.find_object(entry.object_id())
        .map_err(|err| format!("{git_ref}:{path}: {err}"))?;
    let blob = object.try_into_blob()
        .map_err(|_| format!("{git_ref}:{path}: object is not a blob"))?;
    std::str::from_utf8(blob.data.as_ref())
        .map(ToOwned::to_owned)
        .map_err(|err| format!("{git_ref}:{path}: non-UTF-8 blob: {err}"))
}
```

`diff` uses `blob_at_ref` to load TS files at the base and head revisions for structural delta computation without checking the worktree out.

---

## 10. `projection.rs` — output projection / presentation modes

Shared JSON field projection. CLI and future MCP surfaces both need compact, question-shaped output.

```rust
//! Shared JSON field projection.
//!
//! CLI and MCP surfaces both need compact, question-shaped output. Projection
//! lives in the library so those surfaces can share one contract.

use serde_json::Value;

use crate::error::Result;
use crate::types::{InspectResult, SearchResponse};

#[must_use]
pub fn project_fields(value: Value, fields: &[String]) -> Value {
    let fields: Vec<&str> = fields.iter().map(String::as_str).map(str::trim)
        .filter(|field| !field.is_empty()).collect();
    if fields.is_empty() { return value; }
    project_value(value, &fields)
}

pub fn project_inspect_result(result: &InspectResult, fields: &[String]) -> Result<Value> {
    let value = serde_json::to_value(result)?;
    if fields.iter().all(|field| field.trim().is_empty()) {
        return Ok(value);
    }
    Ok(project_fields(value, &inspect_projection_fields(fields)))
}

pub fn project_search_response(result: &SearchResponse, fields: &[String]) -> Result<Value> {
    let value = serde_json::to_value(result)?;
    if fields.iter().all(|field| field.trim().is_empty()) {
        return Ok(value);
    }
    Ok(project_fields(value, &search_projection_fields(fields)))
}

#[must_use]
pub fn prune_empty(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut pruned = serde_json::Map::new();
            for (key, child) in map {
                let child = prune_empty(child);
                if !is_minimal_empty(&child) {
                    pruned.insert(key, child);
                }
            }
            Value::Object(pruned)
        }
        Value::Array(items) => {
            let items = items.into_iter().map(prune_empty)
                .filter(|item| !is_minimal_empty(item)).collect();
            Value::Array(items)
        }
        other => other,
    }
}

fn project_value(value: Value, fields: &[&str]) -> Value {
    match value {
        Value::Object(map) => {
            let mut projected = serde_json::Map::new();
            for (key, child) in map {
                if fields.iter().any(|field| *field == key) {
                    projected.insert(key, child);
                    continue;
                }
                let prefix = format!("{key}.");
                let child_fields: Vec<&str> = fields.iter()
                    .filter_map(|field| field.strip_prefix(&prefix)).collect();
                if !child_fields.is_empty() {
                    let child = project_value(child, &child_fields);
                    if !is_empty_projection(&child) {
                        projected.insert(key, child);
                    }
                }
            }
            Value::Object(projected)
        }
        Value::Array(items) => Value::Array(items.into_iter()
            .map(|item| project_value(item, fields)).collect()),
        other => other,
    }
}

fn search_projection_fields(fields: &[String]) -> Vec<String> {
    let mut projected = Vec::new();
    let mut has_result_field = false;
    for field in fields.iter().map(String::as_str).map(str::trim) {
        if field.is_empty() { continue; }
        let field = match field {
            "snippet" => "code",
            "score" => "finalScore",
            other => other,
        };
        match field {
            "results" | "totalBlocksBeforeTruncation" | "truncated"
            | "truncationMarker" | "totalBytes" | "totalTokens"
            | "diagnostics" => projected.push(field.to_owned()),
            _ if field.starts_with("results.") => {
                has_result_field = true;
                projected.push(field.to_owned());
            }
            _ => {
                has_result_field = true;
                projected.push(format!("results.{field}"));
            }
        }
    }
    if has_result_field {
        projected.push("truncated".to_owned());
        projected.push("totalTokens".to_owned());
        projected.push("diagnostics".to_owned());
    }
    projected.sort();
    projected.dedup();
    projected
}

fn inspect_projection_fields(fields: &[String]) -> Vec<String> {
    let mut projected = Vec::new();
    let mut has_entry_field = false;
    let mut has_entry_file = false;
    for field in fields.iter().map(String::as_str).map(str::trim) {
        if field.is_empty() { continue; }
        match field {
            "entries" | "fileCount" | "diagnostics" => projected.push(field.to_owned()),
            _ if field.starts_with("entries.") => {
                if field == "entries.file" { has_entry_file = true; }
                has_entry_field = true;
                projected.push(field.to_owned());
            }
            _ => {
                if field == "file" { has_entry_file = true; }
                has_entry_field = true;
                projected.push(format!("entries.{field}"));
            }
        }
    }
    if has_entry_field {
        if !has_entry_file { projected.push("entries.file".to_owned()); }
        projected.push("fileCount".to_owned());
        projected.push("diagnostics".to_owned());
    }
    projected.sort();
    projected.dedup();
    projected
}

fn is_empty_projection(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.is_empty(),
        Value::Array(items) => !items.is_empty() && items.iter().all(is_empty_projection),
        _ => false,
    }
}

fn is_minimal_empty(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Bool(true) => false,
        Value::Number(n) => n.as_i64() == Some(0) || n.as_u64() == Some(0) || n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
    }
}
```

Behaviors:

- `project_fields` walks the JSON tree retaining only keys that match the explicit field list or have a child match via dot-notation prefix.
- `project_search_response` aliases `snippet`→`code` and `score`→`finalScore`, auto-wraps unrecognized fields under `results.`, and unconditionally surfaces `truncated`/`totalTokens`/`diagnostics` so projected output still carries truncation context.
- `project_inspect_result` auto-wraps fields under `entries.`, force-includes `entries.file` so projected entries retain identity, and surfaces `fileCount`/`diagnostics`.
- `prune_empty` is the backbone of `--format minimal`: it recursively drops `null`, `false`, `0`/`0.0`, empty strings, empty arrays, and empty objects.

---

## 11. `query_support.rs` — capability-driven query behavior

Shared helpers for symbol-oriented query commands. Owns the analyzer-capability policy that `search`/`inspect`/`dependents`/`refs`/`definition` consume.

```rust
#[derive(Debug, Clone, Copy)]
pub(crate) struct PrecisionRequirement {
    pub operation: &'static str,
    pub minimum: CapabilityPrecision,
}

pub(crate) fn analyze_with_profile(
    analyzer: &dyn LanguageAnalyzer,
    file: &Utf8Path,
    profile: AnalysisProfile,
) -> Result<std::sync::Arc<FileAnalysis>> {
    let owned = file.to_owned();
    analyzer.analyze_with_profile(
        file,
        Box::new(move || {
            fs::read_to_string(owned.as_std_path()).map_err(|e| MonoklError::Io(FileIoError::read(&owned, e)))
        }),
        profile,
    )
}

pub(crate) enum CanonicalSourceOutcome {
    Resolved(Utf8PathBuf),
    Ambiguous { candidates: AmbiguousPaths, diagnostic: Diagnostic },
    Unresolved { diagnostic: Diagnostic },
    External,
    NotImported,
}

pub(crate) fn resolve_canonical_source(
    analyzer: &dyn LanguageAnalyzer,
    from_file: &Utf8Path,
    symbol: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> CanonicalSourceOutcome {
    let analysis = match analyze_with_profile(analyzer, from_file, AnalysisProfile::Structural) {
        Ok(a) => a,
        Err(e) => {
            let diagnostic = Diagnostic {
                kind: DiagnosticKind::Skipped,
                path: Some(from_file.to_owned()),
                message: e.to_string(),
            };
            diagnostics.push(diagnostic.clone());
            return CanonicalSourceOutcome::Unresolved { diagnostic };
        }
    };

    for dep in &analysis.dependencies {
        let imports_symbol = dep.bindings.iter()
            .any(|binding| binding_matches_symbol(binding, symbol));
        if !imports_symbol { continue; }
        let resolution = match &dep.target {
            DependencyTarget::File { resolution, .. }
            | DependencyTarget::RustPath { resolution, .. } => resolution,
            DependencyTarget::Namespace { .. } => continue,
        };
        return match resolution {
            DependencyResolution::Resolved { path } => match analyzer.language_for(from_file) {
                Some(LanguageId::Rust) => CanonicalSourceOutcome::Resolved(
                    follow_rust_canonical_source(analyzer, path, symbol, diagnostics),
                ),
                _ => CanonicalSourceOutcome::Resolved(path.clone()),
            },
            DependencyResolution::Ambiguous { candidates, diagnostic } => {
                CanonicalSourceOutcome::Ambiguous {
                    candidates: candidates.clone(),
                    diagnostic: diagnostic.clone(),
                }
            }
            DependencyResolution::Unresolved { diagnostic } => {
                CanonicalSourceOutcome::Unresolved { diagnostic: diagnostic.clone() }
            }
            DependencyResolution::External => CanonicalSourceOutcome::External,
        };
    }
    CanonicalSourceOutcome::NotImported
}

pub(crate) fn add_precision_diagnostic(
    analyzer: &dyn LanguageAnalyzer,
    path: &Utf8Path,
    diagnostics: &mut Vec<Diagnostic>,
    precision: CapabilityPrecision,
    operation: &'static str,
) {
    if precision < CapabilityPrecision::Structural {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Warning,
            path: Some(path.to_owned()),
            message: format!(
                "{operation} uses {}-precision {} analysis",
                precision_label(precision),
                analyzer.language_for(path).map(language_label).unwrap_or("unknown")
            ),
        });
    }
}

pub(crate) fn add_missing_analyzer_diagnostic(
    operation: &'static str,
    path: &Utf8Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let message = match candidate_language(path) {
        Some(language) => format!(
            "{operation} is not supported for {} files in this build",
            language_label(language)
        ),
        None => format!("{operation} skipped a file with no registered analyzer"),
    };
    diagnostics.push(Diagnostic {
        kind: DiagnosticKind::Skipped,
        path: Some(path.to_owned()),
        message,
    });
}

pub(crate) fn require_precision_capability(
    warned_languages: &mut BTreeSet<LanguageId>,
    analyzer: &dyn LanguageAnalyzer,
    path: &Utf8Path,
    diagnostics: &mut Vec<Diagnostic>,
    precision: CapabilityPrecision,
    requirement: PrecisionRequirement,
) -> bool {
    let language = analyzer.language_for(path);
    if precision < requirement.minimum {
        if language.is_none_or(|l| warned_languages.insert(l)) {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::Skipped,
                path: Some(path.to_owned()),
                message: format!(
                    "{} is not supported for {} files (requires {} precision, analyzer exposes {})",
                    requirement.operation,
                    language.map(language_label).unwrap_or("unknown"),
                    precision_label(requirement.minimum),
                    precision_label(precision)
                ),
            });
        }
        return false;
    }
    if precision < CapabilityPrecision::Exact && language.is_none_or(|l| warned_languages.insert(l)) {
        add_precision_diagnostic(analyzer, path, diagnostics, precision, requirement.operation);
    }
    true
}

pub(crate) fn file_imports_symbol_from(analysis: &FileAnalysis, symbol: &str, canonical: &Utf8Path) -> bool {
    for dep in &analysis.dependencies {
        let Some(resolved) = dependency_target_resolved(&dep.target) else { continue };
        if !paths_equal(resolved, canonical) { continue; }
        if dep.bindings.iter().any(|binding| binding_matches_symbol(binding, symbol)) {
            return true;
        }
    }
    false
}

pub(crate) fn dependency_target_resolved(target: &DependencyTarget) -> Option<&Utf8Path> {
    match target {
        DependencyTarget::File { resolution: DependencyResolution::Resolved { path }, .. }
        | DependencyTarget::RustPath { resolution: DependencyResolution::Resolved { path }, .. } => Some(path.as_path()),
        DependencyTarget::File { .. }
        | DependencyTarget::RustPath { .. }
        | DependencyTarget::Namespace { .. } => None,
    }
}

pub(crate) fn binding_matches_symbol(binding: &DependencyBinding, symbol: &str) -> bool {
    match binding.kind {
        BindingKind::Named | BindingKind::Default | BindingKind::Namespace => {
            binding.imported == symbol || binding.local == symbol
        }
        _ => false,
    }
}

pub(crate) fn paths_equal(a: &Utf8Path, b: &Utf8Path) -> bool {
    if a == b { return true; }
    let ca = a.canonicalize_utf8().ok();
    let cb = b.canonicalize_utf8().ok();
    match (ca, cb) {
        (Some(ca), Some(cb)) => ca == cb,
        _ => false,
    }
}

fn follow_rust_canonical_source(
    analyzer: &dyn LanguageAnalyzer,
    start: &Utf8Path,
    symbol: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Utf8PathBuf {
    let mut current = start.to_owned();
    let mut visited = std::collections::BTreeSet::from([current.clone()]);

    for _ in 0..10 {
        let analysis = match analyze_with_profile(analyzer, &current, AnalysisProfile::Structural) {
            Ok(analysis) => analysis,
            Err(e) => { /* push Degraded, return current */ return current; }
        };

        let has_declaration = analysis.symbols.iter().any(|entry| {
            entry.name == symbol
                && analysis.exports.iter()
                    .any(|export| export.name == symbol && !export.re_export)
        });
        if has_declaration { return current; }

        let Some(re_export_line) = analysis.exports.iter()
            .find(|export| export.name == symbol && export.re_export)
            .map(|export| export.line)
        else { return current; };

        let next = analysis.dependencies.iter().find_map(|dep| {
            (dep.line == re_export_line
                && dep.bindings.iter().any(|binding| binding_matches_symbol(binding, symbol)))
            .then(|| dependency_target_resolved(&dep.target).map(Utf8Path::to_owned))
            .flatten()
        });

        let Some(next) = next else { return current; };
        if !visited.insert(next.clone()) { return current; }
        current = next;
    }
    current
}

pub(crate) fn precision_label(precision: CapabilityPrecision) -> &'static str {
    match precision {
        CapabilityPrecision::Unsupported => "unsupported",
        CapabilityPrecision::Heuristic => "heuristic",
        CapabilityPrecision::Structural => "structural",
        CapabilityPrecision::Exact => "exact",
    }
}

pub(crate) fn language_label(language: LanguageId) -> &'static str {
    match language {
        LanguageId::TypeScript => "TypeScript/JavaScript",
        LanguageId::Rust => "Rust",
        LanguageId::Python => "Python",
        LanguageId::Go => "Go",
        LanguageId::Java => "Java",
    }
}
```

What changed vs. the old query system:

- Pre-0.7.0, each command (`search`, `inspect`, `dependents`, `refs`, `definition`) embedded its own TypeScript/Rust check ("is this a `.rs` file? then do X"). Phase 2+ pushes those checks into `analyzer.capabilities(language)` — a per-language record of what an analyzer can do at what precision, since a single fallback-tier analyzer instance (05-research-and-decisions.md §10) can own several languages with different ceilings.
- `require_precision_capability` is the single decision point. Below-`minimum` precision returns `false` (caller should skip) and pushes a `Skipped` diagnostic once per language; structural-but-not-Exact emits a `Warning` once per language so users see degraded precision once, not per file.
- `add_missing_analyzer_diagnostic` is the single source of the "X is not supported for {lang} files in this build" Skipped diagnostic.
- `follow_rust_canonical_source` walks Rust re-export chains (`pub use foo::Bar` → `foo.rs` → check if `Bar` is declared there; if not, follow the `pub use` line's dep target). Capped at 10 hops to bound cycles; falls back to the current file when the chain hits a non-re-export or a missing dependency.
- `dependency_target_resolved` is the single helper for "did the analyzer manage to resolve this dep target to a workspace file?" — handles both `DependencyTarget::File` and `DependencyTarget::RustPath`, returns `None` for `Namespace` (C# uses these but no resolver exists yet).

---

## 12. `source_scan.rs`

Shared source-file enumeration. Distinguishes "supported" (analyzer-recognized) from "candidate" (source-like file Monokl should account for in totals even when no analyzer ships for it).

```rust
#[derive(Debug, Clone, Default)]
pub struct SourceScanOptions {
    pub allow_tests: bool,
    pub no_gitignore: bool,
    pub languages: Option<Vec<LanguageId>>,
}

pub fn collect_supported_files(root: &Utf8Path, registry: &AnalyzerRegistry, opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
    expand_paths(&[root.to_owned()], registry, opts)
}

pub fn collect_supported_files_for_analyzer(root: &Utf8Path, analyzer: &dyn LanguageAnalyzer, opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
    expand_paths_for_analyzer(&[root.to_owned()], analyzer, opts)
}

pub fn collect_candidate_files(root: &Utf8Path, opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
    expand_candidate_paths(&[root.to_owned()], opts)
}

pub fn expand_paths(paths: &[Utf8PathBuf], registry: &AnalyzerRegistry, opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
    let languages = opts.languages.clone();
    expand_paths_inner(paths, opts, |path| {
        supports_requested_language(registry, path, languages.as_deref())
    })
}

pub fn expand_paths_for_analyzer(paths: &[Utf8PathBuf], analyzer: &dyn LanguageAnalyzer, opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
    expand_paths_inner(paths, opts, |path| analyzer.supports(path))
}

pub fn expand_candidate_paths(paths: &[Utf8PathBuf], opts: &SourceScanOptions) -> Result<Vec<Utf8PathBuf>> {
    expand_paths_inner(paths, opts, is_candidate_source_file)
}

fn expand_paths_inner<F>(paths: &[Utf8PathBuf], opts: &SourceScanOptions, supports: F) -> Result<Vec<Utf8PathBuf>>
where F: Fn(&Utf8Path) -> bool {
    use ignore::WalkBuilder;
    let scan_paths: Vec<Utf8PathBuf> = if paths.is_empty() { vec![Utf8PathBuf::from(".")] } else { paths.to_vec() };
    let mut files = Vec::new();

    for path in scan_paths {
        if path.is_dir() {
            let walk = WalkBuilder::new(path.as_std_path())
                .standard_filters(!opts.no_gitignore).build();
            for entry in walk {
                let entry = entry.map_err(|source| MonoklError::Walk { path: path.clone(), source })?;
                if !entry.file_type().is_some_and(|t| t.is_file()) { continue; }
                let Ok(file) = Utf8PathBuf::from_path_buf(entry.into_path()) else { continue; };
                if !opts.allow_tests && is_test_path(&file) { continue; }
                if supports(&file) { files.push(file); }
            }
        } else {
            if !opts.allow_tests && is_test_path(&path) { continue; }
            if supports(&path) { files.push(path); }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

pub fn is_test_path(path: &Utf8Path) -> bool {
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

fn supports_requested_language(registry: &AnalyzerRegistry, path: &Utf8Path, languages: Option<&[LanguageId]>) -> bool {
    match languages {
        Some(languages) => registry.supports_language(path, languages),
        None => registry.supports(path),
    }
}

pub fn candidate_language(path: &Utf8Path) -> Option<LanguageId> {
    match path.extension().unwrap_or("").to_ascii_lowercase().as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Some(LanguageId::TypeScript),
        "rs" => Some(LanguageId::Rust),
        "py" | "pyi" => Some(LanguageId::Python),
        "go" => Some(LanguageId::Go),
        "java" => Some(LanguageId::Java),
        _ => None,
    }
}

pub fn is_candidate_source_file(path: &Utf8Path) -> bool {
    candidate_language(path).is_some()
}
```

`is_candidate_source_file` lets workspace-wide commands like `classify` and `migration-surface` honestly count Python and C# files toward `total` while emitting `unsupported` diagnostics for them, rather than silently dropping them and overstating adoption.

---

## 13. `io_safety.rs`

User-facing read safety: 50 MB size cap, symlink rejection.

```rust
//! File I/O safety helpers.
//!
//! Bounds input sizes before reading to avoid allocator bombs when the caller
//! hands us `/dev/zero`, a checked-in multi-gigabyte minified bundle, or a
//! similar surprise. Also rejects symlinks at read time so a hostile fixture
//! (`src/data.ts` → `/etc/passwd`) can't exfiltrate via `mnkl extract`.

use camino::Utf8Path;
use io_errors::FileIoError;

use crate::error::{MonoklError, Result};

pub const MAX_INSPECTABLE_FILE_SIZE: u64 = 50 * 1024 * 1024;

pub fn read_to_string_capped(path: &Utf8Path) -> Result<String> {
    let meta = std::fs::symlink_metadata(path.as_std_path())
        .map_err(|e| MonoklError::Io(FileIoError::read(path, e)))?;

    if meta.file_type().is_symlink() {
        return Err(MonoklError::SymlinkRejected { path: path.to_owned() });
    }

    if meta.len() > MAX_INSPECTABLE_FILE_SIZE {
        return Err(MonoklError::FileTooLarge {
            path: path.to_owned(),
            size: meta.len(),
            cap: MAX_INSPECTABLE_FILE_SIZE,
        });
    }

    // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path
    std::fs::read_to_string(path.as_std_path())
        .map_err(|e| MonoklError::Io(FileIoError::read(path, e)))
}
```

Used at user-facing entry points (search, inspect, extract, refs, definition). Internal cache-loading paths trust their inputs and stay on plain `fs::read_to_string`. Pipeline-level root containment (`PathOutsideRoot`) is enforced separately for commands like `dependents`.

---

## 14. Updated CLI (`cli.rs`)

Full updated `Subcmd` enum and the global flags for presentation.

### Global flags

```rust
#[derive(Parser)]
#[command(name = "monokl", about = "AST-aware semantic code search for TypeScript/JavaScript and Rust.", version)]
pub struct Cli {
    /// Pretty-print JSON output. Only applies to JSON/minimal/schema output.
    #[arg(long, global = true, default_value_t = false)]
    pub pretty: bool,

    /// Emit command runtime to stderr as compact JSON.
    #[arg(long, global = true, default_value_t = false)]
    pub timings: bool,

    /// Output format. `auto` renders compact JSON when stdout is piped and
    /// uses command-specific human renderers on TTYs when available.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Auto)]
    pub format: OutputFormat,

    /// Presentation profile. `auto` resolves to `human` for TTY output and
    /// `agent` when stdout is piped.
    #[arg(long, global = true, value_enum, default_value_t = OutputProfile::Auto)]
    pub profile: OutputProfile,

    /// Verbosity of human-facing renderers. `auto` resolves from the selected
    /// profile: `compact` for agents, `standard` for humans.
    #[arg(long, global = true, value_enum, default_value_t = OutputDetail::Auto)]
    pub detail: OutputDetail,

    /// ANSI color policy for human-facing renderers.
    #[arg(long, global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,

    #[command(subcommand)]
    pub cmd: Subcmd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat { Auto, Json, Minimal, Text, Mermaid, Schema }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputProfile { Auto, Agent, Human }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputDetail { Auto, Compact, Standard, Full }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode { Auto, Always, Never }
```

### Kind filter enum

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum InspectKindFilter {
    ReactComponent, ReactHook, Utility, TypeModule, TestFile, StoryFile,
    ConfigModule, BarrelModule, ContextModule, PandaRecipe,
    RustStruct, RustEnum, RustTrait, RustModule, Unknown,
}

impl InspectKindFilter {
    #[must_use]
    pub fn as_kebab_str(self) -> &'static str { /* exhaustive match → kebab-case string */ }
}
```

`ValueEnum` so clap rejects typos at parse time instead of silently returning empty results (the prior `Option<String>` behavior).

### Full Subcmd enum

```rust
#[derive(Subcommand)]
pub enum Subcmd {
    Search {
        query: String,
        #[arg(long, default_value = ".")] path: Utf8PathBuf,
        #[arg(long)] max_results: Option<usize>,
        #[arg(long)] max_tokens: Option<usize>,
        #[arg(long)] max_bytes: Option<usize>,
        #[arg(long)] max_candidates: Option<usize>,
        #[arg(long, alias = "allow-tests")] include_tests: bool,
        #[arg(long)] no_gitignore: bool,
        #[arg(long)] exact: bool,
        #[arg(long)] case_sensitive: bool,
        #[arg(long, value_enum)] language: Option<Language>,
        #[arg(long, value_enum, default_value_t = QueryScope::FullRepo)] scope: QueryScope,
        #[arg(long)] base: Option<String>,
        #[arg(long)] head: Option<String>,
        #[arg(long, value_delimiter = ',', value_name = "FIELD[,FIELD]")] only: Vec<String>,
    },
    Symbols {
        files: Vec<Utf8PathBuf>,
        #[arg(long)] root: Option<Utf8PathBuf>,
        #[arg(long)] lite: bool,
        #[arg(long, value_enum, default_value_t = QueryScope::FullRepo)] scope: QueryScope,
        #[arg(long)] base: Option<String>,
        #[arg(long)] head: Option<String>,
    },
    Extract {
        file: Utf8PathBuf,
        #[arg(long)] line_start: Option<usize>,
        #[arg(long)] line_end: Option<usize>,
    },
    Dependents {
        file: Utf8PathBuf,
        #[arg(long)] root: Utf8PathBuf,
        #[arg(long, value_enum, default_value_t = QueryScope::FullRepo)] scope: QueryScope,
        #[arg(long)] base: Option<String>,
        #[arg(long)] head: Option<String>,
    },
    Deps {
        #[arg(required = true)] paths: Vec<Utf8PathBuf>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    DepsFrom {
        package: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    CountTokens {
        files: Vec<Utf8PathBuf>,
        #[arg(long)] stdin: bool,
    },
    Inspect {
        #[arg(required = true)] paths: Vec<Utf8PathBuf>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long, value_enum)] kind: Option<InspectKindFilter>,
        #[arg(long, value_delimiter = ',', value_name = "FIELD[,FIELD]")] only: Vec<String>,
        #[arg(long, value_delimiter = ',', value_name = "FIELD[,FIELD]")] fields: Vec<String>,
    },
    #[cfg(feature = "lang-ts")]
    Patterns {
        #[arg(default_value = ".")] dir: Utf8PathBuf,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long, value_delimiter = ',', value_name = "FIELD[,FIELD]")] only: Vec<String>,
    },
    #[cfg(feature = "lang-ts")]
    Tokens {
        #[arg(default_value = ".")] dir: Utf8PathBuf,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Definition {
        symbol: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long)] from_file: Option<Utf8PathBuf>,
        #[arg(long, value_enum, default_value_t = QueryScope::FullRepo)] scope: QueryScope,
        #[arg(long)] base: Option<String>,
        #[arg(long)] head: Option<String>,
    },
    Refs {
        symbol: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long)] include_tests: bool,
        #[arg(long, default_value_t = 500)] max_refs: usize,
        #[arg(long)] from_file: Option<Utf8PathBuf>,
        #[arg(long, value_enum, default_value_t = QueryScope::FullRepo)] scope: QueryScope,
        #[arg(long)] base: Option<String>,
        #[arg(long)] head: Option<String>,
        #[arg(long, value_delimiter = ',', value_name = "KIND[,KIND]")] kind: Vec<String>,
    },
    Classify {
        paths: Vec<Utf8PathBuf>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long, value_delimiter = ',', value_name = "PKG[,PKG]")] legacy: Vec<String>,
        #[arg(long, value_delimiter = ',', value_name = "PKG[,PKG]")] target: Vec<String>,
    },
    MigrationSurface {
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long, value_delimiter = ',', value_name = "PKG[,PKG]")] legacy: Vec<String>,
        #[arg(long, value_delimiter = ',', value_name = "PKG[,PKG]")] target: Vec<String>,
    },
    #[cfg(feature = "lang-ts")]
    Diff {
        #[arg(long)] base: String,
        #[arg(long, default_value = "HEAD")] head: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    #[cfg(feature = "lang-ts")]
    Explain {
        file: Utf8PathBuf,
        #[arg(long)] symbol: Option<String>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    #[cfg(feature = "lang-ts")]
    Coverage {
        file: Utf8PathBuf,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    #[cfg(feature = "lang-ts")]
    DataFlow {
        file: Utf8PathBuf,
        #[arg(long)] symbol: Option<String>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    #[cfg(feature = "lang-ts")]
    Similar {
        file: Utf8PathBuf,
        #[arg(long, default_value_t = 5)] count: usize,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Schema {
        #[arg(long)] kind: Option<String>,
    },
}

impl Subcmd {
    pub fn name(&self) -> &'static str { /* command name string for timing emission */ }
}
```

New Phase 2+ subcommands relative to Phase 1.5: `Deps`, `DepsFrom`, `Classify`, `MigrationSurface`.

New flags relative to Phase 1.5:

- Global: `--format`, `--profile`, `--detail`, `--color`, `--timings`.
- `Search`: `--scope`, `--base`, `--head`, `--language`, `--case-sensitive`, `--exact`, `--only`, `--include-tests` (alias for old `--allow-tests`).
- `Symbols`: `--root`, `--scope`, `--base`, `--head`.
- `Dependents`: `--scope`, `--base`, `--head`.
- `Inspect`: `--kind` as `ValueEnum` (rather than `Option<String>`), `--only`, `--fields`.
- `Patterns`: `--only`.
- `Refs`: `--scope`, `--base`, `--head`, `--kind`.
- `Definition`: `--scope`, `--base`, `--head`.

---

## 15. Updated types (`types.rs`)

All new or changed types. Every struct/enum with exact serde attributes.

### `RustData` — added

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect_entry: Option<Box<InspectEntry>>,
}
```

`LangData::Rust(RustData)` is how the analyzer threads its inspect-entry result through the shared `FileAnalysis.lang` slot without inflating non-Rust language data.

### `LangData` — non-exhaustive, language-keyed

`Go`/`Java` replace an earlier `CSharp` variant, matching the dropped-C#/added-Go-and-Java roadmap update (`05-research-and-decisions.md` §6).

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "language", content = "data")]
#[non_exhaustive]
pub enum LangData {
    #[serde(rename = "typescript")] Ts(TsData),
    #[serde(rename = "rust")] Rust(RustData),
    #[serde(rename = "python")] Python(PythonData),
    #[serde(rename = "go")] Go(GoData),
    #[serde(rename = "java")] Java(JavaData),
}

impl LangData {
    pub fn ts(&self) -> Option<&TsData> { if let LangData::Ts(ts) = self { Some(ts) } else { None } }
    pub fn jsx_elements(&self) -> &[JsxElementEntry] {
        self.ts().map_or(&[], |ts| ts.jsx_elements.as_slice())
    }
}
```

### `RustPathAnchor` — added

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RustPathAnchor {
    Crate,
    Super,
    #[serde(rename = "self")] Selff,
    Extern(String),
}
```

### `DependencyTarget` — `RustPath` variant added

```rust
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

/// Defined in Part 1. `Ambiguous` retains every candidate; `External` is distinct from a failed
/// workspace lookup, and `Unresolved` carries the analyzer diagnostic that explains the failure.
```

### `Language` — added `Rust` and `Python`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[cfg_attr(feature = "cli", value(name = "typescript"))] TypeScript,
    #[cfg_attr(feature = "cli", value(name = "javascript"))] JavaScript,
    #[cfg_attr(feature = "cli", value(name = "rust"))] Rust,
    #[cfg_attr(feature = "cli", value(name = "python"))] Python,
}
```

### `QueryScope` + `QueryScopeOptions` — new

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "kebab-case")]
pub enum QueryScope {
    FullRepo,
    ChangedFiles,
    ChangedLines,
    ImpactedNeighbors,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryScopeOptions {
    pub mode: QueryScope,
    pub base: Option<String>,
    pub head: Option<String>,
}

impl Default for QueryScopeOptions {
    fn default() -> Self {
        Self { mode: QueryScope::FullRepo, base: None, head: None }
    }
}
```

### `SearchOptions` — gained `case_sensitive`, `language`, `scope`

```rust
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub query: String,
    pub path: Utf8PathBuf,
    pub allow_tests: bool,
    pub no_gitignore: bool,
    pub limits: SearchLimits,
    pub exact: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    pub language: Option<Language>,
    #[serde(default)]
    pub scope: QueryScopeOptions,
}
```

### Inspect kinds — Rust variants added

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum InspectEntry {
    ReactComponent(ReactComponentEntry),
    ReactHook(ReactHookEntry),
    Utility(UtilityEntry),
    TypeModule(TypeModuleEntry),
    TestFile(TestFileEntry),
    StoryFile(StoryFileEntry),
    ConfigModule(ConfigModuleEntry),
    BarrelModule(BarrelModuleEntry),
    ContextModule(ContextModuleEntry),
    #[cfg(feature = "lang-ts")]
    PandaRecipe(crate::panda::PandaRecipeEntry),
    RustStruct(RustStructEntry),
    RustEnum(RustEnumEntry),
    RustTrait(RustTraitEntry),
    RustModule(RustModuleEntry),
    Unknown(UnknownEntry),
}

impl InspectEntry {
    #[must_use]
    pub fn kind(&self) -> &'static str { /* exhaustive match → kebab-case discriminator string */ }
}
```

### Rust inspect entry types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustStructEntry {
    pub file: Utf8PathBuf,
    pub name: String,
    pub visibility: Visibility,
    pub fields: Vec<RustField>,
    pub derives: Vec<String>,
    pub trait_impls: Vec<String>,
    pub methods: Vec<RustMethod>,
    #[serde(skip_serializing_if = "Option::is_none")] pub error_type: Option<String>,
    pub cfg_features: Vec<String>,
    pub deps: RustImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustField {
    pub name: String,
    pub type_str: String,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustMethod {
    pub name: String,
    pub sig: String,
    pub is_async: bool,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustEnumEntry {
    pub file: Utf8PathBuf,
    pub name: String,
    pub visibility: Visibility,
    pub derives: Vec<String>,
    pub variants: Vec<RustVariant>,
    pub deps: RustImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustVariant {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustTraitEntry {
    pub file: Utf8PathBuf,
    pub name: String,
    pub visibility: Visibility,
    pub required_methods: Vec<RustMethod>,
    pub provided_methods: Vec<RustMethod>,
    pub deps: RustImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustModuleEntry {
    pub file: Utf8PathBuf,
    pub re_exports: Vec<String>,
    pub deps: RustImports,
    pub loc: usize,
}
```

### `RustImports`, `RustTestingImports`

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustImports {
    pub std: Vec<String>,
    pub workspace: Vec<String>,
    pub external: Vec<String>,
    pub local: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub async_runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub web_framework: Option<String>,
    pub serialization: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub error_handling: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub database: Option<String>,
    pub testing: RustTestingImports,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustTestingImports {
    pub unit: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub benchmarks: Option<String>,
}
```

### Focused dependency query types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileDependencySummary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub framework: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub state: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub data_fetching: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub design_system: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub test_utilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub local: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub external: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub rust_std: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub rust_workspace: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub rust_local: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub rust_external: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepsEntry {
    pub file: Utf8PathBuf,
    pub kind: String,
    pub deps: FileDependencySummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepsResult {
    pub entries: Vec<DepsEntry>,
    pub file_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepsFromEntry {
    pub file: Utf8PathBuf,
    pub bindings: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepsFromResult {
    pub package: String,
    pub entries: Vec<DepsFromEntry>,
    pub scanned_count: usize,
    pub match_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationEntry {
    pub file: Utf8PathBuf,
    pub status: String,    // legacy, target, mixed, no-ds, unsupported
    pub score: f32,
    pub legacy: Vec<String>,
    pub target: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResult {
    pub entries: Vec<ClassificationEntry>,
    pub file_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSurfaceSummary {
    pub total: usize,
    pub legacy_files: usize,
    pub target_files: usize,
    pub mixed_files: usize,
    pub no_ds_files: usize,
    pub unsupported_files: usize,
    pub adoption_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSurfaceResult {
    pub summary: MigrationSurfaceSummary,
    pub files: Vec<ClassificationEntry>,
    pub diagnostics: Vec<Diagnostic>,
}
```

### `InspectOptions` — added

```rust
#[derive(Debug, Clone)]
pub struct InspectOptions {
    pub workspace: WorkspaceOptions,
    pub kind_filter: Option<String>,
}

impl InspectOptions {
    pub fn new(workspace: WorkspaceOptions) -> Self {
        Self { workspace, kind_filter: None }
    }

    #[must_use]
    pub fn with_kind_filter(mut self, kind: impl Into<String>) -> Self {
        self.kind_filter = Some(kind.into());
        self
    }
}
```

### `ExtractResult` — wraps blocks with diagnostics

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractResult {
    pub blocks: Vec<CodeBlock>,
    pub diagnostics: Vec<Diagnostic>,
}
```

Before, `extract` returned a bare `Vec<CodeBlock>`; now wrapping in `ExtractResult` lets unsupported-source-file Skipped diagnostics flow through without changing the call signature meaning.

### `SymbolKind` — added Rust kinds

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Function, Method, Constructor, Class, Struct, Enum, Interface,
    TypeAlias, Property, Field, Variable, Module,
    // Rust-specific
    Impl, Macro,
    Other,
}
```

### `SymbolEntry` — gained `owner`, `trait_impl`, `visibility`, `kind_detail`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub trait_impl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub visibility: Option<Visibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub kind_detail: Option<String>,
}
```

### Patterns extensions

```rust
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PatternsResult {
    pub file_count: usize,
    pub component_count: usize,
    pub hook_count: usize,
    pub utility_count: usize,
    pub test_file_count: usize,
    pub story_count: usize,
    pub barrel_count: usize,
    pub config_count: usize,
    pub context_module_count: usize,
    pub hoc_count: usize,
    pub lazy_count: usize,
    pub zod_usage: usize,
    pub hooks_extended: BTreeMap<String, usize>,
    pub styling: BTreeMap<String, usize>,
    pub patterns: BTreeMap<String, usize>,
    pub hooks_top: BTreeMap<String, usize>,
    pub props_style: BTreeMap<String, usize>,
    pub export_style: BTreeMap<String, usize>,
    pub test_coverage: TestCoverage,
    pub state_management: Vec<String>,
    pub data_fetching: Vec<String>,
    pub toolchain: ToolchainImports,
    #[cfg(feature = "lang-ts")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panda: Option<crate::panda::PandaPatterns>,
    pub diagnostics: Vec<Diagnostic>,
}
```

`story_count`, `barrel_count`, `config_count`, `context_module_count`, `hoc_count`, `lazy_count`, `zod_usage`, `hooks_extended`, `toolchain`, `panda` are all Phase 2+ additions.

---

## 16. `output.rs` — updated rendering

CLI presentation policy and renderer dispatch. JSON is canonical; text/Mermaid are layered atop without changing retrieval semantics.

Key types and resolution:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedProfile { Agent, Human }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedFormat { Json, Minimal, Text, Mermaid, Schema }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presentation {
    pub profile: ResolvedProfile,
    pub detail: OutputDetail,
    pub format: ResolvedFormat,
    pub use_color: bool,
    pub pretty_json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind { Structured, RankedList, Summary, Symbols, Graph }

pub fn resolve_presentation(
    format: OutputFormat,
    profile: OutputProfile,
    detail: OutputDetail,
    color: ColorMode,
    pretty: bool,
    kind: RenderKind,
) -> Presentation {
    let stdout_is_tty = std::io::stdout().is_terminal();
    let profile = match profile {
        OutputProfile::Auto => if stdout_is_tty { ResolvedProfile::Human } else { ResolvedProfile::Agent },
        OutputProfile::Agent => ResolvedProfile::Agent,
        OutputProfile::Human => ResolvedProfile::Human,
    };
    let detail = match detail {
        OutputDetail::Auto => match profile {
            ResolvedProfile::Agent => OutputDetail::Compact,
            ResolvedProfile::Human => OutputDetail::Standard,
        },
        other => other,
    };
    let use_color = match color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => stdout_is_tty,
    };
    let format = match format {
        OutputFormat::Auto => if stdout_is_tty {
            match kind {
                RenderKind::Graph => ResolvedFormat::Mermaid,
                RenderKind::RankedList | RenderKind::Summary | RenderKind::Symbols => ResolvedFormat::Text,
                RenderKind::Structured => ResolvedFormat::Json,
            }
        } else { ResolvedFormat::Json },
        OutputFormat::Json => ResolvedFormat::Json,
        OutputFormat::Minimal => ResolvedFormat::Minimal,
        OutputFormat::Text => ResolvedFormat::Text,
        OutputFormat::Mermaid => ResolvedFormat::Mermaid,
        OutputFormat::Schema => ResolvedFormat::Schema,
    };

    Presentation {
        profile, detail, format, use_color,
        pretty_json: pretty || (stdout_is_tty && matches!(profile, ResolvedProfile::Human)),
    }
}
```

Per-command human renderers added:

- `render_search_text` — ranked list with `1. file:line-line  score 0.92` lines and color-coded scores (green ≥0.9, yellow ≥0.7).
- `render_symbols_text` — per-file symbol bullet list.
- `render_refs_text` — `file:line  refKind  confidence 0.95` lines.
- `render_definition_text` (using `render_definition_site` per site) — file/line + signature + (in Full detail) re-export chain.
- `render_dependents_text` — sectioned dependents/imports list with truncation marker.
- `render_dependents_mermaid` — `graph LR` with `target` node and `dep0 → target`/`target → imp0` edges. `prefer_dependents_mermaid` returns `true` only when total nodes ≤ 12 and edges ≤ 20 (auto-mermaid avoids unreadable spam).
- `render_inspect_text`, `render_deps_text`, `render_deps_from_text`, `render_classify_text`, `render_migration_surface_text`.
- `render_diff_text`, `render_explain_text`, `render_coverage_text` (TS-only).
- `render_diagnostics` shared helper renders the diagnostic block with `kind` label (degraded/skipped/warning) bolded and path colorized.

Helpers:

- `human_heading`, `human_label`, `human_path`, `human_muted`, `human_score`, `human_percent` — owo-colors driven, all gated on `presentation.use_color`.
- `human_score` colors `>= 0.9` green, `>= 0.7` yellow, otherwise plain.
- `dependency_summary_lines` formats `FileDependencySummary` as `"framework: react, react-dom"`, etc.

JSON helpers:

- `render_json` / `render_json_compact` / `render_json_output(value, pretty)` — pretty vs. compact branch.
- `fatal(message)` and `render_error(err)` — error exit paths.

---

## 17. Updated node kinds (`analysis/node_kind.rs`)

The file is now feature-gated. The TS-mapping submodule lives inside `mod ts` and re-exports `node_kind_for_statement`/`node_kind_for_declaration` only when `lang-ts` is enabled:

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

The Rust analyzer does _not_ go through this module — it produces `SymbolKind` directly from `ra_ap_syntax::ast::Item` (function → `SymbolKind::Function`, struct → `Struct`, enum → `Enum`, trait → `Interface`, impl → `Impl`, module → `Module`, const → `Variable`, type alias → `TypeAlias`, macro_rules → `Macro`; associated `fn` items inside impl blocks → `Method`).

`SymbolEntry.kind_detail` carries the Rust-specific stable strings: `"rust-struct"`, `"rust-enum"`, `"rust-trait"`, `"rust-function"`, `"rust-type"`, `"rust-const"`, `"rust-module"`. These are kebab-case discriminators that match `InspectEntry::kind()` exactly for the entry-level variants, while `kind_detail` ties symbols-level (top-level item) entries back to the parent type semantic.

`InspectEntry::kind()` returns the kebab-case discriminator: `"react-component"`, `"react-hook"`, `"utility"`, `"type-module"`, `"test-file"`, `"story-file"`, `"config-module"`, `"barrel-module"`, `"context-module"`, `"panda-recipe"`, `"rust-struct"`, `"rust-enum"`, `"rust-trait"`, `"rust-module"`, `"unknown"`.

---

## 18. Agent tool contracts

The agent-facing contracts (post commit `6b41f07`) consist of:

1. **JSON as canonical output**. CLI emits compact JSON to stdout by default when piped; pretty-printed only when `--pretty` is set or human profile is resolved. Stderr is empty on success.
2. **Field projection** — `--only`/`--fields` are JSON-only:
   - `inspect --only kind,deps.external` projects into `entries.kind` and `entries.deps.external` and force-includes `entries.file`, `fileCount`, `diagnostics`.
   - `search --only file,score,snippet` projects into `results.file`, `results.finalScore`, `results.code`; aliases `snippet`→`code` and `score`→`finalScore`. Auto-includes `truncated`, `totalTokens`, `diagnostics`.
   - `patterns --only componentCount,diagnostics` projects top-level fields.
   - `refs --kind import,jsx-element` filters by `RefKind`.
3. **Global `--format` modes**:
   - `json` (default canonical) — compact JSON.
   - `minimal` — prune null, false, 0, empty strings/arrays/objects (`projection::prune_empty`).
   - `text` — command-specific human renderer.
   - `mermaid` — graph source (currently `dependents` only).
   - `schema` — emit the JSON Schema for the command's result type via `schema::inspect_schema`.
   - `auto` — choose by command shape + stdout TTY status.
4. **Capability-driven diagnostics**:
   - Unsupported source-like file: `Diagnostic { kind: Skipped, message: "{op} is not supported for {lang} files in this build" }`.
   - Below-structural-precision analyzer: `Skipped` with `"requires {min} precision, analyzer exposes {actual}"`.
   - Below-Exact-precision analyzer: `Warning` with `"{op} uses {actual}-precision {lang} analysis"`.
   - Empty query: `Warning` with `"zero terms"`.
   - PR-aware scope: `Warning` with `"scope {mode} limited {op} to ... using merge-base(base, head)=…..head"`.
5. **`source_files` vs. `candidate_source_files`** library helpers — agents can ask for "files the current build can actually analyze" vs. "files Monokl should report on even if unsupported".
6. **Snapshot vs. PR-aware scopes** — `diff` explicitly stays on `ChangeSetMode::Snapshot`; review-oriented scopes use `ChangeSetMode::PrAware` with `merge-base(base, head)..head`.
7. **Truncation markers** — search has `truncationMarker` / `truncated`; symbols has `truncationMarker`; dependents/imports both capped at 200 with one combined marker.
8. **`--scope` flags** on `search`, `symbols`, `dependents`, `refs`, `definition` with `--base` / `--head` overrides defaulting to `origin/main`..`HEAD`.

---

## 19. Updated integration tests

### `tests/integration_new_commands.rs`

CLI binary integration via `CARGO_BIN_EXE_loupe`. Gated `#![cfg(feature = "cli")]`. New cases relative to Phase 1:

- `refs_command_returns_json` — verifies `RefsResult` shape and at least one `import` ref.
- `tokens_command_returns_json` — `TokensResult` shape, hardcoded violation floor.
- `definition_command_returns_json` — canonical (`isReExport == false`) definition floor.
- `diff_command_returns_json` — `HEAD..HEAD` is 0 files changed.
- `schema_*` family: `schema_refs_result`, `schema_diff_result`, `schema_tokens_result`, `schema_definition_result`, `schema_panda_recipe`, `schema_context_module`, `schema_agent_command_results` (covers `search-result`, `symbols-result`, `dependents-result`, `deps-result`, `deps-from-result`, `classify-result`, `migration-surface-result`, `count-tokens-result`), `schema_extract_result_is_array_contract`.
- `inspect_only_projects_entry_fields` — `--only kind` retains `kind` and `file`, drops `props`.
- `inspect_only_projects_nested_fields` — `--only deps.external` retains nested path, drops sibling buckets.
- Mixed-workspace tests (`mixed_workspace()` fixture has both `widget.ts` and `worker.py`):
  - `deps_from_mixed_workspace_reports_unsupported_files` — scanned=2, match=1, diagnostic mentions Python.
  - `classify_mixed_workspace_surfaces_unsupported_entry` — both legacy and unsupported entries present.
  - `migration_surface_counts_unsupported_files` — total=2, unsupportedFiles=1, legacyFiles=1.
  - `inspect_mixed_workspace_reports_unsupported_python_file` — Python file produces Unknown entry + Skipped diagnostic.
  - `refs_from_file_reports_unsupported_narrowing_file` — `--from-file worker.py` returns 0 refs + diagnostic.
  - `definition_from_file_reports_unsupported_narrowing_file` — same pattern for `definition`.
  - `dependents_reports_unsupported_source_file` — Python file as dependents target reports 0/0 + Skipped diagnostic.
- Focused dependency-command tests:
  - `deps_command_returns_compact_dependency_summary` — `entries[0].deps.framework` contains `react`.
  - `deps_from_command_finds_package_importers` — `react` matches Button.tsx and useCart.ts.
  - `classify_command_reports_migration_status` — `--legacy react --target @design-system/web` produces legacy entries.
  - `migration_surface_summarizes_classification` — `legacyFiles >= 1`.
  - `refs_kind_filters_result_sites` — `--kind import` returns only import refs.
  - `patterns_only_projects_top_level_fields` — `--only componentCount,diagnostics` drops `styling`.
  - `search_only_projects_result_fields_with_aliases` — `--only file,score` projects `finalScore`, drops `code`.
  - `symbols_root_scans_workspace` — `--root <dir>` scans without explicit file args.
  - `format_minimal_prunes_empty_values` — `--format minimal --only kind` drops empty diagnostics.
  - `format_schema_returns_command_schema_without_running_scan` — `--format schema deps-from react …` returns `{"title": "DepsFromResult", ...}`.
- `schema_keys_match_serialized_instance` — fetches a real `inspect` output for `Button.tsx` and a `schema --kind react-component` schema, then verifies every required schema key is in the serialized instance and every serialized key is in the schema's `properties`. Catches schema drift.

### `tests/integration_search.rs`

Library-level (`monokl::pipeline`) integration. New helper `search_opts(query)` defaults to `allow_tests: true, no_gitignore: true` because fixtures live under `/tests/fixtures/`. Cases:

- `search_finds_function_by_name` — `validatePrice` floor.
- `search_finds_multiple_symbols` — `formatCurrency` should hit `format.ts`.
- `search_source_files_found_when_allow_tests_true` — note explains that the fixture's `format.test.ts` uses Jest globals so it produces no semantic blocks regardless of `allow_tests`.
- `search_result_has_correct_structure` — rank/score/code/line invariants.
- `search_respects_max_results_limit` — `max_results: Some(2)` floor.
- `search_empty_query_returns_empty` — empty query emits a `Warning` diagnostic containing `"zero terms"`.
- `search_excluded_term_filters_results` — `useCart -addItem` excludes `useCart.ts`.
- `search_finds_interface_in_results` — `ValidationResult` interface present.

### `tests/integration_cli_output.rs`

CLI stdout/stderr contract tests. New cases relative to Phase 1:

- `search_unsupported_language_emits_capability_diagnostic` — `--language python` exits 0 with empty results + Skipped diagnostic about Python.
- `extract_unsupported_source_file_emits_diagnostic_in_json` — extract on `worker.py` returns 0 blocks + Skipped diagnostic.
- Format coverage:
  - `search_text_format_emits_human_readable_output` — `--format text` starts with "search" heading.
  - `dependents_mermaid_format_emits_graph_source` — `--format mermaid` starts with `graph LR`.
  - `inspect_text_format_emits_human_readable_output` — `--format text inspect` starts with "inspect" + contains "react-component".
  - `deps_from_text_format_emits_human_readable_output` — heading + package name.
  - `migration_surface_text_output_includes_unsupported_diagnostics` — `unsupported=1` in text + "no analyzer registered for Python files in this build".
  - `search_mermaid_format_exits_nonzero` — explicit failure for unsupported renderer combinations.
  - `inspect_only_with_text_format_exits_nonzero` — projection stays JSON-only even when a text renderer exists.

### Pipeline-internal tests (`pipeline.rs`)

The orchestrator file also has 20+ inline tests including new git-scope ones:

- `search_changed_files_scope_limits_results_to_changed_paths` — `QueryScope::ChangedFiles` with `HEAD~1..HEAD` excludes `stable.ts`, emits "scope changed-files limited search" diagnostic.
- `search_impacted_neighbors_scope_includes_direct_graph_neighbors` — `ImpactedNeighbors` includes the changed file plus one-hop neighbors.
- `search_changed_lines_scope_limits_results_to_changed_lines` — verifies blocks overlap line 2 only.
- `symbols_with_options_omits_import_classification_for_rust_files` — Rust file present in symbols, absent from imports, with aggregated Warning about omitted classification.
- `symbols_with_options_scoped_includes_changed_files_and_neighbors` — `ImpactedNeighbors` includes both changed and neighbor TS files.
- `symbols_with_options_scoped_changed_lines_filters_symbols_in_changed_file` — only changed-line symbols survive.
- `search_supports_rust_files` — `lang:rs` filter returns Rust struct blocks.
- `extract_rust_file_returns_structural_blocks` — both `Struct` and `Impl`/`Method` blocks present.
- `inspect_rust_file_uses_shared_analyzer_entry` — `kind_filter: "rust-struct"` returns `RustStruct` variant.

---

## 20. Benchmark numbers

Real `criterion` numbers from the CHANGELOG and project memory:

- **`inspect`**: 5.8 ms → 2.8 ms per file (≈ 2.1× speedup). Driven by reusing the shared `WorkspaceSession` analyzer and the DashMap hot-path cache, plus avoiding re-parsing when `AnalysisProfile::Structural` finds an existing cached `FileAnalysis`.
- **`patterns`**: 65 ms → 15 ms per workspace (≈ 4.3× speedup). Pattern aggregation reuses the inspect pipeline's per-file results rather than re-walking the workspace, and the shared session cache means tsconfig discovery / analyzer construction happen once.
- The benchmark harness is `[[bench]] name = "pipeline" harness = false` in `Cargo.toml`, gated on `criterion`.

These numbers reflect:

- Single-shot CLI invocations (the previous Phase 1 baseline) being optimized for repeat library calls via `WorkspaceSession` caching.
- The `AnalysisProfile` triage (`Dependencies` skips symbols/exports, `Structural` skips inspect-entry, `Full` is the original behavior) lets focused commands like `deps`/`deps-from`/`classify` ask only for what they need.
- Persistence layer hits on warm runs: `persist::lookup` short-circuits both mtime-stable and content-hash-stable paths before the analyzer even reads the file.

---

result: Produced comprehensive Phase 2+ spec for Monokl covering the 8-PR delta (versions 0.4.0 → 0.7.0): analyzer registry + Rust support via ra-ap-syntax, pipeline modularization, PR-aware git scopes, capability-driven query policy, presentation layer (text/mermaid/projection), DashMap cache split from disk persist layer, io_safety floor, and refreshed CLI + types + integration tests.

---
