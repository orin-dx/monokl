//! The TypeScript and JavaScript analyzer.

mod parse;

use std::sync::Arc;

use camino::Utf8Path;

use super::file_analysis::FileAnalysis;
use super::lang::{AnalysisProfile, AnalyzerCapabilities, CapabilityPrecision, LanguageAnalyzer, LanguageId};
use crate::digest::Digest;
use crate::error::Result;
use crate::types::{LangData, TsData};

const LANGUAGES: [LanguageId; 1] = [LanguageId::TypeScript];

/// Analyzes TypeScript and JavaScript files with OXC.
///
/// It is stateless: every call parses its own source and returns owned data. Dependencies keep their specifier as
/// written; resolving them to files is not done here.
///
/// # Examples
///
/// ```
/// use camino::Utf8Path;
/// use monokl::analysis::LanguageAnalyzer;
/// use monokl::analysis::ts::TsAnalyzer;
///
/// let analyzer = TsAnalyzer::new();
/// assert!(analyzer.supports(Utf8Path::new("src/app.tsx")));
/// assert!(!analyzer.supports(Utf8Path::new("types/global.d.ts")));
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct TsAnalyzer;

impl TsAnalyzer {
    /// Creates an analyzer.
    ///
    /// # Examples
    ///
    /// ```
    /// use monokl::analysis::ts::TsAnalyzer;
    ///
    /// let _analyzer = TsAnalyzer::new();
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl LanguageAnalyzer for TsAnalyzer {
    fn languages(&self) -> &[LanguageId] {
        &LANGUAGES
    }

    fn supports(&self, path: &Utf8Path) -> bool {
        !path.as_str().ends_with(".d.ts")
            && matches!(path.extension(), Some("ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"))
    }

    fn language_for(&self, path: &Utf8Path) -> Option<LanguageId> {
        self.supports(path).then_some(LanguageId::TypeScript)
    }

    fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
        AnalyzerCapabilities {
            blocks: true,
            classified_imports: false,
            resolved_import_graph: CapabilityPrecision::Unsupported,
            refs: CapabilityPrecision::Structural,
            definition: CapabilityPrecision::Structural,
            inspect_detail: CapabilityPrecision::Structural,
        }
    }

    fn analyze_with_profile(
        &self,
        path: &Utf8Path,
        source: Box<dyn FnOnce() -> Result<String>>,
        profile: AnalysisProfile,
    ) -> Result<Arc<FileAnalysis>> {
        let source = source()?;
        let digest = Digest::blake3(source.as_bytes());
        let parse::Extraction {
            symbols,
            dependencies,
            exports,
            blocks,
            jsx_elements,
            type_only_imports,
            had_parse_errors,
        } = parse::extract(path, &source, profile);
        Ok(Arc::new(FileAnalysis {
            source_path: path.to_owned(),
            digest,
            profile,
            had_parse_errors,
            symbols,
            dependencies,
            exports,
            blocks,
            lang: LangData::Ts(TsData { jsx_elements, type_only_imports, unresolved_aliases: Vec::new() }),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use rstest::rstest;

    use super::*;
    use crate::error::MonoklError;
    use crate::types::{BindingKind, DependencyTarget, SymbolKind};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/small-ts");

    fn analyze_source(path: &str, text: &str, profile: AnalysisProfile) -> Arc<FileAnalysis> {
        let text = text.to_owned();
        TsAnalyzer::new().analyze_with_profile(Utf8Path::new(path), Box::new(move || Ok(text)), profile).unwrap()
    }

    fn fixture(relative: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{relative}")).unwrap()
    }

    fn analyze_fixture(relative: &str, profile: AnalysisProfile) -> Arc<FileAnalysis> {
        analyze_source(&format!("src/{relative}"), &fixture(&format!("src/{relative}")), profile)
    }

    type Symbol<'a> = (&'a str, SymbolKind, usize, Option<&'a str>);
    type Binding<'a> = (&'a str, &'a str, BindingKind);

    fn symbols(analysis: &FileAnalysis) -> Vec<Symbol<'_>> {
        analysis.symbols.iter().map(|s| (s.name.as_str(), s.kind, s.line, s.signature.as_deref())).collect()
    }

    fn exports(analysis: &FileAnalysis) -> Vec<(&str, usize, bool)> {
        analysis.exports.iter().map(|e| (e.name.as_str(), e.line, e.re_export)).collect()
    }

    fn dependencies(analysis: &FileAnalysis) -> Vec<(&str, usize, Vec<Binding<'_>>)> {
        analysis
            .dependencies
            .iter()
            .map(|dep| {
                let DependencyTarget::File { specifier, resolved, is_relative } = &dep.target else {
                    panic!("expected a file target");
                };
                assert_eq!(*resolved, None);
                assert_eq!(*is_relative, specifier.starts_with('.'));
                let bindings = dep.bindings.iter().map(|b| (b.imported.as_str(), b.local.as_str(), b.kind)).collect();
                (specifier.as_str(), dep.line, bindings)
            })
            .collect()
    }

    fn blocks(analysis: &FileAnalysis) -> Vec<(usize, usize, SymbolKind, Option<&str>)> {
        analysis
            .blocks
            .iter()
            .map(|b| {
                assert_eq!(b.file, analysis.source_path);
                (b.line_start, b.line_end, b.node_kind, b.symbol_signature.as_deref())
            })
            .collect()
    }

    #[rstest]
    #[case("a.ts", true)]
    #[case("a.tsx", true)]
    #[case("a.js", true)]
    #[case("a.jsx", true)]
    #[case("a.mts", true)]
    #[case("a.cts", true)]
    #[case("a.mjs", true)]
    #[case("a.cjs", true)]
    #[case("src/deep/dir/a.ts", true)]
    #[case("types.d.ts", false)]
    #[case("src/types/global.d.ts", false)]
    #[case("a.rs", false)]
    #[case("a.json", false)]
    #[case("Makefile", false)]
    fn supports_follows_the_extension_rules(#[case] path: &str, #[case] expected: bool) {
        let analyzer = TsAnalyzer::new();
        let path = Utf8Path::new(path);
        assert_eq!(analyzer.supports(path), expected);
        assert_eq!(analyzer.language_for(path), expected.then_some(LanguageId::TypeScript));
    }

    #[test]
    fn it_reports_its_language_and_capabilities() {
        let analyzer = TsAnalyzer::new();
        assert_eq!(analyzer.languages(), [LanguageId::TypeScript]);
        assert_eq!(
            analyzer.capabilities(LanguageId::TypeScript),
            AnalyzerCapabilities {
                blocks: true,
                classified_imports: false,
                resolved_import_graph: CapabilityPrecision::Unsupported,
                refs: CapabilityPrecision::Structural,
                definition: CapabilityPrecision::Structural,
                inspect_detail: CapabilityPrecision::Structural,
            }
        );
    }

    #[test]
    fn the_index_fixture_re_exports_without_declaring() {
        let analysis = analyze_fixture("index.ts", AnalysisProfile::Full);
        assert_eq!(analysis.source_path, "src/index.ts");
        assert_eq!(analysis.digest, Digest::blake3(fixture("src/index.ts").as_bytes()));
        assert!(!analysis.had_parse_errors);
        assert_eq!(symbols(&analysis), []);
        assert_eq!(
            exports(&analysis),
            [
                ("Button", 1, true),
                ("useCart", 2, true),
                ("formatCurrency", 3, true),
                ("truncateString", 3, true),
                ("validatePrice", 4, true),
                ("ButtonProps", 5, true),
                ("CartItem", 5, true),
                ("ValidationResult", 5, true),
                ("PriceValidator", 5, true),
            ]
        );
        assert_eq!(
            dependencies(&analysis),
            [
                ("./components/Button", 1, vec![("Button", "Button", BindingKind::Named)]),
                ("./hooks/useCart", 2, vec![("useCart", "useCart", BindingKind::Named)]),
                (
                    "./utils/format",
                    3,
                    vec![
                        ("formatCurrency", "formatCurrency", BindingKind::Named),
                        ("truncateString", "truncateString", BindingKind::Named),
                    ]
                ),
                ("./utils/validation", 4, vec![("validatePrice", "validatePrice", BindingKind::Named)]),
            ]
        );
        assert_eq!(analysis.lang.ts().unwrap().type_only_imports, Vec::<String>::new());
        assert_eq!(blocks(&analysis), (1..=5).map(|n| (n, n, SymbolKind::Other, None)).collect::<Vec<_>>());
        assert_eq!(analysis.blocks[0].code, "export { Button } from './components/Button';");
    }

    #[test]
    fn the_button_fixture_has_a_component_a_default_export_and_jsx() {
        let analysis = analyze_fixture("components/Button.tsx", AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        let signature = "function Button({ label, price, onClick, disabled }: ButtonProps)";
        assert_eq!(
            symbols(&analysis),
            [("ButtonProps", SymbolKind::Interface, 4, None), ("Button", SymbolKind::Function, 11, Some(signature))]
        );
        assert_eq!(exports(&analysis), [("ButtonProps", 4, false), ("Button", 11, false), ("default", 20, false)]);
        assert_eq!(
            dependencies(&analysis),
            [
                ("react", 1, vec![("default", "React", BindingKind::Default)]),
                ("../utils/validation", 2, vec![("validatePrice", "validatePrice", BindingKind::Named)]),
            ]
        );
        assert_eq!(
            blocks(&analysis),
            [
                (4, 9, SymbolKind::Interface, None),
                (11, 18, SymbolKind::Function, Some(signature)),
                (20, 20, SymbolKind::Other, None),
            ]
        );
        assert_eq!(analysis.blocks[2].code, "export default Button;");
        let jsx = analysis.jsx_elements();
        assert_eq!(jsx.len(), 1);
        assert_eq!((jsx[0].name.as_str(), jsx[0].is_html, jsx[0].line), ("button", true, 14));
        let attributes: Vec<_> = jsx[0]
            .attributes
            .iter()
            .map(|a| (a.name.as_str(), a.string_value.clone(), a.is_expression, a.is_spread))
            .collect();
        assert_eq!(attributes, [("onClick", None, true, false), ("disabled", None, true, false)]);
    }

    #[test]
    fn the_use_cart_fixture_separates_type_only_imports() {
        let analysis = analyze_fixture("hooks/useCart.ts", AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(
            symbols(&analysis),
            [
                ("CartItem", SymbolKind::Interface, 5, None),
                ("useCart", SymbolKind::Function, 11, Some("function useCart()")),
            ]
        );
        assert_eq!(exports(&analysis), [("CartItem", 5, false), ("useCart", 11, false)]);
        assert_eq!(
            dependencies(&analysis),
            [
                ("react", 1, vec![("useState", "useState", BindingKind::Named)]),
                ("../utils/validation", 2, vec![("validatePrice", "validatePrice", BindingKind::Named)]),
            ]
        );
        assert_eq!(analysis.lang.ts().unwrap().type_only_imports, ["../utils/validation"]);
        assert_eq!(
            blocks(&analysis),
            [(5, 9, SymbolKind::Interface, None), (11, 25, SymbolKind::Function, Some("function useCart()"))]
        );
        assert!(analysis.jsx_elements().is_empty());
    }

    #[test]
    fn the_format_fixture_has_two_functions_and_no_dependencies() {
        let analysis = analyze_fixture("utils/format.ts", AnalysisProfile::Full);
        let currency = "function formatCurrency(amount: number, currency: string = 'USD'): string";
        let truncate = "function truncateString(str: string, maxLength: number): string";
        assert_eq!(
            symbols(&analysis),
            [
                ("formatCurrency", SymbolKind::Function, 1, Some(currency)),
                ("truncateString", SymbolKind::Function, 5, Some(truncate)),
            ]
        );
        assert_eq!(exports(&analysis), [("formatCurrency", 1, false), ("truncateString", 5, false)]);
        assert_eq!(dependencies(&analysis), []);
        assert_eq!(
            blocks(&analysis),
            [(1, 3, SymbolKind::Function, Some(currency)), (5, 8, SymbolKind::Function, Some(truncate))]
        );
    }

    #[test]
    fn the_validation_fixture_has_an_interface_a_function_and_a_type_alias() {
        let analysis = analyze_fixture("utils/validation.ts", AnalysisProfile::Full);
        let signature = "function validatePrice(price: number): ValidationResult";
        assert_eq!(
            symbols(&analysis),
            [
                ("ValidationResult", SymbolKind::Interface, 3, None),
                ("validatePrice", SymbolKind::Function, 8, Some(signature)),
                ("PriceValidator", SymbolKind::TypeAlias, 14, None),
            ]
        );
        assert_eq!(
            exports(&analysis),
            [("ValidationResult", 3, false), ("validatePrice", 8, false), ("PriceValidator", 14, false)]
        );
        assert_eq!(
            dependencies(&analysis),
            [("./format", 1, vec![("formatCurrency", "formatCurrency", BindingKind::Named)])]
        );
        assert_eq!(
            blocks(&analysis),
            [
                (3, 6, SymbolKind::Interface, None),
                (8, 12, SymbolKind::Function, Some(signature)),
                (14, 14, SymbolKind::TypeAlias, None),
            ]
        );
        assert_eq!(analysis.blocks[2].code, "export type PriceValidator = (price: number) => ValidationResult;");
    }

    #[test]
    fn profiles_fill_only_what_they_promise() {
        let path = "components/Button.tsx";
        let full = analyze_fixture(path, AnalysisProfile::Full);

        let dependencies_only = analyze_fixture(path, AnalysisProfile::Dependencies);
        assert_eq!(dependencies_only.profile, AnalysisProfile::Dependencies);
        assert_eq!(format!("{:?}", dependencies_only.dependencies), format!("{:?}", full.dependencies));
        assert_eq!(symbols(&dependencies_only), []);
        assert_eq!(exports(&dependencies_only), []);
        assert_eq!(blocks(&dependencies_only), []);
        assert!(dependencies_only.jsx_elements().is_empty());

        let structural = analyze_fixture(path, AnalysisProfile::Structural);
        assert_eq!(structural.profile, AnalysisProfile::Structural);
        assert_eq!(format!("{:?}", structural.dependencies), format!("{:?}", full.dependencies));
        assert_eq!(format!("{:?}", structural.symbols), format!("{:?}", full.symbols));
        assert_eq!(format!("{:?}", structural.exports), format!("{:?}", full.exports));
        assert_eq!(blocks(&structural), []);
        assert!(structural.jsx_elements().is_empty());

        assert_eq!(full.profile, AnalysisProfile::Full);
        assert_eq!(full.blocks.len(), 3);
        assert_eq!(full.jsx_elements().len(), 1);
    }

    #[test]
    fn type_only_imports_survive_the_dependencies_profile() {
        let analysis = analyze_fixture("hooks/useCart.ts", AnalysisProfile::Dependencies);
        assert_eq!(analysis.lang.ts().unwrap().type_only_imports, ["../utils/validation"]);
        assert_eq!(analysis.dependencies.len(), 2);
    }

    #[test]
    fn analyze_defaults_to_the_full_profile() {
        let text = fixture("src/utils/format.ts");
        let analysis = TsAnalyzer::new().analyze(Utf8Path::new("format.ts"), Box::new(move || Ok(text))).unwrap();
        assert_eq!(analysis.profile, AnalysisProfile::Full);
        assert_eq!(analysis.blocks.len(), 2);
    }

    #[test]
    fn declarations_other_than_functions_become_symbols() {
        let text = "\
const a = 1, b = () => 2;
class Plain {}
export class Shown {}
enum Mode { On }
export enum Open { Yes }
export const [first] = [1];
export default class Named {}
";
        let analysis = analyze_source("kinds.ts", text, AnalysisProfile::Structural);
        assert!(!analysis.had_parse_errors);
        assert_eq!(
            symbols(&analysis),
            [
                ("a", SymbolKind::Variable, 1, None),
                ("b", SymbolKind::Variable, 1, None),
                ("Plain", SymbolKind::Class, 2, None),
                ("Shown", SymbolKind::Class, 3, None),
                ("Mode", SymbolKind::Enum, 4, None),
                ("Open", SymbolKind::Enum, 5, None),
                ("Named", SymbolKind::Class, 7, None),
            ]
        );
        assert_eq!(exports(&analysis), [("Shown", 3, false), ("Open", 5, false), ("default", 7, false)]);
    }

    #[test]
    fn an_anonymous_default_function_is_exported_but_has_no_symbol() {
        let analysis = analyze_source("anon.ts", "export default function () {}\n", AnalysisProfile::Full);
        assert_eq!(symbols(&analysis), []);
        assert_eq!(exports(&analysis), [("default", 1, false)]);
        assert_eq!(blocks(&analysis), [(1, 1, SymbolKind::Other, None)]);
    }

    #[test]
    fn imports_record_namespace_side_effect_and_inline_type_bindings() {
        let text = "\
import * as fs from 'node:fs';
import './polyfill';
import def, { a as b, type T } from './m';
export * from './all';
export * as ns from './named';
export type * from './types';
";
        let analysis = analyze_source("imports.ts", text, AnalysisProfile::Dependencies);
        assert!(!analysis.had_parse_errors);
        assert_eq!(
            dependencies(&analysis),
            [
                ("node:fs", 1, vec![("*", "fs", BindingKind::Namespace)]),
                ("./polyfill", 2, vec![]),
                ("./m", 3, vec![("default", "def", BindingKind::Default), ("a", "b", BindingKind::Named)]),
                ("./all", 4, vec![("*", "*", BindingKind::Glob)]),
                ("./named", 5, vec![("*", "ns", BindingKind::Namespace)]),
            ]
        );
    }

    #[test]
    fn jsx_elements_are_collected_at_every_depth_in_document_order() {
        let text = "\
export const App = (p) => (
  <Layout {...p} title=\"home\" flag>
    <Foo.Bar />
    <svg:rect width=\"1\" />
  </Layout>
);
";
        let analysis = analyze_source("app.jsx", text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        let elements: Vec<_> =
            analysis.jsx_elements().iter().map(|e| (e.name.as_str(), e.is_html, e.line, e.attributes.len())).collect();
        assert_eq!(elements, [("Layout", false, 2, 3), ("Foo.Bar", false, 3, 0), ("svg:rect", true, 4, 1)]);
        let layout: Vec<_> = analysis.jsx_elements()[0]
            .attributes
            .iter()
            .map(|a| (a.name.as_str(), a.string_value.as_deref(), a.is_expression, a.is_spread))
            .collect();
        assert_eq!(
            layout,
            [("...", None, false, true), ("title", Some("home"), false, false), ("flag", None, false, false)]
        );
    }

    #[test]
    fn javascript_files_may_contain_jsx() {
        let analysis = analyze_source("view.js", "export const v = <div />;\n", AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(analysis.jsx_elements().len(), 1);
    }

    #[test]
    fn a_syntax_error_is_flagged_and_what_parsed_is_kept() {
        let text = "import { a } from './a';\nexport function good(x: number) { return x; }\nconst x;\nexport function after() {}\n";
        let analysis = analyze_source("broken.ts", text, AnalysisProfile::Full);
        assert!(analysis.had_parse_errors);
        let names: Vec<_> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["good", "x", "after"]);
        assert_eq!(dependencies(&analysis), [("./a", 1, vec![("a", "a", BindingKind::Named)])]);
    }

    #[test]
    fn an_unrecoverable_syntax_error_leaves_nothing_but_the_flag() {
        let analysis = analyze_source("fatal.ts", "export function good() {}\nconst x = ;\n", AnalysisProfile::Full);
        assert_degraded(&analysis);
    }

    #[test]
    fn a_clean_file_has_no_parse_errors() {
        assert!(!analyze_source("ok.ts", "export const x = 1;\n", AnalysisProfile::Full).had_parse_errors);
    }

    fn assert_degraded(analysis: &FileAnalysis) {
        assert!(analysis.had_parse_errors);
        assert_eq!(symbols(analysis), []);
        assert_eq!(exports(analysis), []);
        assert_eq!(dependencies(analysis), []);
        assert_eq!(blocks(analysis), []);
        assert!(analysis.jsx_elements().is_empty());
        assert_eq!(analysis.lang.ts().unwrap().type_only_imports, Vec::<String>::new());
    }

    #[test]
    fn a_file_nested_past_the_limit_is_degraded_without_parsing() {
        let text = format!("import a from './a';\nconst x = {}1{};\n", "[".repeat(5000), "]".repeat(5000));
        let analysis = analyze_source("deep.ts", &text, AnalysisProfile::Full);
        assert_degraded(&analysis);
        assert_eq!(analysis.digest, Digest::blake3(text.as_bytes()));
        assert_eq!(analysis.profile, AnalysisProfile::Full);
    }

    #[test]
    fn a_file_nested_to_the_limit_is_still_parsed() {
        // Runs on a 2 MiB test thread in a debug build, the tightest stack a caller is likely to give the parser.
        for (open, close) in [("[", "]"), ("f(", ")"), ("{a:", "}")] {
            let text = format!("export const x = {}1{};\n", open.repeat(256), close.repeat(256));
            let analysis = analyze_source("limit.ts", &text, AnalysisProfile::Full);
            assert!(!analysis.had_parse_errors, "{open}");
            assert_eq!(symbols(&analysis), [("x", SymbolKind::Variable, 1, None)]);
        }
    }

    #[test]
    fn brackets_in_strings_and_comments_do_not_count_toward_nesting() {
        let text =
            format!("// {}\nexport const s = '{}';\n/* {} */\n", "[".repeat(5000), "(".repeat(5000), "{".repeat(5000));
        let analysis = analyze_source("quiet.ts", &text, AnalysisProfile::Structural);
        assert!(!analysis.had_parse_errors);
        assert_eq!(symbols(&analysis).len(), 1);
    }

    #[test]
    fn the_source_closure_error_is_returned_and_the_closure_runs_once() {
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        let source = Box::new(move || {
            counter.set(counter.get() + 1);
            Err(MonoklError::TooManyTerms { count: 1, limit: 1 })
        });
        let error = TsAnalyzer::new().analyze(Utf8Path::new("a.ts"), source).unwrap_err();
        assert!(matches!(error, MonoklError::TooManyTerms { count: 1, limit: 1 }));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn analyzing_twice_gives_identical_results() {
        let files = ["index.ts", "components/Button.tsx", "hooks/useCart.ts", "utils/format.ts", "utils/validation.ts"];
        for relative in files {
            let first = analyze_fixture(relative, AnalysisProfile::Full);
            let second = analyze_fixture(relative, AnalysisProfile::Full);
            assert_eq!(format!("{first:?}"), format!("{second:?}"));
        }
    }
}
