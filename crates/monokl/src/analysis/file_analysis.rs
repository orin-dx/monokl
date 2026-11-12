//! Everything extracted from one source file.

use std::fmt;

use crate::types::CodeBlock;

/// Why a file was analyzed without its content: the analyzer gave up on it instead of reading it.
///
/// A file with syntax errors is not degraded this way; its recovered content is kept and only
/// [`FileAnalysis::had_parse_errors`] is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Degradation {
    /// The source is longer than the analyzer will parse.
    TooLarge {
        /// The length of the source.
        bytes: usize,
        /// The longest source the analyzer parses.
        cap: usize,
    },
    /// The parse or extraction panicked.
    Panicked,
    /// The analyzer could not start the thread the parse runs on.
    ThreadUnavailable,
    /// The source nests deeper than the analyzer parses. A scan before the parse found it, so the parser never ran.
    TooDeep {
        /// How deep the scan estimates the source nests, in levels of brackets or the equivalent in other nesting.
        depth: usize,
        /// The deepest nesting the analyzer parses.
        cap: usize,
    },
}

impl fmt::Display for Degradation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, cap } => {
                write!(f, "file too large to parse ({bytes} bytes, over the cap of {cap})")
            }
            Self::Panicked => f.write_str("parser panicked"),
            Self::ThreadUnavailable => f.write_str("could not start a parse thread"),
            Self::TooDeep { depth, cap } => {
                write!(f, "file nests too deeply to analyze (depth {depth}, over the cap of {cap})")
            }
        }
    }
}

/// The result of analyzing one file.
///
/// It owns all its data, so it can outlive the parse that produced it.
#[derive(Clone, Debug)]
pub struct FileAnalysis {
    /// Set for any file that did not parse cleanly, including every file with a [`Degradation`].
    pub had_parse_errors: bool,
    /// Why the file's content is missing, when the analyzer gave up on it.
    pub degradation: Option<Degradation>,
    pub blocks: Vec<CodeBlock>,
    /// Blocks of test-only code (Rust `#[cfg(test)]` and `#[test]` items), kept apart from `blocks` so a search can leave
    /// them out. Empty for a language with no such code.
    pub test_blocks: Vec<CodeBlock>,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(Degradation::TooLarge { bytes: 300_000, cap: 260_096 }, "file too large to parse (300000 bytes, over the cap of 260096)")]
    #[case(Degradation::Panicked, "parser panicked")]
    #[case(Degradation::ThreadUnavailable, "could not start a parse thread")]
    #[case(Degradation::TooDeep { depth: 9_000, cap: 8_000 }, "file nests too deeply to analyze (depth 9000, over the cap of 8000)")]
    fn a_degradation_names_its_reason(#[case] degradation: Degradation, #[case] text: &str) {
        assert_eq!(degradation.to_string(), text);
    }
}
