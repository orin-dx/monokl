use std::collections::HashMap;
use std::hash::BuildHasher;

const K1: f64 = 1.5;
const B: f64 = 0.5;
const DP: f64 = 1_000_000.0;

/// Rounds `x` to 6 decimal places, halves away from zero.
///
/// Every score is rounded so output is stable across platforms and readable in agent-facing text.
///
/// # Examples
///
/// ```
/// use monokl::rank::round6;
///
/// assert_eq!(round6(0.1234565), 0.123457);
/// assert_eq!(round6(1.0000004), 1.0);
/// ```
pub fn round6(x: f64) -> f64 {
    (x * DP).round() / DP
}

/// Returns the Okapi BM25 score of one document for `query_terms`, rounded with [`round6`].
///
/// Uses `K1 = 1.5` and `B = 0.5`. `corpus_size` is the number of documents and `df` maps a term to the number of
/// documents containing it; a term missing from `df` has a document frequency of 0. A query term absent from the
/// document contributes nothing. Term frequency is exact, case-sensitive token equality, and a term repeated in
/// `query_terms` counts once per occurrence. Returns 0.0 when either `doc_tokens` or `query_terms` is empty, or when
/// `avg_doc_len` is not a finite number above zero.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
///
/// use monokl::rank::score;
///
/// let doc: Vec<String> = ["foo", "foo", "bar", "baz"].map(String::from).into();
/// let df = HashMap::from([("foo".to_string(), 1)]);
/// assert_eq!(score(&doc, &["foo"], 4.0, 2, &df), 0.99021);
/// ```
pub fn score<S: BuildHasher>(
    doc_tokens: &[String],
    query_terms: &[&str],
    avg_doc_len: f64,
    corpus_size: usize,
    df: &HashMap<String, usize, S>,
) -> f64 {
    // A non-positive or non-finite average length has no meaning as a corpus statistic and would turn the length
    // normalization into a sign flip or NaN.
    if query_terms.is_empty() || doc_tokens.is_empty() || !(avg_doc_len.is_finite() && avg_doc_len > 0.0) {
        return 0.0;
    }
    let doc_len = as_f64(doc_tokens.len());
    let n = as_f64(corpus_size);
    let mut total = 0.0;
    for term in query_terms {
        let tf = doc_tokens.iter().filter(|token| token == term).count();
        if tf == 0 {
            continue;
        }
        let tf = as_f64(tf);
        let df_t = as_f64(df.get(*term).copied().unwrap_or(0));
        let idf = ((n - df_t + 0.5) / (df_t + 0.5) + 1.0).ln();
        let norm_factor = K1 * (1.0 - B + B * (doc_len / avg_doc_len));
        total += idf * ((tf * (K1 + 1.0)) / (tf + norm_factor));
    }
    round6(total)
}

/// Converts a count to `f64`.
#[expect(clippy::cast_precision_loss, reason = "token and document counts stay far below 2^53")]
pub(super) fn as_f64(count: usize) -> f64 {
    count as f64
}

#[cfg(test)]
#[expect(clippy::float_cmp, reason = "scores are rounded to 6 places, so exact equality is the contract under test")]
mod tests {
    use rstest::rstest;

    use super::*;

    fn doc(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(ToString::to_string).collect()
    }

    fn df(entries: &[(&str, usize)]) -> HashMap<String, usize> {
        entries.iter().map(|(term, n)| ((*term).to_string(), *n)).collect()
    }

    #[rstest]
    #[case(0.123_456_5, 0.123_457)]
    #[case(1.000_000_4, 1.0)]
    #[case(0.000_000_5, 0.000_001)]
    #[case(-0.000_000_5, -0.000_001)]
    #[case(2.5, 2.5)]
    #[case(0.0, 0.0)]
    #[case(1.0 / 3.0, 0.333_333)]
    #[case(-0.123_456_5, -0.123_457)]
    fn round6_rounds_half_away_from_zero_at_six_places(#[case] x: f64, #[case] expected: f64) {
        assert_eq!(round6(x), expected);
    }

    #[rstest]
    // ln((2 - 1 + 0.5) / (1 + 0.5) + 1) * (2 * 2.5) / (2 + 1.5 * (0.5 + 0.5 * 4 / 4)) = ln(2) * 5 / 3.5
    #[case::one_term(&["foo"], &[("foo", 1)], 0.99021)]
    #[case::term_frequency_is_case_sensitive(&["Foo"], &[("foo", 1)], 0.0)]
    // Rounded once after summing: 2 * 0.990210258 = 1.980420516.
    #[case::a_repeated_query_term_counts_per_occurrence(&["foo", "foo"], &[("foo", 1)], 1.980_421)]
    // ln((2 - 0 + 0.5) / (0 + 0.5) + 1) * 5 / 3.5 = ln(6) * 5 / 3.5
    #[case::a_term_missing_from_df_has_df_zero(&["foo"], &[], 2.559_656)]
    fn score_matches_the_hand_computed_bm25_value(
        #[case] terms: &[&str],
        #[case] df_entries: &[(&str, usize)],
        #[case] expected: f64,
    ) {
        let tokens = doc(&["foo", "foo", "bar", "baz"]);
        assert_eq!(score(&tokens, terms, 4.0, 2, &df(df_entries)), expected);
    }

    #[test]
    fn score_normalizes_by_document_length() {
        // ln(2.5 / 1.5 + 1) * 2.5 / (1 + 1.5 * (0.5 + 0.5 * 2 / 4))
        let tokens = doc(&["foo", "bar"]);
        assert_eq!(score(&tokens, &["foo"], 4.0, 3, &df(&[("foo", 1)])), 1.153_917);
    }

    #[test]
    fn score_is_negative_without_clamping_when_df_exceeds_the_corpus_size() {
        // ln((1 - 3 + 0.5) / (3 + 0.5) + 1) * 2.5 / (1 + 1.5): the IDF is negative and is not clamped.
        assert_eq!(score(&doc(&["foo"]), &["foo"], 1.0, 1, &df(&[("foo", 3)])), -0.559_616);
    }

    #[test]
    fn score_sums_terms_and_treats_a_term_missing_from_df_as_zero() {
        let tokens = doc(&["foo", "qux", "bar", "foo", "zip", "zap"]);
        assert_eq!(score(&tokens, &["foo", "qux"], 3.0, 5, &df(&[("foo", 2)])), 2.94143);
    }

    #[rstest]
    #[case::zero(0.0)]
    #[case::negative(-2.0)]
    #[case::nan(f64::NAN)]
    #[case::infinite(f64::INFINITY)]
    #[case::negative_infinite(f64::NEG_INFINITY)]
    fn score_is_zero_when_the_average_document_length_is_not_a_positive_number(#[case] avg_doc_len: f64) {
        let df = df(&[("foo", 1)]);
        assert_eq!(score(&doc(&["foo"]), &["foo"], avg_doc_len, 1, &df), 0.0);
        assert_eq!(score(&doc(&["foo", "bar"]), &["foo", "absent"], avg_doc_len, 1, &df), 0.0);
    }

    #[test]
    fn score_is_zero_when_no_query_term_occurs() {
        let tokens = doc(&["foo", "bar"]);
        assert_eq!(score(&tokens, &["qux"], 2.0, 1, &df(&[("qux", 1)])), 0.0);
    }

    #[test]
    fn score_is_zero_for_empty_query_or_empty_document() {
        let df = df(&[("foo", 1)]);
        assert_eq!(score(&doc(&["foo"]), &[], 1.0, 1, &df), 0.0);
        assert_eq!(score(&[], &["foo"], 1.0, 1, &df), 0.0);
        assert_eq!(score(&[], &[], 0.0, 0, &df), 0.0);
    }
}
