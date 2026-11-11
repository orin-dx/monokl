//! The search query language: lexing, parsing, and planning.
pub(crate) mod lexer;
pub(crate) mod parser;
pub(crate) mod plan;

pub use parser::{Modifier, ParsedTerm, parse};
pub use plan::QueryPlan;

#[cfg(test)]
mod tests {
    #[test]
    fn reexported_items_reachable_at_query_root() {
        use crate::query::{Modifier, ParsedTerm, QueryPlan, parse};
        let terms: Vec<ParsedTerm> = parse("+foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Required);
        let qp = QueryPlan::from_terms(terms);
        assert_eq!(qp.required, vec![0]);
    }
}
