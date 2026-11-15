use std::collections::BTreeSet;
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
        signature_span: None,
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

/// How many bytes of a line the rule test gives each hit.
const LINE_WIDTH: usize = 10;

/// The deepest block that ends at `byte`, for a block no other holds there.
fn innermost_ending_at(blocks: &[CodeBlock], byte: usize) -> Option<usize> {
    (0..blocks.len()).filter(|&index| blocks[index].span.end == byte).max_by_key(|&index| depth_of(blocks, index))
}

/// The blocks a hit goes to, by the rules in the order they apply, each written the long way: for every offset in turn,
/// find the deepest block by looking at every block.
fn expected_owners(blocks: &[CodeBlock], starts: &[usize], line: Range<usize>) -> BTreeSet<usize> {
    let mut owners = BTreeSet::new();
    for &byte in starts {
        match innermost(blocks, byte) {
            Some(owner) => owners.insert(owner),
            None => innermost_ending_at(blocks, byte).is_some_and(|owner| owners.insert(owner)),
        };
    }
    if owners.is_empty() {
        owners.extend(line.filter_map(|byte| innermost(blocks, byte)));
    }
    owners
}

proptest! {
    /// Hit `n` is on the bytes `n * LINE_WIDTH ..` and starts matches at the chosen offsets of that line.
    #[test]
    fn a_hit_goes_to_the_blocks_the_rules_name_and_to_no_other(
        roots in forest(),
        offsets in prop::collection::vec(prop::collection::btree_set(0_usize..LINE_WIDTH, 0..=LINE_WIDTH), 0..10),
    ) {
        let mut blocks = Vec::new();
        let mut at = 0;
        for root in &roots {
            at = place(root, at, None, &mut blocks) + 1;
        }
        let hits: Vec<LineHit> = offsets
            .iter()
            .enumerate()
            .filter(|(_, set)| !set.is_empty())
            .map(|(line, set)| LineHit {
                line_number: line + 1,
                text: "x".repeat(LINE_WIDTH),
                match_starts: runs(&set.iter().copied().collect::<Vec<_>>()),
            })
            .collect();
        let line_starts: Vec<Option<usize>> = hits.iter().map(|hit| Some((hit.line_number - 1) * LINE_WIDTH)).collect();
        let refs: Vec<&CodeBlock> = blocks.iter().collect();

        let owned = owned_hits(&refs, &hits, &line_starts);

        prop_assert_eq!(owned.len(), blocks.len());
        for (index, block_hits) in owned.iter().enumerate() {
            let expected: Vec<usize> = hits
                .iter()
                .filter(|hit| {
                    let line_start = (hit.line_number - 1) * LINE_WIDTH;
                    let starts: Vec<usize> =
                        hit.match_starts.iter().flat_map(Clone::clone).map(|offset| line_start + offset).collect();
                    expected_owners(&blocks, &starts, line_start..line_start + LINE_WIDTH).contains(&index)
                })
                .map(|hit| hit.line_number)
                .collect();
            let actual: Vec<usize> = block_hits.iter().map(|hit| hit.line_number).collect();
            prop_assert_eq!(actual, expected);
        }
    }
}

/// Patterns that match nowhere, everywhere, between characters, at the ends of a line, and across other patterns.
const PATTERNS: [&str; 14] = ["a", "b", "ab", "^", "$", r"\b", "a*", "x|a", ".", r"\w+", "(?:)", "a.*b", "a.*", " "];

/// The lines of `source` the patterns match, found by asking the regex crate about each line.
fn matched_lines(source: &str, patterns: &[&str]) -> BTreeSet<usize> {
    let regexes: Vec<regex::Regex> = patterns
        .iter()
        .map(|pattern| regex::RegexBuilder::new(pattern).case_insensitive(true).build().unwrap())
        .collect();
    source
        .split('\n')
        .enumerate()
        .filter(|(index, line)| {
            (*index + 1 < source.split('\n').count() || !line.is_empty()) && regexes.iter().any(|re| re.is_match(line))
        })
        .map(|(index, _)| index + 1)
        .collect()
}

/// The lines of `source` with at least one byte in a block (an empty line counts when a block holds its offset), and
/// those whose closed extent, which includes the offset after the last byte where `$` matches, meets a block's.
fn lines_in_blocks(source: &str, blocks: &[CodeBlock]) -> (BTreeSet<usize>, BTreeSet<usize>) {
    let mut start = 0;
    let (mut inside, mut meeting) = (BTreeSet::new(), BTreeSet::new());
    for (index, line) in source.split('\n').enumerate() {
        let end = start + line.len();
        let held = blocks.iter().any(|block| {
            if line.is_empty() { block.span.contains(&start) } else { block.span.start < end && block.span.end > start }
        });
        if held {
            inside.insert(index + 1);
        }
        if blocks.iter().any(|block| block.span.start <= end && block.span.end >= start) {
            meeting.insert(index + 1);
        }
        start = end + 1;
    }
    (inside, meeting)
}

/// The hits `search_files` finds in a file holding `source`.
fn text_search_hits(source: &str, patterns: &[&str]) -> Vec<LineHit> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.ts"), source).unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let mut found =
        crate::text_search::search_files(&root, patterns, &crate::text_search::TextSearchOptions::default()).unwrap();
    found.hits.remove(&root.join("a.ts")).unwrap_or_default()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Whatever the patterns are, a line they match that has a byte in a block is credited to some block, and no other
    /// line is: a matched line is never lost while a block covers part of it.
    #[test]
    fn every_matched_line_inside_a_block_is_credited_to_some_block(
        roots in forest(),
        lines in prop::collection::vec("[abx {}]{0,10}", 1..6),
        patterns in prop::sample::subsequence(PATTERNS.to_vec(), 1..=3),
    ) {
        let source = lines.join("\n");
        let mut blocks = Vec::new();
        let mut at = 0;
        for root in &roots {
            at = place(root, at, None, &mut blocks) + 1;
        }
        let hits = text_search_hits(&source, &patterns);
        let line_starts = hit_line_starts(&source, &hits);
        let refs: Vec<&CodeBlock> = blocks.iter().collect();

        let owned = owned_hits(&refs, &hits, &line_starts);

        let credited: BTreeSet<usize> = owned.iter().flatten().map(|hit| hit.line_number).collect();
        let matched = matched_lines(&source, &patterns);
        let (inside, meeting) = lines_in_blocks(&source, &blocks);
        let must: BTreeSet<usize> = matched.intersection(&inside).copied().collect();
        let may: BTreeSet<usize> = matched.intersection(&meeting).copied().collect();
        prop_assert!(must.is_subset(&credited), "matched lines in a block left out: {:?}", must.difference(&credited));
        prop_assert!(credited.is_subset(&may), "lines credited that no block meets: {:?}", credited.difference(&may));
    }

    /// Each block gets exactly the lines whose matches the rules give it, where the matches of every pattern are found
    /// on their own (the searcher runs an alternation, which lets one pattern's match swallow another's).
    #[test]
    fn each_block_gets_the_lines_whose_matches_the_rules_give_it(
        roots in forest(),
        lines in prop::collection::vec("[abx {}]{0,10}", 1..6),
        patterns in prop::sample::subsequence(PATTERNS.to_vec(), 1..=3),
    ) {
        let source = lines.join("\n");
        let mut blocks = Vec::new();
        let mut at = 0;
        for root in &roots {
            at = place(root, at, None, &mut blocks) + 1;
        }
        let hits = text_search_hits(&source, &patterns);
        let line_starts = hit_line_starts(&source, &hits);
        let refs: Vec<&CodeBlock> = blocks.iter().collect();

        let owned = owned_hits(&refs, &hits, &line_starts);

        let matchers: Vec<_> = patterns
            .iter()
            .map(|pattern| grep_regex::RegexMatcherBuilder::new().case_insensitive(true).build(pattern).unwrap())
            .collect();
        let mut expected: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
        let mut line_start = 0;
        let count = source.split('\n').count();
        for (index, line) in source.split('\n').enumerate() {
            let mut starts = BTreeSet::new();
            for matcher in &matchers {
                matcher
                    .find_iter(line.as_bytes(), |found| {
                        starts.insert(line_start + found.start());
                        true
                    })
                    .unwrap();
            }
            let is_line = index + 1 < count || !line.is_empty();
            if is_line && !starts.is_empty() {
                let starts: Vec<usize> = starts.into_iter().collect();
                for owner in expected_owners(&blocks, &starts, line_start..line_start + line.len()) {
                    expected[owner].push(index + 1);
                }
            }
            line_start += line.len() + 1;
        }
        let actual: Vec<Vec<usize>> =
            owned.iter().map(|block_hits| block_hits.iter().map(|hit| hit.line_number).collect()).collect();
        prop_assert_eq!(actual, expected);
    }
}
