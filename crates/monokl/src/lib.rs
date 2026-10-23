// Tests panic on failure by design; the workspace's unwrap/expect/panic denials target production code.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// Modules are declared as they are implemented, not ahead of time.
pub mod error;
pub mod file_io;
#[cfg(feature = "cli")]
pub mod output;
/// `Lexer` and `Token` are internal to `query::lexer`. These doctests compile as an external crate would, so
/// they prove the types are unreachable through the public path.
///
/// ```compile_fail,E0432
/// use monokl::query::Token;
/// ```
///
/// ```compile_fail,E0432
/// use monokl::query::Lexer;
/// ```
pub mod query;
pub mod types;

#[cfg(test)]
mod tests_types;
