use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use rstest::rstest;

use super::*;
use crate::analysis::Degradation;
use crate::analysis::registry::AnalyzerRegistry;
use crate::pipeline::{SearchRequest, SearchResult, search};
use crate::types::{CodeBlock, DiagnosticKind, Language, SymbolKind, Visibility};

fn analyze_with(text: &str, profile: AnalysisProfile) -> Arc<FileAnalysis> {
    let text = text.to_owned();
    RustAnalyzer::new().analyze_with_profile(Utf8Path::new("src/lib.rs"), Box::new(move || Ok(text)), profile).unwrap()
}

fn analyze_source(text: &str) -> Arc<FileAnalysis> {
    analyze_with(text, AnalysisProfile::Full)
}

/// First line, last line, kind, signature and parent kind and name of a block.
type Row<'a> = (usize, usize, SymbolKind, &'a str, Option<(SymbolKind, &'a str)>);

fn rows(blocks: &[CodeBlock]) -> Vec<Row<'_>> {
    blocks
        .iter()
        .map(|b| {
            (
                b.line_start,
                b.line_end,
                b.node_kind,
                b.symbol_signature.as_deref().unwrap(),
                b.parent.as_ref().map(|p| (p.kind, p.name.as_str())),
            )
        })
        .collect()
}

#[rstest]
#[case("a.rs", true)]
#[case("src/deep/dir/lib.rs", true)]
#[case("build.rs", true)]
#[case("a.rs.bak", false)]
#[case("a.ts", false)]
#[case("rs", false)]
#[case("Makefile", false)]
fn supports_follows_the_extension(#[case] path: &str, #[case] expected: bool) {
    let analyzer = RustAnalyzer::new();
    let path = Utf8Path::new(path);
    assert_eq!(analyzer.supports(path), expected);
    assert_eq!(analyzer.language_for(path), expected.then_some(LanguageId::Rust));
}

#[test]
fn it_reports_its_language_and_capabilities() {
    let analyzer = RustAnalyzer::new();
    assert_eq!(analyzer.languages(), [LanguageId::Rust]);
    assert_eq!(
        analyzer.capabilities(LanguageId::Rust),
        AnalyzerCapabilities {
            blocks: true,
            classified_imports: false,
            resolved_import_graph: CapabilityPrecision::Unsupported,
            refs: CapabilityPrecision::Unsupported,
            definition: CapabilityPrecision::Unsupported,
            inspect_detail: CapabilityPrecision::Unsupported,
        }
    );
}

const KINDS: &str = "\
use std::fmt;

/// Adds two values.
#[inline]
pub fn add<T: Add>(a: T, b: T) -> T
where
    T: Copy,
{
    a + b
}

pub(crate) struct Point<T> {
    x: T,
}

struct Unit;
pub enum Mode {
    On,
    Off,
}
pub trait Shape {
    fn area(&self) -> f64;
}
pub type Result<T> = std::result::Result<T, Error>;
const LIMIT: usize = 8;
static mut COUNT: u32 = 0;
macro_rules! twice {
    ($e:expr) => {
        $e + $e
    };
}
union Bits {
    a: u32,
}
";

#[test]
fn each_kind_of_item_is_a_block_with_its_header_as_the_signature() {
    let analysis = analyze_source(KINDS);
    assert_eq!(analysis.language(), "rust");
    assert!(!analysis.had_parse_errors);
    assert_eq!(analysis.degradation, None);
    assert_eq!(analysis.digest, Digest::blake3(KINDS.as_bytes()));
    assert_eq!(
        rows(&analysis.blocks),
        [
            (3, 10, SymbolKind::Function, "pub fn add<T: Add>(a: T, b: T) -> T where T: Copy", None),
            (12, 14, SymbolKind::Struct, "pub(crate) struct Point<T>", None),
            (16, 16, SymbolKind::Struct, "struct Unit", None),
            (17, 20, SymbolKind::Enum, "pub enum Mode", None),
            (21, 23, SymbolKind::Interface, "pub trait Shape", None),
            (24, 24, SymbolKind::TypeAlias, "pub type Result<T>", None),
            (25, 25, SymbolKind::Variable, "const LIMIT: usize", None),
            (26, 26, SymbolKind::Variable, "static mut COUNT: u32", None),
            (27, 31, SymbolKind::Macro, "macro_rules! twice", None),
            (32, 34, SymbolKind::Struct, "union Bits", None),
        ]
    );
    assert!(analysis.test_blocks.is_empty());
}

#[test]
fn a_block_keeps_its_attributes_and_doc_comments_but_its_signature_leaves_them_out() {
    let analysis = analyze_source(KINDS);
    assert_eq!(
        analysis.blocks[0].code,
        "/// Adds two values.\n#[inline]\npub fn add<T: Add>(a: T, b: T) -> T\nwhere\n    T: Copy,\n{\n    a + b\n}"
    );
    assert_eq!(analysis.blocks[1].code, "pub(crate) struct Point<T> {\n    x: T,\n}");
    assert_eq!(analysis.blocks[6].code, "const LIMIT: usize = 8;");
    assert!(analysis.blocks.iter().all(|b| b.file == "src/lib.rs"));
}

#[test]
fn symbols_carry_name_kind_line_signature_and_visibility() {
    let analysis = analyze_source(KINDS);
    let symbols: Vec<_> = analysis.symbols.iter().map(|s| (s.name.as_str(), s.kind, s.line, s.visibility)).collect();
    assert_eq!(
        symbols,
        [
            ("add", SymbolKind::Function, 3, Some(Visibility::Public)),
            ("Point", SymbolKind::Struct, 12, Some(Visibility::Crate)),
            ("Unit", SymbolKind::Struct, 16, Some(Visibility::Private)),
            ("Mode", SymbolKind::Enum, 17, Some(Visibility::Public)),
            ("Shape", SymbolKind::Interface, 21, Some(Visibility::Public)),
            ("Result", SymbolKind::TypeAlias, 24, Some(Visibility::Public)),
            ("LIMIT", SymbolKind::Variable, 25, Some(Visibility::Private)),
            ("COUNT", SymbolKind::Variable, 26, Some(Visibility::Private)),
            ("twice", SymbolKind::Macro, 27, Some(Visibility::Private)),
            ("Bits", SymbolKind::Struct, 32, Some(Visibility::Private)),
        ]
    );
    assert_eq!(analysis.symbols[0].signature.as_deref(), Some("pub fn add<T: Add>(a: T, b: T) -> T where T: Copy"));
}

#[test]
fn restricted_visibilities_map_to_module() {
    let analysis = analyze_source("pub(super) fn a() {}\npub(in crate::x) fn b() {}\npub(self) fn c() {}\n");
    let visibilities: Vec<_> = analysis.symbols.iter().map(|s| s.visibility).collect();
    assert_eq!(visibilities, [Some(Visibility::Module); 3]);
}

const IMPLS: &str = "\
struct Cart;

impl Cart {
    pub fn new() -> Self {
        Cart
    }

    fn len(&self) -> usize { 0 }

    pub fn total(&self, tax: f64) -> f64 {
        let base = 1.0;
        base * tax
    }
}

impl<T> fmt::Display for Wrapper<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, \"w\")
    }
}

impl Cart {
    fn two(&self) -> u8 {
        2 }
}
";

#[test]
fn impl_blocks_and_their_long_methods_are_blocks_with_the_impl_as_parent() {
    let analysis = analyze_source(IMPLS);
    let inherent = Some((SymbolKind::Impl, "Cart"));
    let display = Some((SymbolKind::Impl, "fmt::Display for Wrapper<T>"));
    assert_eq!(
        rows(&analysis.blocks),
        [
            (1, 1, SymbolKind::Struct, "struct Cart", None),
            (3, 14, SymbolKind::Impl, "impl Cart", None),
            (4, 6, SymbolKind::Method, "pub fn new() -> Self", inherent),
            (10, 13, SymbolKind::Method, "pub fn total(&self, tax: f64) -> f64", inherent),
            (16, 20, SymbolKind::Impl, "impl<T> fmt::Display for Wrapper<T>", None),
            (17, 19, SymbolKind::Method, "fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result", display),
            (22, 25, SymbolKind::Impl, "impl Cart", None),
        ]
    );
    assert_eq!(analysis.blocks[2].parent.as_ref().map(|p| p.line), Some(3));
    assert_eq!(analysis.blocks[5].parent.as_ref().map(|p| p.line), Some(16));
}

#[test]
fn method_symbols_name_the_owning_type_and_the_implemented_trait() {
    let analysis = analyze_source(IMPLS);
    let methods: Vec<_> = analysis
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .map(|s| (s.name.as_str(), s.line, s.owner.as_deref(), s.trait_impl.as_deref(), s.visibility))
        .collect();
    assert_eq!(
        methods,
        [
            ("new", 4, Some("Cart"), None, Some(Visibility::Public)),
            ("total", 10, Some("Cart"), None, Some(Visibility::Public)),
            ("fmt", 17, Some("Wrapper<T>"), Some("fmt::Display"), None),
        ]
    );
}

#[test]
fn a_trait_method_with_a_body_is_a_method_of_the_trait_and_one_without_is_not() {
    let analysis = analyze_source(
        "pub trait Greet {
    fn name(&self) -> String;

    fn greet(&self) -> String {
        let name = self.name();
        format!(\"hi {name}\")
    }
}
",
    );
    assert_eq!(
        rows(&analysis.blocks),
        [
            (1, 8, SymbolKind::Interface, "pub trait Greet", None),
            (4, 7, SymbolKind::Method, "fn greet(&self) -> String", Some((SymbolKind::Interface, "Greet"))),
        ]
    );
    let method = analysis.symbols.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!((method.owner.as_deref(), method.trait_impl.as_deref()), (Some("Greet"), None));
}

#[test]
fn inline_modules_hold_their_items_as_blocks_with_the_module_as_parent() {
    let analysis = analyze_source(
        "pub mod outer {
    pub fn one() {}

    mod inner {
        pub struct Deep;

        pub fn two() -> u8 {
            2
        }
    }
}

mod elsewhere;
",
    );
    let outer = Some((SymbolKind::Module, "outer"));
    let inner = Some((SymbolKind::Module, "inner"));
    assert_eq!(
        rows(&analysis.blocks),
        [
            (1, 11, SymbolKind::Module, "pub mod outer", None),
            (2, 2, SymbolKind::Function, "pub fn one()", outer),
            (4, 10, SymbolKind::Module, "mod inner", outer),
            (5, 5, SymbolKind::Struct, "pub struct Deep", inner),
            (7, 9, SymbolKind::Function, "pub fn two() -> u8", inner),
        ]
    );
}

#[test]
fn a_module_declared_in_another_file_has_no_block() {
    let analysis = analyze_source("mod elsewhere;\npub mod other;\n");
    assert_eq!(rows(&analysis.blocks), []);
    assert_eq!(analysis.symbols.len(), 0);
}

#[test]
fn use_and_extern_crate_declarations_are_not_blocks() {
    let analysis = analyze_source("use std::fmt;\nextern crate core;\n");
    assert_eq!(rows(&analysis.blocks), []);
    assert_eq!(analysis.symbols.len(), 0);
}

#[test]
fn item_macro_calls_and_extern_blocks_are_blocks_without_symbols() {
    let analysis = analyze_source(
        "thread_local! {\n    static X: u8 = 1;\n}\n\nlazy_static::lazy_static! {\n    static ref Y: u8 = 2;\n}\n\nextern \"C\" {\n    fn ffi(x: u32) -> u32;\n}\n\n#[cfg(test)]\nfixtures! {\n    a\n}\n",
    );
    assert_eq!(
        rows(&analysis.blocks),
        [
            (1, 3, SymbolKind::Macro, "thread_local!", None),
            (5, 7, SymbolKind::Macro, "lazy_static::lazy_static!", None),
            (9, 11, SymbolKind::Other, "extern \"C\"", None),
        ]
    );
    assert_eq!(rows(&analysis.test_blocks), [(13, 16, SymbolKind::Macro, "fixtures!", None)]);
    assert_eq!(analysis.symbols.len(), 0);
}

#[test]
fn a_search_finds_a_name_declared_inside_an_item_macro_or_extern_block() {
    let (_dir, root) = tree(&[(
        "m.rs",
        "thread_local! {\n    static TL_FROB: u32 = 1;\n}\n\nextern \"C\" {\n    fn ffi_decl_word(x: u32) -> u32;\n}\n",
    )]);
    assert_eq!(
        found(&find(&root, "TL_FROB", false, None), &root),
        [("m.rs".to_owned(), 1, 3, SymbolKind::Macro, None, vec![2])]
    );
    assert_eq!(
        found(&find(&root, "ffi_decl_word", false, None), &root),
        [("m.rs".to_owned(), 5, 7, SymbolKind::Other, None, vec![6])]
    );
}

const TESTS: &str = "\
pub fn live() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works() {
        live();
    }

    fn helper() {}
}

#[test]
fn loose() {}

#[tokio::test]
async fn awaited() {}

#[cfg(all(test, feature = \"x\"))]
fn only_in_tests() {}

#[cfg(not(test))]
fn only_outside_tests() {}

#[cfg(any(test, unix))]
fn maybe_tests() {}

#[cfg(feature = \"test\")]
fn feature_named_test() {}

impl Live {
    #[test]
    fn odd() {
        ()
    }
}
";

#[test]
fn code_under_cfg_test_and_test_attributes_is_test_code() {
    let analysis = analyze_source(TESTS);
    assert_eq!(
        rows(&analysis.blocks),
        [
            (1, 1, SymbolKind::Function, "pub fn live()", None),
            (24, 25, SymbolKind::Function, "fn only_outside_tests()", None),
            (27, 28, SymbolKind::Function, "fn maybe_tests()", None),
            (30, 31, SymbolKind::Function, "fn feature_named_test()", None),
            (33, 38, SymbolKind::Impl, "impl Live", None),
        ]
    );
    let module = Some((SymbolKind::Module, "tests"));
    assert_eq!(
        rows(&analysis.test_blocks),
        [
            (3, 13, SymbolKind::Module, "mod tests", None),
            (7, 10, SymbolKind::Function, "fn works()", module),
            (12, 12, SymbolKind::Function, "fn helper()", module),
            (15, 16, SymbolKind::Function, "fn loose()", None),
            (18, 19, SymbolKind::Function, "async fn awaited()", None),
            (21, 22, SymbolKind::Function, "fn only_in_tests()", None),
            (34, 37, SymbolKind::Method, "fn odd()", Some((SymbolKind::Impl, "Live"))),
        ]
    );
}

#[test]
fn test_code_has_no_symbols() {
    let analysis = analyze_source(TESTS);
    let names: Vec<_> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["live", "only_outside_tests", "maybe_tests", "feature_named_test", "Live"]);
}

#[test]
fn an_inner_cfg_test_attribute_makes_the_whole_file_test_code() {
    let analysis = analyze_source("#![cfg(test)]\n\nfn a() {}\nstruct B;\n");
    assert_eq!(rows(&analysis.blocks), []);
    assert_eq!(
        rows(&analysis.test_blocks),
        [(3, 3, SymbolKind::Function, "fn a()", None), (4, 4, SymbolKind::Struct, "struct B", None),]
    );
}

#[test]
fn a_file_with_a_syntax_error_keeps_the_items_around_it() {
    let analysis = analyze_source("fn good() {}\n\nfn broken() {\n    let x = ;\n}\n\nfn also_good() {\n    1\n}\n");
    assert!(analysis.had_parse_errors);
    assert_eq!(analysis.degradation, None);
    assert_eq!(analysis.blocks.len(), 3);
    let first = &analysis.blocks[0];
    assert_eq!((first.line_start, first.line_end, first.node_kind), (1, 1, SymbolKind::Function));
    assert_eq!(first.code, "fn good() {}");
    assert!(analysis.blocks.iter().any(|b| b.symbol_signature.as_deref() == Some("fn also_good()")));
}

#[test]
fn an_incomplete_item_is_still_a_block() {
    let analysis = analyze_source("fn broken(a: u8");
    assert!(analysis.had_parse_errors);
    assert_eq!(rows(&analysis.blocks), [(1, 1, SymbolKind::Function, "fn broken(a: u8", None)]);
}

#[test]
fn a_source_over_the_cap_is_degraded_without_being_parsed() {
    let text = format!("fn a() {{}}{}", " ".repeat(parse::MAX_PARSED_SOURCE_LEN));
    let analysis = analyze_source(&text);
    assert!(analysis.had_parse_errors);
    assert_eq!(
        analysis.degradation,
        Some(Degradation::TooLarge { bytes: text.len(), cap: parse::MAX_PARSED_SOURCE_LEN })
    );
    assert_eq!(rows(&analysis.blocks), []);
}

#[test]
fn a_source_at_the_cap_is_parsed() {
    let head = "fn a() {}";
    let text = format!("{head}{}", " ".repeat(parse::MAX_PARSED_SOURCE_LEN - head.len()));
    let analysis = analyze_source(&text);
    assert_eq!(analysis.degradation, None);
    assert_eq!(rows(&analysis.blocks), [(1, 1, SymbolKind::Function, "fn a()", None)]);
}

#[test]
fn profiles_fill_only_what_they_promise() {
    let dependencies = analyze_with(KINDS, AnalysisProfile::Dependencies);
    assert_eq!(dependencies.profile, AnalysisProfile::Dependencies);
    assert_eq!((dependencies.symbols.len(), dependencies.blocks.len(), dependencies.had_parse_errors), (0, 0, false));

    let structural = analyze_with(KINDS, AnalysisProfile::Structural);
    assert_eq!(structural.profile, AnalysisProfile::Structural);
    assert_eq!((structural.symbols.len(), structural.blocks.len(), structural.test_blocks.len()), (10, 0, 0));

    let full = analyze_with(KINDS, AnalysisProfile::Full);
    assert_eq!(full.profile, AnalysisProfile::Full);
    assert_eq!((full.symbols.len(), full.blocks.len()), (10, 10));
}

#[test]
fn multibyte_text_does_not_shift_lines_or_slices() {
    let analysis = analyze_source("// ünïcödé 日本語\nfn naïve() -> &'static str {\n    \"日本語\"\n}\n");
    assert_eq!(rows(&analysis.blocks), [(1, 4, SymbolKind::Function, "fn naïve() -> &'static str", None)]);
    assert_eq!(analysis.blocks[0].code, "// ünïcödé 日本語\nfn naïve() -> &'static str {\n    \"日本語\"\n}");
}

#[test]
fn analyzing_is_the_same_on_any_thread() {
    let analyzer = Arc::new(RustAnalyzer::new());
    let expected = format!("{:?}", analyze_source(IMPLS).blocks);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let analyzer = Arc::clone(&analyzer);
            std::thread::spawn(move || {
                let analysis =
                    analyzer.analyze(Utf8Path::new("src/lib.rs"), Box::new(|| Ok(IMPLS.to_owned()))).unwrap();
                format!("{:?}", analysis.blocks)
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), expected);
    }
}

fn nested(open: &str, close: &str, depth: usize) -> String {
    format!("pub fn deep() {{ let x = {}1{}; }}\npub fn after() {{}}\n", open.repeat(depth), close.repeat(depth))
}

fn signatures(analysis: &FileAnalysis) -> Vec<&str> {
    analysis.blocks.iter().map(|b| b.symbol_signature.as_deref().unwrap()).collect()
}

/// Lets the shared thread that drops syntax trees finish before the test process ends.
fn let_trees_drop() {
    std::thread::sleep(std::time::Duration::from_millis(300));
}

#[rstest]
#[case::parentheses("(", ")")]
#[case::arrays("[", "]")]
#[case::blocks("{", "}")]
#[case::closures("|a| ", "")]
#[case::negations("- ", "")]
#[case::references("&", "")]
#[case::tuples("(1, ", ")")]
#[case::calls("f(", ")")]
fn expressions_nested_hundreds_deep_are_parsed(#[case] open: &str, #[case] close: &str) {
    let analysis = analyze_source(&nested(open, close, 500));
    assert_eq!(analysis.degradation, None);
    assert_eq!(signatures(&analysis), ["pub fn deep()", "pub fn after()"]);
    let_trees_drop();
}

#[rstest]
#[case::parentheses("(", ")")]
#[case::blocks("{", "}")]
#[case::negations("- ", "")]
#[case::calls("f(", ")")]
fn expressions_nested_past_the_tree_cap_are_degraded_not_dropped_on_the_wrong_stack(
    #[case] open: &str,
    #[case] close: &str,
) {
    let analysis = analyze_source(&nested(open, close, parse::MAX_TREE_DEPTH + 1));
    assert!(analysis.had_parse_errors);
    assert!(matches!(analysis.degradation, Some(Degradation::TooDeep { cap: parse::MAX_TREE_DEPTH, .. })));
    assert_eq!(rows(&analysis.blocks), []);
    let_trees_drop();
}

#[test]
fn modules_nested_just_under_the_tree_cap_are_parsed() {
    let depth = parse::MAX_TREE_DEPTH / 2 - 10;
    let text = format!("{}pub fn leaf() {{}}\n{}", "mod m { ".repeat(depth), "}".repeat(depth));
    let analysis = analyze_source(&text);
    assert_eq!(analysis.degradation, None);
    assert_eq!(analysis.blocks.len(), depth + 1);
    assert_eq!(analysis.blocks[depth].symbol_signature.as_deref(), Some("pub fn leaf()"));
    let_trees_drop();
}

#[test]
fn the_tree_cap_degradation_names_the_depth_and_the_cap() {
    let depth = parse::MAX_TREE_DEPTH;
    let text = format!("{}pub fn leaf() {{}}\n{}", "mod m { ".repeat(depth), "}".repeat(depth));
    let analysis = analyze_source(&text);
    let Some(Degradation::TooDeep { depth: found, cap }) = analysis.degradation else {
        panic!("expected a depth degradation, got {:?}", analysis.degradation);
    };
    assert_eq!(cap, parse::MAX_TREE_DEPTH);
    assert_eq!(found, 2 * depth + 4);
    let_trees_drop();
}

#[test]
fn a_large_realistic_file_is_parsed() {
    let function = "pub fn handler(request: &Request) -> Result<Response, Error> {\n    let body = request.body();\n    Ok(Response::new(body))\n}\n\n";
    let analysis = analyze_source(&function.repeat(600));
    assert_eq!(analysis.degradation, None);
    assert_eq!(analysis.blocks.len(), 600);
}

fn tree(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
    (dir, root)
}

fn registry() -> AnalyzerRegistry {
    AnalyzerRegistry::new(vec![Arc::new(RustAnalyzer::new())])
}

fn find(root: &Utf8Path, query: &str, allow_tests: bool, language: Option<Language>) -> SearchResult {
    let request = SearchRequest { root, query, exact: false, allow_tests, respect_ignore_files: true, language };
    search(&registry(), &request).unwrap()
}

/// File, first line, last line, kind, parent name and matched lines of each result, in rank order.
type Found = (String, usize, usize, SymbolKind, Option<String>, Vec<usize>);

fn found(result: &SearchResult, root: &Utf8Path) -> Vec<Found> {
    result
        .ranked
        .iter()
        .map(|r| {
            let b = &r.block;
            (
                b.file.strip_prefix(root).unwrap().to_string(),
                b.line_start,
                b.line_end,
                b.node_kind,
                r.parent_context.as_ref().map(|p| p.name.clone()),
                b.matched_lines.clone(),
            )
        })
        .collect()
}

const CART_RS: &str = "\
pub struct Cart {
    items: Vec<u32>,
}

impl Cart {
    pub fn total_price(&self) -> u32 {
        let sum = self.items.iter().sum();
        sum
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn total_price_adds_up() {
        assert_eq!(1, 1);
    }
}
";

#[test]
fn a_search_returns_the_method_that_holds_the_match_and_not_its_impl() {
    let (_dir, root) = tree(&[("src/cart.rs", CART_RS)]);
    let result = find(&root, "total_price", false, None);
    assert_eq!(
        found(&result, &root),
        [("src/cart.rs".to_owned(), 6, 9, SymbolKind::Method, Some("Cart".to_owned()), vec![6])]
    );
    assert_eq!(result.diagnostics.len(), 0);
}

#[test]
fn a_match_outside_every_member_returns_the_holder() {
    let (_dir, root) = tree(&[("src/cart.rs", CART_RS)]);
    let result = find(&root, "Vec", false, None);
    assert_eq!(found(&result, &root), [("src/cart.rs".to_owned(), 1, 3, SymbolKind::Struct, None, vec![2])]);
}

#[test]
fn test_code_is_left_out_unless_asked_for() {
    let (_dir, root) = tree(&[("src/cart.rs", CART_RS)]);
    let without = find(&root, "adds_up", false, None);
    assert_eq!(found(&without, &root), []);
    assert_eq!(without.files_total, 1);

    let with = find(&root, "adds_up", true, None);
    assert_eq!(
        found(&with, &root),
        [("src/cart.rs".to_owned(), 18, 21, SymbolKind::Function, Some("tests".to_owned()), vec![19])]
    );
}

#[test]
fn asking_for_tests_adds_the_test_blocks_and_changes_nothing_else() {
    let (_dir, root) = tree(&[("src/cart.rs", CART_RS)]);
    let without = found(&find(&root, "total_price", false, None), &root);
    let with = found(&find(&root, "total_price", true, None), &root);
    assert_eq!(without.len(), 1);
    assert_eq!(with.len(), 2);
    assert!(with.contains(&without[0]));
    assert!(with.contains(&(
        "src/cart.rs".to_owned(),
        18,
        21,
        SymbolKind::Function,
        Some("tests".to_owned()),
        vec![19]
    )));
}

const TEST_METHODS_RS: &str = "\
struct Foo;

impl Foo {
    fn real(&self) -> u32 {
        let a = 1;
        a
    }

    #[cfg(test)]
    fn fixture_secret(&self) -> u32 {
        let leakword = 1;
        leakword
    }

    #[test]
    fn short_one(&self) { let leakword = 2; }
}

trait Tr {
    fn a(&self);

    #[cfg(test)]
    fn tr_test(&self) {
        let leakword = 3;
        let _ = leakword;
    }
}
";

#[test]
fn test_methods_in_a_regular_impl_or_trait_are_not_credited_to_the_holder() {
    let (_dir, root) = tree(&[("l.rs", TEST_METHODS_RS)]);
    for query in ["leakword", "fixture_secret", "short_one", "tr_test"] {
        assert_eq!(found(&find(&root, query, false, None), &root), [], "{query}");
    }
    let with = found(&find(&root, "leakword", true, None), &root);
    let mut spans: Vec<_> = with.iter().map(|row| (row.1, row.2, row.4.clone().unwrap())).collect();
    spans.sort();
    assert!(with.iter().all(|row| row.3 == SymbolKind::Method));
    assert_eq!(spans, [(9, 13, "Foo".to_owned()), (15, 16, "Foo".to_owned()), (22, 26, "Tr".to_owned())]);
}

#[test]
fn a_syntax_error_reports_a_degraded_file_and_still_returns_blocks() {
    let (_dir, root) = tree(&[("src/bad.rs", "fn fine() {\n    alpha();\n}\n\nstruct Oops {\n    a: ,\n")]);
    let result = find(&root, "alpha", false, None);
    assert_eq!(found(&result, &root), [("src/bad.rs".to_owned(), 1, 3, SymbolKind::Function, None, vec![2])]);
    assert_eq!(result.diagnostics.len(), 1);
    let diagnostic = &result.diagnostics[0];
    assert_eq!(diagnostic.kind, DiagnosticKind::Degraded);
    assert_eq!(diagnostic.path.as_deref(), Some(root.join("src/bad.rs").as_path()));
    assert_eq!(diagnostic.message, "parse errors; blocks may be incomplete");
}

#[cfg(feature = "lang-ts")]
#[test]
fn the_language_filter_isolates_rust_from_typescript() {
    let (_dir, root) = tree(&[("a.rs", "fn alpha() {}\n"), ("b.ts", "export function alpha() {}\n")]);
    let registry =
        AnalyzerRegistry::new(vec![Arc::new(RustAnalyzer::new()), Arc::new(crate::analysis::ts::TsAnalyzer::new())]);
    let run = |language| {
        let request = SearchRequest {
            root: &root,
            query: "alpha",
            exact: false,
            allow_tests: false,
            respect_ignore_files: true,
            language,
        };
        let result = search(&registry, &request).unwrap();
        let mut files: Vec<_> = found(&result, &root).into_iter().map(|row| row.0).collect();
        files.sort();
        files
    };
    assert_eq!(run(None), ["a.rs", "b.ts"]);
    assert_eq!(run(Some(Language::Rust)), ["a.rs"]);
    assert_eq!(run(Some(Language::TypeScript)), ["b.ts"]);
    assert_eq!(run(Some(Language::JavaScript)), Vec::<String>::new());
}
