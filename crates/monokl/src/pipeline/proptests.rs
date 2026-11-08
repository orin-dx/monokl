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

/// The smallest block containing `line`, the earliest of equally small ones, by looking at every block.
fn reference_owner(blocks: &[CodeBlock], line: usize) -> Option<usize> {
    (0..blocks.len())
        .filter(|&i| (blocks[i].line_start..=blocks[i].line_end).contains(&line))
        .min_by_key(|&i| (blocks[i].line_end - blocks[i].line_start, i))
}

proptest! {
    #[test]
    fn every_hit_goes_to_the_smallest_block_containing_it_and_to_no_other(
        ranges in prop::collection::vec((1usize..20, 0usize..10), 0..8),
        lines in prop::collection::btree_set(1usize..30, 0..20),
    ) {
        let blocks: Vec<CodeBlock> = ranges.iter().map(|&(start, len)| block(start, start + len)).collect();
        let hits: Vec<LineHit> = lines.iter().map(|&line| hit(line)).collect();
        let owned = owned_hits(&blocks, &hits);

        prop_assert_eq!(owned.len(), blocks.len());
        for (index, block_hits) in owned.iter().enumerate() {
            let expected: Vec<usize> =
                lines.iter().copied().filter(|&line| reference_owner(&blocks, line) == Some(index)).collect();
            let actual: Vec<usize> = block_hits.iter().map(|h| h.line_number).collect();
            prop_assert_eq!(actual, expected);
        }
        let credited: usize = owned.iter().map(Vec::len).sum();
        let coverable = lines.iter().filter(|&&line| reference_owner(&blocks, line).is_some()).count();
        prop_assert_eq!(credited, coverable);
    }
}
