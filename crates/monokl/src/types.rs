//! Monokl's core data types: symbols, dependencies, per-language data, search results, and command options.
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// The classification of a symbol.
///
/// Most variants apply to every language analyzer; `Impl` and `Macro` are Rust-specific, and `Other` is the catch-all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SymbolKind {
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
    Impl,
    Macro,
    Other,
}

/// The visibility of a symbol.
///
/// There is no `Super` variant: Rust's `pub(super)` and `pub(in ...)` map to [`Visibility::Module`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Visibility {
    Public,
    Crate,
    Module,
    Private,
}

/// A symbol found in a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub signature: Option<String>,
    /// The type that owns this symbol, such as the target of the enclosing `impl` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_impl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_detail: Option<String>,
}

/// The kind of a single import binding within a [`DependencyBinding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BindingKind {
    Named,
    Default,
    Namespace,
    Glob,
    NamespaceWide,
}

/// The anchor of a resolved Rust module path.
///
/// `Selff` is renamed to `"self"` on the wire because `self` is a keyword and cannot name a variant.
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

/// The resolved target of a dependency statement.
///
/// Internally tagged on `"kind"`, so each variant's fields sit beside the tag in one JSON object. `rename_all` renames
/// only the tags; the fields stay `snake_case` (`is_relative`, `is_static`). No analyzer constructs `Namespace` yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[non_exhaustive]
pub enum DependencyTarget {
    File { specifier: String, resolved: Option<Utf8PathBuf>, is_relative: bool },
    RustPath { segments: Vec<String>, anchor: RustPathAnchor, resolved: Option<Utf8PathBuf> },
    Namespace { segments: Vec<String>, is_static: bool, alias: Option<String> },
}

/// A single binding introduced by a [`DependencyRecord`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyBinding {
    pub imported: String,
    pub local: String,
    pub kind: BindingKind,
}

/// A single dependency statement in a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRecord {
    pub line: usize,
    /// Always serialized, even when empty; a missing key deserializes to an empty vec. Serde gives a `Vec` field no
    /// missing-key leniency without `#[serde(default)]`.
    #[serde(default)]
    pub bindings: Vec<DependencyBinding>,
    pub target: DependencyTarget,
}

/// A single named export statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    pub name: String,
    pub line: usize,
    pub re_export: bool,
}

/// A single JSX attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsxAttribute {
    pub name: String,
    pub string_value: Option<String>,
    pub is_expression: bool,
    pub is_spread: bool,
}

/// A single JSX element usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsxElementEntry {
    pub name: String,
    pub is_html: bool,
    pub line: usize,
    /// Required on deserialize, unlike [`DependencyRecord::bindings`].
    pub attributes: Vec<JsxAttribute>,
}

/// Rust-specific analysis output, empty until the Rust analyzer lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RustData {}

/// Python-specific analysis output, empty until the Python analyzer lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonData {}

/// Go-specific analysis output, empty until the Go analyzer lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoData {}

/// Java-specific analysis output, empty until the Java analyzer lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaData {}

/// TypeScript-specific analysis output.
///
/// Every field tolerates a missing key, but only `unresolved_aliases` is omitted from output when empty.
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

/// Per-language analysis output.
///
/// Adjacently tagged, so `Ts` serializes as `{"language":"typescript","data":{...}}`. An unrecognized `"language"`
/// fails to deserialize.
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
    /// Returns the TypeScript data if this is [`LangData::Ts`].
    pub fn ts(&self) -> Option<&TsData> {
        if let LangData::Ts(ts) = self { Some(ts) } else { None }
    }

    /// Returns the JSX elements for TypeScript data, or an empty slice for any other language.
    pub fn jsx_elements(&self) -> &[JsxElementEntry] {
        self.ts().map_or(&[], |ts| ts.jsx_elements.as_slice())
    }
}

/// A block of code matched by the search pipeline.
///
/// Deserializing does not check that `line_start <= line_end` or that `matched_lines` falls in range, so consumers
/// must not assume either.
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

/// The symbol enclosing a [`CodeBlock`] match.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentContext {
    pub kind: SymbolKind,
    pub name: String,
    pub line: usize,
}

/// A [`CodeBlock`] with its ranking scores.
///
/// The block's fields are flattened into the same JSON object. `serde_json` writes a non-finite score as `null`.
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

/// The kind of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum DiagnosticKind {
    Degraded,
    Skipped,
    Warning,
}

/// A non-fatal problem reported alongside a command's results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub path: Option<Utf8PathBuf>,
    pub message: String,
}

/// A single line matched by the text searcher.
///
/// It is internal and deliberately has no serde derives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineHit {
    pub line_number: usize,
    pub text: String,
}

/// The language filter for the search command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[non_exhaustive]
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

/// Resource limits for the search command.
///
/// The [`Default`] values do not apply when deserializing: `max_bytes` and `max_candidates` are required, and a
/// missing `max_results` or `max_tokens` becomes `None`.
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
        Self { max_results: Some(50), max_bytes: 2_097_152, max_tokens: Some(20_000), max_candidates: 1_000 }
    }
}

/// Request options for the search command.
///
/// As with [`SearchLimits`], the [`Default`] values do not apply when deserializing; every field except `language`
/// is required.
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

/// The search command's response.
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

/// The symbols command's response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolsResult {
    /// A `BTreeMap` so files serialize in a deterministic order.
    pub files: std::collections::BTreeMap<Utf8PathBuf, Vec<SymbolEntry>>,
    pub total_symbol_count: usize,
    pub truncation_marker: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The dependents command's response.
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

/// How a workspace locates its `tsconfig.json`.
#[derive(Debug, Clone)]
pub enum TsconfigMode {
    Auto,
    Manual(Utf8PathBuf),
    Skip,
}

/// Options for building a workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceOptions {
    pub root: Utf8PathBuf,
    pub tsconfig: TsconfigMode,
}

impl WorkspaceOptions {
    /// Creates options for `root` with [`TsconfigMode::Auto`]; use [`with_tsconfig`](Self::with_tsconfig) to change it.
    pub fn new(root: impl Into<Utf8PathBuf>) -> Self {
        Self { root: root.into(), tsconfig: TsconfigMode::Auto }
    }

    /// Sets the tsconfig mode.
    #[must_use]
    pub fn with_tsconfig(mut self, mode: TsconfigMode) -> Self {
        self.tsconfig = mode;
        self
    }
}

/// A request to extract a file, optionally limited to a line range.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRequest {
    pub file: Utf8PathBuf,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
}
