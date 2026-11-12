//! The ordered set of analyzers that decides which one owns a path.

use std::sync::Arc;

use camino::Utf8Path;

use super::lang::{Language, LanguageAnalyzer};

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
    /// use monokl::analysis::Language;
    /// use monokl::analysis::registry::AnalyzerRegistry;
    ///
    /// assert!(!AnalyzerRegistry::new(Vec::new()).handles(Language::Rust));
    /// ```
    pub fn handles(&self, language: Language) -> bool {
        self.analyzers.iter().any(|analyzer| analyzer.languages().contains(&language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::file_analysis::FileAnalysis;
    use crate::analysis::lang::Language;
    use crate::error::{MonoklError, Result};

    /// Claims every path ending in `extension` and reports its one language for it.
    struct ByExtension {
        extension: &'static str,
        languages: [Language; 1],
    }

    impl LanguageAnalyzer for ByExtension {
        fn languages(&self) -> &[Language] {
            &self.languages
        }
        fn supports(&self, path: &Utf8Path) -> bool {
            path.extension() == Some(self.extension)
        }
        fn analyze(&self, _path: &Utf8Path, _source: String) -> Result<Arc<FileAnalysis>> {
            Err(MonoklError::TooManyTerms { count: 0, limit: 0 })
        }
    }

    fn analyzer(extension: &'static str, language: Language) -> Arc<dyn LanguageAnalyzer> {
        Arc::new(ByExtension { extension, languages: [language] })
    }

    #[test]
    fn first_registered_match_owns_the_path() {
        let registry = AnalyzerRegistry::new(vec![
            analyzer("ts", Language::TypeScript),
            analyzer("rs", Language::Rust),
            analyzer("ts", Language::Rust),
        ]);
        let owner = registry.analyzer_for(Utf8Path::new("src/a.ts")).unwrap();
        assert_eq!(owner.languages(), [Language::TypeScript]);
        let owner = registry.analyzer_for(Utf8Path::new("src/lib.rs")).unwrap();
        assert_eq!(owner.languages(), [Language::Rust]);
    }

    #[test]
    fn unowned_path_has_no_analyzer() {
        let registry = AnalyzerRegistry::new(vec![analyzer("ts", Language::TypeScript)]);
        assert!(registry.analyzer_for(Utf8Path::new("README.md")).is_none());
        assert!(AnalyzerRegistry::new(Vec::new()).analyzer_for(Utf8Path::new("a.ts")).is_none());
    }

    /// The built-in analyzers own a path exactly when `Language::of` names a language, and that language is one of
    /// the owner's.
    #[cfg(all(feature = "lang-ts", feature = "lang-rust"))]
    #[test]
    fn the_built_in_analyzers_agree_with_the_language_rules() {
        use crate::analysis::rust::RustAnalyzer;
        use crate::analysis::ts::TsAnalyzer;

        let registry = AnalyzerRegistry::new(vec![Arc::new(TsAnalyzer::new()), Arc::new(RustAnalyzer::new())]);
        let mut paths: Vec<String> = Language::ALL
            .iter()
            .flat_map(|language| language.rules().extensions)
            .flat_map(|extension| [format!("a.{extension}"), format!("src/a.test.{extension}")])
            .collect();
        paths.extend(
            ["a.d.ts", "a.d.mts", "a.d.cts", "a.d.js", "a.py", "a.json", "a.TS", "a.ts.map", ".ts", "README.md"]
                .map(str::to_owned),
        );
        for path in &paths {
            let path = Utf8Path::new(path);
            let handled = registry.analyzer_for(path).map(|analyzer| analyzer.languages().to_vec());
            match (handled, Language::of(path)) {
                (None, None) => {}
                (Some(handled), Some(language)) => {
                    assert!(handled.contains(&language), "{path}: {handled:?} vs {language:?}");
                }
                (handled, language) => panic!("{path}: analyzer {handled:?} but language {language:?}"),
            }
        }
        for language in Language::ALL {
            assert!(registry.handles(language), "{language:?}");
        }
    }
}
