//! Parser that turns a query string into [`ParsedTerm`]s.
use super::lexer::{Lexer, Token};
use crate::error::{MonoklError, Result};

/// How a term affects matching: `+` requires it, `-` excludes it, and a bare term is optional and scored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modifier {
    Required,
    Excluded,
    Optional,
}

/// A single query term.
///
/// `pattern` is a regex: the user's text escaped as a literal, or the raw text after a `regex:` prefix.
#[derive(Debug, Clone)]
pub struct ParsedTerm {
    pub modifier: Modifier,
    pub pattern: String,
    pub is_regex: bool,
}

/// Parses a query string into terms.
///
/// Malformed input is tolerated rather than rejected: a dangling `+`, `-`, or `regex:` yields no term, and a modifier
/// followed by `regex:` drops both the modifier and the regex prefix. Regex syntax is not validated here.
///
/// # Errors
///
/// Returns [`MonoklError::TooManyTerms`] if the query has more than 64 terms.
pub fn parse(input: &str) -> Result<Vec<ParsedTerm>> {
    const LIMIT: usize = 64;
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut lexer = Lexer::new(input);
    let mut terms: Vec<ParsedTerm> = Vec::new();
    loop {
        let tok = lexer.next_token();
        match tok {
            Token::Eof => break,
            Token::Plus => {
                let content = lexer.next_token();
                if let Some(term) = content_to_term(content, Modifier::Required) {
                    terms.push(term);
                }
            }
            Token::Minus => {
                let content = lexer.next_token();
                if let Some(term) = content_to_term(content, Modifier::Excluded) {
                    terms.push(term);
                }
            }
            Token::RegexPrefix => {
                let pattern_tok = lexer.next_token();
                match pattern_tok {
                    Token::Word(raw) | Token::QuotedString(raw) => {
                        terms.push(ParsedTerm { modifier: Modifier::Optional, pattern: raw, is_regex: true });
                    }
                    _ => {}
                }
            }
            Token::Word(w) => {
                terms.push(ParsedTerm { modifier: Modifier::Optional, pattern: regex::escape(&w), is_regex: false });
            }
            Token::QuotedString(s) => {
                terms.push(ParsedTerm { modifier: Modifier::Optional, pattern: regex::escape(&s), is_regex: false });
            }
        }
    }
    if terms.len() > LIMIT {
        return Err(MonoklError::TooManyTerms { count: terms.len(), limit: LIMIT });
    }
    Ok(terms)
}

fn content_to_term(tok: Token, modifier: Modifier) -> Option<ParsedTerm> {
    match tok {
        Token::Word(w) => Some(ParsedTerm { modifier, pattern: regex::escape(&w), is_regex: false }),
        Token::QuotedString(s) => Some(ParsedTerm { modifier, pattern: regex::escape(&s), is_regex: false }),
        // `+regex:foo` loses its modifier here, and the main loop then reads `foo` as a bare optional word.
        Token::RegexPrefix | Token::Plus | Token::Minus | Token::Eof => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_modifier_derives<T: std::fmt::Debug + Clone + PartialEq + Eq>() {}
    fn assert_parsed_term_derives<T: std::fmt::Debug + Clone>() {}

    // A generic bound can only prove a trait is implemented. These probes also prove absence: the inherent `IS`
    // shadows the trait default only when the bound holds. Each test includes a positive control.
    struct PartialEqProbe<T>(std::marker::PhantomData<T>);
    trait NotPartialEq {
        const IS: bool = false;
    }
    impl<T> NotPartialEq for PartialEqProbe<T> {}
    impl<T: PartialEq> PartialEqProbe<T> {
        const IS: bool = true;
    }

    struct EqProbe<T>(std::marker::PhantomData<T>);
    trait NotEq {
        const IS: bool = false;
    }
    impl<T> NotEq for EqProbe<T> {}
    impl<T: Eq> EqProbe<T> {
        const IS: bool = true;
    }

    #[test]
    fn parsed_term_has_no_partialeq_or_eq() {
        const {
            assert!(!PartialEqProbe::<ParsedTerm>::IS);
            assert!(!EqProbe::<ParsedTerm>::IS);
            assert!(PartialEqProbe::<Modifier>::IS);
            assert!(EqProbe::<Modifier>::IS);
        }
    }

    struct ModifierCopyProbe<T>(std::marker::PhantomData<T>);
    trait NotModifierCopy {
        const IS: bool = false;
    }
    impl<T> NotModifierCopy for ModifierCopyProbe<T> {}
    impl<T: Copy> ModifierCopyProbe<T> {
        const IS: bool = true;
    }

    struct ModifierHashProbe<T>(std::marker::PhantomData<T>);
    trait NotModifierHash {
        const IS: bool = false;
    }
    impl<T> NotModifierHash for ModifierHashProbe<T> {}
    impl<T: std::hash::Hash> ModifierHashProbe<T> {
        const IS: bool = true;
    }

    struct ModifierPartialOrdProbe<T>(std::marker::PhantomData<T>);
    trait NotModifierPartialOrd {
        const IS: bool = false;
    }
    impl<T> NotModifierPartialOrd for ModifierPartialOrdProbe<T> {}
    impl<T: PartialOrd> ModifierPartialOrdProbe<T> {
        const IS: bool = true;
    }

    struct ModifierOrdProbe<T>(std::marker::PhantomData<T>);
    trait NotModifierOrd {
        const IS: bool = false;
    }
    impl<T> NotModifierOrd for ModifierOrdProbe<T> {}
    impl<T: Ord> ModifierOrdProbe<T> {
        const IS: bool = true;
    }

    #[test]
    fn modifier_has_no_copy_hash_partialord_or_ord() {
        const {
            assert!(!ModifierCopyProbe::<Modifier>::IS);
            assert!(!ModifierHashProbe::<Modifier>::IS);
            assert!(!ModifierPartialOrdProbe::<Modifier>::IS);
            assert!(!ModifierOrdProbe::<Modifier>::IS);
            assert!(ModifierCopyProbe::<usize>::IS);
            assert!(ModifierHashProbe::<usize>::IS);
            assert!(ModifierPartialOrdProbe::<usize>::IS);
            assert!(ModifierOrdProbe::<usize>::IS);
        }
    }

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

    #[test]
    fn modifier_and_parsed_term_have_no_serde_derive() {
        const {
            assert!(!SerProbe::<Modifier>::IS);
            assert!(!DeProbe::<Modifier>::IS);
            assert!(!SerProbe::<ParsedTerm>::IS);
            assert!(!DeProbe::<ParsedTerm>::IS);
            assert!(SerProbe::<usize>::IS);
            assert!(DeProbe::<usize>::IS);
        }
    }

    // Reads the source because `pub(crate)` would still compile for in-crate field access.
    #[test]
    fn parsed_term_fields_are_declared_pub_structural() {
        let src = include_str!("parser.rs");
        let decl_start = src
            .find("pub struct ParsedTerm")
            .unwrap_or_else(|| panic!("`pub struct ParsedTerm` not found in parser.rs"));
        let open = src[decl_start..].find('{').unwrap() + decl_start;
        let close = src[open..].find('}').unwrap() + open;
        let field_block = &src[open..=close];
        for field in ["modifier", "pattern", "is_regex"] {
            let needle = format!("{field}:");
            let idx = field_block
                .find(&needle)
                .unwrap_or_else(|| panic!("field `{field}` not found in ParsedTerm struct body"));
            let line_start = field_block[..idx].rfind('\n').map_or(0, |p| p + 1);
            let line_end = field_block[idx..].find('\n').map_or(field_block.len(), |p| idx + p);
            let line = field_block[line_start..line_end].trim_start();
            assert!(
                line.starts_with(&format!("pub {field}:")),
                "ParsedTerm.{field} must be declared exactly `pub {field}:`, found line {line:?}"
            );
        }
    }

    // Reads the source because no derive on `ParsedTerm` makes field order observable.
    #[test]
    fn parsed_term_field_declaration_order_structural() {
        let src = include_str!("parser.rs");
        let decl_start = src
            .find("pub struct ParsedTerm")
            .unwrap_or_else(|| panic!("`pub struct ParsedTerm` not found in parser.rs"));
        let open = src[decl_start..].find('{').unwrap() + decl_start;
        let close = src[open..].find('}').unwrap() + open;
        let block = &src[open..=close];

        let fields = ["modifier", "pattern", "is_regex"];
        let mut last_pos: Option<usize> = None;
        for f in fields {
            let pos = block.find(f).unwrap_or_else(|| panic!("field `{f}` not found in ParsedTerm struct body"));
            if let Some(last) = last_pos {
                assert!(pos > last, "field `{f}` found out of the expected declaration order");
            }
            last_pos = Some(pos);
        }
    }

    // `Modifier` is only ever compared with `==`, so nothing else would fail to compile if a variant were added.
    #[test]
    fn modifier_exhaustive_match_admits_exactly_three_variants() {
        fn describe(m: &Modifier) -> &'static str {
            match m {
                Modifier::Required => "required",
                Modifier::Excluded => "excluded",
                Modifier::Optional => "optional",
            }
        }
        assert_eq!(describe(&Modifier::Required), "required");
        assert_eq!(describe(&Modifier::Excluded), "excluded");
        assert_eq!(describe(&Modifier::Optional), "optional");
    }

    #[test]
    fn modifier_discriminants_pinned_in_declaration_order() {
        assert_eq!(Modifier::Required as isize, 0);
        assert_eq!(Modifier::Excluded as isize, 1);
        assert_eq!(Modifier::Optional as isize, 2);
    }

    #[test]
    fn modifier_and_parsed_term_shapes() {
        assert_modifier_derives::<Modifier>();
        assert_parsed_term_derives::<ParsedTerm>();
        assert_eq!(Modifier::Required, Modifier::Required);
        assert_ne!(Modifier::Required, Modifier::Excluded);
        let t = ParsedTerm { modifier: Modifier::Optional, pattern: "x".to_string(), is_regex: false };
        assert_eq!(t.modifier, Modifier::Optional);
        assert_eq!(t.pattern, "x");
        assert!(!t.is_regex);
    }

    #[test]
    fn empty_and_ascii_whitespace_input_returns_empty_vec() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("   \t\n  ").unwrap().is_empty());
    }

    #[test]
    fn nbsp_only_input_returns_empty_vec_via_unicode_trim() {
        // U+00A0 is Unicode whitespace, so `str::trim` removes it.
        let terms = parse("\u{00A0}").unwrap();
        assert!(terms.is_empty());
    }

    #[test]
    fn nbsp_prefixed_word_is_not_trimmed_away_and_lexer_keeps_the_nbsp() {
        // The trim is only for the emptiness check; the lexer skips ASCII whitespace only, so the NBSP stays in the
        // word.
        let terms = parse("\u{00A0}foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, "\u{00A0}foo");
    }

    #[test]
    fn plus_word_produces_required_escaped_term() {
        // The input needs a regex metacharacter, or a missing `regex::escape` would go unnoticed.
        let terms = parse("+foo.bar").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Required);
        assert_eq!(terms[0].pattern, "foo\\.bar");
        assert!(!terms[0].is_regex);
    }

    #[test]
    fn minus_quoted_produces_excluded_escaped_term() {
        let terms = parse("-\"a.b\"").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Excluded);
        assert_eq!(terms[0].pattern, regex::escape("a.b"));
        assert!(!terms[0].is_regex);
    }

    #[test]
    fn bare_word_and_quoted_string_produce_optional_escaped_terms() {
        // The quoted text needs a regex metacharacter, or a missing `regex::escape` would go unnoticed.
        let terms = parse("foo.bar \"a.b\"").unwrap();
        assert_eq!(terms.len(), 2);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert_eq!(terms[0].pattern, regex::escape("foo.bar"));
        assert!(!terms[0].is_regex);
        assert_eq!(terms[1].modifier, Modifier::Optional);
        assert_eq!(terms[1].pattern, "a\\.b");
    }

    #[test]
    fn dangling_plus_minus_at_eof_produce_no_term() {
        assert!(parse("+").unwrap().is_empty());
        assert!(parse("-").unwrap().is_empty());
    }

    #[test]
    fn plus_followed_by_plus_swallows_the_second_plus_as_content_then_reprocesses_foo() {
        // The second `+` is consumed as the first one's content and is not rewound, so `foo` is read as a bare word.
        let terms = parse("+ +foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, regex::escape("foo"));
    }

    #[test]
    fn plus_and_minus_are_whitespace_insensitive_before_their_term() {
        let with_space = parse("+ foo").unwrap();
        let without_space = parse("+foo").unwrap();
        assert_eq!(with_space.len(), 1);
        assert_eq!(with_space[0].modifier, without_space[0].modifier);
        assert_eq!(with_space[0].pattern, without_space[0].pattern);
        assert_eq!(with_space[0].is_regex, without_space[0].is_regex);

        let with_space_m = parse("- foo").unwrap();
        let without_space_m = parse("-foo").unwrap();
        assert_eq!(with_space_m[0].modifier, without_space_m[0].modifier);
        assert_eq!(with_space_m[0].pattern, without_space_m[0].pattern);
    }

    #[test]
    fn regex_prefix_word_produces_optional_regex_term_raw_pattern() {
        let terms = parse("regex:log.n").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(terms[0].is_regex);
        assert_eq!(terms[0].pattern, "log.n");
    }

    #[test]
    fn regex_prefix_quoted_produces_optional_regex_term_raw_pattern() {
        let terms = parse("regex:\"a.b\"").unwrap();
        assert_eq!(terms.len(), 1);
        assert!(terms[0].is_regex);
        assert_eq!(terms[0].pattern, "a.b");
    }

    #[test]
    fn bare_trailing_regex_prefix_produces_zero_terms() {
        assert!(parse("regex:").unwrap().is_empty());
    }

    #[test]
    fn regexp_without_colon_parses_as_ordinary_word_not_regex_prefix() {
        let terms = parse("regexp").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, "regexp");
    }

    #[test]
    fn uppercase_regex_prefix_is_not_special_cased_by_the_parser() {
        let terms = parse("REGEX:foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, regex::escape("REGEX:foo"));
    }

    #[test]
    fn regex_prefix_followed_by_minus_swallows_the_minus_and_reprocesses_foo() {
        let terms = parse("regex: -foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, "foo");
    }

    #[test]
    fn plus_regex_prefix_degrades_to_optional_escaped_literal_foo() {
        let terms = parse("+regex:foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, regex::escape("foo"));
    }

    #[test]
    fn minus_regex_prefix_degrades_to_optional_escaped_literal_foo() {
        let terms = parse("-regex:foo").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, regex::escape("foo"));
    }

    #[test]
    fn exactly_64_terms_succeeds() {
        let input: Vec<String> = (0..64).map(|i| format!("w{i}")).collect();
        let terms = parse(&input.join(" ")).unwrap();
        assert_eq!(terms.len(), 64);
    }

    #[test]
    fn sixty_five_terms_fails_with_too_many_terms() {
        let input: Vec<String> = (0..65).map(|i| format!("w{i}")).collect();
        let err = parse(&input.join(" ")).unwrap_err();
        match err {
            MonoklError::TooManyTerms { count, limit } => {
                assert_eq!(count, 65);
                assert_eq!(limit, 64);
            }
            other => panic!("expected TooManyTerms, got {other:?}"),
        }
        assert_eq!(err.to_string(), "query has too many terms: 65 > 64");
    }

    // The 64/65 boundary cannot tell a post-loop check from an in-loop check that bails at 65; the reported count can.
    #[test]
    fn limit_check_runs_after_full_loop_with_true_total_count_not_capped() {
        let input: Vec<String> = (0..100).map(|i| format!("w{i}")).collect();
        let err = parse(&input.join(" ")).unwrap_err();
        match err {
            MonoklError::TooManyTerms { count, limit } => {
                assert_eq!(count, 100);
                assert_eq!(limit, 64);
            }
            other => panic!("expected TooManyTerms, got {other:?}"),
        }
        assert_eq!(err.to_string(), "query has too many terms: 100 > 64");
    }

    #[test]
    fn invalid_regex_syntax_still_parses_successfully_unvalidated() {
        let terms = parse("regex:(unclosed").unwrap();
        assert_eq!(terms.len(), 1);
        assert!(terms[0].is_regex);
        assert_eq!(terms[0].pattern, "(unclosed");
    }

    // Compared against a literal because `regex::escape("a b")` is `"a b"` and would hide a whitespace bug.
    #[test]
    fn quoted_string_with_ascii_whitespace_preserved_in_pattern() {
        let terms = parse("\"a b\"").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].modifier, Modifier::Optional);
        assert!(!terms[0].is_regex);
        assert_eq!(terms[0].pattern, "a b");
    }

    // The `RegexPrefix` arm is indistinguishable by behavior from any other `None` arm, so its explicit, wildcard-free
    // handling is checked in the source.
    #[test]
    fn content_to_term_has_explicit_regex_prefix_arm_structural() {
        let src = include_str!("parser.rs");
        let fn_start =
            src.find("fn content_to_term").unwrap_or_else(|| panic!("`fn content_to_term` not found in parser.rs"));
        let open = src[fn_start..].find('{').unwrap() + fn_start;
        let mut depth: i32 = 0;
        let mut close = None;
        for (i, ch) in src[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.unwrap_or_else(|| panic!("no matching close brace found for content_to_term"));
        let body = &src[open..=close];
        assert!(
            body.contains("Token::RegexPrefix |") && !body.contains("_ =>"),
            "content_to_term's match must name `Token::RegexPrefix` in its `None` arm with no wildcard, found body: {body:?}"
        );
    }
}
