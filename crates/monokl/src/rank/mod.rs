//! Ranks code blocks by BM25 relevance, boosted by symbol kind and by query terms in the declared name.

mod bm25;
mod boost;
mod tokenize_doc;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::BuildHasher;

pub use bm25::{round6, score};
pub use boost::{name_match_boost, node_type_boost};
pub use tokenize_doc::tokenize_block;

use crate::types::{CodeBlock, RankedBlock};

/// Scores each block against `query_terms` and returns them best first, with 1-based ranks.
///
/// Each block comes with its document tokens, usually from [`tokenize_block`]. The corpus for BM25 is `blocks`
/// itself: its size and average token count feed [`score`], and `df` gives each term's document frequency. A block's
/// `final_score` is `round6(bm25 * name_match_boost * node_type_boost)`, and its `parent` becomes the ranked block's
/// `parent_context`.
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
///     parent: None,
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
            let final_score =
                round6(bm25_score * name_match_boost(&block, query_terms) * node_type_boost(block.node_kind));
            let parent_context = block.parent.clone();
            RankedBlock { block, final_score, rank: 0, parent_context }
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
        .then_with(|| a.block.node_kind.cmp(&b.block.node_kind))
        .then_with(|| a.block.code.cmp(&b.block.code))
}

/// Maps `-0.0` to `0.0`, which [`f64::total_cmp`] would otherwise order below it.
fn score_order(score: f64) -> f64 {
    if score == 0.0 { 0.0 } else { score }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;
    use rstest::rstest;

    use super::*;
    use crate::types::{ParentContext, SymbolKind};

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
            parent: None,
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
    fn rank_blocks_orders_by_score_and_a_block_matching_no_term_scores_zero() {
        let ranked = rank_blocks(scored_corpus(), &["user", "name"], &scored_df());
        let order: Vec<_> = ranked.iter().map(|r| (r.block.file.as_str(), r.rank)).collect();
        assert_eq!(order, [("a.ts", 1), ("b.ts", 2), ("c.ts", 3)]);
        let scores: Vec<_> = ranked.iter().map(|r| r.final_score).collect();
        assert!(scores[0] > scores[1] && scores[1] > scores[2], "{scores:?}");
        assert!(scores[2].abs() < f64::EPSILON);
    }

    #[test]
    fn the_node_type_multiplier_applies_over_the_kind_tie_break() {
        // Module sorts before Impl on a tie, but Impl's multiplier (1.1) beats Module's (0.9), so Impl ranks first.
        let blocks = vec![
            (block("a.ts", 1, 5, SymbolKind::Module, vec![]), tokens(&["user"])),
            (block("a.ts", 1, 5, SymbolKind::Impl, vec![]), tokens(&["user"])),
        ];
        let ranked = rank_blocks(blocks, &["user"], &scored_df());
        let kinds: Vec<_> = ranked.iter().map(|r| r.block.node_kind).collect();
        assert_eq!(kinds, [SymbolKind::Impl, SymbolKind::Module]);
        assert!(ranked[0].final_score > ranked[1].final_score);
    }

    #[test]
    fn symbol_kind_order_is_declaration_order() {
        use SymbolKind::*;
        let declared = [
            Function,
            Method,
            Constructor,
            Class,
            Struct,
            Enum,
            Interface,
            TypeAlias,
            Property,
            Field,
            Variable,
            Module,
            Impl,
            Macro,
            Other,
        ];
        assert!(declared.is_sorted_by(|a, b| a < b), "{declared:?}");
    }

    #[test]
    fn a_ranked_block_carries_the_parent_of_its_block() {
        let parent = ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned(), line: 1 };
        let mut member = block("a.ts", 3, 5, SymbolKind::Method, vec![4]);
        member.parent = Some(parent.clone());
        let blocks =
            vec![(member, tokens(&["foo"])), (block("a.ts", 1, 9, SymbolKind::Class, vec![1]), tokens(&["foo"]))];
        let ranked = rank_blocks(blocks, &["foo"], &HashMap::from([("foo".to_owned(), 2)]));
        let parents: Vec<_> = ranked.iter().map(|r| (r.block.line_start, r.parent_context.clone())).collect();
        assert_eq!(parents, [(1, None), (3, Some(parent))]);
    }

    #[test]
    fn rank_blocks_keeps_each_block_with_its_own_tokens() {
        let ranked = rank_blocks(scored_corpus(), &["user", "name"], &scored_df());
        let kinds: Vec<_> = ranked.iter().map(|r| (r.block.file.as_str(), r.block.node_kind)).collect();
        assert_eq!(kinds, [("a.ts", SymbolKind::Function), ("b.ts", SymbolKind::Class), ("c.ts", SymbolKind::Other)]);
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
        // and the final score is round6(0.410974 * 1.0 * 1.5), the one place the composite float is the contract.
        let blocks = vec![(block("a.ts", 1, 1, SymbolKind::Function, vec![]), tokens(&["foo", "foo", "bar", "baz"]))];
        let df = HashMap::from([("foo".to_string(), 1)]);
        let ranked = rank_blocks(blocks, &["foo"], &df);
        let scores: Vec<_> = ranked.iter().map(|r| (r.final_score, r.rank)).collect();
        assert_eq!(scores, [(0.616_461, 1)]);
    }

    #[test]
    fn rank_blocks_lifts_a_block_whose_name_holds_the_query_terms_over_one_that_only_repeats_them() {
        // Both terms have document frequency 2 in a corpus of 2, and the average length is (2 + 7) / 2 = 4.5. The
        // repeating block has the higher bm25 (0.556235 against 0.437572), so it would win without the name multiplier.
        // The name holds both terms, so its multiplier is 3.0.
        let function = |file: &str, code: &str| {
            let mut b = block(file, 1, 3, SymbolKind::Function, vec![]);
            b.code = code.to_string();
            b
        };
        let blocks = vec![
            (
                function("a.ts", "function load() { parse(); config(); }"),
                tokens(&["parse", "parse", "parse", "config", "config", "config", "load"]),
            ),
            (function("b.ts", "function parseConfig() {}"), tokens(&["parse", "config"])),
        ];
        let df = HashMap::from([("parse".to_string(), 2), ("config".to_string(), 2)]);
        let ranked = rank_blocks(blocks, &["parse", "config"], &df);
        let order: Vec<_> = ranked.iter().map(|r| (r.block.file.as_str(), r.rank)).collect();
        assert_eq!(order, [("b.ts", 1), ("a.ts", 2)]);
        assert!(ranked[0].final_score > ranked[1].final_score);
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
        let scores: Vec<_> = ranked.iter().map(|r| (r.block.file.as_str(), r.final_score < 0.0)).collect();
        assert_eq!(scores, [("b.ts", false), ("a.ts", true)]);
        assert!(ranked[0].final_score.abs() < f64::EPSILON);
    }
}

#[cfg(test)]
mod proptests;
