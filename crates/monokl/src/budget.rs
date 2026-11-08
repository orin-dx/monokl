//! The agent-layer search budget: cuts a fully ranked, uncapped result down to what one response may carry.
//!
//! Core returns every ranked block and holds no caps; this module is the first thing a caller does with that result.
//! The caps count the text the caller prints for each row, which the caller supplies, so a cap means what the reader
//! sees and not the size of the code blocks behind the rows.
use michi::TruncationOutcome;

use crate::types::{Diagnostic, RankedBlock, SearchResponse};

/// Texts longer than this many bytes are measured by their byte length instead of being tokenized.
///
/// BPE merging is quadratic in the length of an unbroken pretoken (200 KB of one run takes seconds), so a row that
/// carries a minified or base64 line must not reach the tokenizer. Byte length is a proven upper bound on the token count, so
/// the fallback can only make a row look larger, never let one overspend the budget.
const TOKENIZE_BYTE_CAP: usize = 32 * 1024;

/// The three caps one search response must stay within.
///
/// `max_tokens` and `max_bytes` count the printed text of the kept rows, as given to [`apply_budget`].
///
/// # Examples
///
/// ```
/// use monokl::budget::Budget;
///
/// let budget = Budget::default();
/// assert_eq!((budget.max_results, budget.max_tokens, budget.max_bytes), (50, 20_000, 2_097_152));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// The most rows a response may carry.
    pub max_results: usize,
    /// The most tokens (`o200k_base`) the kept rows' printed text may total.
    pub max_tokens: usize,
    /// The most bytes the kept rows' printed text may total.
    pub max_bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_results: 50, max_tokens: 20_000, max_bytes: 2_097_152 }
    }
}

/// Returns the `o200k_base` token count of `text`, or its byte length when it is over the tokenizer's byte cap.
///
/// The byte length is an upper bound on the token count, so the count never understates.
///
/// # Examples
///
/// ```
/// use monokl::budget::count_tokens;
///
/// assert_eq!(count_tokens(""), 0);
/// assert_eq!(count_tokens("hello world"), 2);
/// ```
pub fn count_tokens(text: &str) -> usize {
    if text.len() > TOKENIZE_BYTE_CAP {
        return text.len();
    }
    tiktoken_rs::o200k_base_singleton().encode_with_special_tokens(text).len()
}

/// Keeps the longest rank-order prefix of `ranked` whose printed rows fit `budget` and reports what was cut.
///
/// `row_text` returns the text that is printed for one block; the byte and token caps count that text, not the block's
/// code. Blocks are never split or reordered. The cut falls at the first block that would break any of the three caps,
/// so a row larger than the whole byte budget leaves no results. `total_before` is the count the caller had before any
/// cap, which may exceed `ranked.len()` when an earlier stage already dropped candidates; the response is truncated
/// whenever fewer blocks are kept than that. `total_bytes` sums the kept rows' text length and `total_tokens` its
/// [`count_tokens`] count. `diagnostics` pass through untouched.
///
/// # Examples
///
/// ```
/// use monokl::budget::{apply_budget, Budget};
///
/// let response = apply_budget(Vec::new(), 0, &Budget::default(), |_| String::new(), Vec::new());
/// assert!(!response.truncated);
/// assert_eq!(response.total_blocks_before_truncation, 0);
/// ```
pub fn apply_budget(
    ranked: Vec<RankedBlock>,
    total_before: usize,
    budget: &Budget,
    row_text: impl Fn(&RankedBlock) -> String,
    diagnostics: Vec<Diagnostic>,
) -> SearchResponse {
    let mut kept = Vec::new();
    let mut total_bytes = 0usize;
    let mut total_tokens = 0usize;
    for block in ranked {
        if kept.len() >= budget.max_results {
            break;
        }
        let text = row_text(&block);
        let Some(next_bytes) = total_bytes.checked_add(text.len()).filter(|b| *b <= budget.max_bytes) else { break };
        let Some(next_tokens) = total_tokens.checked_add(count_tokens(&text)).filter(|t| *t <= budget.max_tokens)
        else {
            break;
        };
        total_bytes = next_bytes;
        total_tokens = next_tokens;
        kept.push(block);
    }

    let outcome = TruncationOutcome::new(kept, total_before);
    SearchResponse {
        total_blocks_before_truncation: outcome.total_before,
        truncated: outcome.truncated(),
        results: outcome.kept,
        total_bytes,
        total_tokens,
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
                code: code.to_owned(),
                symbol_signature: None,
                matched_lines: Vec::new(),
                matched_keywords: Vec::new(),
                parent: None,
            },
            bm25_score: 1.0,
            coverage_boost: 0.0,
            node_type_boost: 0.0,
            final_score: 1.0,
            rank,
            parent_context: None,
        }
    }

    /// Budgets with the block's code as its printed row, which is all these tests need from the row seam.
    fn apply(ranked: Vec<RankedBlock>, total_before: usize, budget: &Budget) -> SearchResponse {
        apply_budget(ranked, total_before, budget, |b| b.block.code.clone(), Vec::new())
    }

    fn ranks(response: &SearchResponse) -> Vec<usize> {
        response.results.iter().map(|r| r.rank).collect()
    }

    fn blocks(code: &str, n: usize) -> Vec<RankedBlock> {
        (1..=n).map(|rank| block(code, rank)).collect()
    }

    fn caps(max_results: usize, max_tokens: usize, max_bytes: usize) -> Budget {
        Budget { max_results, max_tokens, max_bytes }
    }

    #[test]
    fn default_budget_is_the_agent_default() {
        assert_eq!(Budget::default(), caps(50, 20_000, 2_097_152));
    }

    #[test]
    fn empty_input_is_untruncated_and_keeps_diagnostics() {
        let diagnostics = vec![Diagnostic {
            kind: DiagnosticKind::Skipped,
            path: Some(Utf8PathBuf::from("a.ts")),
            message: "too large".to_owned(),
        }];
        let response = apply_budget(Vec::new(), 0, &Budget::default(), |_| String::new(), diagnostics);
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.total_blocks_before_truncation, 0);
        assert!(!response.truncated);
        assert_eq!((response.total_bytes, response.total_tokens), (0, 0));
        assert_eq!(response.diagnostics.len(), 1);
        assert_eq!(response.diagnostics[0].kind, DiagnosticKind::Skipped);
        assert_eq!(response.diagnostics[0].path.as_deref().map(camino::Utf8Path::as_str), Some("a.ts"));
        assert_eq!(response.diagnostics[0].message, "too large");
    }

    #[test]
    fn exactly_max_results_is_kept_and_one_over_is_cut() {
        let at = apply(blocks("a", 3), 3, &caps(3, 1000, 1000));
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert!(!at.truncated);

        let over = apply(blocks("a", 4), 4, &caps(3, 1000, 1000));
        assert_eq!(ranks(&over), [1, 2, 3]);
        assert!(over.truncated);
        assert_eq!(over.total_blocks_before_truncation, 4);
    }

    #[test]
    fn exactly_max_bytes_is_kept_and_one_over_is_cut() {
        let at = apply(blocks("abcd", 3), 3, &caps(10, 1000, 12));
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert_eq!(at.total_bytes, 12);
        assert!(!at.truncated);

        let over = apply(blocks("abcd", 3), 3, &caps(10, 1000, 11));
        assert_eq!(ranks(&over), [1, 2]);
        assert_eq!(over.total_bytes, 8);
        assert!(over.truncated);
    }

    #[test]
    fn exactly_max_tokens_is_kept_and_one_over_is_cut() {
        let per_block = count_tokens("fn a() {}");
        assert_eq!(per_block, 4);
        let at = apply(blocks("fn a() {}", 3), 3, &caps(10, per_block * 3, 1000));
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert_eq!(at.total_tokens, 12);
        assert!(!at.truncated);

        let over = apply(blocks("fn a() {}", 3), 3, &caps(10, per_block * 3 - 1, 1000));
        assert_eq!(ranks(&over), [1, 2]);
        assert_eq!(over.total_tokens, 8);
        assert!(over.truncated);
    }

    #[test]
    fn earliest_block_to_break_any_limit_sets_the_cut() {
        // Block 2 breaks the token cap, block 3 would have broken the byte cap.
        let input = vec![block("a", 1), block("fn a() {}", 2), block(&"x".repeat(500), 3)];
        let response = apply(input, 3, &caps(10, 3, 100));
        assert_eq!(ranks(&response), [1]);
        assert!(response.truncated);

        // The byte cap trips on block 2 while the token and result caps still have room.
        let input = vec![block("a", 1), block(&"x ".repeat(60), 2), block("a", 3)];
        let response = apply(input, 3, &caps(10, 1000, 100));
        assert_eq!(ranks(&response), [1]);

        // A later small block is not pulled in past the cut.
        let input = vec![block("a", 1), block("fn a() {}", 2), block("a", 3)];
        let response = apply(input, 3, &caps(10, 3, 100));
        assert_eq!(ranks(&response), [1]);
    }

    #[test]
    fn row_larger_than_the_byte_budget_keeps_nothing() {
        let response = apply(blocks(&"y".repeat(50), 2), 2, &caps(10, 1000, 49));
        assert_eq!(response.results.len(), 0);
        assert!(response.truncated);
        assert_eq!(response.total_blocks_before_truncation, 2);
        assert_eq!((response.total_bytes, response.total_tokens), (0, 0));
    }

    #[test]
    fn the_caps_count_the_row_text_and_not_the_code() {
        // The code is 100 bytes, the printed row 4: only the row counts.
        let input = || vec![block(&"x".repeat(100), 1), block(&"x".repeat(100), 2)];
        let response = apply_budget(input(), 2, &caps(10, 1000, 8), |_| "row1".to_owned(), Vec::new());
        assert_eq!((ranks(&response), response.total_bytes, response.total_tokens), (vec![1, 2], 8, 4));
        assert!(!response.truncated);

        let response = apply_budget(input(), 2, &caps(10, 1000, 7), |_| "row1".to_owned(), Vec::new());
        assert_eq!((ranks(&response), response.total_bytes), (vec![1], 4));
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
    fn token_counts_match_the_o200k_encoder_for_known_strings() {
        let bpe = tiktoken_rs::o200k_base().unwrap();
        for (text, expected) in [("", 0), ("hello world", 2), ("fn a() {}", 4), ("let x = 1;\n", 6)] {
            assert_eq!(count_tokens(text), expected, "{text:?}");
            assert_eq!(bpe.encode_with_special_tokens(text).len(), expected, "{text:?}");
        }
    }

    #[test]
    fn multibyte_text_counts_tokens_not_bytes() {
        let text = "日本語のコメント";
        let bpe = tiktoken_rs::o200k_base().unwrap();
        assert_eq!(count_tokens(text), bpe.encode_with_special_tokens(text).len());
        assert!(count_tokens(text) < text.len());
    }

    #[test]
    fn text_over_the_byte_cap_is_measured_by_its_length() {
        let at_cap = "a ".repeat(TOKENIZE_BYTE_CAP / 2);
        assert_eq!(at_cap.len(), TOKENIZE_BYTE_CAP);
        // Sixteen thousand "a" words plus the trailing space as its own token.
        assert_eq!(count_tokens(&at_cap), TOKENIZE_BYTE_CAP / 2 + 1);

        let over_cap = format!("{at_cap}a");
        assert_eq!(count_tokens(&over_cap), TOKENIZE_BYTE_CAP + 1);
    }

    #[test]
    fn text_well_under_the_byte_cap_is_tokenized_not_measured_by_its_length() {
        let text = "a ".repeat(1024);
        assert_eq!(text.len(), 2048);
        let tokens = tiktoken_rs::o200k_base().unwrap().encode_with_special_tokens(&text).len();
        assert_eq!(count_tokens(&text), tokens);
        assert_eq!(tokens, 1025);
    }

    #[test]
    fn text_far_over_the_byte_cap_is_measured_by_its_length() {
        let text = "a ".repeat(20 * 1024);
        assert_eq!(text.len(), 40 * 1024);
        assert_eq!(count_tokens(&text), 40 * 1024);
    }

    #[test]
    fn totals_do_not_depend_on_how_tight_the_token_cap_is() {
        // With a loose cap every block fits on its byte length alone; the reported total must still be the exact count.
        let input = || vec![block("fn a() {}", 1), block("let x = 1;\n", 2)];
        let loose = apply(input(), 2, &caps(10, 1_000_000, 1_000_000));
        let tight = apply(input(), 2, &caps(10, 10, 1_000_000));
        assert_eq!((loose.total_tokens, tight.total_tokens), (10, 10));
        assert_eq!((loose.total_bytes, tight.total_bytes), (20, 20));
        assert!(!tight.truncated);
    }

    #[test]
    fn apply_budget_is_deterministic() {
        let run = || {
            let r = apply(blocks("fn a() {}", 6), 6, &caps(4, 1000, 1000));
            (ranks(&r), r.total_bytes, r.total_tokens, r.truncated)
        };
        assert_eq!(run(), run());
    }
}
