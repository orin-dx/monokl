//! The languages monokl searches, the file rules that belong to each, and the contract every analyzer implements.

use std::sync::Arc;

use camino::Utf8Path;

use super::file_analysis::FileAnalysis;
use crate::error::Result;

/// A language monokl searches.
///
/// This is the one definition of which files belong to a language and which of them are tests. Text search, the
/// `--language` filter, the analyzers and ranking all ask it, so a new extension is added in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[non_exhaustive]
pub enum Language {
    #[cfg_attr(feature = "cli", value(name = "typescript"))]
    TypeScript,
    #[cfg_attr(feature = "cli", value(name = "javascript"))]
    JavaScript,
    #[cfg_attr(feature = "cli", value(name = "rust"))]
    Rust,
}

/// The file facts of one [`Language`].
#[derive(Debug, PartialEq, Eq)]
pub struct LanguageRules {
    /// File extensions, without the dot, that belong to the language.
    pub extensions: &'static [&'static str],
    /// File-name suffixes that are never searched although their extension is in `extensions`.
    pub excluded_suffixes: &'static [&'static str],
    /// A file whose name without its extension ends with one of these is a test file.
    pub test_stem_suffixes: &'static [&'static str],
    /// A file whose name without its extension is one of these is a test file.
    pub test_stems: &'static [&'static str],
}

const SCRIPT_TEST_STEM_SUFFIXES: &[&str] = &[".test", ".spec"];

const TYPESCRIPT: LanguageRules = LanguageRules {
    extensions: &["ts", "tsx", "mts", "cts"],
    excluded_suffixes: &[".d.ts", ".d.mts", ".d.cts"],
    test_stem_suffixes: SCRIPT_TEST_STEM_SUFFIXES,
    test_stems: &[],
};

const JAVASCRIPT: LanguageRules = LanguageRules {
    extensions: &["js", "jsx", "mjs", "cjs"],
    excluded_suffixes: &[],
    test_stem_suffixes: SCRIPT_TEST_STEM_SUFFIXES,
    test_stems: &[],
};

// `tests.rs` and `test.rs` are the out-of-line `#[cfg(test)] mod tests;` of Rust: the attribute is in the parent file,
// so the file itself can only be recognized by its name.
const RUST: LanguageRules = LanguageRules {
    extensions: &["rs"],
    excluded_suffixes: &[],
    test_stem_suffixes: &["_test", "_tests"],
    test_stems: &["test", "tests"],
};

/// Directory names that make every file under them a test file, whatever its language.
pub const TEST_DIRS: &[&str] = &["__tests__", "test", "tests", "spec"];

impl Language {
    /// Every language, in the order the rules are listed.
    pub const ALL: [Self; 3] = [Self::TypeScript, Self::JavaScript, Self::Rust];

    /// The language of the file at `path`, or `None` for a file of no searched language or a declaration file.
    ///
    /// # Examples
    ///
    /// ```
    /// use camino::Utf8Path;
    /// use monokl::analysis::Language;
    ///
    /// assert_eq!(Language::of(Utf8Path::new("src/app.tsx")), Some(Language::TypeScript));
    /// assert_eq!(Language::of(Utf8Path::new("src/app.mjs")), Some(Language::JavaScript));
    /// assert_eq!(Language::of(Utf8Path::new("types/global.d.ts")), None);
    /// ```
    #[must_use]
    pub fn of(path: &Utf8Path) -> Option<Self> {
        let extension = path.extension()?;
        let language = Self::ALL.into_iter().find(|language| language.rules().extensions.contains(&extension))?;
        let excluded = language.rules().excluded_suffixes.iter().any(|suffix| path.as_str().ends_with(suffix));
        (!excluded).then_some(language)
    }

    /// The file rules of this language.
    #[must_use]
    pub const fn rules(self) -> &'static LanguageRules {
        match self {
            Self::TypeScript => &TYPESCRIPT,
            Self::JavaScript => &JAVASCRIPT,
            Self::Rust => &RUST,
        }
    }

    /// Whether `path`, a file of this language, is named like a test file. Directories are not looked at: see
    /// [`is_test_path`].
    ///
    /// # Examples
    ///
    /// ```
    /// use camino::Utf8Path;
    /// use monokl::analysis::Language;
    ///
    /// assert!(Language::TypeScript.is_test_file(Utf8Path::new("src/cart.test.ts")));
    /// assert!(Language::Rust.is_test_file(Utf8Path::new("src/cart_tests.rs")));
    /// assert!(!Language::Rust.is_test_file(Utf8Path::new("src/cart.test.ts")));
    /// ```
    #[must_use]
    pub fn is_test_file(self, path: &Utf8Path) -> bool {
        let rules = self.rules();
        Self::of(path) == Some(self)
            && path.file_stem().is_some_and(|stem| {
                rules.test_stems.contains(&stem) || rules.test_stem_suffixes.iter().any(|suffix| stem.ends_with(suffix))
            })
    }
}

/// Whether a path relative to the search root is a test file: it is under a test directory, or its name is a test name
/// of its language. Directories above the root never make a file a test.
///
/// # Examples
///
/// ```
/// use camino::Utf8Path;
/// use monokl::analysis::lang::is_test_path;
///
/// assert!(is_test_path(Utf8Path::new("src/__tests__/a.ts")));
/// assert!(!is_test_path(Utf8Path::new("src/a.ts")));
/// ```
#[must_use]
pub fn is_test_path(relative: &Utf8Path) -> bool {
    let in_test_dir = relative
        .parent()
        .is_some_and(|dirs| dirs.components().any(|component| TEST_DIRS.contains(&component.as_str())));
    in_test_dir || Language::of(relative).is_some_and(|language| language.is_test_file(relative))
}

/// Turns source files into [`FileAnalysis`] values.
///
/// An instance may handle several languages.
pub trait LanguageAnalyzer: Send + Sync {
    /// Every language this analyzer handles.
    fn languages(&self) -> &[Language];

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
    use std::sync::Mutex;

    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;
    use crate::error::MonoklError;

    #[rstest]
    #[case("a.ts", Some(Language::TypeScript))]
    #[case("a.tsx", Some(Language::TypeScript))]
    #[case("a.mts", Some(Language::TypeScript))]
    #[case("a.cts", Some(Language::TypeScript))]
    #[case("src/deep/dir/a.ts", Some(Language::TypeScript))]
    #[case("a.js", Some(Language::JavaScript))]
    #[case("a.jsx", Some(Language::JavaScript))]
    #[case("a.mjs", Some(Language::JavaScript))]
    #[case("a.cjs", Some(Language::JavaScript))]
    #[case("a.rs", Some(Language::Rust))]
    #[case("a.d.ts", None)]
    #[case("a.d.mts", None)]
    #[case("a.d.cts", None)]
    #[case("src/types/global.d.ts", None)]
    #[case("a.d.js", Some(Language::JavaScript))]
    #[case("a.d.rs", Some(Language::Rust))]
    #[case("a.py", None)]
    #[case("a.json", None)]
    #[case("a.TS", None)]
    #[case("a.ts.map", None)]
    #[case("ts", None)]
    #[case(".ts", None)]
    #[case("a", None)]
    fn the_language_of_a_path(#[case] path: &str, #[case] expected: Option<Language>) {
        assert_eq!(Language::of(Utf8Path::new(path)), expected);
    }

    #[test]
    fn rules_list_every_extension_once_in_declaration_order() {
        let extensions: Vec<_> =
            Language::ALL.iter().flat_map(|language| language.rules().extensions.iter().copied()).collect();
        assert_eq!(extensions, ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "rs"]);
        for language in Language::ALL {
            for extension in language.rules().extensions {
                assert_eq!(Language::of(Utf8Path::new(&format!("a.{extension}"))), Some(language));
            }
        }
    }

    #[rstest]
    #[case("a.test.ts", true)]
    #[case("a.test.tsx", true)]
    #[case("a.test.mts", true)]
    #[case("a.test.cts", true)]
    #[case("a.spec.ts", true)]
    #[case("a.spec.tsx", true)]
    #[case("a.spec.mts", true)]
    #[case("a.spec.cts", true)]
    #[case(".test.ts", true)]
    #[case("src/a.test.ts", true)]
    #[case("a.ts", false)]
    #[case("test.ts", false)]
    #[case("tests.ts", false)]
    #[case("a_test.ts", false)]
    #[case("a.Test.ts", false)]
    #[case("a.test.d.ts", false)]
    #[case("a.test.js", false)]
    #[case("a.test.rs", false)]
    fn typescript_test_files(#[case] path: &str, #[case] expected: bool) {
        assert_eq!(Language::TypeScript.is_test_file(Utf8Path::new(path)), expected);
    }

    #[rstest]
    #[case("a.test.js", true)]
    #[case("a.test.jsx", true)]
    #[case("a.test.mjs", true)]
    #[case("a.test.cjs", true)]
    #[case("a.spec.js", true)]
    #[case("a.spec.jsx", true)]
    #[case("a.spec.mjs", true)]
    #[case("a.spec.cjs", true)]
    #[case("a.js", false)]
    #[case("test.js", false)]
    #[case("a.test.ts", false)]
    fn javascript_test_files(#[case] path: &str, #[case] expected: bool) {
        assert_eq!(Language::JavaScript.is_test_file(Utf8Path::new(path)), expected);
    }

    #[rstest]
    #[case("tests.rs", true)]
    #[case("test.rs", true)]
    #[case("src/analysis/rust/tests.rs", true)]
    #[case("src/test.rs", true)]
    #[case("cart_tests.rs", true)]
    #[case("cart_test.rs", true)]
    #[case("_test.rs", true)]
    #[case("lib.rs", false)]
    #[case("contests.rs", false)]
    #[case("latest.rs", false)]
    #[case("tests_helper.rs", false)]
    #[case("a.test.rs", false)]
    #[case("a.spec.rs", false)]
    #[case("tests.rs.bak", false)]
    #[case("a.test.ts", false)]
    fn rust_test_files(#[case] path: &str, #[case] expected: bool) {
        assert_eq!(Language::Rust.is_test_file(Utf8Path::new(path)), expected);
    }

    #[rstest]
    #[case("__tests__/a.ts", true)]
    #[case("test/a.ts", true)]
    #[case("tests/a.rs", true)]
    #[case("spec/a.js", true)]
    #[case("src/__tests__/deep/a.ts", true)]
    #[case("src/spec/a.py", true)]
    #[case("src/a.test.ts", true)]
    #[case("src/cart_test.rs", true)]
    #[case("src/a.ts", false)]
    #[case("test", false)]
    #[case("src/test", false)]
    #[case("test.ts", false)]
    #[case("src/testing/a.ts", false)]
    #[case("src/specs/a.ts", false)]
    #[case("Test/a.ts", false)]
    #[case("src/a.test.ts.map", false)]
    fn test_paths(#[case] path: &str, #[case] expected: bool) {
        assert_eq!(is_test_path(Utf8Path::new(path)), expected);
    }

    /// Records what `analyze` receives and returns a canned result for `ok.ts`.
    struct Recorder {
        seen: Mutex<Vec<(Utf8PathBuf, String)>>,
        ok: Arc<FileAnalysis>,
    }

    impl LanguageAnalyzer for Recorder {
        fn languages(&self) -> &[Language] {
            &[Language::TypeScript]
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
        assert_eq!(languages, [Language::TypeScript]);
    }
}
