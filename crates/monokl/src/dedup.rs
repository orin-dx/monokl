//! Removes ranked blocks that mostly repeat a better-ranked block in the same file.

use std::ops::Range;

use camino::Utf8Path;
use rustc_hash::FxHashMap;

use crate::types::{CodeBlock, RankedBlock};

/// Drops each block that overlaps an earlier kept block in the same file by more than half of the smaller block's
/// bytes, then reassigns 1-based ranks.
///
/// Overlap is measured on [`CodeBlock::span`], so two blocks that share a line but not a byte, such as
/// `const a = 1; const b = 2;`, do not overlap. The spans of one analyzed file nest or are disjoint, so a block is
/// dropped when its span lies inside an earlier kept block's, equals it, or holds it; a pair that overlaps without
/// either holding the other can only come from spans not built by an analyzer.
///
/// The input is expected best first, as [`rank_blocks`](crate::rank::rank_blocks) returns it, so the better block of
/// an overlapping pair survives. Blocks in different files never dedupe. A block whose span is empty overlaps nothing
/// and is always kept.
///
/// A member (a block with a `parent`) never duplicates the top-level block that strictly encloses it, in either rank
/// order: the enclosing block holds the member by design. Two members, or two blocks without a parent, dedupe by
/// overlap as above even when one lies inside the other.
///
/// # Examples
///
/// ```
/// use camino::Utf8PathBuf;
/// use monokl::dedup::dedup_blocks;
/// use monokl::types::{CodeBlock, RankedBlock, SymbolKind};
///
/// let ranked = |span: std::ops::Range<usize>, rank| RankedBlock {
///     block: CodeBlock {
///         file: Utf8PathBuf::from("src/a.ts"),
///         line_start: 1,
///         line_end: 1,
///         node_kind: SymbolKind::Function,
///         span,
///         parent_index: None,
///         code: String::new(),
///         symbol_signature: None,
///         signature_span: None,
///         where_start: None,
///         name: None,
///         matched_lines: Vec::new(),
///         parent: None,
///     },
///     final_score: 0.0,
///     rank,
/// };
/// // The first block holds the second, and the third shares their line without sharing a byte.
/// let kept = dedup_blocks(vec![ranked(0..40, 1), ranked(10..20, 2), ranked(41..60, 3)]);
/// let kept: Vec<_> = kept.iter().map(|r| (r.block.span.clone(), r.rank)).collect();
/// assert_eq!(kept, [(0..40, 1), (41..60, 2)]);
/// ```
pub fn dedup_blocks(blocks: Vec<RankedBlock>) -> Vec<RankedBlock> {
    dedup_counting(blocks, &mut || {})
}

/// [`dedup_blocks`] that calls `on_compare` once for every pair of blocks it compares, so a test can count the work.
fn dedup_counting(blocks: Vec<RankedBlock>, on_compare: &mut dyn FnMut()) -> Vec<RankedBlock> {
    let mut files = group_by_file(&blocks);
    let mut is_kept = vec![true; blocks.len()];
    for (idx, block) in blocks.iter().enumerate() {
        let Some(slot) = files.slot_of[idx] else { continue };
        let file = &mut files.groups[files.group_of[idx]];
        let is_duplicate = file.any_kept_overlap(&block.block.span, |earlier| {
            on_compare();
            let earlier = &blocks[earlier].block;
            !encloses_member(earlier, &block.block)
                && !encloses_member(&block.block, earlier)
                && overlaps_significantly(earlier, &block.block)
        });
        if is_duplicate {
            is_kept[idx] = false;
        } else {
            file.mark_kept(slot, block.block.span.end);
        }
    }
    let kept = blocks.into_iter().zip(is_kept).filter_map(|(block, is_kept)| is_kept.then_some(block));
    kept.zip(1..)
        .map(|(mut block, rank)| {
            block.rank = rank;
            block
        })
        .collect()
}

/// The blocks of one input, split by file.
struct FileGroups {
    groups: Vec<FileIndex>,
    /// For each input block, the position of its file in `groups`.
    group_of: Vec<usize>,
    /// For each input block, its slot in its file's index, or `None` for an empty span, which overlaps nothing.
    slot_of: Vec<Option<usize>>,
}

fn group_by_file(blocks: &[RankedBlock]) -> FileGroups {
    let mut group_ids: FxHashMap<&Utf8Path, usize> = FxHashMap::default();
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut group_of = Vec::with_capacity(blocks.len());
    for (idx, block) in blocks.iter().enumerate() {
        let group = *group_ids.entry(block.block.file.as_path()).or_insert_with(|| {
            members.push(Vec::new());
            members.len() - 1
        });
        group_of.push(group);
        if !block.block.span.is_empty() {
            members[group].push(idx);
        }
    }
    let mut slot_of = vec![None; blocks.len()];
    let groups = members
        .into_iter()
        .map(|mut indices| {
            indices.sort_unstable_by_key(|&idx| (blocks[idx].block.span.start, idx));
            for (slot, &idx) in indices.iter().enumerate() {
                slot_of[idx] = Some(slot);
            }
            let starts = indices.iter().map(|&idx| blocks[idx].block.span.start).collect();
            FileIndex::new(starts, indices)
        })
        .collect();
    FileGroups { groups, group_of, slot_of }
}

/// The blocks of one file with a non-empty span, ordered by span start, with a max-end segment tree over the kept ones.
///
/// A candidate can only overlap a kept block that starts before the candidate's end and ends after the candidate's
/// start. The first condition is a prefix of the slots; the tree skips every subtree of that prefix whose kept blocks
/// all end too early, so a query costs time in the number of kept blocks it reaches, not the number kept.
struct FileIndex {
    /// Span start of each slot, ascending.
    starts: Vec<usize>,
    /// Input position of the block in each slot.
    blocks: Vec<usize>,
    /// Number of leaves, a power of two. Leaf `slot` is at `leaf_count + slot`; the root is at 1.
    leaf_count: usize,
    /// Largest span end among the kept blocks below each node, `None` while none is kept.
    max_end: Vec<Option<usize>>,
}

impl FileIndex {
    fn new(starts: Vec<usize>, blocks: Vec<usize>) -> Self {
        let leaf_count = starts.len().next_power_of_two();
        Self { starts, blocks, leaf_count, max_end: vec![None; 2 * leaf_count] }
    }

    fn mark_kept(&mut self, slot: usize, end: usize) {
        let mut node = self.leaf_count + slot;
        self.max_end[node] = Some(end);
        while node > 1 {
            node /= 2;
            let (left, right) = (self.max_end[2 * node], self.max_end[2 * node + 1]);
            self.max_end[node] = left.max(right);
        }
    }

    /// Returns whether `is_duplicate` holds for any kept block that shares a byte with `span`, trying them in slot order
    /// and stopping at the first that holds.
    fn any_kept_overlap(&self, span: &Range<usize>, is_duplicate: impl FnMut(usize) -> bool) -> bool {
        let limit = self.starts.partition_point(|&start| start < span.end);
        Probe { index: self, limit, min_end: span.start, is_duplicate }.visit(1, 0, self.leaf_count)
    }
}

/// One search of a [`FileIndex`]: the slots before `limit` holding a kept block that ends after `min_end`.
struct Probe<'a, F> {
    index: &'a FileIndex,
    limit: usize,
    min_end: usize,
    is_duplicate: F,
}

impl<F: FnMut(usize) -> bool> Probe<'_, F> {
    /// Searches the slots of `lo..hi` below `node`.
    fn visit(&mut self, node: usize, lo: usize, hi: usize) -> bool {
        if lo >= self.limit || self.index.max_end[node].is_none_or(|end| end <= self.min_end) {
            return false;
        }
        if hi - lo == 1 {
            return (self.is_duplicate)(self.index.blocks[lo]);
        }
        let mid = lo + (hi - lo) / 2;
        self.visit(2 * node, lo, mid) || self.visit(2 * node + 1, mid, hi)
    }
}

/// Returns whether `member` has a parent and lies strictly inside `outer`, a block that has none.
fn encloses_member(outer: &CodeBlock, member: &CodeBlock) -> bool {
    member.parent.is_some()
        && outer.parent.is_none()
        && outer.span.start <= member.span.start
        && member.span.end <= outer.span.end
        && outer.span != member.span
}

/// Returns whether the two spans share more than half of the smaller span's bytes.
fn overlaps_significantly(a: &CodeBlock, b: &CodeBlock) -> bool {
    let shared = a.span.end.min(b.span.end).saturating_sub(a.span.start.max(b.span.start));
    shared * 2 > a.span.len().min(b.span.len())
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;
    use crate::types::{ParentContext, SymbolKind};

    fn ranked(file: &str, span: Range<usize>, rank: usize) -> RankedBlock {
        RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from(file),
                // The lines are a decoy: dedup reads the span, so every block claims the same line.
                line_start: 1,
                line_end: 1,
                node_kind: SymbolKind::Function,
                span,
                parent_index: None,
                code: String::new(),
                symbol_signature: None,
                signature_span: None,
                where_start: None,
                name: None,
                matched_lines: Vec::new(),
                parent: None,
            },
            final_score: 0.0,
            rank,
        }
    }

    #[test]
    fn survivors_keep_every_field_but_rank() {
        let mut first = ranked("a.ts", 10..21, 7);
        first.block.line_start = 10;
        first.block.line_end = 20;
        first.block.node_kind = SymbolKind::Class;
        first.block.code = "class A {}".to_string();
        first.block.symbol_signature = Some("class A".to_string());
        first.block.signature_span = Some(10..17);
        first.block.parent_index = Some(3);
        first.block.name = Some("A".to_string());
        first.block.matched_lines = vec![11, 12];
        first.final_score = 2.25;
        first.block.parent = Some(ParentContext { kind: SymbolKind::Module, name: "m".to_string() });
        let duplicate = ranked("a.ts", 12..19, 8);
        let mut other = ranked("b.ts", 1..4, 9);
        other.block.code = "fn b() {}".to_string();
        other.final_score = 0.75;

        let out = dedup_blocks(vec![first.clone(), duplicate, other.clone()]);
        let fields = |r: &RankedBlock| {
            let b = &r.block;
            let parent = b.parent.as_ref().map(|p| (p.kind, p.name.clone()));
            // Two tuples: std implements PartialEq and Debug for tuples of at most 12 elements.
            (
                (b.file.clone(), b.line_start, b.line_end, b.node_kind, b.span.clone(), b.parent_index),
                (b.code.clone(), b.symbol_signature.clone(), b.signature_span.clone(), b.name.clone()),
                (b.matched_lines.clone(), r.final_score.to_bits(), parent),
            )
        };
        assert_eq!(out.iter().map(|r| r.rank).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(out.iter().map(fields).collect::<Vec<_>>(), [fields(&first), fields(&other)]);
    }

    fn survivors(blocks: &[RankedBlock]) -> Vec<(String, Range<usize>, usize)> {
        blocks.iter().map(|r| (r.block.file.to_string(), r.block.span.clone(), r.rank)).collect()
    }

    fn kept(file: &str, span: Range<usize>, rank: usize) -> (String, Range<usize>, usize) {
        (file.to_string(), span, rank)
    }

    #[test]
    fn dedup_blocks_of_no_blocks_is_empty() {
        assert_eq!(survivors(&dedup_blocks(Vec::new())), []);
    }

    // The same cases hold in both languages: dedup reads spans, which both analyzers fill alike.
    #[rstest]
    #[case::contained(5..11)]
    #[case::just_over_half_of_the_smaller(5..15)]
    #[case::single_shared_byte_of_a_one_byte_block(10..11)]
    #[case::small_block_inside_a_large_one(4..6)]
    #[case::identical(1..11)]
    fn dedup_blocks_drops_a_later_block_overlapping_more_than_half(
        #[case] span: Range<usize>,
        #[values("a.ts", "a.rs")] file: &str,
    ) {
        let out = dedup_blocks(vec![ranked(file, 1..11, 1), ranked(file, span, 2)]);
        assert_eq!(survivors(&out), [kept(file, 1..11, 1)]);
    }

    #[rstest]
    #[case::exactly_half_of_the_smaller(6..16)]
    #[case::adjacent(11..21)]
    #[case::disjoint(30..41)]
    #[case::one_shared_byte_of_a_two_byte_block(10..12)]
    fn dedup_blocks_keeps_a_later_block_overlapping_at_most_half(
        #[case] span: Range<usize>,
        #[values("a.ts", "a.rs")] file: &str,
    ) {
        let out = dedup_blocks(vec![ranked(file, 1..11, 1), ranked(file, span.clone(), 2)]);
        assert_eq!(survivors(&out), [kept(file, 1..11, 1), kept(file, span, 2)]);
    }

    #[rstest]
    // Overlap 16..21 is 5 bytes; the smaller block is 10..21 at 11 bytes, and 5 * 2 > 11 is false.
    #[case::five_of_eleven(16..31, true)]
    // Overlap 15..21 is 6 bytes, and 6 * 2 > 11.
    #[case::six_of_eleven(15..31, false)]
    #[case::adjacent_after(21..31, true)]
    // Overlap 12..15 is all 3 bytes of the smaller block.
    #[case::contained_short_block(12..15, false)]
    fn dedup_blocks_drops_only_overlap_above_half_of_the_smaller_block(
        #[case] span: Range<usize>,
        #[case] is_kept: bool,
    ) {
        let out = dedup_blocks(vec![ranked("a.ts", 10..21, 1), ranked("a.ts", span.clone(), 2)]);
        let mut expected = vec![kept("a.ts", 10..21, 1)];
        if is_kept {
            expected.push(kept("a.ts", span, 2));
        }
        assert_eq!(survivors(&out), expected);
    }

    #[test]
    fn dedup_blocks_measures_overlap_against_the_smaller_block() {
        // Two shared bytes are all of the later block but a fiftieth of the earlier one.
        let out = dedup_blocks(vec![ranked("a.ts", 1..101, 1), ranked("a.ts", 40..42, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1..101, 1)]);
        let out = dedup_blocks(vec![ranked("a.ts", 40..42, 1), ranked("a.ts", 1..101, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 40..42, 1)]);
    }

    #[rstest]
    #[case::either_order(vec![(0..12, 1), (13..25, 2)])]
    #[case::reversed(vec![(13..25, 1), (0..12, 2)])]
    #[case::three(vec![(0..12, 1), (13..25, 2), (26..40, 3)])]
    fn two_blocks_that_share_a_line_but_not_a_byte_both_survive(
        #[case] spans: Vec<(Range<usize>, usize)>,
        #[values("a.ts", "a.rs")] file: &str,
    ) {
        // `const a = 1; const b = 2;` and `fn a() {} fn b() {}`: every block is on line 1.
        let out = dedup_blocks(spans.iter().map(|(span, rank)| ranked(file, span.clone(), *rank)).collect());
        assert_eq!(
            survivors(&out),
            spans.iter().zip(1..).map(|((span, _), rank)| kept(file, span.clone(), rank)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_blocks_on_different_lines_that_share_bytes_are_still_duplicates() {
        // The lines are no part of the rule: these claim lines 1 and 9, and the second lies inside the first.
        let mut inside = ranked("a.ts", 5..30, 2);
        inside.block.line_start = 9;
        inside.block.line_end = 9;
        let out = dedup_blocks(vec![ranked("a.ts", 0..40, 1), inside]);
        assert_eq!(survivors(&out), [kept("a.ts", 0..40, 1)]);
    }

    #[test]
    fn dedup_blocks_never_dedupes_across_files() {
        let out = dedup_blocks(vec![ranked("a.ts", 1..11, 1), ranked("b.ts", 1..11, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1..11, 1), kept("b.ts", 1..11, 2)]);
    }

    #[test]
    fn dedup_blocks_compares_against_every_kept_block_and_reassigns_ranks() {
        let out = dedup_blocks(vec![
            ranked("a.ts", 1..11, 1),
            ranked("a.ts", 20..31, 2),
            ranked("b.ts", 1..11, 3),
            ranked("a.ts", 21..31, 4),
            ranked("a.ts", 2..10, 5),
            ranked("a.ts", 40..51, 6),
        ]);
        assert_eq!(
            survivors(&out),
            [kept("a.ts", 1..11, 1), kept("a.ts", 20..31, 2), kept("b.ts", 1..11, 3), kept("a.ts", 40..51, 4)]
        );
    }

    #[test]
    fn dedup_blocks_compares_against_kept_blocks_not_dropped_ones() {
        // 5..15 duplicates 1..13 and is dropped. 13..15 lies wholly inside the dropped block but shares no byte with the
        // kept one.
        let out = dedup_blocks(vec![ranked("a.ts", 1..13, 1), ranked("a.ts", 5..15, 2), ranked("a.ts", 13..15, 3)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1..13, 1), kept("a.ts", 13..15, 2)]);
    }

    fn member_of_class(file: &str, span: Range<usize>, rank: usize) -> RankedBlock {
        let mut member = ranked(file, span, rank);
        member.block.parent = Some(ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() });
        member
    }

    #[rstest]
    #[case::class_ranked_first(true)]
    #[case::member_ranked_first(false)]
    fn a_member_survives_the_block_that_encloses_it_in_either_rank_order(#[case] class_first: bool) {
        let class = ranked("a.ts", 1..21, 1);
        let member = member_of_class("a.ts", 5..11, 2);
        let blocks = if class_first { vec![class, member] } else { vec![member.clone(), class] };
        let out = dedup_blocks(blocks);
        let mut spans: Vec<_> = survivors(&out).into_iter().map(|(_, span, _)| (span.start, span.end)).collect();
        spans.sort_unstable();
        assert_eq!(spans, [(1, 21), (5, 11)]);
        assert_eq!(out.iter().map(|r| r.rank).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_member_starting_where_the_class_starts_is_still_a_member() {
        let out = dedup_blocks(vec![ranked("a.ts", 1..21, 1), member_of_class("a.ts", 1..11, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1..21, 1), kept("a.ts", 1..11, 2)]);
    }

    #[rstest]
    #[case::identical_span(1..21, 1..21)]
    #[case::partly_after(1..21, 15..26)]
    #[case::partly_before(3..21, 1..6)]
    fn a_member_that_is_not_strictly_inside_the_block_is_judged_by_overlap(
        #[case] outer: Range<usize>,
        #[case] member: Range<usize>,
    ) {
        let out = dedup_blocks(vec![ranked("a.ts", outer.clone(), 1), member_of_class("a.ts", member, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", outer, 1)]);
    }

    #[test]
    fn two_members_of_one_class_that_overlap_each_other_dedupe_as_before() {
        let out = dedup_blocks(vec![
            ranked("a.ts", 1..31, 1),
            member_of_class("a.ts", 5..16, 2),
            member_of_class("a.ts", 6..15, 3),
        ]);
        assert_eq!(survivors(&out), [kept("a.ts", 1..31, 1), kept("a.ts", 5..16, 2)]);
    }

    #[test]
    fn a_plain_block_inside_another_plain_block_is_still_a_duplicate() {
        let out = dedup_blocks(vec![ranked("a.ts", 1..21, 1), ranked("a.ts", 5..11, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1..21, 1)]);
    }

    /// How the blocks of the scaling tests lie over files.
    #[derive(Clone, Copy)]
    enum Layout {
        /// Five blocks per file, none overlapping.
        ManyFiles,
        /// Every block in one file, none overlapping.
        OneFile,
        /// Every block in one file, each overlapping the ones around it.
        OneFileOverlapping,
    }

    /// How many pairs `dedup_blocks` compares for `count` blocks laid out as `layout`.
    fn comparisons(count: usize, layout: Layout) -> usize {
        let blocks = (0..count)
            .map(|index| match layout {
                Layout::ManyFiles => {
                    ranked(&format!("src/f{}.ts", index / 5), 1 + (index % 5) * 10..9 + (index % 5) * 10, index + 1)
                }
                Layout::OneFile => ranked("src/one.ts", 1 + index * 4..4 + index * 4, index + 1),
                Layout::OneFileOverlapping => ranked("src/one.ts", 1 + index..21 + index, index + 1),
            })
            .collect();
        let mut compared = 0;
        dedup_counting(blocks, &mut || compared += 1);
        compared
    }

    /// Four times the blocks must cost about four times the comparisons: a pass that compares every pair would cost 16
    /// times as many. Counting comparisons instead of timing them gives the same answer on every machine and load.
    #[rstest]
    fn dedup_compares_a_linear_number_of_pairs(
        #[values(Layout::ManyFiles, Layout::OneFile, Layout::OneFileOverlapping)] layout: Layout,
    ) {
        let (small, large) = (comparisons(2_500, layout), comparisons(10_000, layout));
        // Blocks that overlap nothing are never compared at all, so the small count can be zero.
        assert!(large <= 5 * small.max(1), "{large} comparisons for 10000 blocks against {small} for 2500");
    }

    #[rstest]
    #[case::empty_first(vec![ranked("a.ts", 5..5, 1), ranked("a.ts", 1..11, 2)])]
    #[case::empty_inside_a_kept_block(vec![ranked("a.ts", 1..21, 1), ranked("a.ts", 10..10, 2)])]
    #[case::empty_later(vec![ranked("a.ts", 5..11, 1), ranked("a.ts", 8..8, 2)])]
    #[case::inverted(vec![ranked("a.ts", Range { start: 10, end: 5 }, 1), ranked("a.ts", 5..10, 2)])]
    #[case::both_empty(vec![ranked("a.ts", 9..9, 1), ranked("a.ts", 9..9, 2)])]
    #[case::inverted_beside_a_one_byte_block(vec![ranked("a.ts", 7..8, 1), ranked("a.ts", Range { start: usize::MAX, end: 0 }, 2)])]
    fn dedup_blocks_keeps_blocks_with_an_empty_span_without_panicking(#[case] blocks: Vec<RankedBlock>) {
        let expected: Vec<_> =
            blocks.iter().map(|r| kept(r.block.file.as_str(), r.block.span.clone(), r.rank)).collect();
        assert_eq!(survivors(&dedup_blocks(blocks)), expected);
    }
}

#[cfg(test)]
mod proptests;
