# Inspection & Analysis Commands (v0.1.0 → v0.4.0)

`inspect`, `patterns`, `schema`, `refs`, `definition`, `diff`, `tokens`, `explain`, `coverage`, `data-flow`, `similar` — the `InspectEntry` classifier, Panda CSS recipe analysis, and the first-draft Rust/Python inspectors that Part 3 later replaces with real analyzers.

Part of the [monokl spec](./README.md). Builds on [01-core-architecture.md](./01-core-architecture.md).

---

# Part 2: Phase 1+ Additions

> Added since original spec. Commits: inspect/patterns/schema/refs/definition/diff/tokens/explain/coverage/data-flow/similar + Panda CSS + multi-language stubs.

## Overview of additions

Since the original spec (which covered `search`, `symbols`, `extract`, `dependents`, `count-tokens`, plus pipeline / cache / rank / query / analysis), the monokl crate has been substantially expanded with new commands, a discriminated-union `InspectEntry` model with Rust support, a hand-written JSON Schema generator, and several synthesizing analyses.

### New commands

- `mnkl inspect` — classify a file into a typed `InspectEntry`
- `mnkl patterns` — workspace-level frequency analysis
- `mnkl schema` — emit JSON Schema for inspect/result types
- `mnkl refs` — symbol-level occurrence/reference finder with capture provenance
- `mnkl definition` — canonical declaration finder with re-export chain
- `mnkl diff` — structural git-ref delta
- `mnkl tokens` — design-token discipline audit
- `mnkl explain` — synthesize inspect + refs + dependents
- `mnkl coverage` — exported-symbol coverage report
- `mnkl data-flow` — input / state / output / consumer trace
- `mnkl similar` — six-dimension similarity ranking

### New modules

- `src/inspection.rs` — classifier producing `InspectEntry`
- `src/panda.rs` — Panda CSS recipe analyzer
- `src/schema.rs` — JSON Schema generator
- `src/refs.rs` — symbol occurrence/reference finder
- `src/definition.rs` — declaration finder + re-export chain walker
- `src/diff.rs` — git-driven structural diff
- `src/tokens_analysis.rs` — token compliance audit
- `src/explain.rs` — multi-source synthesizer
- `src/coverage.rs` — exported-symbol test coverage
- `src/data_flow.rs` — data-flow tracer
- `src/similar.rs` — structural similarity ranker
- `src/import_classifier.rs` — semantic import bucketing
- `src/rust_inspection.rs` — regex-based Rust file inspector
- `src/python_inspection.rs` — regex-based Python file inspector (references `PythonClassEntry`, `PythonFunctionEntry`, `PythonModuleEntry`, `PythonClassKind`, `PythonFunctionKind`, `PythonModelStyle`, `PythonField`, `PythonImports`, `PythonMethod` — types not currently in `types.rs`; this module is wired into `lib.rs` but its types are not yet defined)

### New types added in `types.rs`

`InspectEntry` (discriminated union), `ReactComponentEntry`, `PropField`, `ComponentPatterns`, `ExportStyle`, `ReactHookEntry`, `UtilityEntry`, `FunctionSig`, `TypeModuleEntry`, `TypeSig`, `TestFileEntry`, `UnknownEntry`, `StoryFileEntry`, `ConfigModuleEntry`, `BarrelModuleEntry`, `ContextModuleEntry`, `UsageSignals`, `InspectResult`, `PatternsResult`, `TestCoverage`, `RustStructEntry`, `RustField`, `RustMethod`, `RustEnumEntry`, `RustVariant`, `RustTraitEntry`, `RustModuleEntry`, `RustImports`, `RustTestingImports`, `ClassifiedImports`, `StylingImports`, `ToolchainImports`, `DocumentationSignals`, `Confidence`, `Diagnostic`, `DiagnosticKind`, plus new `SymbolKind` variants (`Impl`, `Macro`), `Visibility`, and `LangData`.

## Updated CLI surface

The complete `Subcmd` enum from `cli.rs`:

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
        #[arg(long)] allow_tests: bool,
        #[arg(long)] no_gitignore: bool,
        #[arg(long)] exact: bool,
        #[arg(long, value_enum)] language: Option<Language>,
    },
    Symbols {
        #[arg(required = true)] files: Vec<Utf8PathBuf>,
        #[arg(long)] lite: bool,
    },
    Extract {
        file: Utf8PathBuf,
        #[arg(long)] line_start: Option<usize>,
        #[arg(long)] line_end: Option<usize>,
    },
    Dependents {
        file: Utf8PathBuf,
        #[arg(long)] root: Utf8PathBuf,
    },
    CountTokens {
        files: Vec<Utf8PathBuf>,
        #[arg(long)] stdin: bool,
    },
    Inspect {
        #[arg(required = true)] paths: Vec<Utf8PathBuf>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long)] kind: Option<String>,
    },
    Patterns {
        #[arg(default_value = ".")] dir: Utf8PathBuf,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Tokens {
        #[arg(default_value = ".")] dir: Utf8PathBuf,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Definition {
        symbol: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long)] from_file: Option<Utf8PathBuf>,
    },
    Refs {
        symbol: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
        #[arg(long)] include_tests: bool,
        #[arg(long, default_value_t = 500)] max_refs: usize,
        #[arg(long)] from_file: Option<Utf8PathBuf>,
    },
    Diff {
        #[arg(long)] base: String,
        #[arg(long, default_value = "HEAD")] head: String,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Explain {
        file: Utf8PathBuf,
        #[arg(long)] symbol: Option<String>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Coverage {
        file: Utf8PathBuf,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    DataFlow {
        file: Utf8PathBuf,
        #[arg(long)] symbol: Option<String>,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Similar {
        file: Utf8PathBuf,
        #[arg(long, default_value_t = 5)] count: usize,
        #[arg(long, default_value = ".")] root: Utf8PathBuf,
    },
    Schema {
        #[arg(long)] kind: Option<String>,
    },
}
```

Global flag (set in `Cli`): `--pretty` (auto-enabled on terminal).

Note: `search` keeps `--path` for historical reasons; all other commands use `--root`.

## New types (types.rs additions)

```rust
/// Visibility modifier on a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    Public,
    Crate,
    Module,
    Private,
}

/// Confidence score for an inferred result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Confidence {
    pub score: f32,
    pub evidence: Vec<String>,
    pub caveats: Vec<String>,
}

/// A non-fatal warning or error from the monokl pipeline.
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

/// Classified breakdown of a file's import statements.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifiedImports {
    pub framework: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub styling: Option<StylingImports>,
    pub state: Vec<String>,
    pub data_fetching: Vec<String>,
    pub design_system: Vec<String>,
    pub test_utilities: Vec<String>,
    pub observability: Vec<String>,
    pub local: Vec<String>,
    pub external: Vec<String>,
    pub toolchain: ToolchainImports,
    pub documentation: DocumentationSignals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StylingImports {
    pub approach: String,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainImports {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DocumentationSignals {
    pub has_stories: bool,
    pub has_jsdoc: bool,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactComponentEntry {
    pub file: Utf8PathBuf,
    pub name: String,
    pub props: Vec<PropField>,
    pub hooks: Vec<String>,
    pub patterns: ComponentPatterns,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub styling: Option<StylingImports>,
    pub exports: ExportStyle,
    pub deps: ClassifiedImports,
    pub loc: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signals: Option<UsageSignals>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipe_refs: Vec<String>,
    #[serde(default)]
    pub is_hoc: bool,
    #[serde(default)]
    pub is_lazy: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zod_schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropField {
    pub name: String,
    pub type_str: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPatterns {
    pub forward_ref: bool,
    pub memo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub lazy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportStyle {
    Named,
    Default,
    Both,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactHookEntry {
    pub file: Utf8PathBuf,
    pub name: String,
    pub params: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    pub hooks_used: Vec<String>,
    pub has_side_effects: bool,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UtilityEntry {
    pub file: Utf8PathBuf,
    pub functions: Vec<FunctionSig>,
    pub deps: ClassifiedImports,
    pub loc: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zod_schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionSig {
    pub name: String,
    pub params: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    pub is_async: bool,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeModuleEntry {
    pub file: Utf8PathBuf,
    pub types: Vec<TypeSig>,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeSig {
    pub name: String,
    pub kind: String, // "interface" | "type" | "enum" | "class"
    pub fields: Vec<PropField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestFileEntry {
    pub file: Utf8PathBuf,
    pub test_framework: String,
    pub test_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_file: Option<Utf8PathBuf>,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownEntry {
    pub file: Utf8PathBuf,
    pub symbols: Vec<SymbolEntry>,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryFileEntry {
    pub file: Utf8PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub stories: Vec<String>,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigModuleEntry {
    pub file: Utf8PathBuf,
    pub config_kind: String,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarrelModuleEntry {
    pub file: Utf8PathBuf,
    pub exports: Vec<String>,
    pub source_file_count: usize,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextModuleEntry {
    pub file: Utf8PathBuf,
    pub context_names: Vec<String>,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageSignals {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_coverage: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engineer_confusion: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_velocity: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectResult {
    pub entries: Vec<InspectEntry>,
    pub file_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}

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

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TestCoverage {
    pub has_story: usize,
    pub has_test: usize,
    pub neither: usize,
}

// Rust inspect types
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustImports {
    pub std: Vec<String>,
    pub workspace: Vec<String>,
    pub external: Vec<String>,
    pub local: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_framework: Option<String>,
    pub serialization: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_handling: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub testing: RustTestingImports,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustTestingImports {
    pub unit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benchmarks: Option<String>,
}
```

## `mnkl inspect` — Component inspection

### Purpose

Classify each input TypeScript/JavaScript (or Rust) file as one of: `react-component`, `react-hook`, `utility`, `type-module`, `test-file`, `story-file`, `config-module`, `barrel-module`, `context-module`, `panda-recipe`, `rust-struct`, `rust-enum`, `rust-trait`, `rust-module`, or `unknown`. Emits structured details for each.

### InspectResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectResult {
    pub entries: Vec<InspectEntry>,
    pub file_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}
```

### inspection.rs — algorithm verbatim

Classification priority order (from `classify_analysis`):

1. **StoryFile** — detected before TestFile/PandaRecipe. Detected by `*.stories.*` filename OR `deps.documentation.has_stories` OR a `@storybook/*` dependency.
2. **ConfigModule** — `*.config.{ts|js|tsx|jsx|mts|cts|mjs|cjs}` filename. `config_kind` derived from filename stem against a known list (vite, panda, vitest, tailwind, next, jest, rollup, webpack, esbuild, postcss, eslint, prettier, tsup, storybook, svelte, astro, nuxt, remix) else `"unknown"`.
3. **BarrelModule** — at least 2 re-exports AND ≥80% re-exports (`re_export_count * 5 >= total_exports * 4`).
4. **PandaRecipe** — file contains a `defineRecipe`/`defineSlotRecipe`/`cva`/`sva` call (detected by `crate::panda::is_panda_recipe_file`). Multi-recipe files emit only the first with a `Warning` diagnostic naming the dropped count.
5. **TestFile** — must run before ContextModule. Detected by non-empty `deps.test_utilities` OR `.test.`/`.spec.` filename OR `__tests__/` path component.
6. **ContextModule** — detected after TestFile. Requires either a symbol signature containing `createContext(...)` (with optional `<…>` generics), OR a symbol whose owning block contains the call AND the file imports `createContext` from `react`/`react/*`.
7. **ReactHook** — first symbol whose name starts with `use` AND fourth character is uppercase.
8. **ReactComponent** — file has JSX elements AND first PascalCase symbol (kind Function/Variable/Class).
9. **TypeModule** — non-empty symbols list where every symbol is an Interface/TypeAlias/Enum/Class.
10. **Utility** — has any Function/Variable symbol.
11. **Unknown** — fallback.

Rust files (`.rs`) are routed to `crate::rust_inspection::inspect_rust_file` and bypass the TS analyzer entirely.

Hook name detector:

```rust
fn is_hook_name(name: &str) -> bool {
    if !name.starts_with("use") { return false; }
    let mut chars = name.chars();
    let _ = chars.next(); let _ = chars.next(); let _ = chars.next();
    chars.next().is_some_and(char::is_uppercase)
}
```

PascalCase detector:

```rust
fn is_pascal_case(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}
```

Type-module detector requires non-empty symbols where every symbol matches `Interface | TypeAlias | Enum | Class`.

Test subject inference (`infer_subject_file`):

- Strip `.test` / `.spec` from the file stem to recover the subject's bare stem.
- If trimmed equals raw, abort (no recoverable subject).
- Try sibling files in order: `<stem>.tsx`, `<stem>.ts`, `<stem>.jsx`, `<stem>.js`. Return the first that exists.
- Fall back to scanning `deps.local` for matching basename — but only returns `None` if found (path not resolvable from import alone).

Test count: number of code blocks whose code contains `it(`, `test(`, or `describe(`.

Props extraction (`extract_props`):

1. Find candidate prop type name (in order): from component param annotation; `<ComponentName>Props`; `<ComponentName>PropsType`; `Props`.
2. Find matching `Interface` or `TypeAlias` symbol in `analysis.symbols`.
3. Locate matching `CodeBlock` by `line_start == sym.line` and `node_kind == sym.kind` (or `SymbolKind::Other`).
4. Extract the balanced `{...}` body and parse it with `parse_interface_fields` (splits on top-level `;`/`,`/`\n`, skipping `()`/`[]`/`{}` nested contexts; strips line and block comments; strips `readonly`; recognizes `name?: type`).
5. Skip index signatures (`[k: string]: …`) and method shorthands (`foo(): void`).
6. Falls back to a single placeholder field `(see TypeName)` when body can't be parsed.

Pattern detection (`detect_component_patterns`):

- Checks symbol signatures lowercased for `forwardref`, `memo(`/`react.memo`, `lazy(`/`react.lazy`.
- Checks per-block code containing the component name.
- Checks JSX elements containing `forwardref`.
- `display_name` always `None` (placeholder for `Comp.displayName = "…"` detection).

Used-hooks extractor (`extract_used_hooks`): scans owner's code blocks for `useXxx(` (where `xxx` starts with uppercase), preserves generic params (`useState<boolean>`), de-duplicates.

Story meta extraction:

- `find_meta_identifier_field`: locates `field:` in code, walks forward over an identifier (alpha/underscore/dollar then alnum/underscore/dollar/dot).
- `find_meta_string_field`: locates `field:` then accepts a `"`, `'`, or backtick quote and returns the content up to the matching quote.
- `extract_story_names`: every export except `default`, de-duplicated, source order preserved.

Barrel re-export source counting (`count_reexport_source_files`): distinct re-export lines in `analysis.exports` AND distinct quoted specifiers from a textual `export … from '…'` scan — returns `max(from_lines, textual)`.

HOC detection (`detect_is_hoc`): name starts with `with<Uppercase>` OR any symbol matches that shape OR component block contains `React.ComponentType` / `: ComponentType`.

Lazy detection (`detect_react_lazy`): symbol signature contains `React.lazy` / `= lazy(` OR any block contains `React.lazy(` / `= lazy(` / ` lazy(`.

Zod schema detection (`detect_zod_schemas`):

1. Pre-check that file imports from `zod` or `@zod/core`.
2. For each symbol with `signature_has_zod_call(sig)`, push name.
3. Then for symbols not already pushed, check the block by `line_start == sym.line` and look for `z.<ident>(`.

The `z.<ident>(` detector enforces word boundaries — `z` must not be part of a larger identifier.

Empty result builders use `import_classifier::classify(&[])` for `empty_classified()`.

### CLI flags

```
Inspect {
    #[arg(required = true)] paths: Vec<Utf8PathBuf>,
    #[arg(long, default_value = ".")] root: Utf8PathBuf,
    #[arg(long)] kind: Option<String>,
}
```

`--kind` filters output to the named kind (the kebab-case `kind` discriminant).

## `mnkl patterns` — Pattern frequency analysis

### Purpose

Walk a directory (gitignore-aware), inspect every TS/JS file, and aggregate classification counts, hook usage, styling approaches, props styles, test coverage, panda patterns, and toolchain signals.

### PatternsResult type verbatim

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

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TestCoverage {
    pub has_story: usize,
    pub has_test: usize,
    pub neither: usize,
}
```

Algorithm (in `pipeline::patterns`):

- Walks `dir` with `ignore::WalkBuilder` and `standard_filters(true)`.
- Filters to files supported by `TsAnalyzer` (`.ts`/`.tsx`/`.js`/`.jsx`/`.mts`/`.cts`/`.mjs`/`.cjs`, excluding `.d.ts`).
- Delegates to `inspection::inspect_files` for the entire batch.
- First pass: iterate entries, tallying per-variant counters. For components, aggregates styling approach → count, pattern flags (forwardRef/memo/lazy) → count, hooks → `hooks_top`, props_style (interface if type_str contains `interface`, else type), export style label, and three-way test-coverage classification: `has_story` (has stories), `has_test` (no story but has test_utilities), `neither`.
- For hooks/utilities/test-files, accumulates toolchain via `accumulate_toolchain`, state libs (BTreeSet), data fetching libs.
- Second pass: per-component aggregates `hoc_count`, `lazy_count`, `zod_usage`, and `hooks_extended` for the four extended-hooks set `["useContext", "useReducer", "useMemo", "useSuspense"]` (matching after stripping generics — e.g. `useState<boolean>` strips to `useState`).
- Utilities with non-empty `zod_schemas` also bump `zod_usage`.
- Recipes collected from `InspectEntry::PandaRecipe` entries and aggregated via `crate::panda::aggregate_patterns(&recipes)`.

### panda.rs — verbatim

Public types:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PandaRecipeEntry {
    pub file: Utf8PathBuf,
    pub name: String,
    pub recipe_type: RecipeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<String>>,
    pub variants: BTreeMap<String, Vec<String>>,
    pub compound_variant_count: usize,
    pub default_variants: BTreeMap<String, String>,
    pub responsive: ResponsiveAnalysis,
    pub token_usage: TokenUsage,
    pub deps: ClassifiedImports,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeType {
    DefineRecipe,
    DefineSlotRecipe,
    Cva,
    Sva,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResponsiveAnalysis {
    pub approach: ResponsiveApproach,
    pub breakpoints_used: Vec<String>,
    pub tablet_augmented_properties: Vec<String>,
    pub desktop_exception_properties: Vec<String>,
    pub missing_mobile_baseline: Vec<String>,
    pub uses_touch_condition: bool,
    pub uses_motion_reduce: bool,
    pub uses_high_contrast: bool,
    pub uses_brand_conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResponsiveApproach {
    #[default]
    None,
    MobileFirst,
    DesktopFirst,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub semantic: Vec<String>,
    pub primitive: Vec<String>,
    pub hardcoded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecipeImportInfo {
    pub imports_design_system_recipes: bool,
    pub recipe_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PandaPatterns {
    pub define_recipe_count: usize,
    pub define_slot_recipe_count: usize,
    pub cva_count: usize,
    pub sva_count: usize,
    pub mobile_first_count: usize,
    pub tablet_augmented_count: usize,
    pub desktop_exception_count: usize,
    pub missing_mobile_baseline_count: usize,
    pub uses_touch_count: usize,
    pub uses_motion_reduce_count: usize,
    pub uses_brand_conditions_count: usize,
}
```

Detection logic:

- `is_panda_recipe_file(analysis)`: returns true when any symbol signature matches `RE_RECIPE_CALL` (`\b(defineRecipe|defineSlotRecipe|cva|sva)\s*\(`) or any block code matches.
- `extract_recipes_from_analysis`: first pass over symbols (symbol whose signature matches a recipe call); fallback pass over blocks if first yields nothing.
- `block_recipe_type` returns `Some(RecipeType::DefineSlotRecipe | DefineRecipe | Sva | Cva)` based on captured kind.
- Slots: extracted only for `DefineSlotRecipe` and `Sva` via `RE_SLOTS_ARRAY` (`slots\s*:\s*\[([^\]]*)\]`) then `string_literals(body)`.
- Variants: `extract_variants` walks `variants: { ... }` body, recognizes top-level `key: { ... }` entries (skipping `...spread`), then collects variant value names (identifier or quoted-string keys).
- Compound variants: `extract_braced_body_array(code, "compoundVariants")` then count top-level `{` entries.
- Default variants: `RE_DEFAULT_VARIANTS` extracts `defaultVariants: { ... }` body and `RE_KV_PAIR` extracts each `key: '<value>'` pair.
- Responsive: `analyze_responsive` runs `detect_breakpoint_keys` (any object key sibling of `base:`) excluding CSS shorthands (`px`, `py`, etc.) via `is_likely_css_property`, then `scan_object_properties` recursively walks `prop: { base, tablet, desktop }` shapes, classifying `tablet_augmented_properties`, `desktop_exception_properties`, `missing_mobile_baseline`.

`ResponsiveApproach` resolution:

```rust
r.approach = match (mobile_first_signal, nested_signal, desktop_first_signal) {
    (true, _, true) | (false, true, true) => ResponsiveApproach::Mixed,
    (true, _, false) | (false, true, false) => ResponsiveApproach::MobileFirst,
    (false, false, true) => ResponsiveApproach::DesktopFirst,
    (false, false, false) => ResponsiveApproach::None,
};
```

Conditions: `r.uses_touch_condition = code.contains("_touch")`, `r.uses_motion_reduce = code.contains("_motionReduce")`, `r.uses_high_contrast = code.contains("_highContrast")`. Brand conditions checked via `contains_word_key` against `["brandDark", "brand", "dark"]`.

Token classification (`classify_tokens`):

- Collects only string-literal values via `RE_STRING_LITERAL`.
- Semantic: `RE_SEMANTIC_TOKEN = \b(?:bg|text|border|shadow|fill|stroke|ring)\.[a-zA-Z][a-zA-Z0-9._]*`.
- Primitive: `RE_PRIMITIVE_TOKEN = \bprimitives\.[a-zA-Z0-9._]+`.
- Hardcoded: `RE_HARDCODED_HEX = #[0-9a-fA-F]{3,8}\b`, `RE_HARDCODED_UNIT = \b\d+(?:\.\d+)?(?:px|rem|em|%|vh|vw)\b`, `RE_HARDCODED_FUNC = \b(?:rgba?|hsla?)\([^)]*\)`.

`detect_recipe_imports(deps)` rules:

1. Sets `imports_design_system_recipes = true` if any `deps.design_system` entry starts with `@design-system/foundation`.
2. Fallback: when styling `approach == "panda-css"` AND `deps.design_system` is non-empty AND not already true (third-party Panda DS).
3. `@pandacss/dev` alone (no DS) is NOT enough.
4. Adds candidate names from `styling.imports` filtered by `is_recipe_name_candidate` (lowercase first char, not `css`/`cx`/`cva`/`sva`/`defineRecipe`/`defineSlotRecipe`).

`aggregate_patterns(recipes)` counts each recipe by type, by approach (MobileFirst/Mixed → `mobile_first_count`), and by whether each `responsive` field is non-empty.

### import_classifier.rs verbatim

```rust
#[derive(Debug, Clone)]
pub struct ClassifyOpts {
    pub design_system_prefixes: Vec<String>,
}

impl Default for ClassifyOpts {
    fn default() -> Self {
        Self {
            design_system_prefixes: vec!["@design-system/".to_owned(), "@design-system-internal/".to_owned()],
        }
    }
}

pub fn classify(deps: &[DependencyRecord]) -> ClassifiedImports {
    classify_with_opts(deps, &ClassifyOpts::default())
}
```

Classification predicates:

- `is_framework`: `react`, `react-dom`, `react/*`, `react-dom/*`, or `react-*` (when not also matching test/data-fetching/state).
- `styling_approach`: `@pandacss/*`/`@styled-system/*`/`panda` → `"panda-css"`; `styled-components`/`@emotion/*` → `"styled-components"`; `*.module.css`/`*.module.scss` → `"css-modules"`; `tailwind*`/`clsx`/`classnames` → `"tailwind"`.
- `is_state_management`: `jotai`/`zustand`/`redux`/`@reduxjs/toolkit`/`@reduxjs/*`/`react-redux`/`recoil`/`mobx`/`mobx-*`.
- `is_data_fetching`: `@tanstack/react-query` (and subpath), `swr`/`swr/*`, `apollo-client`, `@apollo/*`, `react-query`.
- `is_design_system_with`: prefix match against opts.
- `is_test_utility`: `vitest`(+subpath/`@vitest/*`), `jest`(+subpath/`jest-*`), `@testing-library/*`, `msw`(+subpath).
- `is_observability`: `opentelemetry`, `@opentelemetry/*`, `datadog-*`, `@datadog/browser-rum`, `@datadog/browser-logs`.
- `is_vite`: `vite`/`vite/*`/`@vitejs/*`. Similar for webpack, rollup, esbuild.

Toolchain test setters:

- `vitest`/`vitest/*`/`@vitest/*` → `toolchain.test = "vitest"`.
- `jest`/`jest/*`/`jest-*` → `toolchain.test = "jest"`.
- `mocha`/`mocha/*` → `toolchain.test = "mocha"`.

`@types/*` → `toolchain.types`. `@storybook/*` → `documentation.has_stories = true`. Relative paths (starting `.`) → `local`, also sets `has_stories` if path contains `.stories.`. Unclassified → `external`. Styling entries merge into a single `StylingImports`: approach = first-detected; imports = union.

## `mnkl schema` — Schema extraction

### Purpose

Emit JSON Schema definitions for `InspectEntry` variants and for result types from other commands. Hand-written (no `schemars`).

### SchemaResult type verbatim

The output is a raw `serde_json::Value`. The `inspect_schema(kind: Option<&str>) -> Value` entry point dispatches by `kind`:

- Inspect variants: `react-component`, `react-hook`, `utility`, `type-module`, `test-file`, `story-file`, `config-module`, `barrel-module`, `context-module`, `panda-recipe`, `rust-struct`, `rust-enum`, `rust-trait`, `rust-module`, `unknown`
- Result types: `tokens-result`, `refs-result`, `definition-result`, `diff-result`, `patterns-result`, `explain-result`, `coverage-result`, `data-flow-result`, `similar-result`
- `None` (or unmatched) → `full_union_schema()` returning a `oneOf` over all `InspectEntry` variants.

### schema.rs verbatim (signature)

```rust
pub fn inspect_schema(kind: Option<&str>) -> Value {
    match kind {
        Some("react-component") => react_component_schema(),
        Some("react-hook") => react_hook_schema(),
        Some("utility") => utility_schema(),
        Some("type-module") => type_module_schema(),
        Some("test-file") => test_file_schema(),
        Some("story-file") => story_file_schema(),
        Some("config-module") => config_module_schema(),
        Some("barrel-module") => barrel_module_schema(),
        Some("context-module") => context_module_schema(),
        Some("panda-recipe") => panda_recipe_schema(),
        Some("rust-struct") => rust_struct_schema(),
        Some("rust-enum") => rust_enum_schema(),
        Some("rust-trait") => rust_trait_schema(),
        Some("rust-module") => rust_module_schema(),
        Some("tokens-result") => tokens_result_schema(),
        Some("refs-result") => refs_result_schema(),
        Some("definition-result") => definition_result_schema(),
        Some("diff-result") => diff_result_schema(),
        Some("patterns-result") => patterns_result_schema(),
        Some("explain-result") => explain_result_schema(),
        Some("coverage-result") => coverage_result_schema(),
        Some("data-flow-result") => data_flow_result_schema(),
        Some("similar-result") => similar_result_schema(),
        Some("unknown") => unknown_schema(),
        _ => full_union_schema(),
    }
}
```

Every schema is built with `serde_json::json!` macros and includes nested helpers like `classified_imports_schema()`, `prop_field_schema()`, `rust_imports_schema()`, `visibility_schema()`, `diagnostics_array_schema()`, `confidence_schema()`, `caller_summary_schema()`, `component_summary_schema()`, `import_summary_schema()`, `recipe_summary_schema()`, `data_source_schema()`, `state_point_schema()`, `data_output_schema()`, `token_violation_schema()`, `file_token_report_schema()`, `ref_site_schema()`, `definition_site_schema()`, `signature_change_schema()`, `structural_changes_schema()`, `file_change_type_schema()`, `file_diff_schema()`, `diff_summary_schema()`, `similar_file_schema()`. Each emits `$schema: "https://json-schema.org/draft/2020-12/schema"`, `type: object`, `title`, `description`, `properties`, and `required`.

## `mnkl refs` — Symbol-level occurrence and reference finder

### Purpose

Find structural and textual candidate occurrences of a named symbol across a workspace. A capture is evidence that the spelling occurs in a relevant role; it is not proof that the occurrence binds to the requested symbol. Semantic reference resolution is deferred behind the owned-occurrence design gate below.

### RefsResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefsResult {
    pub symbol: String,
    pub refs: Vec<RefSite>,
    pub total_ref_count: usize,
    pub truncated: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefSite {
    pub file: Utf8PathBuf,
    pub line: usize,
    pub ref_kind: RefKind,
    pub capture: RefCapture,
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum RefCapture {
    /// The parser identified a role, but no binding identity was available.
    Structural,
    /// A bounded text sweep found the spelling only. This is a candidate, not a resolved reference.
    Textual,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum RefKind {
    Import,
    JsxElement,
    Call,
    TypeRef,
    Value,
    ReExport,
}
```

### refs.rs algorithm verbatim

`find_refs(symbol, opts, include_tests, max_refs, from_file) -> Result<RefsResult>`:

1. Empty symbol or `max_refs == 0` → empty result.
2. If `from_file` is set, call `resolve_canonical_source` and handle every terminal outcome: `Resolved(path)` continues with narrowing; `Ambiguous { candidates, diagnostic }` and `Unresolved { diagnostic }` return no sites and preserve the supplied diagnostic; `External` returns no workspace sites and adds a `Skipped` diagnostic stating that the target is outside the indexed scope; `NotImported` returns no sites without manufacturing an error.
3. Build per-line classifier regexes (all `\b`-bounded, case-sensitive):
   - `mention_re = \b{escaped}\b`
   - `call_re = (?:^|[^A-Za-z0-9_$])(?:new\s+)?{escaped}\s*\(`
   - `type_re = (?::\s*{escaped}\b|extends\s+{escaped}\b|implements\s+{escaped}\b|{escaped}\s*<)`
4. Pre-filter matcher via `grep-regex`.
5. Walk `opts.root` with `ignore::WalkBuilder` standard filters. Skip `.rs` files. Skip test paths unless `include_tests`. Skip files not supported by `TsAnalyzer`.
6. Pre-filter each file with `grep-searcher` — bail early when symbol not mentioned at all.
7. Analyze the file. When `from_file` narrowing is active, skip files that don't import `symbol` from `canonical_source` (with path canonicalization to handle symlinks). The canonical_source file itself is always included.
8. Read file source for snippet extraction (no snippets on read failure, but degraded refs still emitted).
9. Emit candidate occurrences in three phases, retaining the producer in `RefSite.capture`:
   - **Imports and re-exports**: iterate dependencies and exports, retaining ambiguous/unresolved dependency diagnostics rather than selecting a target. Emit role-only matches as `RefCapture::Structural`.
   - **JSX elements**: iterate `analysis.jsx_elements()` for `name == symbol`. Emit `RefCapture::Structural`.
   - **Fallback line sweep**: only for a language/analyzer path that cannot provide semantic occurrences, scan every line; if `mention_re.is_match`, dedup by `(file, line)` and emit `RefCapture::Textual`. The result diagnostic must say that these are name-matched candidates. Classification remains syntactic:
     ```rust
     fn classify_line(line, call_re, type_re) -> RefKind {
         if call_re.is_match(line) { RefKind::Call }
         else if type_re.is_match(line) { RefKind::TypeRef }
         else { RefKind::Value }
     }
     ```
10. Dedup via `BTreeSet<(Utf8PathBuf, usize)>`, preferring `Structural` over `Textual` when two producers identify the same site. Cap at `max_refs` (sets `truncated = true`).

`RefCapture` is intentionally not semantic resolution. The adopted `oxc_semantic` upgrade in Part 5 requires a separate owned occurrence record containing source range and local declaration/binding identity, constructed before the OXC allocator drops and persisted through `TsData`; imported bindings must then compose with `DependencyResolution`. Until that shape is specified and implemented, this command reports structural/textual candidates honestly rather than claiming semantic references. `ref_site_schema()` includes the required `capture` enum, and tests cover structural-vs-text fallback, alias/source narrowing, shadowed same-name candidates remaining unpromoted, ambiguous/unresolved/external `from_file` outcomes, and capture provenance surviving JSON serialization.

Test-path detection:

```rust
fn is_test_path(path: &Utf8Path) -> bool {
    let s = path.as_str();
    s.contains("/__tests__/")
        || s.ends_with(".test.ts") || s.ends_with(".test.tsx") || s.ends_with(".test.js") || s.ends_with(".test.jsx")
        || s.ends_with(".spec.ts") || s.ends_with(".spec.tsx") || s.ends_with(".spec.js") || s.ends_with(".spec.jsx")
}
```

Snippet trimmed to 120 chars.

## `mnkl definition` — Canonical declaration finder

### Purpose

Find every declaration site for a symbol, ranking canonical declarations before re-exports, and walking the re-export chain to the canonical source.

### DefinitionResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionResult {
    pub symbol: String,
    pub definitions: Vec<DefinitionSite>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionSite {
    pub file: Utf8PathBuf,
    pub line: usize,
    pub kind: String,
    pub signature: Option<String>,
    pub is_re_export: bool,
    pub re_export_chain: Vec<Utf8PathBuf>,
}
```

### definition.rs algorithm verbatim

Constants:

```rust
const RESOLVE_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const MAX_RE_EXPORT_HOPS: usize = 10;
```

`find_definition(symbol, opts, from_file) -> Result<DefinitionResult>`:

1. Empty symbol → empty.
2. `from_file` narrowing same as `refs` — resolve canonical source from importer.
3. Walk root, skip `.rs`, skip unsupported, pre-filter with grep-searcher matching `\b{escaped}\b`.
4. For each surviving file, analyze. Track `is_jsx_file = !jsx_elements().is_empty()` and the exported-names set.
5. **Declaration site**: iterate `analysis.symbols` for `name == symbol`. Require the symbol to be in the exports set. Emit a `DefinitionSite` with:
   - `kind = output_kind(sym.kind, &sym.name, is_jsx_file)`
   - `signature = sym.signature.clone()`
   - `is_re_export = false`, `re_export_chain = Vec::new()`
6. **Re-export site**: iterate `analysis.exports` for `name == symbol && re_export`. Read the file source and resolve the chain via `resolve_re_export_chain`. Emit `kind = "re-export"`, `signature = None`, `is_re_export = true`.
7. When `canonical_source` is set, retain only sites where either the file equals canonical (for declarations) or the chain contains canonical (for re-exports), using `paths_equal` (canonicalize-aware).
8. Sort by `(is_re_export, file, line)` — canonical first.

`output_kind`:

```rust
fn output_kind(kind: SymbolKind, name: &str, file_has_jsx: bool) -> &'static str {
    if is_hook_name(name) && matches!(kind, SymbolKind::Function | SymbolKind::Variable) {
        return "react-hook";
    }
    if file_has_jsx
        && is_pascal_case(name)
        && matches!(kind, SymbolKind::Function | SymbolKind::Variable | SymbolKind::Class)
    {
        return "react-component";
    }
    symbol_kind_str(kind)
}

fn symbol_kind_str(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Enum => "enum",
        SymbolKind::Variable => "const",
        SymbolKind::Module => "module",
        _ => "unknown",
    }
}
```

Re-export chain walker (`resolve_re_export_chain`):

- First hop: regex-scan `from_source` for the original re-export specifier (`find_re_export_specifier` tries `export\s*\{[^}]*\b{symbol}\b[^}]*\}\s*from\s*['"]([^'"]+)['"]` then a wildcard `export\s*\*\s*from\s*['"]([^'"]+)['"]`).
- Resolve relative specifier with `resolve_relative_specifier`: tries `<path>`, then each ext in `RESOLVE_EXTENSIONS`, then `<path>/index.<ext>`.
- Subsequent hops: prefer `next_hop_from_dependencies` (a `Named`/`Default`/`Namespace` binding's `resolved` path) else regex sweep.
- Stops when a hop declares the symbol (i.e., contains `symbol` in `analysis.symbols` AND a non-re-export export entry). Bounded by `MAX_RE_EXPORT_HOPS = 10`. Cycles broken via `visited` set.

## `mnkl diff` — Structural git-ref delta

### Purpose

Compute symbol-, prop-, and dependency-level deltas between two git refs. Used by code-review tooling to scope review to load-bearing changes.

### DiffResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub base: String,
    pub head: String,
    pub file_diffs: Vec<FileDiff>,
    pub summary: DiffSummary,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub file: Utf8PathBuf,
    pub change_type: FileChangeType,
    pub file_kind: String,
    pub structural: StructuralChanges,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum FileChangeType {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StructuralChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<SignatureChange>,
    pub props_added: Vec<String>,
    pub props_removed: Vec<String>,
    pub deps_added: Vec<String>,
    pub deps_removed: Vec<String>,
    pub breaking_changes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureChange {
    pub name: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub files_changed: usize,
    pub files_added: usize,
    pub files_deleted: usize,
    pub symbols_added: usize,
    pub symbols_removed: usize,
    pub symbols_modified: usize,
    pub by_kind: BTreeMap<String, usize>,
}
```

### diff.rs algorithm verbatim

`diff(base, head, root)`:

1. `ensure_git_repo(root)` — runs `git -C <root> rev-parse --is-inside-work-tree`.
2. `list_changed_files(base, head, root)` runs `git -C <root> diff --name-status -M base..head` and parses the output. `parse_name_status` recognizes `M`/`A`/`D`/`R<score>\told\tnew`/`C<score>` (Copied → Added)/`T` (typechange → Modified).
3. For each change, `classify_file_kind(path, change_type)`:
   - `*.test.*`/`*.spec.*`/`__tests__/` → `"test"` (wins regardless).
   - `.md`/`.mdx` → `"docs"`.
   - `.github/` dir → `"chore"`.
   - `.json`/`.toml`/`.yml`/`.yaml`/`.lock`/`.gitignore` → `"chore"`.
   - Added: `/fix/` or `/bugfix/` path → `"fix"`; else `"feat"`.
   - Deleted → `"refactor"`.
   - Modified/Renamed: `/components/`, `/pages/`, `/views/` → `"feat"`; `/utils/`, `/helpers/`, `/lib/` → `"refactor"`; `fix` in name → `"fix"`; else `"refactor"`.
4. `analyze_change`: fetches both versions via `git show base:path` / `git show head:path`, computes `diff_imports` always, then computes symbol/prop diffs only for inspectable files (`.ts/.tsx/.js/.jsx/.mjs/.cjs/.rs`).
5. Inspect happens against a tempfile that preserves the original extension (`write_temp_with_extension`).
6. `diff_symbols(base, head)` extracts `(name, signature)` pairs from inspect entries via `symbols_of`:
   - `ReactComponent` → `[(name, component_signature(c))]` where `component_signature(c) = format!("Component({})", props.iter().map(p.name).sorted().join(","))`.
   - `ReactHook` → `[(name, hook_signature(h))]` where `hook_signature = format!("({}) -> {}", params.join(", "), returns.unwrap_or("void"))`.
   - `Utility` → one entry per function with sig `(params) -> returns`.
   - `TypeModule` → one entry per type with sig = type kind string.
   - `PandaRecipe` → `[(name, "panda-recipe")]`.
   - `RustStruct`/`RustEnum`/`RustTrait` → `[(name, "struct"|"enum"|"trait")]`.
   - `Unknown` → all symbols with their signatures.
   - `ContextModule` → context names with sig `"context"`.
   - Test/Module/Story/Config/Barrel → empty.
7. `diff_props` produces `props_added` / `props_removed` as `"name: type"` strings.
8. `diff_imports` calls `scan_imports(src)` on each side, returns `BTreeSet<String>` symmetric diff in form `specifier::name` for named bindings, or bare `specifier` for default/namespace/side-effect.
9. `compute_breaking_changes`:
   - All `removed` symbols → breaking.
   - `modified` symbols where `signature_change_is_breaking(before, after)`: `before_params > after_params` OR return clause changed.
   - All `props_removed` → breaking, prefixed `prop:`.

`scan_imports` parsers:

- `parse_js_import_entries(line)`: handles `import "..."`, `import x from "..."`, `import { a, b as c } from "..."`, `import * as ns from "..."`, `import type { … }`. Each named binding emits `spec::name`.
- `parse_require(line)`: `require("spec")` → `spec`.
- `parse_python_import_entries`: `from spec import a, b` → `spec::a`, `spec::b`; `import spec` → `spec`.

Returns `DiffSummary` aggregating totals + a `by_kind` map.

## `mnkl tokens` — Design-token discipline audit

### Purpose

Audit a directory for hardcoded values, primitive-token references (`primitives.*`), and semantic tokens inside styling contexts (`css(...)`, `cva(...)`, `defineRecipe(...)`, `defineSlotRecipe(...)`, `sva(...)`, JSX `style={{...}}`). Emits a per-file breakdown plus an overall compliance score.

### TokensAnalysisResult type verbatim (named `TokensResult`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokensResult {
    pub dir: Utf8PathBuf,
    pub files_audited: usize,
    pub overall_compliance: f32,
    pub hardcoded_count: usize,
    pub primitive_token_count: usize,
    pub semantic_token_count: usize,
    pub by_file: Vec<FileTokenReport>,
    pub worst_offenders: Vec<Utf8PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTokenReport {
    pub file: Utf8PathBuf,
    pub hardcoded: Vec<TokenViolation>,
    pub primitive_tokens: Vec<TokenViolation>,
    pub semantic_token_count: usize,
    pub compliance_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenViolation {
    pub value: String,
    pub line: usize,
    pub context: String,
}
```

### tokens_analysis.rs verbatim

Regex set:

- `RE_STYLING_CALL = \b(?:css|cva|sva|defineRecipe|defineSlotRecipe)\s*\(`
- `RE_JSX_STYLE_PROP = \bstyle\s*=\s*\{\s*\{`
- `RE_HEX_COLOR = #[0-9a-fA-F]{3,8}\b`
- `RE_COLOR_FUNC = \b(?:rgba?|hsla?)\s*\([^)]*\)`
- `RE_CSS_UNIT = \b\d+(?:\.\d+)?(?:px|rem|em|%|vh|vw|dvh|dvw|svh|svw)\b`
- `RE_PRIMITIVE_TOKEN = ["']primitives\.[a-zA-Z0-9._]+["']`
- `RE_QUOTED_STRING = ["']([^"'\n\r]+)["']`
- `RE_SEMANTIC_TOKEN = ^[a-z][a-zA-Z0-9]*(?:\.[a-z][a-zA-Z0-9]*)+$`

Constants:

```rust
const SEMANTIC_CATEGORIES: &[&str] = &[
    "bg", "text", "border", "shadow", "fill", "stroke", "ring",
    "colors", "spacing", "fontSizes", "fontWeights", "lineHeights",
    "letterSpacings", "radii", "sizes", "zIndex",
];

const HARDCODED_NAMED_COLORS: &[&str] = &[
    "red", "blue", "green", "white", "black", "transparent",
    "yellow", "orange", "purple", "pink", "gray", "grey",
];
```

Algorithm (`audit_tokens`):

1. `collect_source_files(dir)` — walk gitignore-aware, collect `.ts`/`.tsx`/`.js`/`.jsx`.
2. For each file, read source, run `audit_file`:
   - `styling_context_ranges(src)`: collect `[start, end)` byte ranges from both `RE_STYLING_CALL` (using `find_balanced` on `(`/`)`) and `RE_JSX_STYLE_PROP` (using `find_balanced` on inner `{`/`}`).
   - For each range body, run `scan_hex_colors`, `scan_color_functions`, `scan_css_units`, `scan_named_colors`, `scan_primitive_tokens`, and `count_semantic_tokens`.
3. `scan_css_units` skips zero dimensions via `value_is_zero_dimension(value)` (parses numeric prefix and rejects when 0).
4. `scan_named_colors` only fires when the value is a string-literal exactly matching a `HARDCODED_NAMED_COLORS` entry (case-insensitive).
5. `count_semantic_tokens` requires the literal value to match `RE_SEMANTIC_TOKEN` AND its first segment to be in `SEMANTIC_CATEGORIES`.
6. Per-file `compliance_score = semantic_count / (semantic_count + hardcoded.len())`, falling to `1.0` when both are zero.
7. Sort files by `file` ASC. Worst offenders: take top 5 files with non-empty hardcoded, sorted by `hardcoded.len()` DESC then file ASC.
8. Overall: `overall_compliance = semantic / (semantic + hardcoded)`, falling to `1.0` when both zero. Primitives are not part of the denominator.

Line/context translation:

```rust
fn line_and_context_for_offset(src: &str, offset: usize) -> (usize, String) {
    let bytes = src.as_bytes();
    let clamped = offset.min(bytes.len());
    let line_start = src[..clamped].rfind('\n').map_or(0, |p| p + 1);
    let line_end = src[clamped..].find('\n').map_or(bytes.len(), |p| clamped + p);
    let line_number = bytecount_newlines(&src[..line_start]) + 1;
    let raw_line = &src[line_start..line_end];
    let trimmed = raw_line.trim();
    let truncated: String = trimmed.chars().take(80).collect();
    (line_number, truncated)
}
```

`find_balanced` skips delimiters inside string literals (single/double/backtick, honoring `\` escapes).

## `mnkl explain` — Code explanation

### Purpose

Synthesize `inspect` + `refs` + `dependents` into one compressed answer. Compresses callers by `RefKind`, classifies imports, and emits a confidence-scored kind classification so callers don't have to make multiple tool calls.

### ExplainResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResult {
    pub file: Utf8PathBuf,
    pub kind: String,
    pub kind_confidence: Confidence,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentSummary>,
    pub callers: CallerSummary,
    pub imports: ImportSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<RecipeSummary>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComponentSummary {
    pub name: String,
    pub props: Vec<String>,
    pub hooks: Vec<String>,
    pub patterns: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub styling: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CallerSummary {
    pub total: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub in_tests: usize,
    pub in_production: usize,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub design_system: Vec<String>,
    pub local_count: usize,
    pub external_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub styling_approach: Option<String>,
    pub test_utilities: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSummary {
    pub name: String,
    pub recipe_type: String,
    pub variant_keys: Vec<String>,
    pub responsive_approach: String,
    pub token_compliance: f32,
}
```

### explain.rs verbatim algorithm

`explain(file, symbol, opts)`:

1. Run `inspection::inspect_files([file])`.
2. Derive `primary_name`: caller-supplied `symbol` overrides; else `primary_symbol_name(entry)` — component name, hook name, first utility function, first type, first context name, recipe name, rust struct/enum/trait name; `None` for test/story/config/barrel/rust-module/unknown.
3. If `primary_name` exists, run `refs::find_refs(name, opts, include_tests=true, max_refs=500, None)` and aggregate into `CallerSummary`.
4. Derive `kind` label (kebab-case) via `kind_label(entry)`.
5. Compute kind confidence via `compute_kind_confidence(entry)`:
   - `ReactComponent` → `component_confidence`: scored on PascalCase + exported + has_props.
   - `ReactHook` → `hook_confidence`: scored on `use` prefix + has_return + calls_hooks.
   - `PandaRecipe` → score 0.98.
   - `TestFile` → 0.98 if filename, else 0.80.
   - `Utility` → 0.85; `TypeModule` → 0.90; `StoryFile` → 0.95; `ConfigModule` → 0.95; `BarrelModule` → 0.92; `ContextModule` → 0.90; Rust kinds → 0.90 with caveat about textual scanner; `Unknown` → 0.40.
6. Build `ComponentSummary`/`ImportSummary`/`RecipeSummary` (when applicable).
7. `build_summary(entry, primary_name, callers)` returns `"{kind} `{name}` — {caller_phrase}"` where caller_phrase is `no callers found` / `1 caller` / `N callers`.

`aggregate_callers` maps `RefKind` to kebab-case label, counts via `BTreeMap`, splits in_tests vs in_production by `is_test_path`.

Component summary prop compression (`compress_prop`):

- `is_string_literal_union(t)`: contains `|` and every segment is single/double-quoted → `name='v1'|'v2'`.
- Optional + non-union: `name?: type`.
- Required + non-union: `name: type`.
- Then `truncate_at(s, 40)` (chars + `…` suffix if over 40 chars).

`component_patterns_to_list` emits a list with `"forwardRef"`, `"memo"`, `"lazy"` based on flags. `format_styling` returns `"{approach}"` or `"{approach} imports={imports,joined}"`.

`recipe_summary` only fires for `InspectEntry::PandaRecipe`. `token_compliance = semantic / (semantic + primitive + hardcoded)`, fall to 1.0 when total is 0.

## `mnkl coverage` — Coverage analysis

### Purpose

Audit test coverage for a file's exported symbols. For each exported symbol, finds every reference and reports whether any reference lives in a test file. Deterministic — does not run tests.

### CoverageResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageResult {
    pub file: Utf8PathBuf,
    pub score: f32,
    pub exported_count: usize,
    pub tested: Vec<String>,
    pub untested: Vec<String>,
    pub test_files: Vec<Utf8PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}
```

### coverage.rs verbatim algorithm

1. `inspect_files([file])` to get the entry.
2. `exported_names(entry)`:
   - ReactComponent → `[c.name]`
   - ReactHook → `[h.name]`
   - Utility → all `f.name` from `functions`
   - TypeModule → all type names
   - BarrelModule → `b.exports`
   - ContextModule → `c.context_names`
   - PandaRecipe → `[r.name]`
   - RustStruct/Enum/Trait → `[name]`
   - TestFile / StoryFile / ConfigModule / RustModule / Unknown → empty
3. For each exported name, call `refs::find_refs(name, opts, include_tests=true, max_refs=100, None)`.
4. If any ref site `is_test_path(&site.file)`, mark symbol as tested and add file to `test_files` set (a `BTreeSet<Utf8PathBuf>`).
5. `score = tested.len() / exported_count` (`1.0` when no exports).

## `mnkl data-flow` — Data flow tracing

### Purpose

Trace data inputs (props, contexts, fetch, route params, localStorage), state (`useState`/`useReducer`), outputs (exports + JSX return + event callbacks), and production consumers for a TS/JS file. Each inference carries a `Confidence` score.

### DataFlowResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFlowResult {
    pub file: Utf8PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub inputs: Vec<DataSource>,
    pub state: Vec<StatePoint>,
    pub outputs: Vec<DataOutput>,
    pub consumers: Vec<Utf8PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    pub kind: DataSourceKind,
    pub name: String,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DataSourceKind {
    Prop,
    ContextHook,
    ApiCall,
    RouteParam,
    LocalStorage,
    ExternalHook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatePoint {
    pub hook: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_type: Option<String>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataOutput {
    pub kind: OutputKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_str: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputKind {
    Export,
    JsxReturn,
    EventCallback,
}
```

### data_flow.rs verbatim algorithm

1. `inspect_files([file])` for the typed entry.
2. Rebuild `FileAnalysis` via `TsAnalyzer::from_workspace_opts` + `analyze` to access code blocks.
3. **Inputs**:
   - `add_prop_inputs`: one `DataSource::Prop` per prop, confidence 0.95.
   - `add_import_inputs`: every `deps.data_fetching` entry → `ApiCall` 0.92; every `deps.design_system` that matches `useXxx` (uppercase 4th char) → `ExternalHook` 0.80.
   - `add_code_block_inputs` runs `scan_block_for_inputs` on each block:
     - `fetch(` or `axios.` → `ApiCall`. URL extracted via `extract_fetch_url` (tries `fetch(`, `axios.get(`, `axios.post(`, `axios.put(`, `axios.delete(`; returns first quoted literal); score 0.88 with URL, 0.65 without.
     - `useParams(` / `useSearchParams(` → `RouteParam` 0.95.
     - `useContext(...)` with bare identifier → `ContextHook` 0.95.
     - `localStorage.getItem(` / `localStorage.setItem(` → `LocalStorage` 0.95.
4. **State**: `hooks_to_state` parses `useState`/`useReducer` from the hooks list, extracts type arg from `useState<T>` form (`parse_type_arg`).
5. **Outputs** (`build_outputs`): one `OutputKind::Export` per export record. For ReactComponent: also `OutputKind::JsxReturn` with `type_str = "JSX.Element"`, plus every prop where `is_event_handler_name(name)` matches (`o`, `n`, then uppercase letter) → `OutputKind::EventCallback`.
6. **Consumers**: if there's a `primary_name`, run `refs::find_refs(name, opts, include_tests=false, max_refs=200, None)`, then filter to production files (`!is_test_path`) where `ref_kind` is `Import` or `JsxElement`. Returned as a sorted, deduplicated `Vec<Utf8PathBuf>`.

## `mnkl similar` — Similarity search

### Purpose

Find structurally similar TS/JS files by fingerprinting and scoring across six dimensions.

### SimilarResult type verbatim

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarResult {
    pub query_file: Utf8PathBuf,
    pub query_kind: String,
    pub results: Vec<SimilarFile>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarFile {
    pub file: Utf8PathBuf,
    pub kind: String,
    pub score: f32,
    pub matching: Vec<String>,
    pub diverging: Vec<String>,
}
```

### similar.rs verbatim algorithm

1. Inspect the query file → fingerprint.
2. Walk root gitignore-aware, collect supported TS/JS candidates, excluding the query file itself (path canonicalization).
3. Inspect all candidates in one bulk pass.
4. For each candidate, build fingerprint and score.
5. Sort by score DESC then path ASC. Truncate to `count`.

Fingerprint structure:

```rust
struct Fingerprint {
    file: Utf8PathBuf,
    kind: String,
    prop_count: usize,
    hooks: BTreeSet<String>,
    styling_approach: Option<String>,
    dep_categories: BTreeSet<String>,
    export_style: String,
}
```

`prop_count`: components → props.len(); hooks → params.len(); utility → sum of params per function. Others → 0.

`hooks`: components → c.hooks; hooks → h.hooks_used; others → empty.

`styling_approach`: component → c.styling.approach; else from entry deps.

`dep_categories`: collects non-empty categories from `framework`, `styling`, `state`, `dataFetching`, `designSystem`, `testUtilities`, `observability`.

`export_style`: components → label; others → `"n/a"`.

Composite scoring:

```rust
let composite = 0.30 * kind_score
    + 0.15 * prop_score
    + 0.20 * hook_score
    + 0.15 * styling_score
    + 0.10 * dep_score
    + 0.10 * export_score;
```

- `kind_score`: 1.0 if same kind, else 0.0.
- `prop_score = 1 - |diff| / max(a, b, 1)` (clamped 0..1).
- `hook_score`: Jaccard over hook sets (both-empty → 1.0).
- `styling_score`: both Some & equal → 1.0; both None → 1.0 (no noisy entry); mismatch → 0.0.
- `dep_score`: Jaccard over `dep_categories`.
- `export_score`: 1.0 if equal else 0.0.

## New language support modules

### rust_inspection.rs verbatim

Public entry: `inspect_rust_file(file: &Utf8Path) -> InspectEntry`. Lazy file read, on failure returns `InspectEntry::Unknown` with empty data.

Block comment stripping (`strip_block_comments`) replaces `/* ... */` with spaces while preserving newlines.

Regex set:

- `RE_DERIVE = #\[derive\(([^\)]*)\)\]`
- `RE_STRUCT_HEADER = (?m)^\s*(pub(?:\([^\)]*\))?\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)`
- `RE_ENUM_HEADER = (?m)^\s*(pub(?:\([^\)]*\))?\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)`
- `RE_TRAIT_HEADER = (?m)^\s*(pub(?:\([^\)]*\))?\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)`
- `RE_USE = (?m)^\s*(?:pub(?:\([^\)]*\))?\s+)?use\s+([^;]+);`
- `RE_PUB_USE = (?m)^\s*pub(?:\([^\)]*\))?\s+use\s+([^;]+);`
- `RE_PUB_MOD = (?m)^\s*pub(?:\([^\)]*\))?\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;`
- `RE_IMPL_HEADER = (?m)^\s*impl(?:\s*<[^>]*>)?\s+(?:([A-Za-z_][A-Za-z0-9_:<>\s,]*?)\s+for\s+)?([A-Za-z_][A-Za-z0-9_]*)`
- `RE_FN = (?m)^\s*(pub(?:\([^\)]*\))?\s+)?(async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)`
- `RE_CFG_FEATURE = cfg\(\s*feature\s*=\s*"([^"]+)"`

Classification (`classify`) priority order:

1. Trait — first `RawTrait` → `RustTrait`.
2. Struct — `pick_main_struct(structs, methods_by_owner)` picks one with `max_by_key = method_count * 100 + field_count (= body.matches(':').count())` → `RustStruct`.
3. Enum — first `RawEnum` → `RustEnum`.
4. Fallback → `RustModule` with `re_exports` populated by `extract_re_exports`.

Struct extraction supports named-field `{ … }`, tuple `( … )`, and unit `;` forms. Derives collected from a 512-byte UTF-8-aligned lookback window before the header.

`extract_impl_methods` excludes trait impls (skips when `trait_part` is non-empty); `extract_trait_impls` only captures trait impls keyed by type, with trait name reduced to its last `::` segment.

`extract_methods_from_body` parses each `fn name(...)` block, extracts params (balanced parens) and `-> Ty` segment (`extract_return_segment` walks until `{` / `;` at depth 0).

`split_trait_methods` differentiates required (no body, ends with `;`) vs provided (has `{`) via `first_non_sig_char(tail)`.

`parse_struct_fields` splits body on top-level commas, strips line comments, supports visibility prefixes, parses `name: type`.

`parse_enum_variants` recognizes unit, tuple (`(T1, T2)`), and struct (`{ x: i32 }`) forms.

Use-statement classification (`extract_imports`):

- First segment `std`/`core`/`alloc` → `imports.std`.
- `crate`/`super`/`self` → `imports.local`.
- Other first segments dispatched via `classify_external_crate`:
  - `tokio`/`async_std` → `async_runtime`
  - `axum`/`actix_web`/`warp`/`rocket`/`tide`/`poem` → `web_framework`
  - `serde`/`serde_json`/`serde_yaml`/`bincode`/`toml`/`ciborium`/`rmp_serde` → `serialization`
  - `thiserror`/`anyhow`/`miette`/`eyre`/`snafu` → `error_handling`
  - `sqlx`/`diesel`/`sea_orm`/`rusqlite`/`postgres`/`tokio_postgres`/`redis`/`mongodb` → `database`
  - `proptest`/`quickcheck` → `testing.property`
  - `criterion` → `testing.benchmarks`
- `is_workspace_crate`: matches a known-crates allowlist for the workspace — `monokl`/`michi`/`firkin`/`lumen`/`pulse`/`auto_barrel`/`nova`/`oxc_react_docgen`.
- Else added to `external`.
- `testing.unit = source.contains("#[cfg(test)]") || source.contains("#[test]") || source.contains("#[tokio::test]")`.

`extract_result_err_type`: from a signature containing `Result<…, E>` extracts `E` by walking angle-bracket depth.

Visibility parser:

```rust
fn parse_visibility(prefix: &str) -> Visibility {
    let trimmed = prefix.trim();
    if trimmed.is_empty() { return Visibility::Private; }
    if trimmed == "pub" { return Visibility::Public; }
    if trimmed.starts_with("pub(crate)") { return Visibility::Crate; }
    if trimmed.starts_with("pub(super)") || trimmed.starts_with("pub(in") {
        return Visibility::Module;
    }
    if trimmed.starts_with("pub") { return Visibility::Public; }
    Visibility::Private
}
```

### python_inspection.rs verbatim

Public entry: `inspect_python_file(file: &Utf8Path) -> InspectEntry`. Same lazy-read + Unknown-on-failure pattern.

**Important caveat**: this module references types (`PythonClassEntry`, `PythonFunctionEntry`, `PythonModuleEntry`, `PythonClassKind`, `PythonFunctionKind`, `PythonModelStyle`, `PythonField`, `PythonImports`, `PythonMethod`) and `InspectEntry::PythonClass` / `PythonFunction` / `PythonModule` variants that are **not defined** in `types.rs` at the version captured here. The module is wired into `lib.rs` as `pub mod python_inspection;` but its dependent types must land in `types.rs` before it compiles.

Triple-quoted string stripping (`strip_triple_strings`) preserves newlines.

Regex set:

- `RE_CLASS_HEADER = (?m)^([ \t]*)class\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(([^)]*)\))?\s*:`
- `RE_DEF_HEADER = (?m)^([ \t]*)(async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)`
- `RE_IMPORT_FROM = (?m)^[ \t]*from\s+(\.*[A-Za-z_][A-Za-z0-9_.]*|\.+)\s+import\s+([^\n#]+)`
- `RE_IMPORT_PLAIN = (?m)^[ \t]*import\s+([A-Za-z_][A-Za-z0-9_., ]*)`
- `RE_TYPE_CHECKING_BLOCK = (?m)^[ \t]*if\s+TYPE_CHECKING\s*:`
- `RE_ROUTER_INSTANTIATION = =\s*APIRouter\s*\(`
- `RE_ALL_EXPORTS = (?ms)^__all__\s*=\s*\[(.*?)\]`
- `RE_FIELD = ^([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([^=#\n]+?)(?:\s*=\s*(.+))?\s*(?:#.*)?$`

Classification priority:

1. FastAPI router class (inherits `APIRouter` OR body has `@router.*` / `@app.*` methods)
2. Pydantic model (inherits `BaseModel`/`BaseSettings`)
3. Dataclass (`@dataclass` decorator)
4. pytest test class (name starts with `Test`)
5. FastAPI endpoint function (decorator starts with `router.`/`app.`/`api.`)
6. pytest fixture (`@pytest.fixture` / `@fixture`)
7. pytest test (name starts with `test_`, OR file is named `test_*` / `*_test` with only plain functions)
8. Any remaining class → PythonClass
9. Any remaining top-level function → PythonFunction
10. PythonModule (with `all_exports` + `re_exports` + imports)

Imports classification:

- TYPE_CHECKING guard wins (uses `type_checking_block_ranges`).
- Relative paths or `.`-prefixed → `local`.
- `is_stdlib(first)` covers a closed list (abc, argparse, asyncio, base64, …, zlib).
- Role detection: `fastapi`/`django`/`flask`/`starlette` → `framework`; `pydantic` → `models = PydanticV1` (refined to `PydanticV2` if `ConfigDict`/`model_config` present); `dataclasses`/`attrs`/`attr` → models; `sqlalchemy`/`tortoise`/`peewee`/`asyncpg` → `database`; `pytest` → `testing.pytest`; `hypothesis` → `testing.hypothesis`; `unittest` → `testing.unittest` + std.
- `async_style = source has 'async def '` anywhere.

`__all__` parsing (`extract_all_exports`) recognizes `__all__ = ['a', 'b']` and strips quotes.

Re-exports collection (`collect_re_exports`) recovers `from x import y` statements as formatted strings.

## Updated node_kind.rs — new SymbolKind variants

The original `SymbolKind` was extended with these variants (from `types.rs`):

```rust
pub enum SymbolKind {
    // Universal
    Function, Method, Constructor, Class, Struct, Enum,
    Interface, TypeAlias, Property, Field, Variable, Module,
    // Rust-specific (newly added)
    Impl,
    Macro,
    // Other
    Other,
}
```

The `node_kind.rs` module's `node_kind_for_statement` and `node_kind_for_declaration` mappings remain limited to the TypeScript-supported variants (`Function`, `Class`, `Interface`, `TypeAlias`, `Enum`, `Variable`, plus `Other` for import/export wrappers). `Impl`, `Macro`, `Method`, `Constructor`, `Struct`, `Property`, `Field`, `Module` are populated by language-specific extractors (Rust regex scanner) — not by the TS analyzer.

Updated `Visibility` enum is now consumed by Rust struct/method/field outputs and `FunctionSig.visibility` in utility entries.

The HOC/lazy/zod enrichments live on `ReactComponentEntry` (new fields `is_hoc`, `is_lazy`, `zod_schemas`, `recipe_refs`), populated by `inspection.rs`'s `detect_is_hoc`, `detect_react_lazy`, `detect_zod_schemas`, and `crate::panda::detect_recipe_imports`.

## Updated ts_analyzer.rs — what changed

- `WorkspaceOptions::new(root)` and `WorkspaceOptions::with_tsconfig(mode)` are the new entry points; `TsAnalyzer::from_workspace_opts(opts)` constructs an analyzer with tsconfig auto-discovery or manual/skip.
- `TsAnalyzer::config_hash_for(opts)` produces a stable cache config hash (monokl version + OXC version `"0.128"` + blake3 of tsconfig content).
- `find_tsconfig_from_root(root)` walks up from `root` looking for `tsconfig.json`.
- `looks_like_workspace_alias(specifier)` heuristic: non-`.`, non-`@`, contains `/` → true (used to surface unresolved aliases in `TsData.unresolved_aliases`).
- `analyze` now has a 4-tier cache: persisted-by-mtime/size → in-memory by content hash → persisted-by-content-hash → full parse.
- `parse_full` collects `unresolved_aliases` from `dependencies` where target file is unresolved AND `looks_like_workspace_alias(specifier)`.
- The full parse also captures `line_count` and emits `blocks: Vec<CodeBlock>` alongside symbols/dependencies/exports/JSX (one parse pass produces all outputs).
- `extract_block` skips import statements, derives `node_kind` from declaration via `node_kind_for_declaration` for export wrappers (falls back to `SymbolKind::Other`), and computes a `symbol_signature` via `first_line_signature` for Function nodes only.
- `first_line_signature(code)` returns the first line up to `{`, or `None` if blank.
- JSX extraction (`extract_jsx_from_stmt` / `collect_jsx_from_expr`) now recurses into `JSXElement` children, `ArrowFunctionExpression`, `ConditionalExpression`, `LogicalExpression`, `ParenthesizedExpression`, `SequenceExpression`, and `JSXMemberExpression` (member-expr names captured with dot notation, e.g. `Foo.Bar`).
- Variable and class declarations are extracted from both bare statements and `ExportNamedDeclaration` wrappers via `extract_export_decl`.
- `import_kind.is_type()` re-routes type-only imports into `TsData.type_only_imports`, bypassing the dependency graph.
- `module_export_name_to_string` and `jsx_element_name_to_string` are owned-string conversion helpers used to keep all outputs allocated.

> **Extended in Part 4 §2.** Variable-bound arrow functions, function expressions, and call-wrapped exports (`memo(forwardRef(...))`) now get normalized multi-line signatures via `variable_function_signature` and `wrapped_function_body_start`.

## New fixture: HardcodedStyles.tsx

Path: `crates/monokl/tests/fixtures/small-ts/src/components/HardcodedStyles.tsx`

```tsx
import { css } from '@pandacss/dev';
import { Button } from './Button';

// Intentional hardcoded values — used by the tokens integration test to assert
// that the auditor finds real violations (not just that the schema is intact).
// Also imports `Button` so the refs integration test can verify the
// import-ref classification path on real source.
const styles = css({ color: '#ff0000', padding: '16px' });

export function HardcodedStyles() {
  return <div className={styles}><Button label="hi" /></div>;
}
```

What it tests:

- `mnkl tokens` must surface `#ff0000` AND `16px` as hardcoded values inside a `css(...)` styling context.
- `mnkl refs Button` must classify `Button` here as `import` and `jsx-element`.
- It functions as a load-bearing fixture for the integration test asserting `hardcodedCount >= 1`.

## Integration tests for new commands

Path: `crates/monokl/tests/integration_new_commands.rs`

Helpers:

```rust
fn fixture_root() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/small-ts")
}

fn loupe_bin() -> &'static str { env!("CARGO_BIN_EXE_loupe") }

fn run_ok_json(args: &[&str]) -> serde_json::Value {
    let output = Command::new(loupe_bin())
        .args(args)
        .output()
        .expect("failed to spawn monokl binary");
    assert!(output.status.success(), ...);
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!(...))
}
```

Test cases:

- **`refs_command_returns_json`**: runs `refs Button --root <fixture> --include-tests`. Asserts `symbol == "Button"`, `refs` is array, `totalRefCount` is number, `truncated` is bool, `diagnostics` is array, `totalRefCount >= 1`, AND at least one ref has `refKind == "import"`.

- **`tokens_command_returns_json`**: runs `tokens <fixture>/src --root <fixture>`. Asserts `dir` string, `filesAudited` number, `overallCompliance` number, `byFile` array, `worstOffenders` array, `diagnostics` array, AND `hardcodedCount >= 1` (driven by the `HardcodedStyles.tsx` fixture).

- **`definition_command_returns_json`**: runs `definition Button --root <fixture>`. Asserts `symbol == "Button"`, non-empty `definitions`, AND at least one definition has `isReExport == false`.

- **`diff_command_returns_json`**: runs `diff --base HEAD --head HEAD --root <repo_root>`. Asserts shape of `base`, `head`, `fileDiffs` array, `summary` object, `diagnostics` array, AND `summary.filesChanged == 0`.

- **Schema tests** (calls `assert_schema_kind_returns_valid`):
  - `schema_refs_result` — `refs-result`
  - `schema_diff_result` — `diff-result`
  - `schema_tokens_result` — `tokens-result`
  - `schema_definition_result` — `definition-result`
  - `schema_panda_recipe` — `panda-recipe`
  - `schema_context_module` — `context-module`

Each schema test asserts the returned object has `properties` (object) OR `oneOf` (array).

- **`schema_keys_match_serialized_instance`**: end-to-end drift check.
  1. Runs `inspect <fixture>/src/components/Button.tsx --root <fixture> --kind react-component`.
  2. Extracts the first entry from `entries`.
  3. Runs `schema --kind react-component`.
  4. Asserts every key in `required` exists in the entry object.
  5. Asserts every key in the entry object exists in the schema's `properties`.

This test catches drift between `schema.rs` and `types.rs` in both directions (schema missing a field; type missing a documented field).

## Updated pipeline.rs

New module-level functions added since the original spec:

```rust
pub fn inspect(files: &[Utf8PathBuf], opts: &WorkspaceOptions) -> Result<InspectResult> {
    inspection::inspect_files(files, opts)
}

pub fn patterns(dir: &Utf8Path, opts: &WorkspaceOptions) -> Result<PatternsResult> {
    // … walks dir, inspects every TS/JS file, aggregates counts/patterns/styling/coverage …
}

pub fn refs(
    symbol: &str,
    opts: &WorkspaceOptions,
    include_tests: bool,
    max_refs: usize,
    from_file: Option<&camino::Utf8Path>,
) -> Result<refs::RefsResult> {
    refs::find_refs(symbol, opts, include_tests, max_refs, from_file)
}

pub fn definition(
    symbol: &str,
    opts: &WorkspaceOptions,
    from_file: Option<&camino::Utf8Path>,
) -> Result<definition::DefinitionResult> {
    definition::find_definition(symbol, opts, from_file)
}

pub fn diff(base: &str, head: &str, root: &Utf8Path) -> Result<diff_mod::DiffResult> {
    diff_mod::diff(base, head, root)
}

pub fn tokens(dir: &Utf8Path, opts: &WorkspaceOptions) -> Result<crate::tokens_analysis::TokensResult> {
    crate::tokens_analysis::audit_tokens(dir, opts)
}

pub fn explain(file: &Utf8Path, symbol: Option<&str>, opts: &WorkspaceOptions) -> Result<explain_mod::ExplainResult> {
    explain_mod::explain(file, symbol, opts)
}

pub fn coverage(file: &Utf8Path, opts: &WorkspaceOptions) -> Result<coverage_mod::CoverageResult> {
    coverage_mod::coverage(file, opts)
}

pub fn data_flow(
    file: &Utf8Path,
    symbol: Option<&str>,
    opts: &WorkspaceOptions,
) -> Result<data_flow_mod::DataFlowResult> {
    data_flow_mod::data_flow(file, symbol, opts)
}

pub fn similar(file: &Utf8Path, count: usize, opts: &WorkspaceOptions) -> Result<similar_mod::SimilarResult> {
    similar_mod::similar(file, count, opts)
}
```

Internal helper:

```rust
fn accumulate_toolchain(
    tc: &ToolchainImports,
    build: &mut Option<String>,
    test: &mut Option<String>,
    types: &mut Vec<String>,
) {
    if build.is_none() { if let Some(b) = &tc.build { *build = Some(b.clone()); } }
    if test.is_none()  { if let Some(t) = &tc.test  { *test  = Some(t.clone()); } }
    for t in &tc.types { if !types.contains(t) { types.push(t.clone()); } }
}
```

The existing `symbols` pipeline was updated to also populate a `BTreeMap<Utf8PathBuf, ClassifiedImports> imports` field on `SymbolsResult` via `import_classifier::classify(&analysis.dependencies)` per file. The `dependents` pipeline emits a `Warning` diagnostic when no `tsconfig.json` exists at or above the root, calling out that aliased imports may not resolve.

---
