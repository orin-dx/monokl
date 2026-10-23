//! The search query language: lexing, parsing, and planning.
pub mod lexer;
pub mod parser;
pub mod plan;

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

    #[test]
    fn lexer_and_token_only_reachable_via_query_lexer_path() {
        use crate::query::lexer::{Lexer, Token};
        let mut l = Lexer::new("foo");
        assert_eq!(l.next_token(), Token::Word("foo".to_string()));
    }

    // Reachability tests cannot detect an extra re-export, such as `pub type Token = lexer::Token;`, so this compares
    // every non-comment line before the test module against the exact expected declarations.
    #[test]
    fn mod_rs_exact_reexport_surface_structural() {
        let src = include_str!("mod.rs");
        // The test module has its own `use` and `mod` lines, so only the part before it is compared.
        let head = &src[..src.find("#[cfg(test)]").expect("no test module marker found in mod.rs")];

        let head_lines: Vec<&str> =
            head.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).collect();
        assert_eq!(
            head_lines,
            vec![
                "pub mod lexer;",
                "pub mod parser;",
                "pub mod plan;",
                "pub use parser::{Modifier, ParsedTerm, parse};",
                "pub use plan::QueryPlan;",
            ],
            "mod.rs's pre-test portion must contain exactly these 5 declarations, in this exact order, and nothing else"
        );
    }
}
