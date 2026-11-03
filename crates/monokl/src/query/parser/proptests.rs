use proptest::prelude::*;

use super::*;

/// A term as the user would write it: the text before escaping, so that a literal's `pattern` is its escape.
#[derive(Debug, Clone)]
enum Written {
    Literal(Modifier, String),
    Regex(Modifier, String),
}

// No backslash: `\"` is the only escape, so a backslash next to a quote has no faithful rendering.
fn text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 .*+?()\\[\\]{}|^$:\"-]{0,12}"
}

fn modifier() -> impl Strategy<Value = Modifier> {
    prop_oneof![Just(Modifier::Required), Just(Modifier::Excluded), Just(Modifier::Optional)]
}

fn written() -> impl Strategy<Value = Written> {
    prop_oneof![
        (modifier(), text()).prop_map(|(modifier, text)| Written::Literal(modifier, text)),
        (modifier(), text()).prop_map(|(modifier, text)| Written::Regex(modifier, text)),
    ]
}

fn render(terms: &[Written]) -> String {
    let quote = |text: &str| format!("\"{}\"", text.replace('"', "\\\""));
    let rendered: Vec<String> = terms
        .iter()
        .map(|term| match term {
            Written::Literal(Modifier::Required, text) => format!("+{}", quote(text)),
            Written::Literal(Modifier::Excluded, text) => format!("-{}", quote(text)),
            Written::Literal(Modifier::Optional, text) => quote(text),
            Written::Regex(Modifier::Required, text) => format!("+regex:{}", quote(text)),
            Written::Regex(Modifier::Excluded, text) => format!("-regex:{}", quote(text)),
            Written::Regex(Modifier::Optional, text) => format!("regex:{}", quote(text)),
        })
        .collect();
    rendered.join(" ")
}

fn expected(terms: &[Written]) -> Vec<(Modifier, String, bool)> {
    terms
        .iter()
        .filter_map(|term| match term {
            // An empty phrase yields no term.
            Written::Literal(_, text) if text.is_empty() => None,
            Written::Literal(modifier, text) => Some((modifier.clone(), regex::escape(text), false)),
            Written::Regex(modifier, text) => Some((modifier.clone(), text.clone(), true)),
        })
        .collect()
}

proptest! {
    #[test]
    fn parse_never_panics_on_arbitrary_text(input in "\\PC{0,80}") {
        drop(parse(&input));
    }

    #[test]
    fn parse_never_panics_on_query_shaped_text(input in "(\\+|-|regex:|\"|\\\\|[a-z.]| )*") {
        drop(parse(&input));
    }

    #[test]
    fn parse_of_rendered_terms_recovers_the_terms(terms in prop::collection::vec(written(), 0..=64)) {
        let parsed = parse(&render(&terms)).expect("at most 64 terms");
        let parsed: Vec<_> = parsed.into_iter().map(|t| (t.modifier, t.pattern, t.is_regex)).collect();
        prop_assert_eq!(parsed, expected(&terms));
    }
}
