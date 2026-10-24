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

    /// Returns whether the analyzer that owns `path` reports one of `languages` for it.
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::analysis::LanguageId;
    /// use monokl::analysis::registry::AnalyzerRegistry;
    ///
    /// let registry = AnalyzerRegistry::new(Vec::new());
    /// assert!(!registry.supports_language("a.ts".into(), &[LanguageId::TypeScript]));
    /// ```
    pub fn supports_language(&self, path: &Utf8Path, languages: &[LanguageId]) -> bool {
        self.analyzer_for(path)
            .and_then(|analyzer| analyzer.language_for(path))
            .is_some_and(|language| languages.contains(&language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::file_analysis::FileAnalysis;
    use crate::analysis::lang::{AnalysisProfile, AnalyzerCapabilities, CapabilityPrecision};
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
        fn language_for(&self, path: &Utf8Path) -> Option<LanguageId> {
            self.supports(path).then_some(self.languages[0])
        }
        fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
            AnalyzerCapabilities {
                blocks: true,
                classified_imports: false,
                resolved_import_graph: CapabilityPrecision::Unsupported,
                refs: CapabilityPrecision::Unsupported,
                definition: CapabilityPrecision::Unsupported,
                inspect_detail: CapabilityPrecision::Unsupported,
            }
        }
        fn analyze_with_profile(
            &self,
            _path: &Utf8Path,
            _source: Box<dyn FnOnce() -> Result<String>>,
            _profile: AnalysisProfile,
        ) -> Result<Arc<FileAnalysis>> {
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
            analyzer("ts", LanguageId::Go),
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

    #[test]
    fn supports_language_checks_the_owning_analyzers_language() {
        let registry =
            AnalyzerRegistry::new(vec![analyzer("ts", LanguageId::TypeScript), analyzer("rs", LanguageId::Rust)]);
        let ts = Utf8Path::new("a.ts");
        assert!(registry.supports_language(ts, &[LanguageId::TypeScript]));
        assert!(registry.supports_language(ts, &[LanguageId::Go, LanguageId::TypeScript]));
        assert!(!registry.supports_language(ts, &[LanguageId::Rust]));
        assert!(!registry.supports_language(ts, &[]));
        assert!(!registry.supports_language(Utf8Path::new("a.md"), &[LanguageId::TypeScript]));
    }
}
