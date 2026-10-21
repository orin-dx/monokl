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

    /// Returns whether the plan has no terms at all, including excluded ones.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
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

    // A generic bound can only prove a trait is implemented. These probes also prove absence: the inherent `IS`
    // shadows the trait default only when the bound holds.
    struct SerProbe<T>(std::marker::PhantomData<T>);
    trait NotSer {
        const IS: bool = false;
    }
    impl<T> NotSer for SerProbe<T> {}
    impl<T: serde::Serialize> SerProbe<T> {
        const IS: bool = true;
    }

    struct DeProbe<T>(std::marker::PhantomData<T>);
    trait NotDe {
        const IS: bool = false;
    }
    impl<T> NotDe for DeProbe<T> {}
    impl<T: serde::de::DeserializeOwned> DeProbe<T> {
        const IS: bool = true;
    }

    struct DefaultProbe<T>(std::marker::PhantomData<T>);
    trait NotDefault {
        const IS: bool = false;
    }
    impl<T> NotDefault for DefaultProbe<T> {}
    impl<T: Default> DefaultProbe<T> {
        const IS: bool = true;
    }

    #[test]
    fn query_plan_has_no_serde_derive() {
        const {
            assert!(!SerProbe::<QueryPlan>::IS);
            assert!(!DeProbe::<QueryPlan>::IS);
            // Positive control: the probes are not always false.
            assert!(SerProbe::<usize>::IS);
            assert!(DeProbe::<usize>::IS);
        }
    }

    #[test]
    fn query_plan_has_no_default() {
        const {
            assert!(!DefaultProbe::<QueryPlan>::IS);
            assert!(DefaultProbe::<usize>::IS);
        }
    }

    fn assert_query_plan_derives<T: std::fmt::Debug + Clone>() {}

    #[test]
    fn query_plan_shape_and_pub_fields() {
        assert_query_plan_derives::<QueryPlan>();
        let qp = QueryPlan { terms: vec![], required: vec![], excluded: vec![], scored: vec![] };
        assert!(qp.terms.is_empty());
        assert!(qp.required.is_empty());
        assert!(qp.excluded.is_empty());
        assert!(qp.scored.is_empty());
    }

    // Reads the source because `pub(crate)` would still compile in the in-crate test above.
    #[test]
    fn query_plan_fields_are_declared_pub_structural() {
        let src = include_str!("plan.rs");
        let decl_start =
            src.find("pub struct QueryPlan").unwrap_or_else(|| panic!("`pub struct QueryPlan` not found in plan.rs"));
        let open = src[decl_start..].find('{').unwrap() + decl_start;
        let close = src[open..].find('}').unwrap() + open;
        let field_block = &src[open..=close];
        for field in ["terms", "required", "excluded", "scored"] {
            let needle = format!("{field}:");
            let idx = field_block
                .find(&needle)
                .unwrap_or_else(|| panic!("field `{field}` not found in QueryPlan struct body"));
            let line_start = field_block[..idx].rfind('\n').map_or(0, |p| p + 1);
            let line_end = field_block[idx..].find('\n').map_or(field_block.len(), |p| idx + p);
            let line = field_block[line_start..line_end].trim_start();
            assert!(
                line.starts_with(&format!("pub {field}:")),
                "QueryPlan.{field} must be declared exactly `pub {field}:`, found line {line:?}"
            );
        }
    }

    #[test]
    fn query_plan_field_declaration_order_structural() {
        let src = include_str!("plan.rs");
        let decl_start =
            src.find("pub struct QueryPlan").unwrap_or_else(|| panic!("`pub struct QueryPlan` not found in plan.rs"));
        let open = src[decl_start..].find('{').unwrap() + decl_start;
        let close = src[open..].find('}').unwrap() + open;
        let block = &src[open..=close];

        let fields = ["terms", "required", "excluded", "scored"];
        let mut last_pos: Option<usize> = None;
        for f in fields {
            let pos = block.find(f).unwrap_or_else(|| panic!("field `{f}` not found in QueryPlan struct body"));
            if let Some(last) = last_pos {
                assert!(pos > last, "field `{f}` found out of the expected declaration order");
            }
            last_pos = Some(pos);
        }
    }

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
    fn is_empty_true_only_when_terms_is_fully_empty() {
        let qp = QueryPlan::from_terms(Vec::new());
        assert!(qp.is_empty());

        // An exclusion-only plan has no search patterns but is not empty.
        let excl_only = QueryPlan::from_terms(vec![term(Modifier::Excluded, "e0"), term(Modifier::Excluded, "e1")]);
        assert!(!excl_only.is_empty());
        assert!(excl_only.search_patterns().is_empty());
    }

    // Catches an off-by-one such as `self.terms.len() <= 1`, which the zero- and two-term cases miss.
    #[test]
    fn is_empty_false_for_a_single_term_plan() {
        let qp = QueryPlan::from_terms(vec![term(Modifier::Optional, "s0")]);
        assert!(!qp.is_empty());
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
