//! Identifier tokenizing for BM25: splits camel, Pascal, snake and kebab case into lowercase sub-words.

/// Tokens too common in prose or code to say anything about relevance.
///
/// `use` appears twice; the duplicate is harmless and kept so the list reads as the two groups it came from.
#[rustfmt::skip]
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one", "our", "out", "day",
    "get", "has", "him", "his", "how", "its", "may", "new", "now", "old", "see", "two", "use", "way", "who", "did",
    "let", "put", "too", "any",
    // Common code keywords.
    "fn", "var", "const", "mut", "pub", "use", "mod", "impl", "type", "enum", "struct", "trait", "return", "import",
    "export", "from", "default", "class", "extends", "interface",
    // Near-universal Rust tokens.
    "crate", "self", "super", "dyn", "async", "await", "unsafe",
];

/// Splits one identifier into lowercase sub-words, dropping stop words and tokens shorter than 2 characters.
///
/// A new word starts at a lower-to-upper transition, at the last capital of an acronym followed by lowercase
/// (`XMLParser` gives `xml`, `parser`), and at a digit-to-letter transition. A letter-to-digit transition does not
/// split, so `v8` stays whole. Every character other than an ASCII letter or digit is a separator.
///
/// # Examples
///
/// ```
/// use monokl::tokens::tokenize;
///
/// assert_eq!(tokenize("XMLParser"), ["xml", "parser"]);
/// assert_eq!(tokenize("getUserName"), ["user", "name"]);
/// ```
pub fn tokenize(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (idx, &ch) in chars.iter().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            push_word(&mut words, &mut word);
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|prev| chars.get(prev)).copied();
        let next = chars.get(idx + 1).copied();
        if prev.is_some_and(|prev| starts_word(prev, ch, next)) {
            push_word(&mut words, &mut word);
        }
        word.push(ch.to_ascii_lowercase());
    }
    push_word(&mut words, &mut word);
    words.retain(|word| word.len() >= 2 && !STOP_WORDS.contains(&word.as_str()));
    words
}

/// Returns whether `ch`, preceded by `prev` and followed by `next`, begins a new sub-word.
fn starts_word(prev: char, ch: char, next: Option<char>) -> bool {
    (prev.is_ascii_lowercase() && ch.is_ascii_uppercase())
        || (prev.is_ascii_uppercase() && ch.is_ascii_uppercase() && next.is_some_and(|next| next.is_ascii_lowercase()))
        || (prev.is_ascii_digit() && ch.is_ascii_alphabetic())
}

fn push_word(words: &mut Vec<String>, word: &mut String) {
    if !word.is_empty() {
        words.push(std::mem::take(word));
    }
}

/// Splits source text on every character other than `[A-Za-z0-9_]` and tokenizes each fragment with [`tokenize`].
///
/// Duplicates are kept in source order, since BM25 needs term frequency.
///
/// # Examples
///
/// ```
/// use monokl::tokens::tokenize_source;
///
/// assert_eq!(tokenize_source("fooBar(fooBar)"), ["foo", "bar", "foo", "bar"]);
/// ```
pub fn tokenize_source(source: &str) -> Vec<String> {
    source
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|fragment| !fragment.is_empty())
        .flat_map(tokenize)
        .collect()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("XMLParser", &["xml", "parser"])]
    #[case("getUserName", &["user", "name"])]
    #[case("v8Engine", &["v8", "engine"])]
    #[case("HTTP2Server", &["http2", "server"])]
    #[case("user_id", &["user", "id"])]
    #[case("user-id", &["user", "id"])]
    #[case("THE", &[])]
    #[case("a", &[])]
    #[case("", &[])]
    #[case("parseHTTPResponse", &["parse", "http", "response"])]
    #[case("HTMLElement", &["html", "element"])]
    #[case("ALLCAPS", &["allcaps"])]
    #[case("UserName", &["user", "name"])]
    #[case("base64Encode", &["base64", "encode"])]
    #[case("utf8", &["utf8"])]
    #[case("http2Server", &["http2", "server"])]
    #[case("item42Count", &["item42", "count"])]
    #[case("kebab-case-name", &["kebab", "case", "name"])]
    #[case("SCREAMING_SNAKE", &["screaming", "snake"])]
    #[case("__dunder__", &["dunder"])]
    fn tokenize_splits_on_case_digit_and_separator_transitions(#[case] input: &str, #[case] expected: &[&str]) {
        assert_eq!(tokenize(input), expected);
    }

    #[test]
    fn tokenize_drops_tokens_shorter_than_two_characters() {
        assert_eq!(tokenize("aB"), Vec::<String>::new());
        assert_eq!(tokenize("x_yz"), ["yz"]);
        assert_eq!(tokenize("a1"), ["a1"]);
    }

    #[test]
    fn stop_words_are_the_documented_set() {
        // Written out independently of STOP_WORDS, so changing the list fails here.
        let mut expected = [
            "the",
            "and",
            "for",
            "are",
            "but",
            "not",
            "you",
            "all",
            "can",
            "had",
            "her",
            "was",
            "one",
            "our",
            "out",
            "day",
            "get",
            "has",
            "him",
            "his",
            "how",
            "its",
            "may",
            "new",
            "now",
            "old",
            "see",
            "two",
            "use",
            "way",
            "who",
            "did",
            "let",
            "put",
            "too",
            "any",
            "fn",
            "var",
            "const",
            "mut",
            "pub",
            "mod",
            "impl",
            "type",
            "enum",
            "struct",
            "trait",
            "return",
            "import",
            "export",
            "from",
            "default",
            "class",
            "extends",
            "interface",
            "crate",
            "self",
            "super",
            "dyn",
            "async",
            "await",
            "unsafe",
        ];
        expected.sort_unstable();
        let mut words = STOP_WORDS.to_vec();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words, expected);
    }

    #[test]
    fn tokenize_keeps_a_two_character_non_stop_word() {
        assert_eq!(tokenize("id"), ["id"]);
    }

    #[test]
    fn tokenize_drops_every_stop_word() {
        for word in STOP_WORDS {
            assert_eq!(tokenize(word), Vec::<String>::new(), "kept stop word {word:?}");
        }
    }

    #[rstest]
    #[case("crate")]
    #[case("self")]
    #[case("super")]
    #[case("dyn")]
    #[case("async")]
    #[case("await")]
    #[case("unsafe")]
    fn tokenize_drops_rust_keyword_stop_words(#[case] word: &str) {
        assert_eq!(tokenize(&format!("{word}_value")), ["value"]);
    }

    #[test]
    fn tokenize_matches_stop_words_after_lowercasing() {
        assert_eq!(tokenize("GetTheValue"), ["value"]);
    }

    #[rstest]
    #[case("fooébar", &["foo", "bar"])]
    #[case("naïveCount", &["na", "ve", "count"])]
    #[case("naïveCase", &["na", "ve", "case"])]
    #[case("日本lookup", &["lookup"])]
    #[case("ÀBc", &["bc"])]
    fn tokenize_treats_non_ascii_as_a_separator(#[case] input: &str, #[case] expected: &[&str]) {
        assert_eq!(tokenize(input), expected);
    }

    #[test]
    fn tokenize_source_keeps_duplicates_in_source_order() {
        assert_eq!(tokenize_source("fooBar(fooBar)"), ["foo", "bar", "foo", "bar"]);
    }

    #[rstest]
    #[case("", &[])]
    #[case("   ", &[])]
    #[case("user_id", &["user", "id"])]
    #[case("user_id = user_id", &["user", "id", "user", "id"])]
    #[case("let count = getCount();", &["count", "count"])]
    #[case("a.b.cd", &["cd"])]
    #[case("parse-json", &["parse", "json"])]
    #[case("résuméWriter", &["sum", "writer"])]
    #[case("trailingWord", &["trailing", "word"])]
    fn tokenize_source_splits_on_non_identifier_characters(#[case] input: &str, #[case] expected: &[&str]) {
        assert_eq!(tokenize_source(input), expected);
    }
}

#[cfg(test)]
mod proptests;
