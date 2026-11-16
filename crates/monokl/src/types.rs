//! Monokl's core data types: code blocks, ranked results and diagnostics.
use std::ops::Range;

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
    /// The bytes of the analyzed source this block covers, so `source[span] == code`. Offsets count bytes of the text the
    /// analyzer received, which is the file's lossy UTF-8 decoding and differs from the file's own bytes after any
    /// invalid sequence.
    pub span: Range<usize>,
    /// The position of the enclosing block in the file's block list, which is
    /// [`FileAnalysis::blocks`](crate::analysis::FileAnalysis::blocks) followed by
    /// [`FileAnalysis::test_blocks`](crate::analysis::FileAnalysis::test_blocks). `Some` exactly when `parent` is, the
    /// enclosing block's span contains this one's, and its position is lower, so the blocks of a file form a forest.
    pub parent_index: Option<usize>,
    /// The identifier the block declares, as the analyzer read it from the syntax tree: `mergeConfig` for
    /// `function mergeConfig(a, b)`, `Display for Foo` for `impl Display for Foo`. `None` for a block that declares no
    /// name, such as a re-export, an anonymous default export or a destructuring declaration.
    pub name: Option<String>,
    pub code: String,
    pub symbol_signature: Option<String>,
    /// The bytes of the declaration's head, which `symbol_signature` is the one-line form of: from the first byte after
    /// any decorators, attributes and doc comments (so a modifier such as `export` or `pub` is inside) through the
    /// opening `{` of a block body, or, for a declaration with no body, through its last byte. A body that is an
    /// expression, or an initializer after `=`, is not part of it. `Some` exactly when `symbol_signature` is, and
    /// within `span`. The snippet leaves out a matched line whose text lies inside it, because the signature already
    /// shows that line.
    pub signature_span: Option<Range<usize>>,
    /// The offset of the `where` keyword of the declaration's where clause, as the Rust analyzer read it from the syntax
    /// tree; `None` when there is no where clause, and always for TypeScript and JavaScript. When `Some`, it lies inside
    /// `signature_span`, and the clause runs to the end of the head. The signature column shows the signature up to this
    /// offset, so the snippet may show a matched line of the clause.
    pub where_start: Option<usize>,
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
    /// The byte offsets in `text` at which a match starts, in ascending order, as ranges of consecutive offsets: a match
    /// at offset 4 is `4..5` and one at each of 4, 5 and 6 is `4..7`. Where a match ends is not recorded. A line cut to
    /// the searcher's length limit records only the matches that start before the cut, so a hit whose matches all lie
    /// past it has none.
    pub match_starts: Vec<Range<usize>>,
}

/// The search command's response.
#[derive(Debug, Clone)]
pub struct SearchResponse {
    pub results: Vec<RankedBlock>,
    /// The score of the best row of the whole ranking, before any cut or offset, so the first row of a later page can
    /// still be shown as a share of it; 0 when nothing ranked.
    pub top_score: f64,
    pub total_blocks_before_truncation: usize,
    /// How many of the blocks left out of `results` the relevance cut dropped; the row and token caps dropped the rest.
    pub blocks_cut_by_relevance: usize,
    /// How many rows that passed the relevance cut were skipped before `results` starts: the page offset asked for.
    pub offset: usize,
    pub truncated: bool,
    pub diagnostics: Vec<Diagnostic>,
}
