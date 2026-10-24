//! What a language analyzer is and what it produces.

pub mod file_analysis;
pub mod lang;

pub use file_analysis::FileAnalysis;
pub use lang::{AnalysisProfile, AnalyzerCapabilities, CapabilityPrecision, LanguageAnalyzer, LanguageId};
