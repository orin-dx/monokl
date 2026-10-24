//! The contract every language analyzer implements.

use std::sync::Arc;

use camino::Utf8Path;

use super::file_analysis::FileAnalysis;
use crate::error::Result;

/// A language monokl can name.
///
/// Variants are ordered as declared, so a `BTreeSet<LanguageId>` iterates in that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum LanguageId {
    TypeScript,
    Rust,
    Python,
    Go,
    Java,
    C,
    Cpp,
    Bash,
    CSharp,
    Ruby,
    Php,
    Kotlin,
    Scala,
}

/// How well a capability can be answered, from weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityPrecision {
    Unsupported,
    Heuristic,
    Structural,
    Exact,
}

/// What one analyzer can do for one language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalyzerCapabilities {
    pub blocks: bool,
    pub classified_imports: bool,
    pub resolved_import_graph: CapabilityPrecision,
    pub refs: CapabilityPrecision,
    pub definition: CapabilityPrecision,
    pub inspect_detail: CapabilityPrecision,
}

/// How much of a file to extract.
///
/// `Dependencies` fills only `dependencies`. `Structural` adds `symbols` and `exports`. `Full` adds `blocks` and the
/// per-language entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnalysisProfile {
    Dependencies,
    Structural,
    Full,
}

/// Turns source files into [`FileAnalysis`] values.
///
/// An instance may handle several languages. [`language_for`](Self::language_for) is `None` exactly when
/// [`supports`](Self::supports) is `false`, and [`capabilities`](Self::capabilities) is only asked about languages
/// the instance returned from [`languages`](Self::languages) or [`language_for`](Self::language_for).
pub trait LanguageAnalyzer: Send + Sync {
    /// Every language this analyzer handles.
    fn languages(&self) -> &[LanguageId];

    /// Whether this analyzer handles the file at `path`.
    fn supports(&self, path: &Utf8Path) -> bool;

    /// The language `path` belongs to, or `None` when [`supports`](Self::supports) is `false`.
    fn language_for(&self, path: &Utf8Path) -> Option<LanguageId>;

    /// What this analyzer can do for `language`.
    fn capabilities(&self, language: LanguageId) -> AnalyzerCapabilities;

    /// Analyzes one file, extracting what `profile` asks for. `source` is only called if the file needs reading.
    ///
    /// # Errors
    ///
    /// Returns whatever error the implementation hits, including the one `source` returns.
    fn analyze_with_profile(
        &self,
        path: &Utf8Path,
        source: Box<dyn FnOnce() -> Result<String>>,
        profile: AnalysisProfile,
    ) -> Result<Arc<FileAnalysis>>;

    /// Analyzes one file at full fidelity.
    ///
    /// # Errors
    ///
    /// As [`analyze_with_profile`](Self::analyze_with_profile).
    fn analyze(&self, path: &Utf8Path, source: Box<dyn FnOnce() -> Result<String>>) -> Result<Arc<FileAnalysis>> {
        self.analyze_with_profile(path, source, AnalysisProfile::Full)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::{BTreeSet, HashMap};
    use std::rc::Rc;
    use std::sync::Mutex;

    use camino::Utf8PathBuf;

    use super::*;
    use crate::digest::Digest;
    use crate::error::MonoklError;
    use crate::types::{LangData, TsData};

    const ALL: [LanguageId; 13] = [
        LanguageId::TypeScript,
        LanguageId::Rust,
        LanguageId::Python,
        LanguageId::Go,
        LanguageId::Java,
        LanguageId::C,
        LanguageId::Cpp,
        LanguageId::Bash,
        LanguageId::CSharp,
        LanguageId::Ruby,
        LanguageId::Php,
        LanguageId::Kotlin,
        LanguageId::Scala,
    ];

    #[test]
    fn language_ids_are_ordered_as_declared() {
        assert!(ALL.windows(2).all(|pair| pair[0] < pair[1]));
        let set: BTreeSet<_> = [LanguageId::Scala, LanguageId::TypeScript, LanguageId::Rust, LanguageId::Go].into();
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            [LanguageId::TypeScript, LanguageId::Rust, LanguageId::Go, LanguageId::Scala]
        );
    }

    #[test]
    fn language_ids_key_a_map() {
        let map: HashMap<_, _> = ALL.iter().copied().enumerate().map(|(i, l)| (l, i)).collect();
        assert_eq!(map[&LanguageId::Go], 3);
        assert_eq!(map.len(), 13);
    }

    #[test]
    fn precision_orders_weakest_to_strongest() {
        use CapabilityPrecision::{Exact, Heuristic, Structural, Unsupported};
        assert!(Unsupported < Heuristic && Heuristic < Structural && Structural < Exact);
    }

    #[test]
    fn capabilities_are_equal_only_when_every_field_is() {
        use CapabilityPrecision::{Exact, Unsupported};
        let base = AnalyzerCapabilities {
            blocks: true,
            classified_imports: true,
            resolved_import_graph: Exact,
            refs: Exact,
            definition: Exact,
            inspect_detail: Exact,
        };
        assert_eq!(base, base);
        let changed = [
            AnalyzerCapabilities { blocks: false, ..base },
            AnalyzerCapabilities { classified_imports: false, ..base },
            AnalyzerCapabilities { resolved_import_graph: Unsupported, ..base },
            AnalyzerCapabilities { refs: Unsupported, ..base },
            AnalyzerCapabilities { definition: Unsupported, ..base },
            AnalyzerCapabilities { inspect_detail: Unsupported, ..base },
        ];
        assert!(changed.iter().all(|c| *c != base));
    }

    fn analysis(path: &str, profile: AnalysisProfile) -> FileAnalysis {
        FileAnalysis {
            source_path: Utf8PathBuf::from(path),
            digest: Digest::blake3(b"x"),
            profile,
            had_parse_errors: false,
            symbols: Vec::new(),
            dependencies: Vec::new(),
            exports: Vec::new(),
            blocks: Vec::new(),
            lang: LangData::Ts(TsData::default()),
        }
    }

    /// Records what `analyze_with_profile` receives and returns a canned result for `ok.ts`.
    struct Recorder {
        seen: Mutex<Vec<(Utf8PathBuf, AnalysisProfile)>>,
        ok: Arc<FileAnalysis>,
    }

    impl LanguageAnalyzer for Recorder {
        fn languages(&self) -> &[LanguageId] {
            &[LanguageId::TypeScript]
        }
        fn supports(&self, _path: &Utf8Path) -> bool {
            true
        }
        fn language_for(&self, _path: &Utf8Path) -> Option<LanguageId> {
            Some(LanguageId::TypeScript)
        }
        fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
            AnalyzerCapabilities {
                blocks: true,
                classified_imports: true,
                resolved_import_graph: CapabilityPrecision::Exact,
                refs: CapabilityPrecision::Structural,
                definition: CapabilityPrecision::Structural,
                inspect_detail: CapabilityPrecision::Structural,
            }
        }
        fn analyze_with_profile(
            &self,
            path: &Utf8Path,
            _source: Box<dyn FnOnce() -> Result<String>>,
            profile: AnalysisProfile,
        ) -> Result<Arc<FileAnalysis>> {
            self.seen.lock().unwrap().push((path.to_owned(), profile));
            if path.file_name() == Some("ok.ts") {
                Ok(Arc::clone(&self.ok))
            } else {
                Err(MonoklError::TooManyTerms { count: 1, limit: 1 })
            }
        }
    }

    fn recorder() -> Recorder {
        Recorder { seen: Mutex::new(Vec::new()), ok: Arc::new(analysis("ok.ts", AnalysisProfile::Full)) }
    }

    #[test]
    fn analyze_runs_the_full_profile_and_passes_results_through() {
        let analyzer = recorder();
        let called = Rc::new(Cell::new(false));
        let source = || {
            let flag = Rc::clone(&called);
            Box::new(move || {
                flag.set(true);
                Ok(String::new())
            }) as Box<dyn FnOnce() -> Result<String>>
        };
        let got = analyzer.analyze(Utf8Path::new("a/ok.ts"), source());
        assert!(Arc::ptr_eq(&got.unwrap(), &analyzer.ok));
        let err = analyzer.analyze(Utf8Path::new("bad.ts"), source()).unwrap_err();
        assert!(matches!(err, MonoklError::TooManyTerms { count: 1, limit: 1 }), "got {err:?}");
        assert_eq!(
            *analyzer.seen.lock().unwrap(),
            [
                (Utf8PathBuf::from("a/ok.ts"), AnalysisProfile::Full),
                (Utf8PathBuf::from("bad.ts"), AnalysisProfile::Full)
            ]
        );
        assert!(!called.get(), "analyze must not call the source closure itself");
    }

    #[test]
    fn an_analyzer_is_shareable_across_threads() {
        let analyzer: Arc<dyn LanguageAnalyzer> = Arc::new(recorder());
        let languages = std::thread::spawn(move || analyzer.languages().to_vec()).join().unwrap();
        assert_eq!(languages, [LanguageId::TypeScript]);
    }
}
