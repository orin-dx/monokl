use std::ops::Range;

use camino::Utf8Path;
use proptest::prelude::*;

use super::*;

/// A block with the nodes it holds, laid out left to right.
#[derive(Clone, Debug)]
struct Node {
    /// A leaf's width in bytes.
    width: usize,
    /// Whether a node with exactly one child covers the same bytes as it.
    same_span: bool,
    children: Vec<Node>,
}

fn forest() -> impl Strategy<Value = Vec<Node>> {
    let leaf = (1_usize..4).prop_map(|width| Node { width, same_span: false, children: Vec::new() });
    let node = leaf.prop_recursive(4, 24, 4, |inner| {
        (any::<bool>(), prop::collection::vec(inner, 1..4)).prop_map(|(same_span, children)| Node {
            width: 0,
            same_span,
            children,
        })
    });
    prop::collection::vec(node, 0..4)
}

/// Appends `node` and the nodes under it to `blocks` in tree order, starting at byte `start`, and returns where it ends.
/// A node's children sit inside it in order, apart from each other, so every block's span holds its children's.
fn place(node: &Node, start: usize, parent_index: Option<usize>, blocks: &mut Vec<CodeBlock>) -> usize {
    let index = blocks.len();
    blocks.push(CodeBlock { span: start..start, parent_index, ..empty_block() });
    let tight = node.same_span && node.children.len() == 1;
    let end = if node.children.is_empty() {
        start + node.width
    } else {
        let mut at = start + usize::from(!tight);
        for (position, child) in node.children.iter().enumerate() {
            at += usize::from(position > 0);
            at = place(child, at, Some(index), blocks);
        }
        at + usize::from(!tight)
    };
    blocks[index].span = start..end;
    end
}

fn empty_block() -> CodeBlock {
    CodeBlock {
        file: Utf8Path::new("a.ts").to_owned(),
        line_start: 1,
        line_end: 1,
        node_kind: crate::types::SymbolKind::Function,
        span: 0..0,
        parent_index: None,
        code: String::new(),
        symbol_signature: None,
        name: None,
        matched_lines: Vec::new(),
        parent: None,
    }
}

/// How many blocks enclose `index`, found by following `parent_index` to the root.
fn depth_of(blocks: &[CodeBlock], index: usize) -> usize {
    let mut depth = 0;
    let mut current = index;
    while let Some(parent) = blocks[current].parent_index {
        depth += 1;
        current = parent;
    }
    depth
}

/// The deepest block whose span holds `byte`.
fn innermost(blocks: &[CodeBlock], byte: usize) -> Option<usize> {
    (0..blocks.len()).filter(|&index| blocks[index].span.contains(&byte)).max_by_key(|&index| depth_of(blocks, index))
}

/// Ranges of consecutive offsets in the ascending `positions`.
fn runs(positions: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for &position in positions {
        match runs.last_mut() {
            Some(run) if run.end == position => run.end += 1,
            _ => runs.push(position..position + 1),
        }
    }
    runs
}

proptest! {
    #[test]
    fn a_hit_goes_to_the_deepest_block_holding_each_match_and_to_no_other(
        roots in forest(),
        positions in prop::collection::btree_set(0_usize..80, 0..24),
        sizes in prop::collection::vec(1_usize..4, 1..8),
    ) {
        let mut blocks = Vec::new();
        let mut at = 0;
        for root in &roots {
            at = place(root, at, None, &mut blocks) + 1;
        }
        // Each hit takes the next few positions, so the starts ascend across hits as they do across lines, and positions
        // at consecutive offsets are one run, as the searcher records them.
        let positions: Vec<usize> = positions.into_iter().collect();
        let mut hits = Vec::new();
        let mut taken = 0;
        for size in sizes.iter().cycle() {
            if taken >= positions.len() {
                break;
            }
            let group = &positions[taken..positions.len().min(taken + size)];
            hits.push(LineHit {
                line_number: hits.len() + 1,
                text: String::new(),
                match_starts: runs(group),
            });
            taken += size;
        }
        let refs: Vec<&CodeBlock> = blocks.iter().collect();
        let line_starts = vec![Some(0); hits.len()];

        let owned = owned_hits(&refs, &hits, &line_starts);

        prop_assert_eq!(owned.len(), blocks.len());
        for (index, block_hits) in owned.iter().enumerate() {
            let expected: Vec<usize> = hits
                .iter()
                .filter(|hit| {
                    let mut starts = hit.match_starts.iter().flat_map(Clone::clone);
                    starts.any(|byte| innermost(&blocks, byte) == Some(index))
                })
                .map(|hit| hit.line_number)
                .collect();
            let actual: Vec<usize> = block_hits.iter().map(|hit| hit.line_number).collect();
            prop_assert_eq!(actual, expected);
        }
    }
}
