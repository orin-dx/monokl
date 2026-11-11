//! Tokenizer for the search query language.

/// A query token: `+`, `-`, the `regex:` prefix, a quoted string, a bare word, or end of input.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Plus,
    Minus,
    RegexPrefix,
    QuotedString(String),
    Word(String),
    Eof,
}

/// A cursor over a query string that yields [`Token`]s.
pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// Creates a lexer positioned at the start of `input`.
    pub fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn remaining(&self) -> &str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_ascii_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn read_word(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.peek() {
                None => break,
                Some(ch) if ch.is_ascii_whitespace() => break,
                Some(ch) => {
                    s.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        s
    }

    // Only `\"` is an escape; any other backslash is kept literally. An unterminated string ends silently at EOF.
    fn read_quoted(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.advance() {
                None | Some('"') => break,
                Some('\\') => match self.peek() {
                    Some('"') => {
                        self.pos += 1;
                        s.push('"');
                    }
                    _ => s.push('\\'),
                },
                Some(ch) => s.push(ch),
            }
        }
        s
    }

    /// Returns the next token, or [`Token::Eof`] once the input is exhausted.
    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        match self.peek() {
            None => Token::Eof,
            Some('+') => {
                self.pos += 1;
                Token::Plus
            }
            Some('-') => {
                self.pos += 1;
                Token::Minus
            }
            Some('"') => {
                self.pos += 1;
                Token::QuotedString(self.read_quoted())
            }
            Some(_) => {
                if self.remaining().starts_with("regex:") {
                    self.pos += "regex:".len();
                    Token::RegexPrefix
                } else {
                    Token::Word(self.read_word())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_token_eof_on_empty_input() {
        let mut l = Lexer::new("");
        assert_eq!(l.next_token(), Token::Eof);
    }

    #[test]
    fn next_token_plus_consumes_one_byte_regardless_of_follower() {
        let mut l = Lexer::new("+");
        assert_eq!(l.next_token(), Token::Plus);
        assert_eq!(l.pos, 1);

        let mut l2 = Lexer::new("+x");
        assert_eq!(l2.next_token(), Token::Plus);
        assert_eq!(l2.pos, 1);
    }

    #[test]
    fn next_token_minus_consumes_one_byte_regardless_of_follower() {
        let mut l = Lexer::new("-");
        assert_eq!(l.next_token(), Token::Minus);
        assert_eq!(l.pos, 1);
    }

    #[test]
    fn next_token_quote_consumes_opening_quote_then_delegates() {
        let mut l = Lexer::new("\"ab\"");
        assert_eq!(l.next_token(), Token::QuotedString("ab".to_string()));
    }

    #[test]
    fn next_token_regex_prefix_checked_before_word_fallback() {
        let mut l = Lexer::new("regex:foo");
        assert_eq!(l.next_token(), Token::RegexPrefix);
        assert_eq!(l.pos, 6);
        assert_eq!(l.next_token(), Token::Word("foo".to_string()));
    }

    #[test]
    fn next_token_word_fallback_for_non_special_non_regex_input() {
        let mut l = Lexer::new("hello");
        assert_eq!(l.next_token(), Token::Word("hello".to_string()));
    }

    // Without the colon in the check, "regexp" would be consumed whole as a six-byte `RegexPrefix`.
    #[test]
    fn regex_prefix_requires_the_trailing_colon() {
        let mut l = Lexer::new("regexp");
        assert_eq!(l.next_token(), Token::Word("regexp".to_string()));
        assert_eq!(l.next_token(), Token::Eof);

        let mut l2 = Lexer::new("regex foo");
        assert_eq!(l2.next_token(), Token::Word("regex".to_string()));
        assert_eq!(l2.next_token(), Token::Word("foo".to_string()));
    }

    #[test]
    fn next_token_regex_prefix_check_is_case_sensitive_uppercase() {
        let mut l = Lexer::new("REGEX:foo");
        assert_eq!(l.next_token(), Token::Word("REGEX:foo".to_string()));
    }

    #[test]
    fn next_token_regex_prefix_check_is_case_sensitive_mixed_case() {
        let mut l = Lexer::new("Regex:foo");
        assert_eq!(l.next_token(), Token::Word("Regex:foo".to_string()));
    }

    #[test]
    fn next_token_skips_leading_ascii_whitespace_unconditionally() {
        let mut l = Lexer::new("   \t\n  +foo");
        assert_eq!(l.next_token(), Token::Plus);
        assert_eq!(l.next_token(), Token::Word("foo".to_string()));
    }

    #[test]
    fn word_does_not_split_on_internal_plus() {
        let mut l = Lexer::new("foo+bar");
        assert_eq!(l.next_token(), Token::Word("foo+bar".to_string()));
        assert_eq!(l.next_token(), Token::Eof);
    }

    #[test]
    fn word_does_not_split_on_internal_minus_quote_or_regex_prefix() {
        let mut l = Lexer::new("a-b\"cregex:d");
        assert_eq!(l.next_token(), Token::Word("a-b\"cregex:d".to_string()));
    }

    #[test]
    fn word_stops_at_whitespace() {
        let mut l = Lexer::new("foo bar");
        assert_eq!(l.next_token(), Token::Word("foo".to_string()));
        assert_eq!(l.next_token(), Token::Word("bar".to_string()));
    }

    // Distinguishes `is_ascii_whitespace` from a narrower predicate such as space-only.
    #[test]
    fn word_stops_at_every_ascii_whitespace_kind_not_just_space() {
        let mut l = Lexer::new("foo\tbar\nbaz\rqux");
        assert_eq!(l.next_token(), Token::Word("foo".to_string()));
        assert_eq!(l.next_token(), Token::Word("bar".to_string()));
        assert_eq!(l.next_token(), Token::Word("baz".to_string()));
        assert_eq!(l.next_token(), Token::Word("qux".to_string()));
        assert_eq!(l.next_token(), Token::Eof);
    }

    // Catches a hand-rolled whitespace predicate that forgets form feed.
    #[test]
    fn word_stops_at_form_feed() {
        let mut l = Lexer::new("foo\x0Cbar");
        assert_eq!(l.next_token(), Token::Word("foo".to_string()));
        assert_eq!(l.next_token(), Token::Word("bar".to_string()));
    }

    #[test]
    fn quoted_basic_no_backslash() {
        let mut l = Lexer::new("\"foo\"");
        assert_eq!(l.next_token(), Token::QuotedString("foo".to_string()));
    }

    #[test]
    fn quoted_escaped_quote_appended_literally_does_not_terminate() {
        let mut l = Lexer::new("\"a\\\"b\"");
        let tok = l.next_token();
        assert_eq!(tok, Token::QuotedString("a\"b".to_string()));
        if let Token::QuotedString(s) = tok {
            assert_eq!(s.chars().count(), 3);
        }
    }

    #[test]
    fn quoted_double_backslash_neither_is_an_escape() {
        let mut l = Lexer::new("\"a\\\\b\"");
        let tok = l.next_token();
        assert_eq!(tok, Token::QuotedString("a\\\\b".to_string()));
        if let Token::QuotedString(s) = tok {
            assert_eq!(s.chars().count(), 4);
            assert_eq!(s.matches('\\').count(), 2);
        }
    }

    #[test]
    fn quoted_unterminated_with_trailing_backslash_tolerated_silently() {
        let mut l = Lexer::new("\"a\\");
        let tok = l.next_token();
        assert_eq!(tok, Token::QuotedString("a\\".to_string()));
        if let Token::QuotedString(s) = tok {
            assert_eq!(s.chars().count(), 2);
        }
    }

    #[test]
    fn quoted_unterminated_no_backslash_returns_accumulated_chars_no_error() {
        let mut l = Lexer::new("\"abc");
        assert_eq!(l.next_token(), Token::QuotedString("abc".to_string()));
    }

    #[test]
    fn word_consumes_multibyte_utf8_char_as_one_char_no_panic() {
        let mut l = Lexer::new("+café");
        assert_eq!(l.next_token(), Token::Plus);
        assert_eq!(l.next_token(), Token::Word("café".to_string()));
    }

    #[test]
    fn quoted_consumes_multibyte_utf8_chars_no_panic() {
        let mut l = Lexer::new("\"日本語\"");
        assert_eq!(l.next_token(), Token::QuotedString("日本語".to_string()));
    }

    // Catches a mutant that truncates or drops whitespace inside quotes; the other quoted tests have none.
    #[test]
    fn quoted_string_contains_ascii_whitespace_verbatim() {
        let mut l = Lexer::new("\"a b\tc\"");
        assert_eq!(l.next_token(), Token::QuotedString("a b\tc".to_string()));
        assert_eq!(l.next_token(), Token::Eof);
    }
}
