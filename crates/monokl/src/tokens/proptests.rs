use proptest::prelude::*;

use super::*;

fn assert_well_formed(tokens: &[String]) -> Result<(), TestCaseError> {
    for token in tokens {
        prop_assert!(token.len() >= 2, "short token {token:?}");
        prop_assert!(
            token.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
            "token {token:?} is not lowercase ASCII alphanumeric"
        );
        prop_assert!(!STOP_WORDS.contains(&token.as_str()), "stop word {token:?}");
    }
    let joined = tokens.concat();
    prop_assert!(joined.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()), "separator in {joined:?}");
    Ok(())
}

proptest! {
    #[test]
    fn tokenize_yields_only_lowercase_alphanumeric_non_stop_words(input in "\\PC{0,40}") {
        assert_well_formed(&tokenize(&input))?;
    }

    #[test]
    fn tokenize_source_yields_only_lowercase_alphanumeric_non_stop_words(input in "[ -~\\n\u{e9}\u{65e5}]{0,60}") {
        assert_well_formed(&tokenize_source(&input))?;
    }
}
