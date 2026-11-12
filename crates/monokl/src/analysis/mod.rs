//! What a language analyzer is and what it produces.

pub mod file_analysis;
pub mod lang;
#[cfg(any(feature = "lang-rust", feature = "lang-ts"))]
mod nesting;
#[cfg(any(feature = "lang-rust", feature = "lang-ts"))]
mod parse_stack;
pub mod registry;
#[cfg(feature = "lang-rust")]
pub mod rust;
#[cfg(all(test, feature = "lang-rust", feature = "lang-ts"))]
mod soundness_tests;
#[cfg(feature = "lang-ts")]
pub mod ts;

pub use file_analysis::{Degradation, FileAnalysis};
pub use lang::{Language, LanguageAnalyzer};
