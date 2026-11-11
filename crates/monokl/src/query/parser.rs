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
/// Malformed input is tolerated rather than rejected: a dangling `+`, `-`, or `regex:`, or an empty phrase or `regex:""` pattern, yields no term. A `+` or `-`
/// before `regex:` makes the regex term required or excluded. Regex syntax is not validated here.
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
            Token::Plus => push_modified(&mut lexer, Modifier::Required, &mut terms),
            Token::Minus => push_modified(&mut lexer, Modifier::Excluded, &mut terms),
            Token::RegexPrefix => push_regex(&mut lexer, Modifier::Optional, &mut terms),
            Token::Word(text) | Token::QuotedString(text) => push_literal(Modifier::Optional, &text, &mut terms),
        }
    }
    if terms.len() > LIMIT {
        return Err(MonoklError::TooManyTerms { count: terms.len(), limit: LIMIT });
    }
    Ok(terms)
}

/// Reads the term after a `+` or `-`; a `regex:` prefix there makes it a regex term that keeps the modifier.
fn push_modified(lexer: &mut Lexer<'_>, modifier: Modifier, terms: &mut Vec<ParsedTerm>) {
    match lexer.next_token() {
        Token::RegexPrefix => push_regex(lexer, modifier, terms),
        Token::Word(text) | Token::QuotedString(text) => push_literal(modifier, &text, terms),
        Token::Plus | Token::Minus | Token::Eof => {}
    }
}

/// Adds a literal term. An empty phrase (`""`) yields no term: it would match every line.
fn push_literal(modifier: Modifier, text: &str, terms: &mut Vec<ParsedTerm>) {
    if !text.is_empty() {
        terms.push(ParsedTerm { modifier, pattern: regex::escape(text), is_regex: false });
    }
}

/// Reads the pattern after a `regex:` prefix. A dangling prefix or an empty pattern yields no term: it would match every line.
fn push_regex(lexer: &mut Lexer<'_>, modifier: Modifier, terms: &mut Vec<ParsedTerm>) {
    if let Token::Word(pattern) | Token::QuotedString(pattern) = lexer.next_token()
        && !pattern.is_empty()
    {
        terms.push(ParsedTerm { modifier, pattern, is_regex: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn empty_regex_pattern_produces_no_term() {
        assert!(parse("regex:\"\"").unwrap().is_empty());
        assert!(parse("-regex:\"\"").unwrap().is_empty());
        assert!(parse("+regex:\"\"").unwrap().is_empty());
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
    fn plus_and_minus_before_regex_prefix_keep_their_modifier() {
        for (query, modifier) in [("+regex:foo(", Modifier::Required), ("-regex:foo(", Modifier::Excluded)] {
            let terms = parse(query).unwrap();
            assert_eq!(terms.len(), 1, "{query}");
            assert_eq!(terms[0].modifier, modifier, "{query}");
            assert!(terms[0].is_regex, "{query}");
            assert_eq!(terms[0].pattern, "foo(", "{query}");
        }
    }

    #[test]
    fn quoted_regex_pattern_keeps_whitespace_and_the_modifier() {
        let terms = parse("-regex:\"function use[A-Z]\" bar").unwrap();
        assert_eq!(terms.len(), 2);
        assert_eq!(terms[0].modifier, Modifier::Excluded);
        assert!(terms[0].is_regex);
        assert_eq!(terms[0].pattern, "function use[A-Z]");
        assert_eq!(terms[1].pattern, "bar");
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

    #[test]
    fn empty_phrase_yields_no_term_with_or_without_a_modifier() {
        assert!(parse("\"\"").unwrap().is_empty());
        assert!(parse("+\"\" -\"\"").unwrap().is_empty());
        let terms = parse("foo \"\"").unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].pattern, "foo");
    }
}

#[cfg(test)]
mod proptests;
