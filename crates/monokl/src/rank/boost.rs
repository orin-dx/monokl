use super::bm25::{as_f64, round6};
use crate::tokens::tokenize_source;
use crate::types::{CodeBlock, SymbolKind};

/// Returns the fraction of the block's matched lines that fall inside `line_start..=line_end`, rounded with
/// [`round6`].
///
/// Returns 0.0 when `matched_lines` is empty. A block whose `line_end` is before its `line_start` contains no lines.
///
/// # Examples
///
/// ```
/// use camino::Utf8PathBuf;
/// use monokl::rank::coverage_boost;
/// use monokl::types::{CodeBlock, SymbolKind};
///
/// let block = CodeBlock {
///     file: Utf8PathBuf::from("src/a.ts"),
///     line_start: 10,
///     line_end: 20,
///     node_kind: SymbolKind::Function,
///     code: String::new(),
///     symbol_signature: None,
///     matched_lines: vec![12, 30, 40],
///     matched_keywords: Vec::new(),
///     parent: None,
/// };
/// assert_eq!(coverage_boost(&block), 0.333333);
/// ```
pub fn coverage_boost(block: &CodeBlock) -> f64 {
    if block.matched_lines.is_empty() {
        return 0.0;
    }
    let lines = block.line_start..=block.line_end;
    let inside = block.matched_lines.iter().filter(|line| lines.contains(*line)).count();
    round6(as_f64(inside) / as_f64(block.matched_lines.len()))
}

/// Returns the multiplier for how many query terms the block's declared name contains, from 1.0 to 3.0.
///
/// The name is the head of the block: its signature, or without one its first line that is not a comment or a
/// decorator, cut at the first `(`, `<`, `{`, `=` or `:`. So parameters, return types and the body do not count. In a
/// Rust file the parentheses of a restricted visibility (`pub(crate)`) do not cut the head. The
/// head is tokenized like a document, and the multiplier is `round6(1 + 2 * matched / query_terms.len())`, where
/// `matched` counts the query terms, repeats included, that the head contains. Returns 1.0 when `query_terms` is empty.
///
/// A body can mention every query word without the block being about them; a name that holds them says what it is.
///
/// # Examples
///
/// ```
/// use camino::Utf8PathBuf;
/// use monokl::rank::name_match_boost;
/// use monokl::types::{CodeBlock, SymbolKind};
///
/// let block = CodeBlock {
///     file: Utf8PathBuf::from("src/a.ts"),
///     line_start: 1,
///     line_end: 3,
///     node_kind: SymbolKind::Function,
///     code: String::new(),
///     symbol_signature: Some("function mergeConfig(a: Config, b: Config)".to_string()),
///     matched_lines: Vec::new(),
///     matched_keywords: Vec::new(),
///     parent: None,
/// };
/// assert_eq!(name_match_boost(&block, &["merge", "config", "deep", "copy"]), 2.0);
/// ```
pub fn name_match_boost(block: &CodeBlock, query_terms: &[&str]) -> f64 {
    if query_terms.is_empty() {
        return 1.0;
    }
    let text = block.symbol_signature.as_deref().unwrap_or(&block.code);
    let line = text.lines().map(str::trim).find(|line| !is_comment_or_decorator(line)).unwrap_or_default();
    let name_tokens = if block.file.extension() == Some("rs") {
        tokenize_source(&unrestricted(line))
    } else {
        tokenize_source(head_of(line))
    };
    let matched = query_terms.iter().filter(|term| name_tokens.iter().any(|token| token == *term)).count();
    round6(1.0 + 2.0 * as_f64(matched) / as_f64(query_terms.len()))
}

fn head_of(line: &str) -> &str {
    line.split(['(', '<', '{', '=', ':']).next().unwrap_or_default()
}

/// The head of a Rust declaration line with a restricted visibility (`pub(crate)`, `pub(super)`, `pub(in path)`) read as
/// a plain `pub`, whose `(` would otherwise end the head before the name. The other modifiers (`async`, `const`,
/// `unsafe`, `extern "C"`, `default`) hold no cut character and stay in the head.
fn unrestricted(line: &str) -> String {
    let restriction = line
        .strip_prefix("pub")
        .map(str::trim_start)
        .filter(|rest| rest.starts_with('('))
        .and_then(|rest| rest.split_once(')'));
    match restriction {
        Some((_, rest)) => format!("pub {}", head_of(rest)),
        None => head_of(line).to_owned(),
    }
}

/// Whether a trimmed line carries no declaration: it is blank, a comment or comment continuation, or a decorator.
fn is_comment_or_decorator(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with('*')
        || line.starts_with('@')
}

/// Returns the score multiplier for a block's symbol kind.
///
/// Callable units rank highest, then type definitions, then bindings; `Other` ranks lowest.
///
/// # Examples
///
/// ```
/// use monokl::rank::node_type_boost;
/// use monokl::types::SymbolKind;
///
/// assert_eq!(node_type_boost(SymbolKind::Function), 1.5);
/// assert_eq!(node_type_boost(SymbolKind::Other), 0.7);
/// ```
pub fn node_type_boost(kind: SymbolKind) -> f64 {
    match kind {
        SymbolKind::Function => 1.5,
        SymbolKind::Class | SymbolKind::Method | SymbolKind::Constructor => 1.4,
        SymbolKind::Interface | SymbolKind::Struct => 1.3,
        SymbolKind::TypeAlias | SymbolKind::Enum => 1.2,
        SymbolKind::Variable | SymbolKind::Impl => 1.1,
        SymbolKind::Property | SymbolKind::Field | SymbolKind::Macro => 1.0,
        SymbolKind::Module => 0.9,
        SymbolKind::Other => 0.7,
    }
}

#[cfg(test)]
#[expect(clippy::float_cmp, reason = "scores are rounded to 6 places, so exact equality is the contract under test")]
mod tests {
    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;

    fn block(line_start: usize, line_end: usize, matched_lines: Vec<usize>) -> CodeBlock {
        CodeBlock {
            file: Utf8PathBuf::from("src/a.ts"),
            line_start,
            line_end,
            node_kind: SymbolKind::Function,
            code: String::new(),
            symbol_signature: None,
            matched_lines,
            matched_keywords: Vec::new(),
            parent: None,
        }
    }

    fn rust(symbol_signature: &str) -> CodeBlock {
        CodeBlock { file: Utf8PathBuf::from("src/a.rs"), ..named("", Some(symbol_signature)) }
    }

    fn named(code: &str, symbol_signature: Option<&str>) -> CodeBlock {
        CodeBlock {
            code: code.to_string(),
            symbol_signature: symbol_signature.map(ToString::to_string),
            ..block(1, 9, vec![])
        }
    }

    // The head of the block (up to its first `(`, `<`, `{`, `=` or `:`) is tokenized and compared with the terms:
    // the multiplier is round6(1 + 2 * matched_terms / total_terms).
    #[rstest]
    // Head "function toJSONSchema" tokenizes to function, to, json, schema: 2 of 3 terms, 1 + 2 * 2/3.
    #[case::signature_head(named("", Some("function toJSONSchema<T>(schema: T): Json")), &["json", "schema", "convert"], 2.333_333)]
    // No signature: the first line, "export const safeParse", gives safe, parse; `export` and `const` are stop words.
    #[case::first_code_line(named("export const safeParse: $SafeParse = _safeParse(x);", None), &["safe", "parse", "result"], 2.333_333)]
    #[case::every_term_in_the_name(named("", Some("mergeConfig(a, b)")), &["merge", "config"], 3.0)]
    #[case::one_of_two_terms(named("", Some("mergeConfig(a, b)")), &["merge", "other"], 2.0)]
    #[case::no_term_in_the_name(named("", Some("mergeConfig(a, b)")), &["other"], 1.0)]
    #[case::the_body_is_not_the_name(named("function f() { return parse(); }", None), &["parse"], 1.0)]
    #[case::the_parameters_are_not_the_name(named("", Some("function f(parse: string)")), &["parse"], 1.0)]
    #[case::comment_lines_are_skipped(named("// parse it\n/* more */\n * parse\nclass Parser {}", None), &["parser", "it"], 2.0)]
    #[case::decorator_lines_are_skipped(named("@Injectable()\nclass Foo {}", None), &["foo", "injectable"], 2.0)]
    #[case::the_signature_wins_over_the_code(named("function other() {}", Some("function wanted()")), &["wanted"], 3.0)]
    #[case::repeated_query_terms_count_per_occurrence(named("", Some("parse()")), &["parse", "parse", "other", "other"], 2.0)]
    #[case::no_terms(named("", Some("parse()")), &[], 1.0)]
    #[case::rust_plain_pub(rust("pub fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_pub_crate(rust("pub(crate) fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_pub_super(rust("pub(super) fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_pub_self(rust("pub(self) fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_pub_in_path(rust("pub(in crate::config) fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_space_before_the_restriction(rust("pub (crate) fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_restriction_and_async(rust("pub(crate) async fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_restriction_and_const_fn(rust("pub(crate) const fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_restriction_and_unsafe_extern(rust("pub(crate) unsafe extern \"C\" fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_modifiers_without_visibility(rust("async unsafe fn load_config()"), &["load", "config"], 3.0)]
    #[case::rust_default_method(rust("default fn load_config(&self)"), &["load", "config"], 3.0)]
    #[case::rust_restricted_struct(rust("pub(crate) struct LoadConfig<T>"), &["load", "config"], 3.0)]
    #[case::rust_restricted_const(rust("pub(crate) const LOAD_CONFIG: u32"), &["load", "config"], 3.0)]
    #[case::rust_restricted_parameters_are_not_the_name(rust("pub(crate) fn run(load_config: u32)"), &["load"], 1.0)]
    #[case::rust_restricted_generics_are_not_the_name(rust("pub(crate) fn run<LoadConfig>()"), &["load"], 1.0)]
    #[case::only_rust_reads_a_restriction(named("", Some("pub(crate) load_config()")), &["load", "config"], 1.0)]
    #[case::empty_block(named("", None), &["parse"], 1.0)]
    fn name_match_boost_rewards_query_terms_in_the_declared_name(
        #[case] block: CodeBlock,
        #[case] terms: &[&str],
        #[case] expected: f64,
    ) {
        assert_eq!(name_match_boost(&block, terms), expected);
    }

    #[rstest]
    #[case(SymbolKind::Function, 1.5)]
    #[case(SymbolKind::Class, 1.4)]
    #[case(SymbolKind::Method, 1.4)]
    #[case(SymbolKind::Constructor, 1.4)]
    #[case(SymbolKind::Interface, 1.3)]
    #[case(SymbolKind::Struct, 1.3)]
    #[case(SymbolKind::TypeAlias, 1.2)]
    #[case(SymbolKind::Enum, 1.2)]
    #[case(SymbolKind::Variable, 1.1)]
    #[case(SymbolKind::Impl, 1.1)]
    #[case(SymbolKind::Property, 1.0)]
    #[case(SymbolKind::Field, 1.0)]
    #[case(SymbolKind::Macro, 1.0)]
    #[case(SymbolKind::Module, 0.9)]
    #[case(SymbolKind::Other, 0.7)]
    fn node_type_boost_returns_the_per_kind_multiplier(#[case] kind: SymbolKind, #[case] expected: f64) {
        assert_eq!(node_type_boost(kind), expected);
    }

    #[rstest]
    #[case::no_matched_lines(block(1, 10, vec![]), 0.0)]
    #[case::all_inside(block(1, 10, vec![1, 5, 10]), 1.0)]
    #[case::one_of_three_inside(block(10, 20, vec![12, 30, 40]), 0.333_333)]
    #[case::two_of_three_inside(block(10, 20, vec![10, 20, 21]), 0.666_667)]
    #[case::none_inside(block(10, 20, vec![9, 21]), 0.0)]
    #[case::single_line_block(block(7, 7, vec![7, 8]), 0.5)]
    #[case::duplicate_matched_lines_count_twice(block(1, 5, vec![3, 3, 9]), 0.666_667)]
    #[case::every_line_of_the_block(block(3, 5, vec![3, 4, 5]), 1.0)]
    #[case::one_of_three_inside_a_short_block(block(3, 5, vec![1, 4, 9]), 0.333_333)]
    #[case::a_duplicate_inside_counts_per_entry(block(3, 5, vec![4, 4, 9]), 0.666_667)]
    #[case::inverted_block_contains_nothing(block(10, 5, vec![5, 7, 10]), 0.0)]
    fn coverage_boost_is_the_rounded_fraction_of_matched_lines_inside_the_block(
        #[case] block: CodeBlock,
        #[case] expected: f64,
    ) {
        assert_eq!(coverage_boost(&block), expected);
    }
}
