use proptest::prelude::*;

use super::*;

/// A term as the user would write it: the text before escaping, so that a literal's `pattern` is its escape.
#[derive(Debug, Clone)]
enum Written {
    Literal(Modifier, String),
    Regex(String),
}

// No backslash: `\"` is the only escape, so a backslash next to a quote has no faithful rendering.
fn text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 .*+?()\\[\\]{}|^$:\"-]{0,12}"
}

fn written() -> impl Strategy<Value = Written> {
    prop_oneof![
        (prop_oneof![Just(Modifier::Required), Just(Modifier::Excluded), Just(Modifier::Optional)], text())
            .prop_map(|(modifier, text)| Written::Literal(modifier, text)),
        // A modifier before `regex:` is dropped by the parser, so a regex term is always optional.
        text().prop_map(Written::Regex),
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
            Written::Regex(text) => format!("regex:{}", quote(text)),
        })
        .collect();
    rendered.join(" ")
}

fn expected(terms: &[Written]) -> Vec<(Modifier, String, bool)> {
    terms
        .iter()
        .map(|term| match term {
            Written::Literal(modifier, text) => (modifier.clone(), regex::escape(text), false),
            Written::Regex(text) => (Modifier::Optional, text.clone(), true),
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
