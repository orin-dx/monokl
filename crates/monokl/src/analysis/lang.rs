//! The contract every language analyzer implements.

use std::sync::Arc;

use camino::Utf8Path;

use super::file_analysis::FileAnalysis;
use crate::error::Result;

/// A language an analyzer handles.
///
/// Variants are ordered as declared, so a `BTreeSet<LanguageId>` iterates in that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum LanguageId {
    TypeScript,
    Rust,
}

/// Turns source files into [`FileAnalysis`] values.
///
/// An instance may handle several languages.
pub trait LanguageAnalyzer: Send + Sync {
    /// Every language this analyzer handles.
    fn languages(&self) -> &[LanguageId];

    /// Whether this analyzer handles the file at `path`.
    fn supports(&self, path: &Utf8Path) -> bool;

    /// Analyzes the file at `path`, whose text is `source`.
    ///
    /// # Errors
    ///
    /// Returns whatever error the implementation hits.
    fn analyze(&self, path: &Utf8Path, source: String) -> Result<Arc<FileAnalysis>>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    use camino::Utf8PathBuf;

    use super::*;
    use crate::error::MonoklError;

    #[test]
    fn language_ids_are_ordered_as_declared() {
        assert!(LanguageId::TypeScript < LanguageId::Rust);
        let set: BTreeSet<_> = [LanguageId::Rust, LanguageId::TypeScript].into();
        assert_eq!(set.into_iter().collect::<Vec<_>>(), [LanguageId::TypeScript, LanguageId::Rust]);
    }

    /// Records what `analyze` receives and returns a canned result for `ok.ts`.
    struct Recorder {
        seen: Mutex<Vec<(Utf8PathBuf, String)>>,
        ok: Arc<FileAnalysis>,
    }

    impl LanguageAnalyzer for Recorder {
        fn languages(&self) -> &[LanguageId] {
            &[LanguageId::TypeScript]
        }
        fn supports(&self, _path: &Utf8Path) -> bool {
            true
        }
        fn analyze(&self, path: &Utf8Path, source: String) -> Result<Arc<FileAnalysis>> {
            self.seen.lock().unwrap().push((path.to_owned(), source));
            if path.file_name() == Some("ok.ts") {
                Ok(Arc::clone(&self.ok))
            } else {
                Err(MonoklError::TooManyTerms { count: 1, limit: 1 })
            }
        }
    }

    fn recorder() -> Recorder {
        let ok =
            FileAnalysis { had_parse_errors: false, degradation: None, blocks: Vec::new(), test_blocks: Vec::new() };
        Recorder { seen: Mutex::new(Vec::new()), ok: Arc::new(ok) }
    }

    #[test]
    fn analyze_passes_the_path_and_text_through_and_returns_the_result() {
        let analyzer = recorder();
        let got = analyzer.analyze(Utf8Path::new("a/ok.ts"), "one".to_owned());
        assert!(Arc::ptr_eq(&got.unwrap(), &analyzer.ok));
        let err = analyzer.analyze(Utf8Path::new("bad.ts"), "two".to_owned()).unwrap_err();
        assert!(matches!(err, MonoklError::TooManyTerms { count: 1, limit: 1 }), "got {err:?}");
        assert_eq!(
            *analyzer.seen.lock().unwrap(),
            [(Utf8PathBuf::from("a/ok.ts"), "one".to_owned()), (Utf8PathBuf::from("bad.ts"), "two".to_owned())]
        );
    }

    #[test]
    fn an_analyzer_is_shareable_across_threads() {
        let analyzer: Arc<dyn LanguageAnalyzer> = Arc::new(recorder());
        let languages = std::thread::spawn(move || analyzer.languages().to_vec()).join().unwrap();
        assert_eq!(languages, [LanguageId::TypeScript]);
    }
}
