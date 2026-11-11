//! Partitioning of parsed query terms into required, excluded, and scored groups.
use super::parser::{Modifier, ParsedTerm};

/// A parsed query split by modifier.
///
/// The index vectors point into `terms` and keep each group's original relative order.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub terms: Vec<ParsedTerm>,
    pub required: Vec<usize>,
    pub excluded: Vec<usize>,
    pub scored: Vec<usize>,
}

impl QueryPlan {
    /// Partitions `terms` by modifier.
    pub fn from_terms(terms: Vec<ParsedTerm>) -> Self {
        let required: Vec<usize> =
            terms.iter().enumerate().filter(|(_, t)| t.modifier == Modifier::Required).map(|(i, _)| i).collect();
        let excluded: Vec<usize> =
            terms.iter().enumerate().filter(|(_, t)| t.modifier == Modifier::Excluded).map(|(i, _)| i).collect();
        let scored: Vec<usize> =
            terms.iter().enumerate().filter(|(_, t)| t.modifier == Modifier::Optional).map(|(i, _)| i).collect();
        Self { terms, required, excluded, scored }
    }

    /// Returns the required patterns followed by the scored patterns.
    ///
    /// # Panics
    ///
    /// Panics if `required` or `scored` holds an index outside `terms`, which only a hand-built plan can do.
    pub fn search_patterns(&self) -> Vec<&str> {
        self.required.iter().chain(self.scored.iter()).map(|&i| self.terms[i].pattern.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_terms_on_empty_vec_produces_all_empty_fields() {
        let qp = QueryPlan::from_terms(Vec::new());
        assert!(qp.terms.is_empty());
        assert!(qp.required.is_empty());
        assert!(qp.excluded.is_empty());
        assert!(qp.scored.is_empty());
    }

    fn term(modifier: Modifier, pattern: &str) -> ParsedTerm {
        ParsedTerm { modifier, pattern: pattern.to_string(), is_regex: false }
    }

    #[test]
    fn from_terms_partitions_preserving_relative_order() {
        let terms = vec![
            term(Modifier::Optional, "s0"),
            term(Modifier::Required, "r0"),
            term(Modifier::Excluded, "e0"),
            term(Modifier::Required, "r1"),
            term(Modifier::Optional, "s1"),
            term(Modifier::Excluded, "e1"),
        ];
        let qp = QueryPlan::from_terms(terms);
        assert_eq!(qp.required, vec![1, 3]);
        assert_eq!(qp.excluded, vec![2, 5]);
        assert_eq!(qp.scored, vec![0, 4]);
        let mut all: Vec<usize> =
            qp.required.iter().chain(qp.excluded.iter()).chain(qp.scored.iter()).copied().collect();
        all.sort_unstable();
        assert_eq!(all, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn exclusion_only_plan_has_no_search_patterns() {
        let qp = QueryPlan::from_terms(vec![term(Modifier::Excluded, "e0"), term(Modifier::Excluded, "e1")]);
        assert!(qp.search_patterns().is_empty());
    }

    #[test]
    fn search_patterns_required_then_scored_excludes_excluded() {
        let terms =
            vec![term(Modifier::Optional, "s0"), term(Modifier::Required, "r0"), term(Modifier::Excluded, "e0")];
        let qp = QueryPlan::from_terms(terms);
        // Grouped by category, not by input position.
        assert_eq!(qp.search_patterns(), vec!["r0", "s0"]);
    }

    #[test]
    fn search_patterns_preserves_each_categorys_own_relative_order() {
        let terms = vec![
            term(Modifier::Required, "r0"),
            term(Modifier::Optional, "s0"),
            term(Modifier::Required, "r1"),
            term(Modifier::Optional, "s1"),
        ];
        let qp = QueryPlan::from_terms(terms);
        assert_eq!(qp.search_patterns(), vec!["r0", "r1", "s0", "s1"]);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn search_patterns_panics_on_out_of_bounds_index_in_directly_constructed_plan() {
        let qp = QueryPlan { terms: vec![], required: vec![5], excluded: vec![], scored: vec![] };
        drop(qp.search_patterns());
    }

    // `search_patterns` reads only `required` and `scored`; it never cross-checks `excluded`.
    #[test]
    fn search_patterns_directly_constructed_plan_ignores_excluded_membership() {
        let qp = QueryPlan {
            terms: vec![term(Modifier::Excluded, "e0"), term(Modifier::Required, "r0")],
            required: vec![0, 1],
            excluded: vec![0],
            scored: vec![],
        };
        assert_eq!(qp.search_patterns(), vec!["e0", "r0"]);
    }
}
