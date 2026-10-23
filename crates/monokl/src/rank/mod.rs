//! Ranks code blocks by BM25 relevance, boosted by symbol kind and by how many matched lines each block covers.

mod bm25;
mod boost;
mod tokenize_doc;

use std::collections::HashMap;
use std::hash::BuildHasher;

pub use bm25::{round6, score};
pub use boost::{coverage_boost, node_type_boost};
pub use tokenize_doc::tokenize_block;

use crate::types::{CodeBlock, RankedBlock};

/// Scores each block against `query_terms` and returns them best first, with 1-based ranks.
///
/// Each block comes with its document tokens, usually from [`tokenize_block`]. The corpus for BM25 is `blocks`
/// itself: its size and average token count feed [`score`], and `df` gives each term's document frequency. A block's
/// `final_score` is `round6(bm25 * node_type_boost + coverage_boost)`.
///
/// The order is total: `final_score` descending, then `file`, `line_start` and `line_end` ascending. Reordering the
/// input never reorders the output.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
///
/// use camino::Utf8PathBuf;
/// use monokl::rank::{rank_blocks, tokenize_block};
/// use monokl::types::{CodeBlock, SymbolKind};
///
/// let block = |file: &str, code: &str| CodeBlock {
///     file: Utf8PathBuf::from(file),
///     line_start: 1,
///     line_end: 1,
///     node_kind: SymbolKind::Function,
///     code: code.to_string(),
///     symbol_signature: None,
///     matched_lines: Vec::new(),
///     matched_keywords: Vec::new(),
/// };
/// let blocks: Vec<_> = [block("b.ts", "parseConfig()"), block("a.ts", "readFile()")]
///     .into_iter()
///     .map(|b| {
///         let tokens = tokenize_block(&b);
///         (b, tokens)
///     })
///     .collect();
/// let df = HashMap::from([("config".to_string(), 1)]);
///
/// let ranked = rank_blocks(blocks, &["config"], &df);
/// assert_eq!(ranked[0].block.file, "b.ts");
/// assert_eq!(ranked[0].rank, 1);
/// assert_eq!(ranked[1].final_score, 0.0);
/// ```
pub fn rank_blocks<S: BuildHasher>(
    blocks: Vec<(CodeBlock, Vec<String>)>,
    query_terms: &[&str],
    df: &HashMap<String, usize, S>,
) -> Vec<RankedBlock> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let corpus_size = blocks.len();
    let total_tokens: usize = blocks.iter().map(|(_, tokens)| tokens.len()).sum();
    let avg_doc_len = bm25::as_f64(total_tokens) / bm25::as_f64(corpus_size);
    let mut ranked: Vec<RankedBlock> = blocks
        .into_iter()
        .map(|(block, doc_tokens)| {
            let bm25_score = score(&doc_tokens, query_terms, avg_doc_len, corpus_size, df);
            let coverage_boost = coverage_boost(&block);
            let node_type_boost = node_type_boost(block.node_kind);
            RankedBlock {
                block,
                bm25_score,
                coverage_boost,
                node_type_boost,
                final_score: round6(bm25_score * node_type_boost + coverage_boost),
                rank: 0,
                parent_context: None,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.final_score
            .total_cmp(&a.final_score)
            .then_with(|| a.block.file.cmp(&b.block.file))
            .then_with(|| a.block.line_start.cmp(&b.block.line_start))
            .then_with(|| a.block.line_end.cmp(&b.block.line_end))
    });
    for (rank, block) in (1..).zip(&mut ranked) {
        block.rank = rank;
    }
    ranked
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::types::SymbolKind;

    fn block(
        file: &str,
        line_start: usize,
        line_end: usize,
        node_kind: SymbolKind,
        matched_lines: Vec<usize>,
    ) -> CodeBlock {
        CodeBlock {
            file: Utf8PathBuf::from(file),
            line_start,
            line_end,
            node_kind,
            code: String::new(),
            symbol_signature: None,
            matched_lines,
            matched_keywords: Vec::new(),
        }
    }

    fn tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    /// The fields that identify a ranked block and its position, for comparing whole outputs.
    fn positions(ranked: &[RankedBlock]) -> Vec<(String, usize, usize, f64, usize)> {
        ranked
            .iter()
            .map(|r| (r.block.file.to_string(), r.block.line_start, r.block.line_end, r.final_score, r.rank))
            .collect()
    }

    fn scored_corpus() -> Vec<(CodeBlock, Vec<String>)> {
        vec![
            (block("b.ts", 1, 5, SymbolKind::Class, vec![]), tokens(&["user"])),
            (block("c.ts", 1, 5, SymbolKind::Other, vec![3]), tokens(&["foo", "bar"])),
            (block("a.ts", 1, 10, SymbolKind::Function, vec![2, 20]), tokens(&["user", "name", "user"])),
        ]
    }

    fn scored_df() -> HashMap<String, usize> {
        HashMap::from([("user".to_string(), 2), ("name".to_string(), 1)])
    }

    #[test]
    fn rank_blocks_of_no_blocks_is_empty() {
        assert_eq!(positions(&rank_blocks(Vec::new(), &["user"], &scored_df())), []);
    }

    #[test]
    fn rank_blocks_sets_every_score_component() {
        let ranked = rank_blocks(scored_corpus(), &["user", "name"], &scored_df());
        let scores: Vec<_> = ranked
            .iter()
            .map(|r| (r.block.file.as_str(), r.bm25_score, r.node_type_boost, r.coverage_boost, r.final_score, r.rank))
            .collect();
        assert_eq!(
            scores,
            [
                ("a.ts", 1.459_351, 1.5, 0.5, 2.689_027, 1),
                ("c.ts", 0.0, 0.7, 1.0, 1.0, 2),
                ("b.ts", 0.552_945, 1.4, 0.0, 0.774_123, 3),
            ]
        );
        assert_eq!(ranked.iter().filter(|r| r.parent_context.is_some()).count(), 0);
    }

    #[test]
    fn rank_blocks_keeps_each_block_with_its_own_tokens() {
        let ranked = rank_blocks(scored_corpus(), &["user", "name"], &scored_df());
        let kinds: Vec<_> = ranked.iter().map(|r| (r.block.file.as_str(), r.block.node_kind)).collect();
        assert_eq!(kinds, [("a.ts", SymbolKind::Function), ("c.ts", SymbolKind::Other), ("b.ts", SymbolKind::Class)]);
    }

    #[test]
    fn rank_blocks_breaks_score_ties_by_file_then_line_start_then_line_end() {
        let tied = |file, start, end| (block(file, start, end, SymbolKind::Function, vec![]), tokens(&["other"]));
        let blocks =
            vec![tied("b.ts", 5, 9), tied("a.ts", 10, 12), tied("a.ts", 1, 20), tied("a.ts", 5, 9), tied("a.ts", 1, 3)];
        let ranked = rank_blocks(blocks, &["user"], &scored_df());
        assert_eq!(
            positions(&ranked),
            [
                ("a.ts".to_string(), 1, 3, 0.0, 1),
                ("a.ts".to_string(), 1, 20, 0.0, 2),
                ("a.ts".to_string(), 5, 9, 0.0, 3),
                ("a.ts".to_string(), 10, 12, 0.0, 4),
                ("b.ts".to_string(), 5, 9, 0.0, 5),
            ]
        );
    }

    #[test]
    fn rank_blocks_output_is_independent_of_input_order() {
        let tied = |file, start, end| (block(file, start, end, SymbolKind::Function, vec![]), tokens(&["other"]));
        let blocks = vec![
            (block("z.ts", 40, 50, SymbolKind::Function, vec![]), tokens(&["user"])),
            tied("b.ts", 5, 9),
            tied("a.ts", 10, 12),
            tied("a.ts", 1, 20),
            tied("a.ts", 5, 9),
            tied("a.ts", 1, 3),
        ];
        let expected = positions(&rank_blocks(blocks.clone(), &["user"], &scored_df()));
        assert_eq!(expected[0].0, "z.ts");

        let mut seen = 0;
        for_each_permutation(&mut blocks.clone(), 0, &mut |permuted| {
            assert_eq!(positions(&rank_blocks(permuted.to_vec(), &["user"], &scored_df())), expected);
            seen += 1;
        });
        assert_eq!(seen, 720);
    }

    /// Calls `visit` once per ordering of `items[k..]`.
    fn for_each_permutation<T>(items: &mut [T], k: usize, visit: &mut impl FnMut(&[T])) {
        if k == items.len() {
            visit(items);
            return;
        }
        for i in k..items.len() {
            items.swap(k, i);
            for_each_permutation(items, k + 1, visit);
            items.swap(k, i);
        }
    }

    #[test]
    fn rank_blocks_orders_negative_scores_below_zero() {
        // A term in more documents than the corpus holds has a negative IDF, so its score sinks below a non-match.
        let blocks = vec![
            (block("a.ts", 1, 1, SymbolKind::Function, vec![]), tokens(&["common"])),
            (block("b.ts", 1, 1, SymbolKind::Function, vec![]), tokens(&["other"])),
        ];
        let df = HashMap::from([("common".to_string(), 9)]);
        let ranked = rank_blocks(blocks, &["common"], &df);
        let scores: Vec<_> = ranked.iter().map(|r| (r.block.file.as_str(), r.bm25_score, r.final_score)).collect();
        assert_eq!(scores, [("b.ts", 0.0, 0.0), ("a.ts", -1.15268, -1.72902)]);
    }
}
