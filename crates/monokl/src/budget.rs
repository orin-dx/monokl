//! The agent-layer search budget: cuts a fully ranked, uncapped result down to what one response may carry.
//!
//! Core returns every ranked block and holds no caps; this module is the first thing a caller does with that result.
use michi::TruncationOutcome;

use crate::types::{Diagnostic, RankedBlock, SearchLimits, SearchResponse};

/// Texts longer than this many bytes are measured by their byte length instead of being tokenized.
///
/// BPE merging is quadratic in the length of an unbroken pretoken (200 KB of one run takes seconds), so minified
/// bundles and base64 blobs must not reach the tokenizer. Byte length is a proven upper bound on the token count, so
/// the fallback can only make a block look larger, never let one overspend the budget.
const TOKENIZE_BYTE_CAP: usize = 32 * 1024;

const GUIDANCE: &str = "refine the query with +term or -term";

/// The three caps one search response must stay within.
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
    /// The most blocks a response may carry.
    pub max_results: usize,
    /// The most tokens (`o200k_base`) the kept blocks' code may total.
    pub max_tokens: usize,
    /// The most bytes the kept blocks' code may total.
    pub max_bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_results: 50, max_tokens: 20_000, max_bytes: 2_097_152 }
    }
}

impl From<&SearchLimits> for Budget {
    /// Fills an unset result or token cap with the default and clamps the byte cap to the 2 MiB ceiling.
    fn from(limits: &SearchLimits) -> Self {
        let default = Self::default();
        Self {
            max_results: limits.max_results.unwrap_or(default.max_results),
            max_tokens: limits.max_tokens.unwrap_or(default.max_tokens),
            max_bytes: limits.max_bytes.min(default.max_bytes),
        }
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
    tiktoken_rs::o200k_base_singleton().lock().encode_with_special_tokens(text).len()
}

/// Keeps the longest rank-order prefix of `ranked` that fits `budget` and reports what was cut.
///
/// Blocks are never split or reordered. The cut falls at the first block that would break any of the three caps, so a
/// block larger than the whole byte budget leaves no results. `total_before` is the count the caller had before any
/// cap, which may exceed `ranked.len()` when an earlier stage already dropped candidates; the response is truncated
/// whenever fewer blocks are kept than that. `total_bytes` sums the kept blocks' code length and `total_tokens` its
/// [`count_tokens`] count. `diagnostics` pass through untouched.
///
/// # Examples
///
/// ```
/// use monokl::budget::{apply_budget, Budget};
///
/// let response = apply_budget(Vec::new(), 0, &Budget::default(), Vec::new());
/// assert!(!response.truncated);
/// assert_eq!(response.truncation_marker, None);
/// ```
pub fn apply_budget(
    ranked: Vec<RankedBlock>,
    total_before: usize,
    budget: &Budget,
    diagnostics: Vec<Diagnostic>,
) -> SearchResponse {
    let mut kept = Vec::new();
    let mut total_bytes = 0usize;
    let mut total_tokens = 0usize;
    for block in ranked {
        if kept.len() >= budget.max_results {
            break;
        }
        let bytes = block.block.code.len();
        let Some(next_bytes) = total_bytes.checked_add(bytes).filter(|b| *b <= budget.max_bytes) else { break };
        let tokens = count_tokens(&block.block.code);
        let Some(next_tokens) = total_tokens.checked_add(tokens).filter(|t| *t <= budget.max_tokens) else { break };
        total_bytes = next_bytes;
        total_tokens = next_tokens;
        kept.push(block);
    }

    let outcome = TruncationOutcome::new(kept, total_before);
    let truncation_marker = outcome.hint(GUIDANCE).map(|hint| hint.as_str().to_owned());
    SearchResponse {
        total_blocks_before_truncation: outcome.total_before,
        truncated: outcome.truncated(),
        truncation_marker,
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
            },
            bm25_score: 1.0,
            coverage_boost: 0.0,
            node_type_boost: 0.0,
            final_score: 1.0,
            rank,
            parent_context: None,
        }
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
    fn budget_from_search_limits_fills_defaults_and_clamps_bytes() {
        let unset = SearchLimits { max_results: None, max_bytes: 10_000_000, max_tokens: None, max_candidates: 1 };
        assert_eq!(Budget::from(&unset), caps(50, 20_000, 2_097_152));
        let set = SearchLimits { max_results: Some(3), max_bytes: 100, max_tokens: Some(7), max_candidates: 1 };
        assert_eq!(Budget::from(&set), caps(3, 7, 100));
    }

    #[test]
    fn empty_input_is_untruncated_and_keeps_diagnostics() {
        let diagnostics = vec![Diagnostic {
            kind: DiagnosticKind::Skipped,
            path: Some(Utf8PathBuf::from("a.ts")),
            message: "too large".to_owned(),
        }];
        let response = apply_budget(Vec::new(), 0, &Budget::default(), diagnostics);
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.total_blocks_before_truncation, 0);
        assert!(!response.truncated);
        assert_eq!(response.truncation_marker, None);
        assert_eq!((response.total_bytes, response.total_tokens), (0, 0));
        assert_eq!(response.diagnostics.len(), 1);
        assert_eq!(response.diagnostics[0].kind, DiagnosticKind::Skipped);
        assert_eq!(response.diagnostics[0].path.as_deref().map(camino::Utf8Path::as_str), Some("a.ts"));
        assert_eq!(response.diagnostics[0].message, "too large");
    }

    #[test]
    fn exactly_max_results_is_kept_and_one_over_is_cut() {
        let at = apply_budget(blocks("a", 3), 3, &caps(3, 1000, 1000), Vec::new());
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert!(!at.truncated);

        let over = apply_budget(blocks("a", 4), 4, &caps(3, 1000, 1000), Vec::new());
        assert_eq!(ranks(&over), [1, 2, 3]);
        assert!(over.truncated);
        assert_eq!(over.truncation_marker.as_deref(), Some("showing 3 of 4; refine the query with +term or -term"));
    }

    #[test]
    fn exactly_max_bytes_is_kept_and_one_over_is_cut() {
        let at = apply_budget(blocks("abcd", 3), 3, &caps(10, 1000, 12), Vec::new());
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert_eq!(at.total_bytes, 12);
        assert!(!at.truncated);

        let over = apply_budget(blocks("abcd", 3), 3, &caps(10, 1000, 11), Vec::new());
        assert_eq!(ranks(&over), [1, 2]);
        assert_eq!(over.total_bytes, 8);
        assert!(over.truncated);
    }

    #[test]
    fn exactly_max_tokens_is_kept_and_one_over_is_cut() {
        let per_block = count_tokens("fn a() {}");
        assert_eq!(per_block, 4);
        let at = apply_budget(blocks("fn a() {}", 3), 3, &caps(10, per_block * 3, 1000), Vec::new());
        assert_eq!(ranks(&at), [1, 2, 3]);
        assert_eq!(at.total_tokens, 12);
        assert!(!at.truncated);

        let over = apply_budget(blocks("fn a() {}", 3), 3, &caps(10, per_block * 3 - 1, 1000), Vec::new());
        assert_eq!(ranks(&over), [1, 2]);
        assert_eq!(over.total_tokens, 8);
        assert!(over.truncated);
    }

    #[test]
    fn earliest_block_to_break_any_limit_sets_the_cut() {
        // Block 2 breaks the token cap, block 3 would have broken the byte cap.
        let input = vec![block("a", 1), block("fn a() {}", 2), block(&"x".repeat(500), 3)];
        let response = apply_budget(input, 3, &caps(10, 3, 100), Vec::new());
        assert_eq!(ranks(&response), [1]);
        assert!(response.truncated);

        // The byte cap trips on block 2 while the token and result caps still have room.
        let input = vec![block("a", 1), block(&"x ".repeat(60), 2), block("a", 3)];
        let response = apply_budget(input, 3, &caps(10, 1000, 100), Vec::new());
        assert_eq!(ranks(&response), [1]);

        // A later small block is not pulled in past the cut.
        let input = vec![block("a", 1), block("fn a() {}", 2), block("a", 3)];
        let response = apply_budget(input, 3, &caps(10, 3, 100), Vec::new());
        assert_eq!(ranks(&response), [1]);
    }

    #[test]
    fn block_larger_than_the_byte_budget_keeps_nothing() {
        let response = apply_budget(blocks(&"y".repeat(50), 2), 2, &caps(10, 1000, 49), Vec::new());
        assert_eq!(response.results.len(), 0);
        assert!(response.truncated);
        assert_eq!(response.truncation_marker.as_deref(), Some("showing 0 of 2; refine the query with +term or -term"));
        assert_eq!((response.total_bytes, response.total_tokens), (0, 0));
    }

    #[test]
    fn total_before_beyond_input_marks_the_response_truncated() {
        let response = apply_budget(blocks("a", 2), 9, &Budget::default(), Vec::new());
        assert_eq!(ranks(&response), [1, 2]);
        assert_eq!(response.total_blocks_before_truncation, 9);
        assert!(response.truncated);
        assert_eq!(response.truncation_marker.as_deref(), Some("showing 2 of 9; refine the query with +term or -term"));
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
    fn totals_do_not_depend_on_how_tight_the_token_cap_is() {
        // With a loose cap every block fits on its byte length alone; the reported total must still be the exact count.
        let input = || vec![block("fn a() {}", 1), block("let x = 1;\n", 2)];
        let loose = apply_budget(input(), 2, &caps(10, 1_000_000, 1_000_000), Vec::new());
        let tight = apply_budget(input(), 2, &caps(10, 10, 1_000_000), Vec::new());
        assert_eq!((loose.total_tokens, tight.total_tokens), (10, 10));
        assert_eq!((loose.total_bytes, tight.total_bytes), (20, 20));
        assert!(!tight.truncated);
    }

    #[test]
    fn apply_budget_is_deterministic() {
        let run = || {
            let r = apply_budget(blocks("fn a() {}", 6), 6, &caps(4, 1000, 1000), Vec::new());
            (ranks(&r), r.total_bytes, r.total_tokens, r.truncation_marker)
        };
        assert_eq!(run(), run());
    }
}
