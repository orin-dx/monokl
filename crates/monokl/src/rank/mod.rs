//! Ranks code blocks by BM25 relevance, boosted by symbol kind and by how many matched lines each block covers.

mod bm25;
mod boost;
mod tokenize_doc;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::BuildHasher;

pub use bm25::{round6, score};
pub use boost::{coverage_boost, node_type_boost};
pub use tokenize_doc::tokenize_block;

use crate::types::{CodeBlock, RankedBlock, SymbolKind};

/// Scores each block against `query_terms` and returns them best first, with 1-based ranks.
///
/// Each block comes with its document tokens, usually from [`tokenize_block`]. The corpus for BM25 is `blocks`
/// itself: its size and average token count feed [`score`], and `df` gives each term's document frequency. A block's
/// `final_score` is `round6(bm25 * node_type_boost + coverage_boost)`.
///
/// The order is `final_score` descending (with `-0.0` equal to `0.0`), then ascending `file` by path components,
/// `line_start`, `line_end`, `node_kind` in `SymbolKind` declaration order, and `code` by bytes. Reordering the input
/// never reorders the output, except that blocks equal on all six keys keep their relative input order.
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
    ranked.sort_by(best_first);
    for (rank, block) in (1..).zip(&mut ranked) {
        block.rank = rank;
    }
    ranked
}

/// Orders ranked blocks best first by a total order over everything that tells two blocks apart: `final_score`
/// descending, then `file` (component-wise), `line_start`, `line_end`, `node_kind` and `code` ascending.
///
/// `-0.0` and `0.0` compare equal so the sign of a zero score cannot reorder blocks; every other score, NaN included,
/// follows [`f64::total_cmp`].
fn best_first(a: &RankedBlock, b: &RankedBlock) -> Ordering {
    score_order(b.final_score)
        .total_cmp(&score_order(a.final_score))
        .then_with(|| a.block.file.cmp(&b.block.file))
        .then_with(|| a.block.line_start.cmp(&b.block.line_start))
        .then_with(|| a.block.line_end.cmp(&b.block.line_end))
        .then_with(|| kind_order(a.block.node_kind).cmp(&kind_order(b.block.node_kind)))
        .then_with(|| a.block.code.cmp(&b.block.code))
}

/// Maps `-0.0` to `0.0`, which [`f64::total_cmp`] would otherwise order below it.
fn score_order(score: f64) -> f64 {
    if score == 0.0 { 0.0 } else { score }
}

/// Returns the position of `kind` in `SymbolKind`'s declaration order.
fn kind_order(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Function => 0,
        SymbolKind::Method => 1,
        SymbolKind::Constructor => 2,
        SymbolKind::Class => 3,
        SymbolKind::Struct => 4,
        SymbolKind::Enum => 5,
        SymbolKind::Interface => 6,
        SymbolKind::TypeAlias => 7,
        SymbolKind::Property => 8,
        SymbolKind::Field => 9,
        SymbolKind::Variable => 10,
        SymbolKind::Module => 11,
        SymbolKind::Impl => 12,
        SymbolKind::Macro => 13,
        SymbolKind::Other => 14,
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;

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

    // Only the signature differs, and it is not a sort key, so the sort must be stable for these to keep input order.
    #[rstest]
    #[case(["first", "second"])]
    #[case(["second", "first"])]
    fn blocks_equal_on_every_sort_key_keep_input_order(#[case] signatures: [&str; 2]) {
        let input = signatures.map(|sig| {
            let mut b = block("a.ts", 1, 5, SymbolKind::Function, Vec::new());
            b.symbol_signature = Some(sig.to_string());
            (b, tokens(&["foo"]))
        });
        let df: HashMap<String, usize> = HashMap::from([("foo".to_string(), 2)]);
        let ranked = rank_blocks(input.to_vec(), &["foo"], &df);
        let order: Vec<_> = ranked.iter().map(|r| r.block.symbol_signature.as_deref()).collect();
        assert_eq!(order, signatures.map(Some));
    }

    /// The fields that identify a ranked block and its position, for comparing whole outputs.
    fn positions(ranked: &[RankedBlock]) -> Vec<(String, usize, usize, f64, usize)> {
        ranked
            .iter()
            .map(|r| (r.block.file.to_string(), r.block.line_start, r.block.line_end, r.final_score, r.rank))
            .collect()
    }

    /// Every sort key of a ranked block plus its rank.
    fn sort_keys(ranked: &[RankedBlock]) -> Vec<(String, usize, usize, SymbolKind, String, f64, usize)> {
        ranked
            .iter()
            .map(|r| {
                let b = &r.block;
                (b.file.to_string(), b.line_start, b.line_end, b.node_kind, b.code.clone(), r.final_score, r.rank)
            })
            .collect()
    }

    /// A block that matches no query term and covers no matched line, so its final score is 0.0.
    fn unscored(
        file: &str,
        line_start: usize,
        line_end: usize,
        kind: SymbolKind,
        code: &str,
    ) -> (CodeBlock, Vec<String>) {
        let mut b = block(file, line_start, line_end, kind, vec![]);
        b.code = code.to_string();
        (b, tokens(&["other"]))
    }

    fn with_final_score(file: &str, final_score: f64) -> RankedBlock {
        RankedBlock {
            block: block(file, 1, 1, SymbolKind::Function, vec![]),
            bm25_score: 0.0,
            coverage_boost: 0.0,
            node_type_boost: 1.5,
            final_score,
            rank: 0,
            parent_context: None,
        }
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
        let blocks = vec![
            (block("z.ts", 40, 50, SymbolKind::Function, vec![]), tokens(&["user"])),
            unscored("b.ts", 5, 9, SymbolKind::Function, ""),
            unscored("a.ts", 1, 20, SymbolKind::Variable, ""),
            unscored("a.ts", 1, 20, SymbolKind::Class, ""),
            unscored("a.ts", 5, 9, SymbolKind::Function, "b()"),
            unscored("a.ts", 5, 9, SymbolKind::Function, "a()"),
        ];
        let expected = sort_keys(&rank_blocks(blocks.clone(), &["user"], &scored_df()));
        let key = |file: &str, start, end, kind, code: &str, rank| {
            (file.to_string(), start, end, kind, code.to_string(), 0.0, rank)
        };
        assert_eq!(expected[0].0, "z.ts");
        assert_eq!(
            expected[1..],
            [
                key("a.ts", 1, 20, SymbolKind::Class, "", 2),
                key("a.ts", 1, 20, SymbolKind::Variable, "", 3),
                key("a.ts", 5, 9, SymbolKind::Function, "a()", 4),
                key("a.ts", 5, 9, SymbolKind::Function, "b()", 5),
                key("b.ts", 5, 9, SymbolKind::Function, "", 6),
            ]
        );

        let mut seen = 0;
        for_each_permutation(&mut blocks.clone(), 0, &mut |permuted| {
            assert_eq!(sort_keys(&rank_blocks(permuted.to_vec(), &["user"], &scored_df())), expected);
            seen += 1;
        });
        assert_eq!(seen, 720);
    }

    #[test]
    fn rank_blocks_breaks_equal_scores_by_file_before_line() {
        let blocks = vec![
            unscored("b.ts", 1, 5, SymbolKind::Function, ""),
            unscored("a.ts", 9, 9, SymbolKind::Function, ""),
            unscored("a.ts", 1, 5, SymbolKind::Function, ""),
        ];
        assert_eq!(
            positions(&rank_blocks(blocks, &["user"], &scored_df())),
            [
                ("a.ts".to_string(), 1, 5, 0.0, 1),
                ("a.ts".to_string(), 9, 9, 0.0, 2),
                ("b.ts".to_string(), 1, 5, 0.0, 3),
            ]
        );
    }

    #[rstest]
    #[case::variable_first(SymbolKind::Variable, SymbolKind::Class)]
    #[case::class_first(SymbolKind::Class, SymbolKind::Variable)]
    fn rank_blocks_breaks_equal_spans_by_symbol_kind_declaration_order(
        #[case] first: SymbolKind,
        #[case] second: SymbolKind,
    ) {
        let blocks = vec![unscored("a.ts", 1, 5, first, ""), unscored("a.ts", 1, 5, second, "")];
        let kinds: Vec<_> = rank_blocks(blocks, &["user"], &scored_df()).iter().map(|r| r.block.node_kind).collect();
        assert_eq!(kinds, [SymbolKind::Class, SymbolKind::Variable]);
    }

    #[rstest]
    #[case::b_first("b()", "a()")]
    #[case::a_first("a()", "b()")]
    fn rank_blocks_breaks_equal_kinds_by_code_bytes(#[case] first: &str, #[case] second: &str) {
        let blocks = vec![
            unscored("a.ts", 1, 5, SymbolKind::Function, first),
            unscored("a.ts", 1, 5, SymbolKind::Function, second),
        ];
        let code: Vec<_> = rank_blocks(blocks, &["user"], &scored_df()).iter().map(|r| r.block.code.clone()).collect();
        assert_eq!(code, ["a()", "b()"]);
    }

    #[test]
    fn rank_blocks_orders_paths_component_by_component() {
        // As strings "a.b.ts" sorts first ('.' < '/'), but the component "a" sorts before "a.b.ts".
        let blocks = vec![
            unscored("a.b.ts", 1, 1, SymbolKind::Function, ""),
            unscored("a/z.ts", 1, 1, SymbolKind::Function, ""),
        ];
        let files: Vec<_> =
            rank_blocks(blocks, &["user"], &scored_df()).iter().map(|r| r.block.file.to_string()).collect();
        assert_eq!(files, ["a/z.ts", "a.b.ts"]);
    }

    #[rstest]
    #[case::negative_zero_in_the_later_file(with_final_score("b.ts", -0.0), with_final_score("a.ts", 0.0))]
    #[case::negative_zero_in_the_earlier_file(with_final_score("a.ts", -0.0), with_final_score("b.ts", 0.0))]
    fn best_first_treats_negative_zero_as_equal_to_zero(#[case] first: RankedBlock, #[case] second: RankedBlock) {
        let mut ranked = [first, second];
        ranked.sort_by(best_first);
        let files: Vec<_> = ranked.iter().map(|r| r.block.file.as_str()).collect();
        assert_eq!(files, ["a.ts", "b.ts"]);
    }

    #[test]
    fn best_first_orders_higher_scores_first_and_nan_by_total_order() {
        let mut ranked = [
            with_final_score("a.ts", -1.0),
            with_final_score("b.ts", f64::NAN),
            with_final_score("c.ts", 2.0),
            with_final_score("d.ts", 0.0),
        ];
        ranked.sort_by(best_first);
        let files: Vec<_> = ranked.iter().map(|r| r.block.file.as_str()).collect();
        assert_eq!(files, ["b.ts", "c.ts", "d.ts", "a.ts"]);
    }

    #[test]
    fn rank_blocks_uses_the_input_as_its_corpus() {
        // One block: corpus size 1, average length 4. ln((1 - 1 + 0.5) / (1 + 0.5) + 1) * (2 * 2.5) / (2 + 1.5) = 0.410974,
        // and the final score is round6(0.410974 * 1.5 + 0.0).
        let blocks = vec![(block("a.ts", 1, 1, SymbolKind::Function, vec![]), tokens(&["foo", "foo", "bar", "baz"]))];
        let df = HashMap::from([("foo".to_string(), 1)]);
        let ranked = rank_blocks(blocks, &["foo"], &df);
        let scores: Vec<_> = ranked.iter().map(|r| (r.bm25_score, r.coverage_boost, r.final_score, r.rank)).collect();
        assert_eq!(scores, [(0.410_974, 0.0, 0.616_461, 1)]);
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

#[cfg(test)]
mod proptests;
