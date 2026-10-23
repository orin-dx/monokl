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

    fn assert_token_derives<T: std::fmt::Debug + Clone + PartialEq>() {}

    #[test]
    fn token_derive_bound_and_variant_shapes() {
        assert_token_derives::<Token>();
        let variants = [
            Token::Plus,
            Token::Minus,
            Token::RegexPrefix,
            Token::Eof,
            Token::QuotedString(String::new()),
            Token::Word(String::new()),
        ];
        assert_eq!(variants.len(), 6);
        assert_eq!(Token::Plus, Token::Plus);
        assert_ne!(Token::Plus, Token::Minus);
    }

    // A generic bound can only prove a trait is implemented. These probes also prove absence: the inherent `IS`
    // shadows the trait default only when the bound holds. Each test pairs them with `usize` as a positive control.
    struct EqProbe<T>(std::marker::PhantomData<T>);
    trait NotEq {
        const IS: bool = false;
    }
    impl<T> NotEq for EqProbe<T> {}
    impl<T: Eq> EqProbe<T> {
        const IS: bool = true;
    }

    struct HashProbe<T>(std::marker::PhantomData<T>);
    trait NotHash {
        const IS: bool = false;
    }
    impl<T> NotHash for HashProbe<T> {}
    impl<T: std::hash::Hash> HashProbe<T> {
        const IS: bool = true;
    }

    struct PartialOrdProbe<T>(std::marker::PhantomData<T>);
    trait NotPartialOrd {
        const IS: bool = false;
    }
    impl<T> NotPartialOrd for PartialOrdProbe<T> {}
    impl<T: PartialOrd> PartialOrdProbe<T> {
        const IS: bool = true;
    }

    struct OrdProbe<T>(std::marker::PhantomData<T>);
    trait NotOrd {
        const IS: bool = false;
    }
    impl<T> NotOrd for OrdProbe<T> {}
    impl<T: Ord> OrdProbe<T> {
        const IS: bool = true;
    }

    #[test]
    fn token_has_no_eq_hash_partialord_or_ord() {
        const {
            assert!(!EqProbe::<Token>::IS);
            assert!(!HashProbe::<Token>::IS);
            assert!(!PartialOrdProbe::<Token>::IS);
            assert!(!OrdProbe::<Token>::IS);
            assert!(EqProbe::<usize>::IS);
            assert!(HashProbe::<usize>::IS);
            assert!(PartialOrdProbe::<usize>::IS);
            assert!(OrdProbe::<usize>::IS);
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
    fn token_has_no_serde_derive() {
        const {
            assert!(!SerProbe::<Token>::IS);
            assert!(!DeProbe::<Token>::IS);
            assert!(SerProbe::<usize>::IS);
            assert!(DeProbe::<usize>::IS);
        }
    }

    // Reads the source because a payload-carrying enum has no discriminant cast to observe declaration order.
    #[test]
    fn token_variant_declaration_order_structural() {
        let src = include_str!("lexer.rs");
        let decl_start = src.find("pub enum Token").unwrap_or_else(|| panic!("`pub enum Token` not found in lexer.rs"));
        let open = src[decl_start..].find('{').unwrap() + decl_start;
        let close = src[open..].find('}').unwrap() + open;
        let block = &src[open..=close];

        let variants = ["Plus", "Minus", "RegexPrefix", "QuotedString", "Word", "Eof"];
        let mut last_pos: Option<usize> = None;
        for v in variants {
            let pos = block.find(v).unwrap_or_else(|| panic!("variant `{v}` not found in Token enum body"));
            if let Some(last) = last_pos {
                assert!(pos > last, "variant `{v}` found out of the expected declaration order");
            }
            last_pos = Some(pos);
        }
    }

    // The structural derive check below only scans back to the previous blank line; these probes catch a derive
    // placed anywhere.
    struct LexerDebugProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerDebug {
        const IS: bool = false;
    }
    impl<T> NotLexerDebug for LexerDebugProbe<T> {}
    impl<T: std::fmt::Debug> LexerDebugProbe<T> {
        const IS: bool = true;
    }

    struct LexerCloneProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerClone {
        const IS: bool = false;
    }
    impl<T> NotLexerClone for LexerCloneProbe<T> {}
    impl<T: Clone> LexerCloneProbe<T> {
        const IS: bool = true;
    }

    struct LexerPartialEqProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerPartialEq {
        const IS: bool = false;
    }
    impl<T> NotLexerPartialEq for LexerPartialEqProbe<T> {}
    impl<T: PartialEq> LexerPartialEqProbe<T> {
        const IS: bool = true;
    }

    struct LexerEqProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerEq {
        const IS: bool = false;
    }
    impl<T> NotLexerEq for LexerEqProbe<T> {}
    impl<T: Eq> LexerEqProbe<T> {
        const IS: bool = true;
    }

    struct LexerHashProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerHash {
        const IS: bool = false;
    }
    impl<T> NotLexerHash for LexerHashProbe<T> {}
    impl<T: std::hash::Hash> LexerHashProbe<T> {
        const IS: bool = true;
    }

    struct LexerPartialOrdProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerPartialOrd {
        const IS: bool = false;
    }
    impl<T> NotLexerPartialOrd for LexerPartialOrdProbe<T> {}
    impl<T: PartialOrd> LexerPartialOrdProbe<T> {
        const IS: bool = true;
    }

    struct LexerOrdProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerOrd {
        const IS: bool = false;
    }
    impl<T> NotLexerOrd for LexerOrdProbe<T> {}
    impl<T: Ord> LexerOrdProbe<T> {
        const IS: bool = true;
    }

    struct LexerDefaultProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerDefault {
        const IS: bool = false;
    }
    impl<T> NotLexerDefault for LexerDefaultProbe<T> {}
    impl<T: Default> LexerDefaultProbe<T> {
        const IS: bool = true;
    }

    struct LexerCopyProbe<T>(std::marker::PhantomData<T>);
    trait NotLexerCopy {
        const IS: bool = false;
    }
    impl<T> NotLexerCopy for LexerCopyProbe<T> {}
    impl<T: Copy> LexerCopyProbe<T> {
        const IS: bool = true;
    }

    #[test]
    fn lexer_implements_none_of_the_common_derivable_traits() {
        const {
            assert!(!LexerDebugProbe::<Lexer<'static>>::IS);
            assert!(!LexerCloneProbe::<Lexer<'static>>::IS);
            assert!(!LexerPartialEqProbe::<Lexer<'static>>::IS);
            assert!(!LexerEqProbe::<Lexer<'static>>::IS);
            assert!(!LexerHashProbe::<Lexer<'static>>::IS);
            assert!(!LexerPartialOrdProbe::<Lexer<'static>>::IS);
            assert!(!LexerOrdProbe::<Lexer<'static>>::IS);
            assert!(!LexerDefaultProbe::<Lexer<'static>>::IS);
            assert!(!LexerCopyProbe::<Lexer<'static>>::IS);
            assert!(LexerDebugProbe::<usize>::IS);
            assert!(LexerCloneProbe::<usize>::IS);
            assert!(LexerPartialEqProbe::<usize>::IS);
            assert!(LexerEqProbe::<usize>::IS);
            assert!(LexerHashProbe::<usize>::IS);
            assert!(LexerPartialOrdProbe::<usize>::IS);
            assert!(LexerOrdProbe::<usize>::IS);
            assert!(LexerDefaultProbe::<usize>::IS);
            assert!(LexerCopyProbe::<usize>::IS);
        }
    }

    #[test]
    fn lexer_struct_has_no_derive_and_private_fields_structural() {
        // `include_str!` includes this test too, so the searches are scoped to the struct's declaration and the
        // field needles are built at runtime rather than spelled out here.
        let src = include_str!("lexer.rs");
        let decl_start =
            src.find("pub struct Lexer<'a>").unwrap_or_else(|| panic!("`pub struct Lexer<'a>` not found in lexer.rs"));
        let preceding = &src[..decl_start];
        let window_start = preceding.rfind("\n\n").map_or(0, |p| p + 2);
        let window = &preceding[window_start..decl_start];
        assert!(
            !window.contains("#[derive("),
            "found unexpected #[derive(...)] immediately before `pub struct Lexer`: {window:?}"
        );

        let open = src[decl_start..].find('{').unwrap() + decl_start;
        let close = src[open..].find('}').unwrap() + open;
        let field_block = &src[open..=close];
        for field in ["input", "pos"] {
            // A prefix check on the whole line also catches `pub(crate)` and `pub(super)`.
            let needle = format!("{field}:");
            let idx =
                field_block.find(&needle).unwrap_or_else(|| panic!("field `{field}` not found in Lexer struct body"));
            let line_start = field_block[..idx].rfind('\n').map_or(0, |p| p + 1);
            let line_end = field_block[idx..].find('\n').map_or(field_block.len(), |p| idx + p);
            let line = field_block[line_start..line_end].trim_start();
            assert!(
                line.starts_with(&needle),
                "Lexer.{field} must be private, found line {line:?} not starting with `{needle}` -- a pub/pub(crate)/pub(super) modifier would prefix it"
            );
        }
    }

    // Reads the source because no compile-time check can prove a method is uncallable from outside the module.
    #[test]
    fn lexer_helper_methods_are_private_structural() {
        let src = include_str!("lexer.rs");
        for name in ["remaining", "peek", "advance", "skip_whitespace", "read_word", "read_quoted"] {
            let needle = format!("fn {name}");
            let idx = src.find(&needle).unwrap_or_else(|| panic!("helper method `{name}` not found in lexer.rs"));
            let line_start = src[..idx].rfind('\n').map_or(0, |p| p + 1);
            let line_end = src[idx..].find('\n').map_or(src.len(), |p| idx + p);
            let line = src[line_start..line_end].trim_start();
            assert!(
                line.starts_with("fn "),
                "helper method `{name}` is not declared with a bare `fn ` prefix (found line {line:?}); it must be private, not pub/pub(crate)/pub(super)"
            );
        }
        assert!(src.contains("pub fn new"));
        assert!(src.contains("pub fn next_token"));
    }

    // The per-name check above misses a newly added public method, a second impl block, and `pub(crate)`. This scans
    // every `impl ... Lexer ... {` block, whatever its lifetime spelling, brace-matching each one, and requires the
    // `pub`-prefixed methods across all of them to be exactly `new` and `next_token`.
    #[test]
    fn lexer_exactly_two_public_methods_structural() {
        let src = include_str!("lexer.rs");

        let mut pub_fn_lines: Vec<String> = Vec::new();
        let mut occurrences = 0;
        let mut byte_offset = 0usize;
        for line in src.split_inclusive('\n') {
            let trimmed = line.trim();
            let is_lexer_impl_decl = trimmed.starts_with("impl") && trimmed.contains("Lexer") && trimmed.ends_with('{');
            let line_len = line.len();
            if !is_lexer_impl_decl {
                byte_offset += line_len;
                continue;
            }
            occurrences += 1;
            let brace_rel = line
                .rfind('{')
                .unwrap_or_else(|| panic!("line matched as an impl-block decl but has no `{{`: {line:?}"));
            let open = byte_offset + brace_rel;
            byte_offset += line_len;
            debug_assert_eq!(&src[open..=open], "{");

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
            let close =
                close.unwrap_or_else(|| panic!("no matching close brace found for impl block starting at byte {open}"));
            let block = &src[open..=close];

            for l in block.lines() {
                let trimmed = l.trim_start();
                if trimmed.starts_with("pub") && trimmed.contains("fn ") {
                    let fn_pos = trimmed.find("fn ").unwrap_or_else(|| {
                        panic!("`contains(\"fn \")` true but `find(\"fn \")` failed for line {trimmed:?}")
                    });
                    let paren_rel = trimmed[fn_pos..].find('(').unwrap_or(trimmed.len() - fn_pos);
                    pub_fn_lines.push(trimmed[..fn_pos + paren_rel].trim_end().to_string());
                }
            }
        }
        assert!(occurrences > 0, "no `impl ... Lexer ... {{` block found in lexer.rs");

        assert_eq!(pub_fn_lines, vec!["pub fn new".to_string(), "pub fn next_token".to_string()]);
    }

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
