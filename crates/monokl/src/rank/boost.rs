use super::bm25::{as_f64, round6};
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
/// };
/// assert_eq!(coverage_boost(&block), 0.333333);
/// ```
pub fn coverage_boost(block: &CodeBlock) -> f64 {
    if block.matched_lines.is_empty() {
        return 0.0;
    }
    let lines = block.line_start..=block.line_end;
    let inside = block.matched_lines.iter().filter(|line| lines.contains(line)).count();
    round6(as_f64(inside) / as_f64(block.matched_lines.len()))
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
        }
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
    #[case::inverted_block_contains_nothing(block(10, 5, vec![5, 7, 10]), 0.0)]
    fn coverage_boost_is_the_rounded_fraction_of_matched_lines_inside_the_block(
        #[case] block: CodeBlock,
        #[case] expected: f64,
    ) {
        assert_eq!(coverage_boost(&block), expected);
    }
}
