//! Everything extracted from one source file.

use std::fmt;

use camino::Utf8PathBuf;

use super::lang::AnalysisProfile;
use crate::digest::Digest;
use crate::types::{CodeBlock, DependencyRecord, ExportRecord, JsxElementEntry, LangData, SymbolEntry};

/// Why a file was analyzed without its content: the analyzer gave up on it instead of reading it.
///
/// A file with syntax errors is not degraded this way; its recovered content is kept and only
/// [`FileAnalysis::had_parse_errors`] is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Degradation {
    /// The source is longer than the analyzer will parse.
    TooLarge {
        /// The length of the source.
        bytes: usize,
        /// The longest source the analyzer parses.
        cap: usize,
    },
    /// The parse or extraction panicked.
    Panicked,
    /// The analyzer could not start the thread the parse runs on.
    ThreadUnavailable,
    /// The syntax tree nests deeper than the analyzer can drop safely.
    TooDeep {
        /// The depth of the tree.
        depth: usize,
        /// The deepest tree the analyzer keeps.
        cap: usize,
    },
}

impl fmt::Display for Degradation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, cap } => {
                write!(f, "file too large to parse ({bytes} bytes, over the cap of {cap})")
            }
            Self::Panicked => f.write_str("parser panicked"),
            Self::ThreadUnavailable => f.write_str("could not start a parse thread"),
            Self::TooDeep { depth, cap } => {
                write!(f, "file nests too deeply to analyze (depth {depth}, over the cap of {cap})")
            }
        }
    }
}

/// The result of analyzing one file.
///
/// It owns all its data, so it can outlive the parse that produced it. Under [`AnalysisProfile::Dependencies`] only
/// `dependencies` is filled; under [`AnalysisProfile::Structural`] `blocks` is empty. `profile` records which, so a
/// cache never serves a partial result as a full one.
#[derive(Clone, Debug)]
pub struct FileAnalysis {
    pub source_path: Utf8PathBuf,
    pub digest: Digest,
    pub profile: AnalysisProfile,
    /// Set for any file that did not parse cleanly, including every file with a [`Degradation`].
    pub had_parse_errors: bool,
    /// Why the file's content is missing, when the analyzer gave up on it.
    pub degradation: Option<Degradation>,
    pub symbols: Vec<SymbolEntry>,
    pub dependencies: Vec<DependencyRecord>,
    pub exports: Vec<ExportRecord>,
    pub blocks: Vec<CodeBlock>,
    /// Blocks of test-only code (Rust `#[cfg(test)]` and `#[test]` items), kept apart from `blocks` so a search can leave
    /// them out. Empty for a language with no such code and under profiles other than [`AnalysisProfile::Full`].
    pub test_blocks: Vec<CodeBlock>,
    pub lang: LangData,
}

impl FileAnalysis {
    /// The language name.
    #[must_use]
    pub fn language(&self) -> &str {
        match &self.lang {
            LangData::Ts(_) => "typescript",
            LangData::Rust => "rust",
        }
    }

    /// The JSX elements of the file.
    #[must_use]
    pub fn jsx_elements(&self) -> &[JsxElementEntry] {
        self.lang.jsx_elements()
    }

    /// Whether this is a TypeScript file.
    #[must_use]
    pub fn is_typescript(&self) -> bool {
        matches!(self.lang, LangData::Ts(_))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::types::TsData;

    fn analysis(lang: LangData) -> FileAnalysis {
        FileAnalysis {
            source_path: Utf8PathBuf::from("src/a.ts"),
            digest: Digest::blake3(b"x"),
            profile: AnalysisProfile::Structural,
            had_parse_errors: true,
            degradation: None,
            symbols: Vec::new(),
            dependencies: Vec::new(),
            exports: Vec::new(),
            blocks: Vec::new(),
            test_blocks: Vec::new(),
            lang,
        }
    }

    fn jsx(name: &str) -> JsxElementEntry {
        JsxElementEntry { name: name.to_string(), is_html: false, line: 1, attributes: Vec::new() }
    }

    #[test]
    fn a_typescript_analysis_names_its_language() {
        let analysis = analysis(LangData::Ts(TsData::default()));
        assert_eq!(analysis.language(), "typescript");
        assert!(analysis.is_typescript());
        assert!(analysis.jsx_elements().is_empty());
    }

    #[rstest]
    #[case(Degradation::TooLarge { bytes: 300_000, cap: 260_096 }, "file too large to parse (300000 bytes, over the cap of 260096)")]
    #[case(Degradation::Panicked, "parser panicked")]
    #[case(Degradation::ThreadUnavailable, "could not start a parse thread")]
    #[case(Degradation::TooDeep { depth: 9_000, cap: 8_000 }, "file nests too deeply to analyze (depth 9000, over the cap of 8000)")]
    fn a_degradation_names_its_reason(#[case] degradation: Degradation, #[case] text: &str) {
        assert_eq!(degradation.to_string(), text);
    }

    #[test]
    fn typescript_files_expose_their_jsx_elements_in_order() {
        let ts = TsData { jsx_elements: vec![jsx("App"), jsx("Button")], ..TsData::default() };
        let analysis = analysis(LangData::Ts(ts));
        let names: Vec<_> = analysis.jsx_elements().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["App", "Button"]);
    }

    fn assert_owned<T: Clone + std::fmt::Debug + Send + Sync + 'static>(_: &T) {}

    #[test]
    fn a_file_analysis_is_owned_and_clones_faithfully() {
        let original = analysis(LangData::Ts(TsData::default()));
        assert_owned(&original);
        let clone = original.clone();
        assert_eq!(clone.source_path, original.source_path);
        assert_eq!(clone.digest, original.digest);
        assert_eq!(clone.profile, AnalysisProfile::Structural);
        assert_eq!(clone.had_parse_errors, original.had_parse_errors);
        assert_eq!(clone.degradation, original.degradation);
        assert_eq!(format!("{clone:?}"), format!("{original:?}"));
    }
}
