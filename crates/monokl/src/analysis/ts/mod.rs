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
        !(path.as_str().ends_with(".d.ts") || path.as_str().ends_with(".d.mts") || path.as_str().ends_with(".d.cts"))
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
            degradation,
        } = parse::extract(path, &source, profile);
        Ok(Arc::new(FileAnalysis {
            source_path: path.to_owned(),
            digest,
            profile,
            had_parse_errors,
            degradation,
            symbols,
            dependencies,
            exports,
            blocks,
            test_blocks: Vec::new(),
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
    use crate::analysis::{Degradation, nesting};
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
                let DependencyTarget::File { specifier, resolved, is_relative } = &dep.target;
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
    #[case("types.d.mts", false)]
    #[case("types.d.cts", false)]
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
        assert_eq!(analysis.lang.ts().type_only_imports, Vec::<String>::new());
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
        assert_eq!(analysis.lang.ts().type_only_imports, ["../utils/validation"]);
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
        assert_eq!(analysis.lang.ts().type_only_imports, ["../utils/validation"]);
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
    fn every_top_level_declaration_kind_yields_its_symbol_export_and_block() {
        let text = "\
function f(a: number): string {
  return String(a);
}
class C {
  m() {}
}
interface I {
  x: number;
}
type T = string;
enum E {
  A,
}
const v = 1;
export function g() {}
export default function h() {}
export { v };
export const y = 1;
";
        let analysis = analyze_source("kinds.ts", text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(
            symbols(&analysis),
            [
                ("f", SymbolKind::Function, 1, Some("function f(a: number): string")),
                ("C", SymbolKind::Class, 4, None),
                ("I", SymbolKind::Interface, 7, None),
                ("T", SymbolKind::TypeAlias, 10, None),
                ("E", SymbolKind::Enum, 11, None),
                ("v", SymbolKind::Variable, 14, None),
                ("g", SymbolKind::Function, 15, Some("function g()")),
                ("h", SymbolKind::Function, 16, Some("function h()")),
                ("y", SymbolKind::Variable, 18, None),
            ]
        );
        assert_eq!(exports(&analysis), [("g", 15, false), ("default", 16, false), ("v", 17, false), ("y", 18, false)]);
        assert_eq!(
            blocks(&analysis),
            [
                (1, 3, SymbolKind::Function, Some("function f(a: number): string")),
                (4, 6, SymbolKind::Class, None),
                (7, 9, SymbolKind::Interface, None),
                (10, 10, SymbolKind::TypeAlias, None),
                (11, 13, SymbolKind::Enum, None),
                (14, 14, SymbolKind::Variable, None),
                (15, 15, SymbolKind::Function, Some("function g()")),
                (16, 16, SymbolKind::Function, None),
                (17, 17, SymbolKind::Other, None),
                (18, 18, SymbolKind::Variable, None),
            ]
        );
    }

    #[rstest]
    #[case::function("export default function X() {}\n", SymbolKind::Function)]
    #[case::async_function("export default async function X() {}\n", SymbolKind::Function)]
    #[case::anonymous_function("export default function () {}\n", SymbolKind::Function)]
    #[case::class("export default class X {}\n", SymbolKind::Class)]
    #[case::anonymous_class("export default class {}\n", SymbolKind::Class)]
    #[case::interface("export default interface X {}\n", SymbolKind::Interface)]
    #[case::arrow("export default () => {};\n", SymbolKind::Other)]
    #[case::identifier("export default X;\n", SymbolKind::Other)]
    #[case::renamed_specifier("const x = 1;\nexport { x as default };\n", SymbolKind::Other)]
    #[case::plain_function("export function X() {}\n", SymbolKind::Function)]
    fn a_default_export_block_has_the_kind_of_what_it_declares(#[case] source: &str, #[case] kind: SymbolKind) {
        let analysis = analyze_source("kind.ts", source, AnalysisProfile::Full);
        assert_eq!(analysis.blocks.last().map(|b| b.node_kind), Some(kind));
    }

    #[test]
    fn an_anonymous_default_function_is_exported_but_has_no_symbol() {
        let analysis = analyze_source("anon.ts", "export default function () {}\n", AnalysisProfile::Full);
        assert_eq!(symbols(&analysis), []);
        assert_eq!(exports(&analysis), [("default", 1, false)]);
        assert_eq!(blocks(&analysis), [(1, 1, SymbolKind::Function, None)]);
    }

    type MemberBlock<'a> = (usize, usize, SymbolKind, Option<&'a str>, Option<(SymbolKind, &'a str, usize)>);

    /// Every block as `(line_start, line_end, kind, signature, parent)`.
    fn blocks_with_parents(analysis: &FileAnalysis) -> Vec<MemberBlock<'_>> {
        analysis
            .blocks
            .iter()
            .map(|b| {
                let parent = b.parent.as_ref().map(|p| (p.kind, p.name.as_str(), p.line));
                (b.line_start, b.line_end, b.node_kind, b.symbol_signature.as_deref(), parent)
            })
            .collect()
    }

    const CLASS_WITH_MEMBERS: &str = "\
export class Foo {
  readonly id = 1;
  constructor(private readonly deps: Deps) {
    this.deps = deps;
  }
  async fetch(id: string): Promise<User> {
    return this.deps.get(id);
  }
  short() { return 1; }
  static create(): Foo {
    return new Foo(deps);
  }
  handle = async (req: Req) => {
    return req;
  };
  plain = function () {
    return 1;
  };
  get name(): string {
    return 'x';
  }
  two() {
  }
  counter = 0;
  alias = (a: number) => a + 1;
}
";

    #[test]
    fn a_class_yields_a_block_for_each_long_method_constructor_and_function_valued_property() {
        let analysis = analyze_source("members.ts", CLASS_WITH_MEMBERS, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        let foo = Some((SymbolKind::Class, "Foo", 1));
        assert_eq!(
            blocks_with_parents(&analysis),
            [
                (1, 26, SymbolKind::Class, None, None),
                (3, 5, SymbolKind::Constructor, Some("constructor(private readonly deps: Deps)"), foo),
                (6, 8, SymbolKind::Method, Some("async fetch(id: string): Promise<User>"), foo),
                (10, 12, SymbolKind::Method, Some("static create(): Foo"), foo),
                (13, 15, SymbolKind::Property, Some("handle = async (req: Req) =>"), foo),
                (16, 18, SymbolKind::Property, Some("plain = function ()"), foo),
                (19, 21, SymbolKind::Method, Some("get name(): string"), foo),
            ]
        );
    }

    #[test]
    fn a_member_block_holds_exactly_the_member_text() {
        let analysis = analyze_source("members.ts", CLASS_WITH_MEMBERS, AnalysisProfile::Full);
        let codes: Vec<&str> = analysis.blocks.iter().skip(1).take(2).map(|b| b.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "constructor(private readonly deps: Deps) {\n    this.deps = deps;\n  }",
                "async fetch(id: string): Promise<User> {\n    return this.deps.get(id);\n  }"
            ]
        );
    }

    #[rstest]
    #[case("export class Foo {\n  m() {\n  }\n}\n", 0)]
    #[case("export class Foo {\n  m() {\n    return 1;\n  }\n}\n", 1)]
    #[case("export class Foo {\n  m = () => {\n  };\n}\n", 0)]
    #[case("export class Foo {\n  m = () => {\n    return 1;\n  };\n}\n", 1)]
    fn a_member_shorter_than_three_lines_has_no_block(#[case] source: &str, #[case] members: usize) {
        let analysis = analyze_source("short.ts", source, AnalysisProfile::Full);
        assert_eq!(analysis.blocks.len(), 1 + members);
    }

    #[rstest]
    #[case::declared("class Foo {\n  m() {\n    return 1;\n  }\n}\n")]
    #[case::exported("export class Foo {\n  m() {\n    return 1;\n  }\n}\n")]
    #[case::default_exported("export default class Foo {\n  m() {\n    return 1;\n  }\n}\n")]
    fn members_of_a_declared_or_exported_class_name_their_class(#[case] source: &str) {
        let analysis = analyze_source("named.ts", source, AnalysisProfile::Full);
        let parents: Vec<_> =
            analysis.blocks.iter().map(|b| b.parent.as_ref().map(|p| (p.kind, p.name.as_str()))).collect();
        assert_eq!(parents, [None, Some((SymbolKind::Class, "Foo"))]);
    }

    #[rstest]
    #[case::anonymous_default("export default class {\n  m() {\n    return 1;\n  }\n}\n")]
    #[case::nested_in_a_function("function f() {\n  class Foo {\n    m() {\n      return 1;\n    }\n  }\n}\n")]
    #[case::class_expression("const Foo = class {\n  m() {\n    return 1;\n  }\n};\n")]
    fn a_class_without_a_name_or_outside_the_top_level_gets_no_member_blocks(#[case] source: &str) {
        let analysis = analyze_source("other.ts", source, AnalysisProfile::Full);
        assert_eq!(analysis.blocks.len(), 1);
    }

    #[test]
    fn methods_without_a_body_and_other_class_elements_get_no_block() {
        let source = "\
export abstract class Base {
  abstract run(
    input: string,
  ): void;
  overload(a: string): void;
  overload(a: number): void;
  overload(a: unknown): void {
    return;
  }
  static {
    init();
    init();
  }
  [key: string]: unknown;
}
";
        let analysis = analyze_source("abstract.ts", source, AnalysisProfile::Full);
        assert_eq!(
            blocks_with_parents(&analysis),
            [
                (1, 15, SymbolKind::Class, None, None),
                (7, 9, SymbolKind::Method, Some("overload(a: unknown): void"), Some((SymbolKind::Class, "Base", 1))),
            ]
        );
    }

    #[test]
    fn decorators_are_left_out_of_a_member_signature_but_stay_in_its_block() {
        let source = "\
export class Ctl {
  @Get(':id')
  find(@Param('id') id: string) {
    return id;
  }
}
";
        let analysis = analyze_source("decorated.ts", source, AnalysisProfile::Full);
        let member = &analysis.blocks[1];
        assert_eq!(
            (member.line_start, member.line_end, member.symbol_signature.as_deref()),
            (2, 5, Some("find(@Param('id') id: string)"))
        );
        assert_eq!(member.code, "@Get(':id')\n  find(@Param('id') id: string) {\n    return id;\n  }");
    }

    #[test]
    fn only_the_full_profile_extracts_member_blocks() {
        for profile in [AnalysisProfile::Dependencies, AnalysisProfile::Structural] {
            let analysis = analyze_source("members.ts", CLASS_WITH_MEMBERS, profile);
            assert_eq!(analysis.blocks.len(), 0, "{profile:?}");
        }
    }

    #[test]
    fn member_blocks_are_deterministic() {
        let first = analyze_source("members.ts", CLASS_WITH_MEMBERS, AnalysisProfile::Full);
        let second = analyze_source("members.ts", CLASS_WITH_MEMBERS, AnalysisProfile::Full);
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
    }

    const OBJECT_WITH_MEMBERS: &str = "\
export const api = {
  version: 1,
  async load(id: string): Promise<User> {
    return get(id);
  },
  save: (user: User) => {
    put(user);
  },
  fetch: async function (url: string) {
    return url;
  },
  get name(): string {
    return 'x';
  },
  set name(value: string) {
    store(value);
  },
  short() { return 1; },
  alias: (a: number) => a + 1,
  two: () => {
  },
  data: {
    nested() {
      return 1;
    },
  },
  ...rest,
};
";

    #[test]
    fn an_object_literal_yields_a_block_for_each_long_function_valued_property_and_method() {
        let analysis = analyze_source("api.ts", OBJECT_WITH_MEMBERS, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        let api = Some((SymbolKind::Variable, "api", 1));
        assert_eq!(
            blocks_with_parents(&analysis),
            [
                (1, 28, SymbolKind::Variable, None, None),
                (3, 5, SymbolKind::Method, Some("async load(id: string): Promise<User>"), api),
                (6, 8, SymbolKind::Property, Some("save: (user: User) =>"), api),
                (9, 11, SymbolKind::Property, Some("fetch: async function (url: string)"), api),
                (12, 14, SymbolKind::Method, Some("get name(): string"), api),
                (15, 17, SymbolKind::Method, Some("set name(value: string)"), api),
            ]
        );
    }

    #[test]
    fn an_object_member_block_holds_exactly_the_member_text() {
        let analysis = analyze_source("api.ts", OBJECT_WITH_MEMBERS, AnalysisProfile::Full);
        let codes: Vec<&str> = analysis.blocks.iter().skip(1).take(2).map(|b| b.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "async load(id: string): Promise<User> {\n    return get(id);\n  }",
                "save: (user: User) => {\n    put(user);\n  }"
            ]
        );
    }

    #[rstest]
    #[case::declared("const api = OBJ;\n", (SymbolKind::Variable, "api"))]
    #[case::exported("export const api = OBJ;\n", (SymbolKind::Variable, "api"))]
    #[case::let_binding("let api = OBJ;\n", (SymbolKind::Variable, "api"))]
    #[case::as_const("export const api = OBJ as const;\n", (SymbolKind::Variable, "api"))]
    #[case::satisfies("export const api = OBJ satisfies Api;\n", (SymbolKind::Variable, "api"))]
    #[case::parenthesized("export const api = (OBJ);\n", (SymbolKind::Variable, "api"))]
    #[case::typed("export const api: Api = OBJ;\n", (SymbolKind::Variable, "api"))]
    #[case::default_export("export default OBJ;\n", (SymbolKind::Other, "default"))]
    #[case::default_export_as("export default OBJ as Api;\n", (SymbolKind::Other, "default"))]
    fn members_of_a_declared_or_exported_object_literal_name_it(
        #[case] source: &str,
        #[case] parent: (SymbolKind, &str),
    ) {
        let source = source.replace("OBJ", "{\n  m() {\n    return 1;\n  },\n}");
        let analysis = analyze_source("named.ts", &source, AnalysisProfile::Full);
        let parents: Vec<_> =
            analysis.blocks.iter().map(|b| b.parent.as_ref().map(|p| (p.kind, p.name.as_str()))).collect();
        assert_eq!(parents, [None, Some(parent)]);
    }

    #[test]
    fn each_declarator_of_a_statement_owns_its_own_members() {
        let source = "const a = {\n  m() {\n    return 1;\n  },\n}, b = {\n  n() {\n    return 2;\n  },\n};\n";
        let analysis = analyze_source("two.ts", source, AnalysisProfile::Full);
        assert_eq!(
            blocks_with_parents(&analysis),
            [
                (1, 9, SymbolKind::Variable, None, None),
                (2, 4, SymbolKind::Method, Some("m()"), Some((SymbolKind::Variable, "a", 1))),
                (6, 8, SymbolKind::Method, Some("n()"), Some((SymbolKind::Variable, "b", 5))),
            ]
        );
    }

    /// Pins the extraction boundary: these shapes are documented gaps or too small to split, and must stay inside
    /// their holder's block.
    #[rstest]
    #[case::too_short_method("const o = {\n  m() {\n  },\n};\n")]
    #[case::too_short_arrow("const o = {\n  m: () => {\n  },\n};\n")]
    #[case::data_property("const o = {\n  m: {\n    a: 1,\n    b: 2,\n  },\n};\n")]
    #[case::call_value("const o = {\n  m: wrap(() => {\n    return 1;\n  }),\n};\n")]
    #[case::spread("const o = {\n  ...other,\n  ...more,\n  ...rest,\n};\n")]
    #[case::computed_data("const o = {\n  [key]: 1,\n  [other]: 2,\n  x: 3,\n};\n")]
    #[case::inside_a_function("function f() {\n  const o = {\n    m() {\n      return 1;\n    },\n  };\n}\n")]
    #[case::inside_a_block("{\n  const o = {\n    m() {\n      return 1;\n    },\n  };\n}\n")]
    #[case::conditional("const o = c ? {\n  m() {\n    return 1;\n  },\n} : null;\n")]
    #[case::call_argument("const o = make({\n  m() {\n    return 1;\n  },\n});\n")]
    #[case::destructured("const { a } = {\n  m() {\n    return 1;\n  },\n};\n")]
    #[case::assigned("module.exports = {\n  m() {\n    return 1;\n  },\n};\n")]
    #[case::class_property_object("export class C {\n  o = {\n    m() {\n      return 1;\n    },\n  };\n}\n")]
    fn only_a_top_level_or_exported_object_literal_with_long_functions_gets_member_blocks(#[case] source: &str) {
        let analysis = analyze_source("none.ts", source, AnalysisProfile::Full);
        let members: Vec<_> = analysis.blocks.iter().filter(|b| b.parent.is_some()).map(|b| b.line_start).collect();
        assert_eq!(members, Vec::<usize>::new());
    }

    /// Only the direct properties of the literal are split; whatever a member holds stays inside its block.
    #[rstest]
    #[case::arrow_in_arrow("export const o = {\n  a: () => {\n    const inner = () => {\n      return 1;\n    };\n  },\n};\n", &[(2, 6)])]
    #[case::object_in_method("export const o = {\n  a() {\n    return {\n      b() {\n        return 1;\n      },\n    };\n  },\n};\n", &[(2, 8)])]
    #[case::object_in_property("export const o = {\n  a: {\n    b() {\n      return 1;\n    },\n  },\n  c() {\n    return 2;\n  },\n};\n", &[(7, 9)])]
    #[case::class_in_method("export const o = {\n  a() {\n    class K {\n      m() {\n        return 1;\n      }\n    }\n  },\n};\n", &[(2, 8)])]
    #[case::sibling_after_nested("export const o = {\n  a() {\n    return () => {\n      return 1;\n    };\n  },\n  b() {\n    return 2;\n  },\n};\n", &[(2, 6), (7, 9)])]
    fn nested_functions_stay_inside_the_member_that_holds_them(
        #[case] source: &str,
        #[case] members: &[(usize, usize)],
    ) {
        let analysis = analyze_source("nested.ts", source, AnalysisProfile::Full);
        let found: Vec<_> =
            analysis.blocks.iter().filter(|b| b.parent.is_some()).map(|b| (b.line_start, b.line_end)).collect();
        assert_eq!(found, members);
    }

    #[test]
    fn a_class_and_an_object_in_one_file_each_name_their_own_members() {
        let source =
            "export class C {\n  m() {\n    return 1;\n  }\n}\nexport const o = {\n  n() {\n    return 2;\n  },\n};\n";
        let analysis = analyze_source("both.ts", source, AnalysisProfile::Full);
        assert_eq!(
            blocks_with_parents(&analysis),
            [
                (1, 5, SymbolKind::Class, None, None),
                (2, 4, SymbolKind::Method, Some("m()"), Some((SymbolKind::Class, "C", 1))),
                (6, 10, SymbolKind::Variable, None, None),
                (7, 9, SymbolKind::Method, Some("n()"), Some((SymbolKind::Variable, "o", 6))),
            ]
        );
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
        assert_eq!(analysis.degradation, None);
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
        assert_eq!(analysis.lang.ts().type_only_imports, Vec::<String>::new());
    }

    #[rstest]
    #[case::brackets("[", "]")]
    #[case::calls("f(", ")")]
    #[case::object_literals("{a:", "}")]
    fn brackets_nested_two_thousand_deep_are_parsed(#[case] open: &str, #[case] close: &str) {
        let text = nested("import a from './a';\nexport const x = ", open, "1", close, 2000);
        let analysis = analyze_source("deep.ts", &text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(symbols(&analysis), [("x", SymbolKind::Variable, 2, None)]);
        assert_eq!(dependencies(&analysis), [("./a", 1, vec![("default", "a", BindingKind::Default)])]);
    }

    #[test]
    fn a_string_continued_across_crlf_line_breaks_may_hold_many_parentheses() {
        let text = format!("export const s = \"{}\";\r\nexport function after() {{}}\r\n", "(\\\r\n".repeat(300));
        let analysis = analyze_source("continued.ts", &text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        let after = Some("function after()");
        assert_eq!(
            symbols(&analysis),
            [("s", SymbolKind::Variable, 1, None), ("after", SymbolKind::Function, 302, after)]
        );
    }

    #[test]
    fn a_postfix_increment_before_a_division_is_not_a_regex() {
        let text = format!(
            "export function f(values: number[], count: number, total: number) {{\n{}}}\nexport const y = 1;\n",
            "  values.push(count++ / total);\n".repeat(300)
        );
        let analysis = analyze_source("division.ts", &text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(
            symbols(&analysis),
            [
                ("f", SymbolKind::Function, 1, Some("function f(values: number[], count: number, total: number)")),
                ("y", SymbolKind::Variable, 303, None)
            ]
        );
    }

    #[test]
    fn a_file_longer_than_the_parse_limit_is_degraded_for_its_size() {
        let head = "export const x = 1;\n";
        let text = format!("{head}{}", " ".repeat(parse::MAX_PARSED_SOURCE_LEN + 1 - head.len()));
        let analysis = analyze_source("long.ts", &text, AnalysisProfile::Full);
        assert_degraded(&analysis);
        assert_eq!(
            analysis.degradation,
            Some(Degradation::TooLarge { bytes: parse::MAX_PARSED_SOURCE_LEN + 1, cap: parse::MAX_PARSED_SOURCE_LEN })
        );
        assert_eq!(analysis.digest, Digest::blake3(text.as_bytes()));
    }

    #[test]
    fn a_file_as_long_as_the_parse_limit_is_still_parsed() {
        let head = "export const x = 1;\n";
        let text = format!("{head}{}", " ".repeat(parse::MAX_PARSED_SOURCE_LEN - head.len()));
        let analysis = analyze_source("long.ts", &text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(analysis.degradation, None);
        assert_eq!(symbols(&analysis), [("x", SymbolKind::Variable, 1, None)]);
    }

    #[test]
    fn a_quoted_regex_before_deeply_nested_type_brackets_still_parses() {
        // Brackets in a type cost the most stack per byte.
        let text = format!(
            "export function f() {{ return /'/; }} export type T = {}number{};\n",
            "[".repeat(3_000),
            "]".repeat(3_000)
        );
        let analysis = analyze_source("regex.ts", &text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(
            symbols(&analysis),
            [("f", SymbolKind::Function, 1, Some("function f()")), ("T", SymbolKind::TypeAlias, 1, None)]
        );
    }

    /// `head`, then `open` `depth` times, `middle`, and `close` `depth` times.
    fn nested(head: &str, open: &str, middle: &str, close: &str, depth: usize) -> String {
        format!("{head}{}{middle}{};\n", open.repeat(depth), close.repeat(depth))
    }

    #[rstest]
    #[case::generic_types(
        "deep.ts",
        nested("export type T = ", "Array<", "number", ">", 4_000),
        ("T", SymbolKind::TypeAlias),
        0
    )]
    #[case::arrow_functions("deep.ts", nested("export const f = ", "() => ", "1", "", 5_000), ("f", SymbolKind::Variable), 0)]
    #[case::conditionals("deep.ts", nested("export const t = ", "a ? b : ", "c", "", 3_500), ("t", SymbolKind::Variable), 0)]
    #[case::jsx_elements(
        "deep.tsx",
        nested("export const j = ", "<div>", "", "</div>", 1_500),
        ("j", SymbolKind::Variable),
        1_500
    )]
    fn deep_nesting_without_brackets_still_parses(
        #[case] path: &str,
        #[case] text: String,
        #[case] symbol: (&str, SymbolKind),
        #[case] jsx_elements: usize,
    ) {
        let analysis = analyze_source(path, &text, AnalysisProfile::Full);
        assert!(!analysis.had_parse_errors);
        assert_eq!(symbols(&analysis), [(symbol.0, symbol.1, 1, None)]);
        assert_eq!(blocks(&analysis), [(1, 1, symbol.1, None)]);
        assert_eq!(analysis.jsx_elements().len(), jsx_elements);
    }

    /// `export const x = <open>^depth <tail> <close>^depth;`
    fn expr(depth: usize, open: &str, tail: &str, close: &str) -> String {
        format!("export const x = {}{tail}{};\n", open.repeat(depth), close.repeat(depth))
    }

    type Shape = (&'static str, &'static str, Box<dyn Fn(usize) -> String>);

    /// The ways TypeScript nests that the parser and the extractor recurse over, each as a path and a function of its
    /// depth. Every one of these was measured to cost at most 1.7 KB of stack per unit of the nesting estimate in a build
    /// without optimizations, which is what the fixed stack is sized from.
    fn shapes() -> Vec<Shape> {
        let shape = |name, path, text: fn(usize) -> String| -> Shape { (name, path, Box::new(text)) };
        vec![
            shape("arrays", "a.ts", |d| expr(d, "[", "1", "]")),
            shape("parentheses", "a.ts", |d| expr(d, "(", "1", ")")),
            shape("objects", "a.ts", |d| expr(d, "{a:", "1", "}")),
            shape("calls", "a.ts", |d| expr(d, "f(", "1", ")")),
            shape("arrows", "a.ts", |d| expr(d, "() => ", "1", "")),
            shape("conditionals", "a.ts", |d| expr(d, "a ? b : ", "c", "")),
            shape("negations", "a.ts", |d| expr(d, "!", "a", "")),
            shape("assignments", "a.ts", |d| expr(d, "a = ", "1", "")),
            shape("member chains", "a.ts", |d| format!("export const x = a{};\n", ".b".repeat(d))),
            shape("sums", "a.ts", |d| format!("export const x = 1{};\n", "+1".repeat(d))),
            shape("call chains", "a.ts", |d| format!("export const x = a{};\n", "()".repeat(d))),
            shape("casts", "a.ts", |d| format!("export const x = a{};\n", " as T".repeat(d))),
            shape("array types", "a.ts", |d| format!("export type T = {}number{};\n", "[".repeat(d), "]".repeat(d))),
            shape("generic types", "a.ts", |d| format!("export type T = {}number{};\n", "A<".repeat(d), ">".repeat(d))),
            shape("conditional types", "a.ts", |d| format!("export type T = {}A;\n", "A extends B ? C : ".repeat(d))),
            shape("generic arguments with commas", "a.ts", |d| {
                format!("export type T = {}A{};\n", "A<x,".repeat(d), ">".repeat(d))
            }),
            shape("function types", "a.ts", |d| format!("export type T = {}A;\n", "() => ".repeat(d))),
            shape("parenthesized types", "a.ts", |d| format!("export type T = {}A{};\n", "(".repeat(d), ")".repeat(d))),
            shape("object types", "a.ts", |d| format!("export type T = {}A{};\n", "{a:".repeat(d), "}".repeat(d))),
            shape("keyof chains", "a.ts", |d| format!("export type T = {}A;\n", "keyof ".repeat(d))),
            shape("jsx elements", "a.tsx", |d| expr(d, "<div>", "", "</div>")),
            shape("jsx containers", "a.tsx", |d| expr(d, "<div>{", "1", "}</div>")),
            shape("jsx attributes", "a.tsx", |d| expr(d, "<div a={", "1", "}/>")),
            shape("blocks", "a.ts", |d| format!("{}{}\n", "{".repeat(d), "}".repeat(d))),
            shape("if statements", "a.ts", |d| format!("{}x;\n", "if(1) ".repeat(d))),
            shape("else if chains", "a.ts", |d| format!("if(1){{}}{}\n", " else if(1){}".repeat(d))),
            shape("templates", "a.ts", |d| expr(d, "`${", "1", "}`")),
            shape("class expressions", "a.ts", |d| expr(d, "class{x=", "1", "}")),
            shape("class heritage", "a.ts", |d| {
                format!("export const x = {}A{};\n", "class extends ".repeat(d), " {}".repeat(d))
            }),
            shape("function expressions", "a.ts", |d| expr(d, "function(){return ", "1", "}")),
            shape("awaits", "a.ts", |d| expr(d, "await ", "a", "")),
            shape("news", "a.ts", |d| expr(d, "new ", "a", "")),
            shape("spreads", "a.ts", |d| expr(d, "[...", "a", "]")),
            shape("arrow bodies", "a.ts", |d| expr(d, "(a) => (", "1", ")")),
            shape("namespaces", "a.ts", |d| format!("{}{}\n", "namespace A { ".repeat(d), "}".repeat(d))),
            shape("labels", "a.ts", |d| format!("{}x;\n", "a: ".repeat(d))),
            shape("imports", "a.ts", |d| expr(d, "import(", "'a'", ")")),
            shape("do loops", "a.ts", |d| format!("{}x;{}\n", "do ".repeat(d), " while(1)".repeat(d))),
        ]
    }

    /// The deepest `depth` for which `text(depth)` is nested within the cap.
    fn deepest_accepted(path: &str, text: &dyn Fn(usize) -> String) -> usize {
        let syntax = nesting::Syntax::TypeScript { jsx: path.ends_with('x') };
        let accepted = |depth: usize| nesting::check(&text(depth), syntax, parse::MAX_NESTING_LEVELS).is_ok();
        let (mut accepted_depth, mut rejected_depth) = (1, 16 * parse::MAX_NESTING_LEVELS);
        assert!(accepted(accepted_depth) && !accepted(rejected_depth));
        while rejected_depth - accepted_depth > 1 {
            let middle = usize::midpoint(accepted_depth, rejected_depth);
            if accepted(middle) {
                accepted_depth = middle;
            } else {
                rejected_depth = middle;
            }
        }
        accepted_depth
    }

    /// Every shape at the deepest depth the estimate accepts runs through the real parser and extractor on the fixed
    /// stack. A stack that is too small aborts the test process, so this passing is the safety factor holding in an
    /// unoptimized build, which uses the most stack per level.
    #[test]
    fn every_shape_at_the_cap_is_parsed_on_the_fixed_stack() {
        for (name, path, text) in shapes() {
            let depth = deepest_accepted(path, &*text);
            let analysis = analyze_source(path, &text(depth), AnalysisProfile::Full);
            assert_eq!(analysis.degradation, None, "{name} at depth {depth}");
            assert!(!analysis.had_parse_errors, "{name} at depth {depth}");
        }
    }

    #[test]
    fn every_shape_one_past_the_cap_is_degraded_without_being_parsed() {
        for (name, path, text) in shapes() {
            let depth = deepest_accepted(path, &*text) + 1;
            let analysis = analyze_source(path, &text(depth), AnalysisProfile::Full);
            assert_degraded(&analysis);
            assert_eq!(
                analysis.degradation,
                Some(Degradation::TooDeep { depth: parse::MAX_NESTING_LEVELS + 1, cap: parse::MAX_NESTING_LEVELS }),
                "{name} at depth {depth}"
            );
        }
    }

    #[test]
    fn a_file_at_the_cap_keeps_its_rows_and_one_above_it_has_none() {
        let build =
            |depth: usize| format!("export const before = 1;\n{}export const after = 2;\n", expr(depth, "[", "1", "]"));
        let depth = deepest_accepted("a.ts", &build);
        let at = analyze_source("a.ts", &build(depth), AnalysisProfile::Full);
        let names: Vec<_> = at.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["before", "x", "after"]);
        let above = analyze_source("a.ts", &build(depth + 1), AnalysisProfile::Full);
        assert_degraded(&above);
        assert_eq!(
            above.degradation,
            Some(Degradation::TooDeep { depth: parse::MAX_NESTING_LEVELS + 1, cap: parse::MAX_NESTING_LEVELS })
        );
    }

    #[rstest]
    #[case::arrays("a.ts", "[", "]")]
    #[case::object_types("a.ts", "{a:", "}")]
    #[case::jsx_containers("a.tsx", "<a>{", "}</a>")]
    fn a_hundred_thousand_levels_are_degraded_at_once(#[case] path: &str, #[case] open: &str, #[case] close: &str) {
        let text = expr(100_000, open, "1", close);
        let started = std::time::Instant::now();
        let analysis = analyze_source(path, &text, AnalysisProfile::Full);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
        assert_degraded(&analysis);
        assert!(matches!(analysis.degradation, Some(Degradation::TooDeep { cap: parse::MAX_NESTING_LEVELS, .. })));
    }

    #[rstest]
    #[case::a_string_in_jsx_text("a.tsx", "export const x = <p>'{", "}'</p>;")]
    #[case::a_comment_in_jsx_text("a.jsx", "export const x = <p>//{", "}\n</p>;")]
    #[case::a_regex_in_jsx_text("a.js", "export const x = <p>/{", "}/</p>;")]
    #[case::a_double_quote_in_jsx_text("a.tsx", "export const x = <p>\"{", "}\"</p>;")]
    fn nesting_hidden_in_jsx_text_that_looks_like_a_string_or_comment_is_still_counted(
        #[case] path: &str,
        #[case] open: &str,
        #[case] close: &str,
    ) {
        let text = format!("{open}{}1{}{close}\n", "(".repeat(10_000), ")".repeat(10_000));
        let braces = format!("{open}{}1{}{close}\n", "{a:".repeat(10_000), "}".repeat(10_000));
        for text in [text, braces] {
            let analysis = analyze_source(path, &text, AnalysisProfile::Full);
            assert!(
                matches!(analysis.degradation, Some(Degradation::TooDeep { .. })),
                "{path}: {:?}",
                analysis.degradation
            );
        }
    }

    #[rstest]
    #[case::operators("!")]
    #[case::awaits("await ")]
    #[case::casts("as T ")]
    fn nesting_hidden_behind_a_brace_in_a_string_is_counted_in_a_jsx_file(#[case] chain: &str) {
        let text = format!("export const x = <p>'{{{}a}}'</p>;\n", chain.repeat(50_000));
        let analysis = analyze_source("a.tsx", &text, AnalysisProfile::Full);
        assert!(matches!(analysis.degradation, Some(Degradation::TooDeep { .. })), "{:?}", analysis.degradation);
    }

    #[test]
    fn a_closing_bracket_in_a_string_does_not_end_the_nesting_before_it() {
        let text = format!(
            "export const x = {}1{};\nexport const y = ')))))))))))))';\n",
            "[".repeat(3_000),
            "]".repeat(3_000)
        );
        let analysis = analyze_source("a.tsx", &text, AnalysisProfile::Full);
        assert_eq!(analysis.degradation, None);
        let names: Vec<_> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["x", "y"]);
    }

    #[test]
    fn a_generated_file_of_three_and_a_half_megabytes_yields_its_rows() {
        let entry =
            "export const entry000000 = { id: 0, name: 'entry', tags: ['a', 'b'], nested: { x: 1, y: [1, 2, 3] } };\n";
        let count = 3_400_000 / entry.len();
        let text: String = (0..count).map(|index| entry.replace("000000", &format!("{index:06}"))).collect();
        assert!(text.len() > 3_300_000 && text.len() < parse::MAX_PARSED_SOURCE_LEN);
        let analysis = analyze_source("generated.ts", &text, AnalysisProfile::Structural);
        assert_eq!(analysis.degradation, None);
        assert_eq!(analysis.symbols.len(), count);
        assert_eq!(analysis.symbols[count - 1].name, format!("entry{:06}", count - 1));
    }

    #[test]
    fn a_long_chain_of_additions_is_parsed_up_to_the_cap() {
        let text = format!("export const x = 1{};\n", "+1".repeat(4_000));
        let analysis = analyze_source("sum.ts", &text, AnalysisProfile::Full);
        assert_eq!(analysis.degradation, None);
        assert_eq!(symbols(&analysis), [("x", SymbolKind::Variable, 1, None)]);
    }

    #[test]
    fn code_without_semicolons_is_not_charged_for_the_lines_before() {
        let text = "const a = f(1)\n".repeat(20_000);
        let analysis = analyze_source("nosemi.ts", &text, AnalysisProfile::Structural);
        assert_eq!(analysis.degradation, None);
        assert_eq!(analysis.symbols.len(), 20_000);
    }

    #[test]
    fn a_long_string_a_long_comment_and_a_long_template_cost_nothing() {
        let text = format!(
            "export const a = '{}';\n/* {} */\nexport const b = `{}`;\n// {}\nexport const c = /{}/;\n",
            "(".repeat(100_000),
            "[".repeat(100_000),
            "{".repeat(100_000),
            "(".repeat(100_000),
            "(".repeat(100_000),
        );
        let analysis = analyze_source("long.ts", &text, AnalysisProfile::Structural);
        assert_eq!(analysis.degradation, None);
        let names: Vec<_> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
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
