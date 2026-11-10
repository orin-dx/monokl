//! What a language analyzer is and what it produces.

pub mod file_analysis;
pub mod lang;
pub mod registry;
#[cfg(feature = "lang-rust")]
pub mod rust;
#[cfg(feature = "lang-ts")]
pub mod ts;

pub use file_analysis::{Degradation, FileAnalysis};
pub use lang::{AnalysisProfile, AnalyzerCapabilities, CapabilityPrecision, LanguageAnalyzer, LanguageId};
