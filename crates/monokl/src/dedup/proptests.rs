use camino::Utf8PathBuf;
use proptest::prelude::*;

use super::*;
use crate::types::{ParentContext, SymbolKind};

fn ranked(file: &str, span: Range<usize>, parent: Option<ParentContext>) -> RankedBlock {
    RankedBlock {
        block: CodeBlock {
            file: Utf8PathBuf::from(file),
            line_start: 1,
            line_end: 1,
            node_kind: SymbolKind::Method,
            span,
            parent_index: None,
            code: String::new(),
            symbol_signature: None,
            signature_span: None,
            where_start: None,
            name: None,
            matched_lines: Vec::new(),
            parent,
        },
        final_score: 0.0,
        rank: 0,
    }
}

/// Whether `later` repeats `earlier` under the rule the docs state: neither is a member the other encloses, and they
/// share more than half of the smaller span.
fn repeats(earlier: &CodeBlock, later: &CodeBlock) -> bool {
    !encloses_member(earlier, later) && !encloses_member(later, earlier) && overlaps_significantly(earlier, later)
}

/// Blocks that may be members, many per file, with nested and partly overlapping spans of widely varying length, some
/// empty and some inverted.
fn dense_block_strategy() -> impl Strategy<Value = RankedBlock> {
    let parent = ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() };
    (
        prop::sample::select(vec!["a.ts", "b.rs", "c/d.ts"]),
        0usize..120,
        prop_oneof![8 => 0usize..24, 3 => 24usize..120, 1 => Just(0usize)],
        prop::option::weighted(0.4, Just(parent)),
        0usize..100,
        prop::bool::weighted(0.05),
    )
        .prop_map(|(file, start, len, parent, rank, inverted)| {
            let span = if inverted { start + len..start } else { start..start + len };
            RankedBlock { rank, ..ranked(file, span, parent) }
        })
}

proptest! {
    // These three together say what dedup keeps: the survivors are a subsequence of the input, no survivor repeats an
    // earlier one, and every other block repeats a survivor before it.
    #[test]
    fn survivors_are_in_input_order_with_consecutive_ranks(blocks in prop::collection::vec(dense_block_strategy(), 0..120)) {
        let input: Vec<_> = blocks.iter().map(|r| (r.block.file.clone(), r.block.span.clone())).collect();
        let out = dedup_blocks(blocks);
        prop_assert_eq!(out.iter().map(|r| r.rank).collect::<Vec<_>>(), (1..=out.len()).collect::<Vec<_>>());
        let mut remaining = input.iter();
        for survivor in &out {
            let found = remaining.any(|(file, span)| *file == survivor.block.file && *span == survivor.block.span);
            prop_assert!(found, "{:?} is not in the input in order", survivor.block.span);
        }
    }

    #[test]
    fn no_survivor_repeats_an_earlier_survivor_in_its_file(blocks in prop::collection::vec(dense_block_strategy(), 0..120)) {
        let out = dedup_blocks(blocks);
        for (later_at, later) in out.iter().enumerate() {
            for earlier in &out[..later_at] {
                prop_assert!(
                    earlier.block.file != later.block.file || !repeats(&earlier.block, &later.block),
                    "{:?} repeats {:?}",
                    later.block.span,
                    earlier.block.span
                );
            }
        }
    }

    #[test]
    fn every_dropped_block_repeats_a_survivor_before_it(blocks in prop::collection::vec(dense_block_strategy(), 0..120)) {
        // The score numbers each block, so a survivor is told from an identical block that was dropped.
        let blocks: Vec<_> = blocks
            .into_iter()
            .enumerate()
            .map(|(id, block)| RankedBlock { final_score: f64::from(u32::try_from(id).unwrap()), ..block })
            .collect();
        let out = dedup_blocks(blocks.clone());
        let mut survivors = out.iter().peekable();
        let mut before: Vec<&CodeBlock> = Vec::new();
        for block in &blocks {
            let is_survivor = survivors.next_if(|s| s.final_score.to_bits() == block.final_score.to_bits()).is_some();
            if is_survivor {
                before.push(&block.block);
                continue;
            }
            let repeated = before.iter().any(|earlier| earlier.file == block.block.file && repeats(earlier, &block.block));
            prop_assert!(repeated, "{:?} was dropped without repeating a survivor", block.block.span);
        }
        prop_assert!(survivors.next().is_none());
    }
}

/// Spans that share no byte with each other, in a shuffled order, each claiming the same line.
fn disjoint_spans() -> impl Strategy<Value = Vec<Range<usize>>> {
    prop::collection::vec((0usize..4, 1usize..12), 0..40)
        .prop_map(|layout| {
            let mut at = 0;
            layout
                .into_iter()
                .map(|(gap, len)| {
                    at += gap;
                    let span = at..at + len;
                    at += len;
                    span
                })
                .collect::<Vec<_>>()
        })
        .prop_shuffle()
}

proptest! {
    #[test]
    fn blocks_that_share_no_byte_all_survive_whatever_lines_they_claim(spans in disjoint_spans(), lines in 0usize..4) {
        let blocks: Vec<_> = spans
            .iter()
            .map(|span| {
                let mut block = ranked("a.ts", span.clone(), None);
                block.block.line_start = lines;
                block.block.line_end = lines;
                block
            })
            .collect();
        let out = dedup_blocks(blocks);
        prop_assert_eq!(out.iter().map(|r| r.block.span.clone()).collect::<Vec<_>>(), spans);
    }
}

/// A class span over `0..` and members of at least three bytes that never overlap each other, in any order.
fn class_with_members() -> impl Strategy<Value = Vec<RankedBlock>> {
    prop::collection::vec((0usize..3, 3usize..8), 0..6)
        .prop_map(|layout| {
            let parent = ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() };
            let mut members = Vec::new();
            let mut at = 2;
            for (gap, len) in layout {
                at += gap;
                members.push(ranked("a.ts", at..at + len, Some(parent.clone())));
                at += len;
            }
            let mut blocks = vec![ranked("a.ts", 1..at + 1, None)];
            blocks.extend(members);
            blocks
        })
        .prop_shuffle()
}

proptest! {
    #[test]
    fn a_class_and_all_of_its_members_survive_in_any_rank_order(blocks in class_with_members()) {
        let mut expected: Vec<_> = blocks.iter().map(|r| (r.block.span.start, r.block.span.end)).collect();
        expected.sort_unstable();
        let mut actual: Vec<_> = dedup_blocks(blocks).iter().map(|r| (r.block.span.start, r.block.span.end)).collect();
        actual.sort_unstable();
        prop_assert_eq!(actual, expected);
    }
}
