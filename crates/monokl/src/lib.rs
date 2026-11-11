// Tests panic on failure by design; the workspace's unwrap/expect/panic denials target production code.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// Modules are declared as they are implemented, not ahead of time.
pub mod analysis;
#[cfg(feature = "cli")]
pub mod budget;
#[cfg(feature = "cli")]
pub mod cli;
pub mod dedup;
pub mod digest;
pub mod error;
pub mod file_io;
#[cfg(feature = "cli")]
pub mod output;
pub mod pipeline;
pub(crate) mod query;
pub mod rank;
pub mod text_search;
pub mod tokens;
pub mod types;
