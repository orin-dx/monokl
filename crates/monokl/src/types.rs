//! Monokl's core data types: symbols, dependencies, per-language data and search results.
use camino::Utf8PathBuf;

/// The classification of a symbol.
///
/// Most variants apply to every language analyzer; `Impl` and `Macro` are Rust-specific, and `Other` is the catch-all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Visibility {
    Public,
    Crate,
    Module,
    Private,
}

/// A symbol found in a source file.
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub signature: Option<String>,
    /// The type that owns this symbol, such as the target of the enclosing `impl` block.
    pub owner: Option<String>,
    pub trait_impl: Option<String>,
    pub visibility: Option<Visibility>,
    pub kind_detail: Option<String>,
}

/// The kind of a single import binding within a [`DependencyBinding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BindingKind {
    Named,
    Default,
    Namespace,
    Glob,
}

/// The resolved target of a dependency statement.
///
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DependencyTarget {
    File { specifier: String, resolved: Option<Utf8PathBuf>, is_relative: bool },
}

/// A single binding introduced by a [`DependencyRecord`].
#[derive(Debug, Clone)]
pub struct DependencyBinding {
    pub imported: String,
    pub local: String,
    pub kind: BindingKind,
}

/// A single dependency statement in a source file.
#[derive(Debug, Clone)]
pub struct DependencyRecord {
    pub line: usize,
    pub bindings: Vec<DependencyBinding>,
    pub target: DependencyTarget,
}

/// A single named export statement.
#[derive(Debug, Clone)]
pub struct ExportRecord {
    pub name: String,
    pub line: usize,
    pub re_export: bool,
}

/// A single JSX attribute.
#[derive(Debug, Clone)]
pub struct JsxAttribute {
    pub name: String,
    pub string_value: Option<String>,
    pub is_expression: bool,
    pub is_spread: bool,
}

/// A single JSX element usage.
#[derive(Debug, Clone)]
pub struct JsxElementEntry {
    pub name: String,
    pub is_html: bool,
    pub line: usize,
    pub attributes: Vec<JsxAttribute>,
}

/// TypeScript-specific analysis output.
#[derive(Debug, Clone, Default)]
pub struct TsData {
    pub jsx_elements: Vec<JsxElementEntry>,
    pub type_only_imports: Vec<String>,
    pub unresolved_aliases: Vec<String>,
}

/// Per-language analysis output.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LangData {
    Ts(TsData),
}

impl LangData {
    /// Returns the TypeScript data.
    pub fn ts(&self) -> &TsData {
        match self {
            LangData::Ts(ts) => ts,
        }
    }

    /// Returns the JSX elements of the TypeScript data.
    pub fn jsx_elements(&self) -> &[JsxElementEntry] {
        &self.ts().jsx_elements
    }
}

/// A block of code matched by the search pipeline.
#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub file: Utf8PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub node_kind: SymbolKind,
    pub code: String,
    pub symbol_signature: Option<String>,
    pub matched_lines: Vec<usize>,
    pub matched_keywords: Vec<String>,
    /// The block this one is a member of, such as the class of a method; `None` for a top-level block.
    pub parent: Option<ParentContext>,
}

/// The symbol enclosing a [`CodeBlock`], such as the class or object literal a method block belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentContext {
    pub kind: SymbolKind,
    pub name: String,
    pub line: usize,
}

/// A [`CodeBlock`] with its ranking scores.
#[derive(Debug, Clone)]
pub struct RankedBlock {
    pub block: CodeBlock,
    pub bm25_score: f64,
    pub coverage_boost: f64,
    pub node_type_boost: f64,
    pub final_score: f64,
    pub rank: usize,
    pub parent_context: Option<ParentContext>,
}

/// The kind of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticKind {
    Degraded,
    Skipped,
    Warning,
    /// The file held bytes that are not UTF-8, so it was searched and extracted with each invalid sequence replaced by
    /// U+FFFD.
    LossyRead,
}

/// A non-fatal problem reported alongside a command's results.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub path: Option<Utf8PathBuf>,
    pub message: String,
}

/// A single line matched by the text searcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineHit {
    pub line_number: usize,
    pub text: String,
}

/// The language filter for the search command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The search command's response.
#[derive(Debug, Clone)]
pub struct SearchResponse {
    pub results: Vec<RankedBlock>,
    pub total_blocks_before_truncation: usize,
    pub truncated: bool,
    pub total_bytes: usize,
    pub total_tokens: usize,
    pub diagnostics: Vec<Diagnostic>,
}
