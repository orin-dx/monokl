use super::bm25::{as_f64, round6};
use crate::tokens::tokenize_source;
use crate::types::{CodeBlock, SymbolKind};

/// Returns the multiplier for how many query terms the block's declared name contains, from 1.0 to 3.0.
///
/// The name is [`CodeBlock::name`], the identifier the analyzer read from the syntax tree, so parameters, return types
/// and the body do not count. It is tokenized like a document, and the multiplier is
/// `round6(1 + 2 * matched / query_terms.len())`, where `matched` counts the query terms, repeats included, that the
/// name contains. Returns 1.0 when `query_terms` is empty or the block declares no name.
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
///     name: Some("mergeConfig".to_string()),
///     code: String::new(),
///     symbol_signature: Some("function mergeConfig(a: Config, b: Config)".to_string()),
///     matched_lines: Vec::new(),
///     parent: None,
/// };
/// assert_eq!(name_match_boost(&block, &["merge", "config", "deep", "copy"]), 2.0);
/// ```
pub fn name_match_boost(block: &CodeBlock, query_terms: &[&str]) -> f64 {
    if query_terms.is_empty() {
        return 1.0;
    }
    let name_tokens = block.name.as_deref().map(tokenize_source).unwrap_or_default();
    let matched = query_terms.iter().filter(|term| name_tokens.iter().any(|token| token == *term)).count();
    round6(1.0 + 2.0 * as_f64(matched) / as_f64(query_terms.len()))
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

    /// A block in `file` with the given declared name, a signature and a body that both mention `parse`.
    fn declaring(file: &str, name: Option<&str>) -> CodeBlock {
        CodeBlock {
            file: Utf8PathBuf::from(file),
            line_start: 1,
            line_end: 9,
            node_kind: SymbolKind::Function,
            name: name.map(ToString::to_string),
            code: "function other(parse: string) { return parse(); }".to_string(),
            symbol_signature: Some("function other(parse: string)".to_string()),
            matched_lines: Vec::new(),
            parent: None,
        }
    }

    // The declared name is tokenized and compared with the terms: the multiplier is
    // round6(1 + 2 * matched_terms / total_terms).
    #[rstest]
    // "toJSONSchema" tokenizes to to, json, schema: 2 of 3 terms, 1 + 2 * 2/3.
    #[case::partial_match(Some("toJSONSchema"), &["json", "schema", "convert"], 2.333_333)]
    #[case::every_term_in_the_name(Some("mergeConfig"), &["merge", "config"], 3.0)]
    #[case::snake_case(Some("load_config"), &["load", "config"], 3.0)]
    #[case::one_of_two_terms(Some("mergeConfig"), &["merge", "other"], 2.0)]
    #[case::no_term_in_the_name(Some("mergeConfig"), &["other"], 1.0)]
    #[case::repeated_query_terms_count_per_occurrence(Some("parse"), &["parse", "parse", "other", "other"], 2.0)]
    #[case::an_impl_name_holds_its_trait_and_target(Some("fmt::Display for Wrapper<T>"), &["display", "wrapper"], 3.0)]
    #[case::no_terms(Some("parse"), &[], 1.0)]
    #[case::a_block_without_a_name_gets_no_boost(None, &["parse", "other"], 1.0)]
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
