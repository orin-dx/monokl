use std::collections::BTreeSet;

use camino::Utf8PathBuf;
use proptest::prelude::*;

use super::*;
use crate::types::SymbolKind;

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
                matched_lines: Vec::new(),
                matched_keywords: Vec::new(),
            },
            bm25_score: 0.0,
            coverage_boost: 0.0,
            node_type_boost: 1.5,
            final_score: 0.0,
            rank,
            parent_context: None,
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
    fn dedup_blocks_matches_the_set_based_reference(blocks in prop::collection::vec(block_strategy(), 0..24)) {
        let expected = reference(&blocks);
        let actual: Vec<_> = dedup_blocks(blocks)
            .iter()
            .map(|r| (r.block.file.clone(), r.block.line_start, r.block.line_end, r.rank))
            .collect();
        prop_assert_eq!(actual, expected);
    }
}
