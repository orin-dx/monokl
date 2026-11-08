use camino::Utf8PathBuf;
use proptest::prelude::*;

use super::*;
use crate::types::SymbolKind;

fn block(line_start: usize, line_end: usize) -> CodeBlock {
    CodeBlock {
        file: Utf8PathBuf::from("a.ts"),
        line_start,
        line_end,
        node_kind: SymbolKind::Function,
        code: String::new(),
        symbol_signature: None,
        matched_lines: Vec::new(),
        matched_keywords: Vec::new(),
        parent: None,
    }
}

fn hit(line_number: usize) -> LineHit {
    LineHit { line_number, text: String::new() }
}

/// The blocks containing `line` that hold no smaller block also containing it, by looking at every pair.
fn reference_owners(blocks: &[CodeBlock], line: usize) -> Vec<usize> {
    let containing: Vec<usize> =
        (0..blocks.len()).filter(|&i| (blocks[i].line_start..=blocks[i].line_end).contains(&line)).collect();
    containing
        .iter()
        .copied()
        .filter(|&outer| {
            !containing.iter().any(|&inner| {
                let (a, b) = (&blocks[outer], &blocks[inner]);
                b.line_start >= a.line_start
                    && b.line_end <= a.line_end
                    && (b.line_start, b.line_end) != (a.line_start, a.line_end)
            })
        })
        .collect()
}

proptest! {
    #[test]
    fn every_hit_goes_to_each_innermost_block_containing_it_and_to_no_other(
        ranges in prop::collection::vec((1usize..20, 0usize..10), 0..8),
        lines in prop::collection::btree_set(1usize..30, 0..20),
    ) {
        let blocks: Vec<CodeBlock> = ranges.iter().map(|&(start, len)| block(start, start + len)).collect();
        let hits: Vec<LineHit> = lines.iter().map(|&line| hit(line)).collect();
        let owned = owned_hits(&blocks, &hits);

        prop_assert_eq!(owned.len(), blocks.len());
        for (index, block_hits) in owned.iter().enumerate() {
            let expected: Vec<usize> =
                lines.iter().copied().filter(|&line| reference_owners(&blocks, line).contains(&index)).collect();
            let actual: Vec<usize> = block_hits.iter().map(|h| h.line_number).collect();
            prop_assert_eq!(actual, expected);
        }
        let credited: usize = owned.iter().map(Vec::len).sum();
        let expected_credits: usize = lines.iter().map(|&line| reference_owners(&blocks, line).len()).sum();
        prop_assert_eq!(credited, expected_credits);
        for &line in &lines {
            let covered = blocks.iter().any(|b| (b.line_start..=b.line_end).contains(&line));
            prop_assert_eq!(!reference_owners(&blocks, line).is_empty(), covered);
        }
    }
}
