use camino::Utf8PathBuf;
use proptest::prelude::*;

use super::*;
use crate::types::SymbolKind;

const MATCHING: &str = "hit";
const OTHER: &str = "miss";

/// A block over `line_start..=line_start + marks.len() - 1` whose line `i` reads `hit` when `marks[i]` is set.
fn block(line_start: usize, marks: &[bool]) -> CodeBlock {
    let code: Vec<&str> = marks.iter().map(|&m| if m { MATCHING } else { OTHER }).collect();
    CodeBlock {
        file: Utf8PathBuf::from("a.ts"),
        line_start,
        line_end: line_start + marks.len() - 1,
        node_kind: SymbolKind::Function,
        code: code.join("\n"),
        symbol_signature: None,
        matched_lines: Vec::new(),
        parent: None,
    }
}

fn hit(line_number: usize) -> LineHit {
    LineHit { line_number, text: String::new() }
}

fn own_text(block: &CodeBlock, line: usize) -> &str {
    block.code.split('\n').nth(line - block.line_start).unwrap_or("")
}

/// The blocks that should own `line`, by looking at every pair: those containing it whose own text matches (all of
/// them when none does), minus any that holds a smaller one of those.
fn reference_owners(blocks: &[CodeBlock], line: usize) -> Vec<usize> {
    let containing: Vec<usize> =
        (0..blocks.len()).filter(|&i| (blocks[i].line_start..=blocks[i].line_end).contains(&line)).collect();
    let matching: Vec<usize> = containing.iter().copied().filter(|&i| own_text(&blocks[i], line) == MATCHING).collect();
    let candidates = if matching.is_empty() { containing } else { matching };
    candidates
        .iter()
        .copied()
        .filter(|&outer| {
            !candidates.iter().any(|&inner| {
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
    fn every_hit_goes_to_each_innermost_block_whose_own_text_matches_and_to_no_other(
        specs in prop::collection::vec((1usize..20, prop::collection::vec(any::<bool>(), 1..11)), 0..8),
        lines in prop::collection::btree_set(1usize..30, 0..20),
    ) {
        let blocks: Vec<CodeBlock> = specs.iter().map(|(start, marks)| block(*start, marks)).collect();
        let hits: Vec<LineHit> = lines.iter().map(|&line| hit(line)).collect();
        let refs: Vec<&CodeBlock> = blocks.iter().collect();
        let owned = owned_hits(&refs, &hits, &|text| text == MATCHING);

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
