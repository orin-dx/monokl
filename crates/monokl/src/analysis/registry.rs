//! The ordered set of analyzers that decides which one owns a path.

use std::sync::Arc;

use camino::Utf8Path;

use super::lang::{LanguageAnalyzer, LanguageId};

/// Analyzers in registration order.
///
/// Callers ask the registry which analyzer owns a file instead of naming a concrete analyzer.
///
/// # Examples
///
/// ```
/// use monokl::analysis::registry::AnalyzerRegistry;
///
/// let registry = AnalyzerRegistry::new(Vec::new());
/// assert!(registry.analyzer_for("a.ts".into()).is_none());
/// ```
pub struct AnalyzerRegistry {
    analyzers: Vec<Arc<dyn LanguageAnalyzer>>,
}

impl AnalyzerRegistry {
    /// Builds a registry that consults `analyzers` in the given order.
    pub fn new(analyzers: Vec<Arc<dyn LanguageAnalyzer>>) -> Self {
        Self { analyzers }
    }

    /// Returns the first registered analyzer whose [`LanguageAnalyzer::supports`] accepts `path`.
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::analysis::registry::AnalyzerRegistry;
    ///
    /// assert!(AnalyzerRegistry::new(Vec::new()).analyzer_for("a.ts".into()).is_none());
    /// ```
    pub fn analyzer_for(&self, path: &Utf8Path) -> Option<&dyn LanguageAnalyzer> {
        self.analyzers.iter().find(|analyzer| analyzer.supports(path)).map(Arc::as_ref)
    }

    /// Whether any registered analyzer handles `language`.
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::analysis::LanguageId;
    /// use monokl::analysis::registry::AnalyzerRegistry;
    ///
    /// assert!(!AnalyzerRegistry::new(Vec::new()).handles(LanguageId::Rust));
    /// ```
    pub fn handles(&self, language: LanguageId) -> bool {
        self.analyzers.iter().any(|analyzer| analyzer.languages().contains(&language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::file_analysis::FileAnalysis;
    use crate::analysis::lang::LanguageId;
    use crate::error::{MonoklError, Result};

    /// Claims every path ending in `extension` and reports its one language for it.
    struct ByExtension {
        extension: &'static str,
        languages: [LanguageId; 1],
    }

    impl LanguageAnalyzer for ByExtension {
        fn languages(&self) -> &[LanguageId] {
            &self.languages
        }
        fn supports(&self, path: &Utf8Path) -> bool {
            path.extension() == Some(self.extension)
        }
        fn analyze(&self, _path: &Utf8Path, _source: String) -> Result<Arc<FileAnalysis>> {
            Err(MonoklError::TooManyTerms { count: 0, limit: 0 })
        }
    }

    fn analyzer(extension: &'static str, language: LanguageId) -> Arc<dyn LanguageAnalyzer> {
        Arc::new(ByExtension { extension, languages: [language] })
    }

    #[test]
    fn first_registered_match_owns_the_path() {
        let registry = AnalyzerRegistry::new(vec![
            analyzer("ts", LanguageId::TypeScript),
            analyzer("rs", LanguageId::Rust),
            analyzer("ts", LanguageId::Rust),
        ]);
        let owner = registry.analyzer_for(Utf8Path::new("src/a.ts")).unwrap();
        assert_eq!(owner.languages(), [LanguageId::TypeScript]);
        let owner = registry.analyzer_for(Utf8Path::new("src/lib.rs")).unwrap();
        assert_eq!(owner.languages(), [LanguageId::Rust]);
    }

    #[test]
    fn unowned_path_has_no_analyzer() {
        let registry = AnalyzerRegistry::new(vec![analyzer("ts", LanguageId::TypeScript)]);
        assert!(registry.analyzer_for(Utf8Path::new("README.md")).is_none());
        assert!(AnalyzerRegistry::new(Vec::new()).analyzer_for(Utf8Path::new("a.ts")).is_none());
    }
}
