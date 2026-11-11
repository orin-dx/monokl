//! Removes ranked blocks that mostly repeat a better-ranked block in the same file.

use camino::Utf8Path;
use rustc_hash::FxHashMap;

use crate::types::{CodeBlock, RankedBlock};

/// Drops each block that overlaps an earlier kept block in the same file by more than half of the smaller block's
/// lines, then reassigns 1-based ranks.
///
/// The input is expected best first, as [`rank_blocks`](crate::rank::rank_blocks) returns it, so the better block of
/// an overlapping pair survives. Blocks in different files never dedupe. A block whose `line_end` is before its
/// `line_start` overlaps nothing and is always kept.
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
///         parent: None,
///     },
///     final_score: 0.0,
///     rank,
///     parent_context: None,
/// };
/// let kept = dedup_blocks(vec![ranked(1, 10, 1), ranked(5, 10, 2), ranked(20, 30, 3)]);
/// let kept: Vec<_> = kept.iter().map(|r| (r.block.line_start, r.rank)).collect();
/// assert_eq!(kept, [(1, 1), (20, 2)]);
/// ```
pub fn dedup_blocks(blocks: Vec<RankedBlock>) -> Vec<RankedBlock> {
    let mut files = group_by_file(&blocks);
    let mut is_kept = vec![true; blocks.len()];
    for (idx, block) in blocks.iter().enumerate() {
        let Some(slot) = files.slot_of[idx] else { continue };
        let file = &mut files.groups[files.group_of[idx]];
        let is_duplicate = file.any_kept_overlap(block.block.line_start, block.block.line_end, |earlier| {
            let earlier = &blocks[earlier].block;
            !encloses_member(earlier, &block.block)
                && !encloses_member(&block.block, earlier)
                && overlaps_significantly(earlier, &block.block)
        });
        if is_duplicate {
            is_kept[idx] = false;
        } else {
            file.mark_kept(slot, block.block.line_end);
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
    /// For each input block, its slot in its file's index, or `None` for an inverted block, which overlaps nothing.
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
        if block.block.line_start <= block.block.line_end {
            members[group].push(idx);
        }
    }
    let mut slot_of = vec![None; blocks.len()];
    let groups = members
        .into_iter()
        .map(|mut indices| {
            indices.sort_unstable_by_key(|&idx| (blocks[idx].block.line_start, idx));
            for (slot, &idx) in indices.iter().enumerate() {
                slot_of[idx] = Some(slot);
            }
            let starts = indices.iter().map(|&idx| blocks[idx].block.line_start).collect();
            FileIndex::new(starts, indices)
        })
        .collect();
    FileGroups { groups, group_of, slot_of }
}

/// The non-inverted blocks of one file, ordered by start line, with a max-`line_end` segment tree over the kept ones.
///
/// A candidate can only overlap a kept block that starts at or before the candidate's end and ends at or after the
/// candidate's start. The first condition is a prefix of the slots; the tree skips every subtree of that prefix whose
/// kept blocks all end too early, so a query costs time in the number of kept blocks it reaches, not the number kept.
struct FileIndex {
    /// Start line of each slot, ascending.
    starts: Vec<usize>,
    /// Input position of the block in each slot.
    blocks: Vec<usize>,
    /// Number of leaves, a power of two. Leaf `slot` is at `leaf_count + slot`; the root is at 1.
    leaf_count: usize,
    /// Largest `line_end` among the kept blocks below each node, `None` while none is kept.
    max_end: Vec<Option<usize>>,
}

impl FileIndex {
    fn new(starts: Vec<usize>, blocks: Vec<usize>) -> Self {
        let leaf_count = starts.len().next_power_of_two();
        Self { starts, blocks, leaf_count, max_end: vec![None; 2 * leaf_count] }
    }

    fn mark_kept(&mut self, slot: usize, line_end: usize) {
        let mut node = self.leaf_count + slot;
        self.max_end[node] = Some(line_end);
        while node > 1 {
            node /= 2;
            let (left, right) = (self.max_end[2 * node], self.max_end[2 * node + 1]);
            self.max_end[node] = left.max(right);
        }
    }

    /// Returns whether `is_duplicate` holds for any kept block whose lines intersect `line_start..=line_end`, trying
    /// them in slot order and stopping at the first that holds.
    fn any_kept_overlap(&self, line_start: usize, line_end: usize, is_duplicate: impl FnMut(usize) -> bool) -> bool {
        let limit = self.starts.partition_point(|&start| start <= line_end);
        Probe { index: self, limit, min_end: line_start, is_duplicate }.visit(1, 0, self.leaf_count)
    }
}

/// One search of a [`FileIndex`]: the slots before `limit` holding a kept block that ends at or after `min_end`.
struct Probe<'a, F> {
    index: &'a FileIndex,
    limit: usize,
    min_end: usize,
    is_duplicate: F,
}

impl<F: FnMut(usize) -> bool> Probe<'_, F> {
    /// Searches the slots of `lo..hi` below `node`.
    fn visit(&mut self, node: usize, lo: usize, hi: usize) -> bool {
        if lo >= self.limit || self.index.max_end[node].is_none_or(|end| end < self.min_end) {
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
        && outer.line_start <= member.line_start
        && member.line_end <= outer.line_end
        && (outer.line_start, outer.line_end) != (member.line_start, member.line_end)
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
    use crate::types::{ParentContext, SymbolKind};

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
                parent: None,
            },
            final_score: 0.0,
            rank,
            parent_context: None,
        }
    }

    #[test]
    fn survivors_keep_every_field_but_rank() {
        let mut first = ranked("a.ts", 10, 20, 7);
        first.block.node_kind = SymbolKind::Class;
        first.block.code = "class A {}".to_string();
        first.block.symbol_signature = Some("class A".to_string());
        first.block.matched_lines = vec![11, 12];
        first.block.matched_keywords = vec!["a".to_string()];
        first.final_score = 2.25;
        first.parent_context = Some(crate::types::ParentContext { kind: SymbolKind::Module, name: "m".to_string() });
        let duplicate = ranked("a.ts", 12, 18, 8);
        let mut other = ranked("b.ts", 1, 3, 9);
        other.block.code = "fn b() {}".to_string();
        other.final_score = 0.75;

        let out = dedup_blocks(vec![first.clone(), duplicate, other.clone()]);
        let fields = |r: &RankedBlock| {
            let b = &r.block;
            let parent = r.parent_context.as_ref().map(|p| (p.kind, p.name.clone()));
            // Two tuples: std implements PartialEq and Debug for tuples of at most 12 elements.
            (
                (b.file.clone(), b.line_start, b.line_end, b.node_kind, b.code.clone(), b.symbol_signature.clone()),
                (b.matched_lines.clone(), b.matched_keywords.clone(), r.final_score.to_bits(), parent),
            )
        };
        assert_eq!(out.iter().map(|r| r.rank).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(out.iter().map(fields).collect::<Vec<_>>(), [fields(&first), fields(&other)]);
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

    #[rstest]
    // Overlap 16..=20 is 5 lines; the smaller block is 10..=20 at 11 lines, and 5 * 2 > 11 is false.
    #[case::five_of_eleven(16, 30, true)]
    // Overlap 15..=20 is 6 lines, and 6 * 2 > 11.
    #[case::six_of_eleven(15, 30, false)]
    #[case::adjacent_after(21, 30, true)]
    // Overlap 12..=14 is all 3 lines of the smaller block.
    #[case::contained_short_block(12, 14, false)]
    fn dedup_blocks_drops_only_overlap_above_half_of_the_smaller_block(
        #[case] line_start: usize,
        #[case] line_end: usize,
        #[case] is_kept: bool,
    ) {
        let out = dedup_blocks(vec![ranked("a.ts", 10, 20, 1), ranked("a.ts", line_start, line_end, 2)]);
        let mut expected = vec![kept("a.ts", 10, 20, 1)];
        if is_kept {
            expected.push(kept("a.ts", line_start, line_end, 2));
        }
        assert_eq!(survivors(&out), expected);
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

    fn member_of_class(file: &str, line_start: usize, line_end: usize, rank: usize) -> RankedBlock {
        let mut member = ranked(file, line_start, line_end, rank);
        member.block.parent = Some(ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() });
        member
    }

    #[rstest]
    #[case::class_ranked_first(true)]
    #[case::member_ranked_first(false)]
    fn a_member_survives_the_block_that_encloses_it_in_either_rank_order(#[case] class_first: bool) {
        let class = ranked("a.ts", 1, 20, 0);
        let member = member_of_class("a.ts", 5, 10, 0);
        let (class, member) = (RankedBlock { rank: 1, ..class }, RankedBlock { rank: 2, ..member });
        let blocks = if class_first { vec![class, member] } else { vec![member.clone(), class] };
        let out = dedup_blocks(blocks);
        let mut starts: Vec<_> = survivors(&out).into_iter().map(|(_, start, end, _)| (start, end)).collect();
        starts.sort_unstable();
        assert_eq!(starts, [(1, 20), (5, 10)]);
        assert_eq!(out.iter().map(|r| r.rank).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_member_sharing_its_first_line_with_the_class_header_is_still_a_member() {
        let out = dedup_blocks(vec![ranked("a.ts", 1, 20, 1), member_of_class("a.ts", 1, 10, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 20, 1), kept("a.ts", 1, 10, 2)]);
    }

    #[rstest]
    #[case::identical_range((1, 20), (1, 20))]
    #[case::partly_below((1, 20), (15, 25))]
    #[case::partly_above((3, 20), (1, 5))]
    fn a_member_that_is_not_strictly_inside_the_block_is_judged_by_overlap(
        #[case] outer: (usize, usize),
        #[case] member: (usize, usize),
    ) {
        let out =
            dedup_blocks(vec![ranked("a.ts", outer.0, outer.1, 1), member_of_class("a.ts", member.0, member.1, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", outer.0, outer.1, 1)]);
    }

    #[test]
    fn two_members_of_one_class_that_overlap_each_other_dedupe_as_before() {
        let out = dedup_blocks(vec![
            ranked("a.ts", 1, 30, 1),
            member_of_class("a.ts", 5, 15, 2),
            member_of_class("a.ts", 6, 14, 3),
        ]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 30, 1), kept("a.ts", 5, 15, 2)]);
    }

    #[test]
    fn a_plain_block_inside_another_plain_block_is_still_a_duplicate() {
        let out = dedup_blocks(vec![ranked("a.ts", 1, 20, 1), ranked("a.ts", 5, 10, 2)]);
        assert_eq!(survivors(&out), [kept("a.ts", 1, 20, 1)]);
    }

    #[rstest]
    #[case::inverted_first(vec![ranked("a.ts", 10, 5, 1), ranked("a.ts", 5, 10, 2)])]
    #[case::inverted_inside_a_kept_block(vec![ranked("a.ts", 1, 20, 1), ranked("a.ts", 10, 5, 2)])]
    #[case::inverted_later(vec![ranked("a.ts", 5, 10, 1), ranked("a.ts", 8, 7, 2)])]
    #[case::both_inverted(vec![ranked("a.ts", 9, 3, 1), ranked("a.ts", 9, 3, 2)])]
    #[case::inverted_beside_a_one_line_block(vec![ranked("a.ts", 7, 7, 1), ranked("a.ts", usize::MAX, 0, 2)])]
    fn dedup_blocks_keeps_inverted_blocks_without_panicking(#[case] blocks: Vec<RankedBlock>) {
        let expected: Vec<_> =
            blocks.iter().map(|r| kept(r.block.file.as_str(), r.block.line_start, r.block.line_end, r.rank)).collect();
        assert_eq!(survivors(&dedup_blocks(blocks)), expected);
    }
}

#[cfg(test)]
mod proptests;
