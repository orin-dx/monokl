use std::collections::BTreeSet;

use camino::Utf8PathBuf;
use proptest::prelude::*;

use super::*;
use crate::types::{ParentContext, SymbolKind};

fn block_strategy() -> impl Strategy<Value = RankedBlock> {
    (prop::sample::select(vec!["a.ts", "b.ts", "c/d.ts"]), 0usize..14, 0usize..14, 0usize..100).prop_map(
        |(file, line_start, line_end, rank)| RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from(file),
                line_start,
                line_end,
                node_kind: SymbolKind::Function,
                code: String::new(),
                symbol_signature: None,
                name: None,
                matched_lines: Vec::new(),
                parent: None,
            },
            final_score: 0.0,
            rank,
        },
    )
}

/// The lines a block covers, empty for an inverted range.
fn lines(block: &RankedBlock) -> BTreeSet<usize> {
    (block.block.line_start..=block.block.line_end).collect()
}

/// Set-based restatement of the rule: a block is dropped when it shares more than half of the smaller block's lines
/// with a block already kept in the same file.
fn reference(blocks: &[RankedBlock]) -> Vec<(Utf8PathBuf, usize, usize, usize)> {
    let mut kept: Vec<&RankedBlock> = Vec::new();
    for candidate in blocks {
        let candidate_lines = lines(candidate);
        let duplicate = kept.iter().any(|earlier| {
            let earlier_lines = lines(earlier);
            let shared = earlier_lines.intersection(&candidate_lines).count();
            let smaller = earlier_lines.len().min(candidate_lines.len());
            earlier.block.file == candidate.block.file && smaller > 0 && shared * 2 > smaller
        });
        if !duplicate {
            kept.push(candidate);
        }
    }
    kept.iter()
        .enumerate()
        .map(|(idx, r)| (r.block.file.clone(), r.block.line_start, r.block.line_end, idx + 1))
        .collect()
}

proptest! {
    #[test]
    fn dedup_blocks_matches_the_set_based_reference(blocks in prop::collection::vec(block_strategy(), 0..64)) {
        let expected = reference(&blocks);
        let actual: Vec<_> = dedup_blocks(blocks)
            .iter()
            .map(|r| (r.block.file.clone(), r.block.line_start, r.block.line_end, r.rank))
            .collect();
        prop_assert_eq!(actual, expected);
    }
}

fn ranked(line_start: usize, line_end: usize, parent: Option<ParentContext>) -> RankedBlock {
    RankedBlock {
        block: CodeBlock {
            file: Utf8PathBuf::from("a.ts"),
            line_start,
            line_end,
            node_kind: SymbolKind::Method,
            code: String::new(),
            symbol_signature: None,
            name: None,
            matched_lines: Vec::new(),
            parent,
        },
        final_score: 0.0,
        rank: 0,
    }
}

/// A class block over lines `1..` and members of at least three lines that never overlap each other, in any order.
fn class_with_members() -> impl Strategy<Value = Vec<RankedBlock>> {
    prop::collection::vec((0usize..3, 3usize..8), 0..6)
        .prop_map(|layout| {
            let parent = ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() };
            let mut members = Vec::new();
            let mut line = 2;
            for (gap, len) in layout {
                line += gap;
                members.push(ranked(line, line + len - 1, Some(parent.clone())));
                line += len;
            }
            let mut blocks = vec![ranked(1, line, None)];
            blocks.extend(members);
            blocks
        })
        .prop_shuffle()
}

proptest! {
    #[test]
    fn a_class_and_all_of_its_members_survive_in_any_rank_order(blocks in class_with_members()) {
        let mut expected: Vec<_> = blocks.iter().map(|r| (r.block.line_start, r.block.line_end)).collect();
        expected.sort_unstable();
        let mut actual: Vec<_> = dedup_blocks(blocks).iter().map(|r| (r.block.line_start, r.block.line_end)).collect();
        actual.sort_unstable();
        prop_assert_eq!(actual, expected);
    }
}

/// Blocks that may be members, many per file, with nested and partly overlapping ranges of widely varying length.
fn dense_block_strategy() -> impl Strategy<Value = RankedBlock> {
    let parent = ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() };
    (
        prop::sample::select(vec!["a.ts", "b.ts"]),
        0usize..60,
        prop_oneof![8 => 0usize..12, 3 => 12usize..60, 1 => Just(0usize)],
        prop::option::weighted(0.4, Just(parent)),
    )
        .prop_map(|(file, line_start, len, parent)| {
            let mut block = ranked(line_start, line_start + len, parent);
            block.block.file = Utf8PathBuf::from(file);
            block
        })
}

/// The rule applied to every earlier kept block in turn, with no index.
fn quadratic_reference(blocks: &[RankedBlock]) -> Vec<(Utf8PathBuf, usize, usize, usize)> {
    let mut kept: Vec<&RankedBlock> = Vec::new();
    for candidate in blocks {
        let duplicate = kept.iter().any(|earlier| {
            earlier.block.file == candidate.block.file
                && !encloses_member(&earlier.block, &candidate.block)
                && !encloses_member(&candidate.block, &earlier.block)
                && overlaps_significantly(&earlier.block, &candidate.block)
        });
        if !duplicate {
            kept.push(candidate);
        }
    }
    kept.iter()
        .enumerate()
        .map(|(idx, r)| (r.block.file.clone(), r.block.line_start, r.block.line_end, idx + 1))
        .collect()
}

proptest! {
    #[test]
    fn dedup_blocks_matches_the_unindexed_rule_on_dense_files_with_members(
        mut blocks in prop::collection::vec(dense_block_strategy(), 0..200),
        inverted in prop::collection::vec((0usize..200, 1usize..20), 0..6),
    ) {
        for (at, back) in inverted {
            if let Some(block) = blocks.get_mut(at) {
                block.block.line_end = block.block.line_start.saturating_sub(back);
            }
        }
        let expected = quadratic_reference(&blocks);
        let actual: Vec<_> = dedup_blocks(blocks)
            .iter()
            .map(|r| (r.block.file.clone(), r.block.line_start, r.block.line_end, r.rank))
            .collect();
        prop_assert_eq!(actual, expected);
    }
}
