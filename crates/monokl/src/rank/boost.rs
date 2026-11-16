use super::bm25::{as_f64, round6};
use crate::tokens::tokenize_source;
use crate::types::{CodeBlock, SymbolKind};

/// Returns the multiplier for how well the block's declared name matches the query, from 1.0 to 7.0.
///
/// The name is [`CodeBlock::name`], the identifier the analyzer read from the syntax tree, so parameters, return types
/// and the body do not count. Its tokens are those of [`tokenize_source`], deduplicated, so stop words such as `use` in
/// `useWatch` and `from` in `FromRequestParts` are not part of the name. Two shares say how the name and the query
/// overlap:
///
/// - `R`: query terms, repeats counted, that the name holds, over all query terms.
/// - `P`: name tokens that are query terms, over all name tokens.
///
/// The multiplier is `round6(1 + 2 * (R + P / 2) / 1.5 + 4 * exact)`, where `exact` is 1 when `R` and `P` are both 1
/// and 0 otherwise: the name is made of the query's words, and the query is made of the name's. Returns 1.0 when
/// `query_terms` is empty or the name has no tokens, including when the block declares no name.
///
/// A body can mention every query word without the block being about them; a name that holds them says what it is, and
/// a name that is exactly the query is the declaration the query asks for.
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
///     span: 0..0,
///     parent_index: None,
///     name: Some("mergeConfig".to_string()),
///     code: String::new(),
///     symbol_signature: Some("function mergeConfig(a: Config, b: Config)".to_string()),
///     signature_span: None,
///     where_start: None,
///     matched_lines: Vec::new(),
///     parent: None,
/// };
/// assert_eq!(name_match_boost(&block, &["merge", "config"]), 7.0);
/// assert_eq!(name_match_boost(&block, &["merge", "config", "deep", "copy"]), 2.333_333);
/// ```
pub fn name_match_boost(block: &CodeBlock, query_terms: &[&str]) -> f64 {
    let mut name_tokens = block.name.as_deref().map(tokenize_source).unwrap_or_default();
    name_tokens.sort_unstable();
    name_tokens.dedup();
    if query_terms.is_empty() || name_tokens.is_empty() {
        return 1.0;
    }
    let matched = query_terms.iter().filter(|term| name_tokens.iter().any(|token| token == *term)).count();
    let named = name_tokens.iter().filter(|token| query_terms.contains(&token.as_str())).count();
    let recall = as_f64(matched) / as_f64(query_terms.len());
    let precision = as_f64(named) / as_f64(name_tokens.len());
    let exact = matched == query_terms.len() && named == name_tokens.len();
    round6(1.0 + 2.0 * (recall + 0.5 * precision) / 1.5 + if exact { 4.0 } else { 0.0 })
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
// `allow`, not `expect`: clippy 1.99 stopped flagging float `assert_eq!`, so an expectation is unfulfilled there.
#[allow(clippy::float_cmp, reason = "scores are rounded to 6 places, so exact equality is the contract under test")]
mod tests {
    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;
    #[cfg(any(feature = "lang-ts", feature = "lang-rust"))]
    use crate::analysis::LanguageAnalyzer;
    #[cfg(feature = "lang-rust")]
    use crate::analysis::rust::RustAnalyzer;
    #[cfg(feature = "lang-ts")]
    use crate::analysis::ts::TsAnalyzer;

    /// A block in `file` with the given declared name, a signature and a body that both mention `parse`.
    fn declaring(file: &str, name: Option<&str>) -> CodeBlock {
        CodeBlock {
            file: Utf8PathBuf::from(file),
            line_start: 1,
            line_end: 9,
            node_kind: SymbolKind::Function,
            span: 0..49,
            parent_index: None,
            name: name.map(ToString::to_string),
            code: "function other(parse: string) { return parse(); }".to_string(),
            symbol_signature: Some("function other(parse: string)".to_string()),
            signature_span: None,
            where_start: None,
            matched_lines: Vec::new(),
            parent: None,
        }
    }

    // The declared name's distinct tokens N and the query terms Q (repeats counted) give R = matched terms / |Q| and
    // P = name tokens in Q / |N|; the multiplier is round6(1 + 2 * (R + P / 2) / 1.5 + 4 * [R == 1 and P == 1]).
    #[rstest]
    // "toJSONSchema" tokenizes to to, json, schema: R = 2/3, P = 2/3.
    #[case::partial_match(Some("toJSONSchema"), &["json", "schema", "convert"], 2.333_333)]
    #[case::full_name_match(Some("mergeConfig"), &["merge", "config"], 7.0)]
    #[case::full_name_match_in_another_order(Some("mergeConfig"), &["config", "merge"], 7.0)]
    #[case::snake_case(Some("load_config"), &["load", "config"], 7.0)]
    // R = 1/2, P = 1/2.
    #[case::one_of_two_terms(Some("mergeConfig"), &["merge", "other"], 2.0)]
    // R = 1, P = 1/2: every term is in the name, but the name says more than the query.
    #[case::name_longer_than_the_query(Some("parseConfig"), &["parse"], 2.666_667)]
    // R = 1/2, P = 1: the name is all query, the query asks for more.
    #[case::query_longer_than_the_name(Some("parse"), &["parse", "other"], 2.333_333)]
    #[case::no_term_in_the_name(Some("mergeConfig"), &["other"], 1.0)]
    // R = 2/4, P = 1: a repeated term counts per occurrence.
    #[case::repeated_query_terms_count_per_occurrence(Some("parse"), &["parse", "parse", "other", "other"], 2.333_333)]
    #[case::repeated_query_terms_all_in_the_name(Some("parse"), &["parse", "parse"], 7.0)]
    // The name's tokens are distinct: "parseParse" is one token, so P = 1.
    #[case::repeated_name_tokens_count_once(Some("parseParse"), &["parse"], 7.0)]
    #[case::repeated_name_tokens_with_a_longer_query(Some("parseParse"), &["parse", "other"], 2.333_333)]
    // `use` is a stop word, so "useWatch" is the single token `watch`.
    #[case::a_stop_word_in_the_name_is_not_a_token(Some("useWatch"), &["watch"], 7.0)]
    #[case::a_stop_word_in_the_name_with_a_longer_query(Some("useWatch"), &["watch", "form", "values", "subscribe"], 2.0)]
    // `from` is a stop word: "FromRequestParts" is request, parts. R = 2/3, P = 1.
    #[case::a_stop_word_name_among_longer_query(Some("FromRequestParts"), &["extractor", "request", "parts"], 2.555_556)]
    #[case::a_stop_word_name_matches_fully(Some("FromRequestParts"), &["request", "parts"], 7.0)]
    // "fmt::Display for Wrapper<T>" is fmt, display, wrapper (`for` is a stop word, `T` too short): R = 1, P = 2/3.
    #[case::an_impl_name_holds_its_trait_and_target(Some("fmt::Display for Wrapper<T>"), &["display", "wrapper"], 2.777_778)]
    #[case::an_impl_name_matches_fully(Some("fmt::Display for Wrapper<T>"), &["fmt", "display", "wrapper"], 7.0)]
    #[case::no_terms(Some("parse"), &[], 1.0)]
    #[case::a_block_without_a_name_gets_no_boost(None, &["parse", "other"], 1.0)]
    #[case::a_name_of_only_stop_words_gets_no_boost(Some("use"), &["use", "parse"], 1.0)]
    #[case::an_empty_name_gets_no_boost(Some(""), &["parse"], 1.0)]
    fn name_match_boost_rewards_query_terms_in_the_declared_name(
        #[case] name: Option<&str>,
        #[case] terms: &[&str],
        #[case] expected: f64,
    ) {
        // The block's signature and body mention `other` and `parse` as well, and neither counts.
        for file in ["src/a.ts", "src/a.rs"] {
            assert_eq!(name_match_boost(&declaring(file, name), terms), expected, "{file}");
        }
    }

    /// The block of `source` that declares `name`, as `analyzer` read it.
    #[cfg(any(feature = "lang-ts", feature = "lang-rust"))]
    fn analyzed(analyzer: &dyn LanguageAnalyzer, file: &str, source: &str, name: &str) -> CodeBlock {
        let analysis = analyzer.analyze(camino::Utf8Path::new(file), source.to_string()).unwrap();
        analysis.blocks.iter().find(|block| block.name.as_deref() == Some(name)).unwrap().clone()
    }

    // The same contract for names the analyzers read from the syntax tree: the TypeScript and Rust cases mirror each
    // other.
    #[cfg(feature = "lang-ts")]
    #[rstest]
    #[case::function("export function useWatch(props: Props) { return props.watch; }", "useWatch", &["watch"], 7.0)]
    #[case::function_among_more_terms("export function useWatch() {}", "useWatch", &["watch", "form", "values", "subscribe"], 2.0)]
    #[case::interface("export interface FromRequestParts<S> { run(): void }", "FromRequestParts", &["extractor", "request", "parts"], 2.555_556)]
    #[case::class("export class FromRequestParts {}", "FromRequestParts", &["request", "parts"], 7.0)]
    fn name_match_boost_reads_the_name_the_ts_analyzer_extracts(
        #[case] source: &str,
        #[case] name: &str,
        #[case] terms: &[&str],
        #[case] expected: f64,
    ) {
        let block = analyzed(&TsAnalyzer::new(), "src/a.ts", source, name);
        assert_eq!(name_match_boost(&block, terms), expected);
    }

    #[cfg(feature = "lang-rust")]
    #[rstest]
    #[case::function("pub fn use_watch(props: Props) -> u8 { props.watch() }", "use_watch", &["watch"], 7.0)]
    #[case::function_among_more_terms("pub fn use_watch() {}", "use_watch", &["watch", "form", "values", "subscribe"], 2.0)]
    #[case::trait_("pub trait FromRequestParts<S> { fn run(); }", "FromRequestParts", &["extractor", "request", "parts"], 2.555_556)]
    #[case::struct_("pub struct FromRequestParts;", "FromRequestParts", &["request", "parts"], 7.0)]
    fn name_match_boost_reads_the_name_the_rust_analyzer_extracts(
        #[case] source: &str,
        #[case] name: &str,
        #[case] terms: &[&str],
        #[case] expected: f64,
    ) {
        let block = analyzed(&RustAnalyzer::new(), "src/a.rs", source, name);
        assert_eq!(name_match_boost(&block, terms), expected);
    }

    #[test]
    fn the_signature_and_the_body_are_not_the_name() {
        assert_eq!(name_match_boost(&declaring("src/a.ts", Some("wanted")), &["parse", "other"]), 1.0);
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
}
