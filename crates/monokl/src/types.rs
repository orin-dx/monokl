//! Monokl's core data types: code blocks, ranked results and diagnostics.
use camino::Utf8PathBuf;

/// The classification of a symbol.
///
/// Ordering is declaration order: ranking breaks ties by it, so reordering the variants reorders equal-score results.
///
/// Most variants apply to every language analyzer; `Impl` and `Macro` are Rust-specific, and `Other` is the catch-all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

impl SymbolKind {
    /// The `camelCase` name agent output prints for this kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::TypeAlias => "typeAlias",
            Self::Property => "property",
            Self::Field => "field",
            Self::Variable => "variable",
            Self::Module => "module",
            Self::Impl => "impl",
            Self::Macro => "macro",
            Self::Other => "other",
        }
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
    /// The block this one is a member of, such as the class of a method; `None` for a top-level block.
    pub parent: Option<ParentContext>,
}

/// The symbol enclosing a [`CodeBlock`], such as the class or object literal a method block belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentContext {
    pub kind: SymbolKind,
    pub name: String,
}

/// A [`CodeBlock`] with its ranking scores.
#[derive(Debug, Clone)]
pub struct RankedBlock {
    pub block: CodeBlock,
    pub final_score: f64,
    pub rank: usize,
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

impl DiagnosticKind {
    /// What happened to a file this kind of diagnostic names, as a verb phrase for output.
    #[must_use]
    pub fn verb(self) -> &'static str {
        match self {
            Self::Degraded => "degraded",
            Self::Skipped => "skipped",
            Self::Warning => "warned",
            Self::LossyRead => "read lossily",
        }
    }
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

/// The search command's response.
#[derive(Debug, Clone)]
pub struct SearchResponse {
    pub results: Vec<RankedBlock>,
    pub total_blocks_before_truncation: usize,
    pub truncated: bool,
    pub diagnostics: Vec<Diagnostic>,
}
