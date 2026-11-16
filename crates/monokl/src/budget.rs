//! The agent-layer search budget: cuts a fully ranked, uncapped result down to what one response may carry.
//!
//! Core returns every ranked block and holds no caps; this module is the first thing a caller does with that result.
//! Three rules cut it, in this order:
//!
//! 1. The relevance cut drops the rows scoring below [`Budget::min_score`] times the top score, but never one of the
//!    first [`MIN_ROWS`].
//! 2. The row cap, [`Budget::max_results`], applies to what is left and wins over the relevance cut's floor.
//! 3. The token cap counts the text the caller prints for each row, which the caller supplies, so a cap means what the
//!    reader sees and not the size of the code blocks behind the rows.
use michi::TruncationOutcome;

use crate::types::{Diagnostic, RankedBlock, SearchResponse};

/// The rows the relevance cut always keeps, when that many exist.
pub const MIN_ROWS: usize = 8;

/// What one search response must stay within: a relevance cut and two caps.
///
/// `max_tokens` counts the printed text of the kept rows, as given to [`apply_budget`].
///
/// # Examples
///
/// ```
/// use monokl::budget::Budget;
///
/// let budget = Budget::default();
/// assert_eq!((budget.max_results, budget.max_tokens, budget.min_score), (20, 20_000, 0.3));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The most rows a response may carry. It wins over the floor of [`MIN_ROWS`].
    pub max_results: usize,
    /// The most estimated tokens (see [`count_tokens`]) the kept rows' printed text may total.
    pub max_tokens: usize,
    /// The share of the top score a row needs to be kept, from `0.0` (no relevance cut) to `1.0` (only rows that tie
    /// with the top row). The first [`MIN_ROWS`] rows are kept whatever their score.
    pub min_score: f64,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_results: 20, max_tokens: 20_000, min_score: 0.3 }
    }
}

/// Returns the estimated token count of `text`: its byte length divided by three, rounded up.
///
/// Code and TOON rows run 3.0 to 3.6 bytes per `o200k_base` token, so this reads about a tenth high and rarely low. A
/// tokenizer would be exact for one vocabulary only, and no vocabulary is the one every reader uses. Building a real one
/// costs about 55 ms on the first row, more than the rest of a search.
///
/// # Examples
///
/// ```
/// use monokl::budget::count_tokens;
///
/// assert_eq!(count_tokens(""), 0);
/// assert_eq!(count_tokens("hello world"), 4);
/// ```
pub fn count_tokens(text: &str) -> usize {
    text.len().div_ceil(3)
}

/// How many leading rows of `ranked` the relevance cut keeps: those scoring at least `min_score` times the top row's
/// score, and never fewer than [`MIN_ROWS`] (or all rows, when there are fewer). `ranked` is in rank order, so the
/// scores fall and the kept rows are a prefix. A top score of 0 makes every row relevant.
fn relevant_rows(ranked: &[RankedBlock], min_score: f64) -> usize {
    let Some(top) = ranked.first() else { return 0 };
    let floor = min_score * top.final_score;
    let above = ranked.iter().take_while(|row| row.final_score >= floor).count();
    above.max(MIN_ROWS.min(ranked.len()))
}

/// Keeps the longest rank-order prefix of `ranked` that passes the relevance cut and whose printed rows fit `budget`,
/// and reports what was cut.
///
/// `row_texts` holds the text that is printed for each block, in the same order; the token cap counts that text, not
/// the block's code. Blocks are never split or reordered. The relevance cut keeps the rows scoring at least
/// [`Budget::min_score`] times the top score, and at least [`MIN_ROWS`] rows; the row cap then applies to what it kept,
/// so a `max_results` below [`MIN_ROWS`] wins. The token cut falls at the first block that would break the cap, so a row
/// larger than the whole token budget leaves no results. `total_before` is the count the caller had before any cut,
/// which may exceed `ranked.len()` when an earlier stage already dropped candidates; the response is truncated whenever
/// fewer blocks are kept than that. `diagnostics` pass through untouched.
///
/// # Examples
///
/// ```
/// use monokl::budget::{apply_budget, Budget};
///
/// let response = apply_budget(Vec::new(), 0, &Budget::default(), &[], Vec::new());
/// assert!(!response.truncated);
/// assert_eq!(response.total_blocks_before_truncation, 0);
/// assert_eq!(response.blocks_cut_by_relevance, 0);
/// ```
pub fn apply_budget(
    ranked: Vec<RankedBlock>,
    total_before: usize,
    budget: &Budget,
    row_texts: &[String],
    diagnostics: Vec<Diagnostic>,
) -> SearchResponse {
    let mut kept = Vec::new();
    let mut total_tokens = 0usize;
    debug_assert_eq!(ranked.len(), row_texts.len(), "each block needs its printed row");
    let relevant = relevant_rows(&ranked, budget.min_score);
    let blocks_cut_by_relevance = ranked.len() - relevant;
    for (block, text) in ranked.into_iter().zip(row_texts).take(relevant) {
        if kept.len() >= budget.max_results {
            break;
        }
        let Some(next_tokens) = total_tokens.checked_add(count_tokens(text)).filter(|t| *t <= budget.max_tokens) else {
            break;
        };
        total_tokens = next_tokens;
        kept.push(block);
    }

    let outcome = TruncationOutcome::new(kept, total_before);
    SearchResponse {
        total_blocks_before_truncation: outcome.total_before,
        blocks_cut_by_relevance,
        truncated: outcome.truncated(),
        results: outcome.kept,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::types::{CodeBlock, DiagnosticKind, SymbolKind};

    fn block(code: &str, rank: usize) -> RankedBlock {
        RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from(format!("src/f{rank}.rs")),
                line_start: 1,
                line_end: 1,
                node_kind: SymbolKind::Function,
                span: 0..code.len(),
                parent_index: None,
                code: code.to_owned(),
                symbol_signature: None,
                signature_span: None,
                where_start: None,
                name: None,
                matched_lines: Vec::new(),
                parent: None,
            },
            final_score: 1.0,
            rank,
        }
    }

    /// Budgets with the block's code as its printed row, which is all these tests need from the row seam.
    fn apply(ranked: Vec<RankedBlock>, total_before: usize, budget: &Budget) -> SearchResponse {
        let texts: Vec<String> = ranked.iter().map(|b| b.block.code.clone()).collect();
        apply_budget(ranked, total_before, budget, &texts, Vec::new())
    }

    fn ranks(response: &SearchResponse) -> Vec<usize> {
        response.results.iter().map(|r| r.rank).collect()
    }

    fn blocks(code: &str, n: usize) -> Vec<RankedBlock> {
        (1..=n).map(|rank| block(code, rank)).collect()
    }

    /// Caps with the relevance cut off, so a test of a cap does not depend on scores.
    fn caps(max_results: usize, max_tokens: usize) -> Budget {
        Budget { max_results, max_tokens, min_score: 0.0 }
    }

    /// One block per score, in the order given, ranked from 1.
    fn scored(scores: &[f64]) -> Vec<RankedBlock> {
        scores.iter().zip(1..).map(|(&final_score, rank)| RankedBlock { final_score, ..block("a", rank) }).collect()
    }

    /// The ranks that survive the relevance cut of `min_score` alone, with the row and token caps out of the way.
    fn relevant(scores: &[f64], min_score: f64) -> Vec<usize> {
        let budget = Budget { max_results: usize::MAX, max_tokens: usize::MAX, min_score };
        let ranked = scored(scores);
        let total = ranked.len();
        let texts: Vec<String> = ranked.iter().map(|b| b.block.code.clone()).collect();
        ranks(&apply_budget(ranked, total, &budget, &texts, Vec::new()))
    }

    /// The scores `head` followed by `n` copies of `low`.
    fn then(head: &[f64], low: f64, n: usize) -> Vec<f64> {
        head.iter().copied().chain(std::iter::repeat_n(low, n)).collect()
    }

    fn one_to(n: usize) -> Vec<usize> {
        (1..=n).collect()
    }

    #[test]
    fn default_budget_is_the_agent_default() {
        assert_eq!(Budget::default(), Budget { max_results: 20, max_tokens: 20_000, min_score: 0.3 });
        assert_eq!(MIN_ROWS, 8);
    }

    #[test]
    fn a_row_is_kept_when_it_scores_at_least_the_fraction_of_the_top_score() {
        // 0.5 * 0.5 is exactly 0.25 in binary, so the boundary is exact. Nine rows lie above it, so the floor of 8
        // does not decide these.
        assert_eq!(relevant(&[vec![0.5; 9], vec![0.25, 0.2499, 0.1]].concat(), 0.5), one_to(10));
        assert_eq!(relevant(&[vec![1.0; 9], vec![0.5, 0.4999]].concat(), 0.5), one_to(10));
        assert_eq!(relevant(&[vec![8.0, 4.0], vec![2.4; 8], vec![2.3999, 0.1]].concat(), 0.3), one_to(10));
    }

    #[test]
    fn the_first_eight_rows_are_kept_whatever_their_score() {
        assert_eq!(relevant(&then(&[1.0], 0.05, 11), 0.3), one_to(8));
        assert_eq!(relevant(&then(&[1.0; 3], 0.05, 9), 0.3), one_to(8));
        assert_eq!(relevant(&then(&[1.0; 8], 0.01, 4), 0.3), one_to(8));
        assert_eq!(relevant(&then(&[1.0; 9], 0.01, 3), 0.3), one_to(9));
    }

    #[test]
    fn fewer_rows_than_the_floor_are_all_kept() {
        assert_eq!(relevant(&[], 0.3), Vec::<usize>::new());
        assert_eq!(relevant(&[1.0], 0.3), [1]);
        assert_eq!(relevant(&[1.0, 0.0], 0.3), [1, 2]);
        assert_eq!(relevant(&then(&[1.0], 0.0, 6), 0.3), one_to(7));
        assert_eq!(relevant(&then(&[1.0], 0.0, 7), 0.3), one_to(8));
    }

    #[test]
    fn a_row_that_ties_with_the_top_is_kept_at_the_strictest_fraction() {
        assert_eq!(relevant(&[vec![1.0; 10], vec![0.99, 0.1]].concat(), 1.0), one_to(10));
        assert_eq!(relevant(&then(&[1.0], 0.9, 11), 1.0), one_to(8));
    }

    #[test]
    fn a_top_score_of_zero_cuts_nothing() {
        assert_eq!(relevant(&[0.0; 12], 0.3), one_to(12));
    }

    #[test]
    fn a_fraction_of_zero_turns_the_relevance_cut_off() {
        assert_eq!(relevant(&then(&[1.0, 0.1, 0.01, 0.001], 0.0, 8), 0.0), one_to(12));
    }

    #[test]
    fn a_not_a_number_score_ends_the_relevant_rows_but_the_floor_still_holds() {
        assert_eq!(relevant(&then(&[1.0, f64::NAN], 0.9, 10), 0.3), one_to(8));
    }

    #[test]
    fn the_row_cap_wins_over_the_floor_and_over_the_relevance_cut() {
        let at = |max_results, scores: &[f64]| {
            let ranked = scored(scores);
            let total = ranked.len();
            let texts: Vec<String> = ranked.iter().map(|b| b.block.code.clone()).collect();
            let budget = Budget { max_results, max_tokens: usize::MAX, min_score: 0.3 };
            apply_budget(ranked, total, &budget, &texts, Vec::new())
        };
        assert_eq!(ranks(&at(1, &then(&[1.0], 0.1, 3))), [1]);
        assert_eq!(ranks(&at(5, &[1.0; 10])), one_to(5));
        assert_eq!(ranks(&at(0, &[1.0, 0.9, 0.8])), Vec::<usize>::new());
        let twenty_of_thirty = at(20, &[1.0; 30]);
        assert_eq!(twenty_of_thirty.results.len(), 20);
        assert_eq!(twenty_of_thirty.total_blocks_before_truncation, 30);
    }

    #[test]
    fn the_response_counts_the_rows_each_rule_cut() {
        let ranked = scored(&then(&[1.0; 9], 0.1, 2));
        let texts: Vec<String> = ranked.iter().map(|b| b.block.code.clone()).collect();

        let by_relevance = apply_budget(ranked.clone(), 11, &Budget::default(), &texts, Vec::new());
        assert_eq!(ranks(&by_relevance), one_to(9));
        assert_eq!((by_relevance.blocks_cut_by_relevance, by_relevance.total_blocks_before_truncation), (2, 11));
        assert!(by_relevance.truncated);

        let by_both =
            apply_budget(ranked.clone(), 11, &Budget { max_results: 5, ..Budget::default() }, &texts, Vec::new());
        assert_eq!(ranks(&by_both), one_to(5));
        assert_eq!((by_both.blocks_cut_by_relevance, by_both.total_blocks_before_truncation), (2, 11));

        let by_cap_only = apply_budget(ranked, 11, &caps(5, 1000), &texts, Vec::new());
        assert_eq!(ranks(&by_cap_only), one_to(5));
        assert_eq!((by_cap_only.blocks_cut_by_relevance, by_cap_only.total_blocks_before_truncation), (0, 11));
    }

    #[test]
    fn the_token_cap_applies_to_the_rows_the_relevance_cut_kept() {
        let ranked = scored(&then(&[1.0; 9], 0.1, 1));
        let texts = vec!["abc".to_owned(); 10];
        let budget = Budget { max_results: 20, max_tokens: 2, min_score: 0.3 };
        let response = apply_budget(ranked, 10, &budget, &texts, Vec::new());
        assert_eq!(ranks(&response), [1, 2]);
        assert_eq!((response.blocks_cut_by_relevance, response.total_blocks_before_truncation), (1, 10));
    }

    #[test]
    fn empty_input_is_untruncated_and_keeps_diagnostics() {
        let diagnostics = vec![Diagnostic {
            kind: DiagnosticKind::Skipped,
            path: Some(Utf8PathBuf::from("a.ts")),
            message: "too large".to_owned(),
        }];
        let response = apply_budget(Vec::new(), 0, &Budget::default(), &[], diagnostics);
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.total_blocks_before_truncation, 0);
        assert!(!response.truncated);
        assert_eq!(response.diagnostics.len(), 1);
        assert_eq!(response.diagnostics[0].kind, DiagnosticKind::Skipped);
        assert_eq!(response.diagnostics[0].path.as_deref().map(camino::Utf8Path::as_str), Some("a.ts"));
        assert_eq!(response.diagnostics[0].message, "too large");
    }

    #[test]
    fn exactly_max_results_is_kept_and_one_over_is_cut() {
        let at = apply(blocks("a", 3), 3, &caps(3, 1000));
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert!(!at.truncated);

        let over = apply(blocks("a", 4), 4, &caps(3, 1000));
        assert_eq!(ranks(&over), [1, 2, 3]);
        assert!(over.truncated);
        assert_eq!(over.total_blocks_before_truncation, 4);
    }

    #[test]
    fn exactly_max_tokens_is_kept_and_one_over_is_cut() {
        let per_block = count_tokens("fn a() {}");
        assert_eq!(per_block, 3);
        let at = apply(blocks("fn a() {}", 3), 3, &caps(10, per_block * 3));
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert!(!at.truncated);

        let over = apply(blocks("fn a() {}", 3), 3, &caps(10, per_block * 3 - 1));
        assert_eq!(ranks(&over), [1, 2]);
        assert!(over.truncated);
    }

    #[test]
    fn a_later_small_block_is_not_pulled_in_past_the_cut() {
        let input = vec![block("a", 1), block("fn a() {}", 2), block("a", 3)];
        let response = apply(input, 3, &caps(10, 3));
        assert_eq!(ranks(&response), [1]);
        assert!(response.truncated);
    }

    #[test]
    fn row_larger_than_the_token_budget_keeps_nothing() {
        let response = apply(blocks(&"y".repeat(50), 2), 2, &caps(10, 16));
        assert_eq!(response.results.len(), 0);
        assert!(response.truncated);
        assert_eq!(response.total_blocks_before_truncation, 2);
    }

    #[test]
    fn the_caps_count_the_row_text_and_not_the_code() {
        // The code is 100 bytes, the printed row 4 (two tokens): only the row counts.
        let input = || vec![block(&"x".repeat(100), 1), block(&"x".repeat(100), 2)];
        let texts = ["row1".to_owned(), "row2".to_owned()];
        let response = apply_budget(input(), 2, &caps(10, 4), &texts, Vec::new());
        assert_eq!(ranks(&response), [1, 2]);
        assert!(!response.truncated);

        let response = apply_budget(input(), 2, &caps(10, 3), &texts, Vec::new());
        assert_eq!(ranks(&response), [1]);
        assert!(response.truncated);
    }

    #[test]
    fn total_before_beyond_input_marks_the_response_truncated() {
        let response = apply(blocks("a", 2), 9, &Budget::default());
        assert_eq!(ranks(&response), [1, 2]);
        assert_eq!(response.total_blocks_before_truncation, 9);
        assert!(response.truncated);
    }

    #[test]
    fn a_token_is_estimated_as_three_bytes_rounded_up() {
        for (text, expected) in [("", 0), ("a", 1), ("abc", 1), ("abcd", 2), ("hello world", 4), ("fn a() {}", 3)] {
            assert_eq!(count_tokens(text), expected, "{text:?}");
        }
    }

    #[test]
    fn the_estimate_counts_bytes_not_characters() {
        // Eight characters, twenty-four bytes.
        assert_eq!(count_tokens("日本語のコメント"), 8);
    }

    #[test]
    fn a_long_unbroken_run_is_counted_at_once() {
        let text = "a".repeat(200 * 1024);
        assert_eq!(count_tokens(&text), 200 * 1024 / 3 + 1);
    }

    #[test]
    fn apply_budget_is_deterministic() {
        let run = || {
            let r = apply(blocks("fn a() {}", 6), 6, &caps(4, 1000));
            (ranks(&r), r.truncated)
        };
        assert_eq!(run(), run());
    }
}
