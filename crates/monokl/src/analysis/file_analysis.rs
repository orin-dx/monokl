//! Everything extracted from one source file.

use camino::Utf8PathBuf;

use super::lang::AnalysisProfile;
use crate::digest::Digest;
use crate::types::{CodeBlock, DependencyRecord, ExportRecord, JsxElementEntry, LangData, SymbolEntry};

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
    pub had_parse_errors: bool,
    pub symbols: Vec<SymbolEntry>,
    pub dependencies: Vec<DependencyRecord>,
    pub exports: Vec<ExportRecord>,
    pub blocks: Vec<CodeBlock>,
    pub lang: LangData,
}

impl FileAnalysis {
    /// The language name, matching the tag `LangData` serializes with.
    #[must_use]
    pub fn language(&self) -> &str {
        match &self.lang {
            LangData::Ts(_) => "typescript",
            LangData::Rust(_) => "rust",
            LangData::Python(_) => "python",
            LangData::Go(_) => "go",
            LangData::Java(_) => "java",
        }
    }

    /// The JSX elements of a TypeScript file, or an empty slice for any other language.
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
    use crate::types::{GoData, JavaData, PythonData, RustData, TsData};

    fn analysis(lang: LangData) -> FileAnalysis {
        FileAnalysis {
            source_path: Utf8PathBuf::from("src/a.ts"),
            digest: Digest::blake3(b"x"),
            profile: AnalysisProfile::Structural,
            had_parse_errors: true,
            symbols: Vec::new(),
            dependencies: Vec::new(),
            exports: Vec::new(),
            blocks: Vec::new(),
            lang,
        }
    }

    fn jsx(name: &str) -> JsxElementEntry {
        JsxElementEntry { name: name.to_string(), is_html: false, line: 1, attributes: Vec::new() }
    }

    #[rstest]
    #[case(LangData::Ts(TsData::default()), "typescript", true)]
    #[case(LangData::Rust(RustData::default()), "rust", false)]
    #[case(LangData::Python(PythonData::default()), "python", false)]
    #[case(LangData::Go(GoData::default()), "go", false)]
    #[case(LangData::Java(JavaData::default()), "java", false)]
    fn language_names_follow_the_lang_data_variant(
        #[case] lang: LangData,
        #[case] name: &str,
        #[case] typescript: bool,
    ) {
        let analysis = analysis(lang);
        assert_eq!(analysis.language(), name);
        assert_eq!(analysis.is_typescript(), typescript);
        assert!(analysis.jsx_elements().is_empty());
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
        assert_eq!(format!("{clone:?}"), format!("{original:?}"));
    }
}
