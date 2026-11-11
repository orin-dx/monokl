//! The Rust analyzer.

mod parse;

use std::sync::Arc;

use camino::Utf8Path;

use super::file_analysis::FileAnalysis;
use super::lang::{LanguageAnalyzer, LanguageId};
use crate::error::Result;

const LANGUAGES: [LanguageId; 1] = [LanguageId::Rust];

/// Analyzes Rust files with rust-analyzer's error-tolerant syntax crate.
///
/// It is stateless: every call parses its own source and returns owned data. Items under `#[cfg(test)]` or `#[test]`
/// are test code; their blocks are in [`FileAnalysis::test_blocks`], not [`FileAnalysis::blocks`].
///
/// # Examples
///
/// ```
/// use camino::Utf8Path;
/// use monokl::analysis::LanguageAnalyzer;
/// use monokl::analysis::rust::RustAnalyzer;
///
/// let analyzer = RustAnalyzer::new();
/// assert!(analyzer.supports(Utf8Path::new("src/lib.rs")));
/// assert!(!analyzer.supports(Utf8Path::new("src/app.ts")));
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct RustAnalyzer;

impl RustAnalyzer {
    /// Creates an analyzer.
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::analysis::rust::RustAnalyzer;
    ///
    /// let _analyzer = RustAnalyzer::new();
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl LanguageAnalyzer for RustAnalyzer {
    fn languages(&self) -> &[LanguageId] {
        &LANGUAGES
    }

    fn supports(&self, path: &Utf8Path) -> bool {
        path.extension() == Some("rs")
    }

    fn analyze(&self, path: &Utf8Path, source: String) -> Result<Arc<FileAnalysis>> {
        let parse::Extraction { blocks, test_blocks, had_parse_errors, degradation } = parse::extract(path, &source);
        Ok(Arc::new(FileAnalysis { had_parse_errors, degradation, blocks, test_blocks }))
    }
}

#[cfg(all(test, feature = "lang-ts"))]
pub(crate) use parse::MAX_NESTING_LEVELS;

/// Parses and extracts `source` on the calling thread, skipping the cap, for a soundness test that sets its own stack.
#[cfg(all(test, feature = "lang-ts"))]
pub(crate) fn parse_on_stack(source: &str) {
    parse::parse_here(source);
}

#[cfg(test)]
mod tests;
