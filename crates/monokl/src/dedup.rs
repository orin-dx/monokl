//! Removes ranked blocks that mostly repeat a better-ranked block in the same file.

use crate::types::{CodeBlock, RankedBlock};

/// Drops each block that overlaps an earlier kept block in the same file by more than half of the smaller block's
/// lines, then reassigns 1-based ranks.
///
/// The input is expected best first, as [`rank_blocks`](crate::rank::rank_blocks) returns it, so the better block of
/// an overlapping pair survives. Blocks in different files never dedupe. A block whose `line_end` is before its
/// `line_start` overlaps nothing and is always kept.
///
/// # Examples
///
/// ```
/// use camino::Utf8PathBuf;
/// use monokl::dedup::dedup_blocks;
/// use monokl::types::{CodeBlock, RankedBlock, SymbolKind};
///
/// let ranked = |line_start, line_end, rank| RankedBlock {
///     block: CodeBlock {
///         file: Utf8PathBuf::from("src/a.ts"),
///         line_start,
///         line_end,
///         node_kind: SymbolKind::Function,
///         code: String::new(),
///         symbol_signature: None,
///         matched_lines: Vec::new(),
///         matched_keywords: Vec::new(),
///     },
///     bm25_score: 0.0,
///     coverage_boost: 0.0,
///     node_type_boost: 1.5,
///     final_score: 0.0,
///     rank,
///     parent_context: None,
/// };
/// let kept = dedup_blocks(vec![ranked(1, 10, 1), ranked(5, 10, 2), ranked(20, 30, 3)]);
/// let kept: Vec<_> = kept.iter().map(|r| (r.block.line_start, r.rank)).collect();
/// assert_eq!(kept, [(1, 1), (20, 2)]);
/// ```
pub fn dedup_blocks(blocks: Vec<RankedBlock>) -> Vec<RankedBlock> {
    let mut kept: Vec<RankedBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let is_duplicate = kept.iter().any(|earlier| {
            earlier.block.file == block.block.file && overlaps_significantly(&earlier.block, &block.block)
        });
        if !is_duplicate {
            kept.push(block);
        }
    }
    for (rank, block) in (1..).zip(&mut kept) {
        block.rank = rank;
    }
    kept
}

/// Returns whether the two line ranges share more than half of the smaller range's lines.
fn overlaps_significantly(a: &CodeBlock, b: &CodeBlock) -> bool {
    let overlap_start = a.line_start.max(b.line_start);
    let overlap_end = a.line_end.min(b.line_end);
    if overlap_start > overlap_end {
        return false;
    }
    let overlap_lines = overlap_end - overlap_start + 1;
    // Nothing guarantees `line_start <= line_end`, so the lengths saturate rather than risk an underflow panic.
    let smaller_len = (a.line_end.saturating_sub(a.line_start) + 1).min(b.line_end.saturating_sub(b.line_start) + 1);
    overlap_lines * 2 > smaller_len
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;
    use crate::types::SymbolKind;

    fn ranked(file: &str, line_start: usize, line_end: usize, rank: usize) -> RankedBlock {
        RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from(file),
                line_start,
                line_end,
                node_kind: SymbolKind::Function,
                code: String::new(),
                symbol_signature: None,
                matched_lines: Vec::new(),
                matched_keywords: Vec::new(),
            },
            bm25_score: 0.0,
            coverage_boost: 0.0,
            node_type_boost: 1.5,
            final_score: 0.0,
            rank,
            parent_context: None,
        }
    }

    fn survivors(blocks: &[RankedBlock]) -> Vec<(String, usize, usize, usize)> {
        blocks.iter().map(|r| (r.block.file.to_string(), r.block.line_start, r.block.line_end, r.rank)).collect()
    }

    fn kept(file: &str, line_start: usize, line_end: usize, rank: usize) -> (String, usize, usize, usize) {
        (file.to_string(), line_start, line_end, rank)
    }

    #[test]
    fn dedup_blocks_of_no_blocks_is_empty() {
        assert_eq!(survivors(&dedup_blocks(Vec::new())), []);
    }

    #[rstest]
    #[case::contained(5, 10)]
    #[case::just_over_half_of_the_smaller(5, 14)]
    #[case::single_shared_edge_line_of_a_one_line_block(10, 10)]
    #[case::small_block_inside_a_large_one(4, 5)]
    #[case::identical(1, 10)]
    fn dedup_blocks_drops_a_later_block_overlapping_more_than_half(#[case] line_start: usize, #[case] line_end: usize) {
        let out = dedup_blocks(vec![ranked("a.ts", 1, 10, 1), ranked("a.ts", line_start, line_end, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 10, 1)]);
    }

    #[rstest]
    #[case::exactly_half_of_the_smaller(6, 15)]
    #[case::adjacent(11, 20)]
    #[case::disjoint(30, 40)]
    #[case::one_shared_line_of_a_two_line_block(10, 11)]
    fn dedup_blocks_keeps_a_later_block_overlapping_at_most_half(#[case] line_start: usize, #[case] line_end: usize) {
        let out = dedup_blocks(vec![ranked("a.ts", 1, 10, 1), ranked("a.ts", line_start, line_end, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 10, 1), kept("a.ts", line_start, line_end, 2)]);
    }

    #[test]
    fn dedup_blocks_measures_overlap_against_the_smaller_block() {
        // Two shared lines are all of the later block but a fiftieth of the earlier one.
        let out = dedup_blocks(vec![ranked("a.ts", 1, 100, 1), ranked("a.ts", 40, 41, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 100, 1)]);
        let out = dedup_blocks(vec![ranked("a.ts", 40, 41, 1), ranked("a.ts", 1, 100, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 40, 41, 1)]);
    }

    #[test]
    fn dedup_blocks_never_dedupes_across_files() {
        let out = dedup_blocks(vec![ranked("a.ts", 1, 10, 1), ranked("b.ts", 1, 10, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 10, 1), kept("b.ts", 1, 10, 2)]);
    }

    #[test]
    fn dedup_blocks_compares_against_every_kept_block_and_reassigns_ranks() {
        let out = dedup_blocks(vec![
            ranked("a.ts", 1, 10, 1),
            ranked("a.ts", 20, 30, 2),
            ranked("b.ts", 1, 10, 3),
            ranked("a.ts", 21, 30, 4),
            ranked("a.ts", 2, 9, 5),
            ranked("a.ts", 40, 50, 6),
        ]);
        assert_eq!(
            survivors(&out),
            [kept("a.ts", 1, 10, 1), kept("a.ts", 20, 30, 2), kept("b.ts", 1, 10, 3), kept("a.ts", 40, 50, 4)]
        );
    }

    #[test]
    fn dedup_blocks_compares_against_kept_blocks_not_dropped_ones() {
        // 5..=14 duplicates 1..=12 and is dropped. 13..=14 lies wholly inside the dropped block but shares no line with
        // the kept one.
        let out = dedup_blocks(vec![ranked("a.ts", 1, 12, 1), ranked("a.ts", 5, 14, 2), ranked("a.ts", 13, 14, 3)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 12, 1), kept("a.ts", 13, 14, 2)]);
    }

    #[rstest]
    #[case::inverted_first(vec![ranked("a.ts", 10, 5, 1), ranked("a.ts", 5, 10, 2)])]
    #[case::inverted_later(vec![ranked("a.ts", 5, 10, 1), ranked("a.ts", 8, 7, 2)])]
    #[case::both_inverted(vec![ranked("a.ts", 9, 3, 1), ranked("a.ts", 9, 3, 2)])]
    #[case::inverted_beside_a_one_line_block(vec![ranked("a.ts", 7, 7, 1), ranked("a.ts", usize::MAX, 0, 2)])]
    fn dedup_blocks_keeps_inverted_blocks_without_panicking(#[case] blocks: Vec<RankedBlock>) {
        let expected: Vec<_> =
            blocks.iter().map(|r| kept(r.block.file.as_str(), r.block.line_start, r.block.line_end, r.rank)).collect();
        assert_eq!(survivors(&dedup_blocks(blocks)), expected);
    }
}
