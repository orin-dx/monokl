//! The crate-wide [`MonoklError`] type and its [`Result`] alias.
//!
//! Every fallible `pub` and `pub(crate)` function returns [`Result`], except the output layer
//! (`output::agent::render_projection`), which returns michi's `DomainError`.
use camino::Utf8PathBuf;

use crate::file_io::FileIoError;

/// Errors produced by monokl.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum MonoklError {
    /// A file read failed.
    #[error(transparent)]
    Io(#[from] FileIoError),

    /// A directory walk failed.
    ///
    /// `path` is the root of the walk in progress, not the entry that failed.
    #[error("walker error at {path}")]
    Walk {
        path: Utf8PathBuf,
        #[source]
        source: ignore::Error,
    },

    /// A search pattern failed to compile.
    #[error("regex compilation failed")]
    RegexBuild(#[from] grep_regex::Error),

    /// A path was not valid UTF-8.
    ///
    /// `path` is a [`std::path::PathBuf`] rather than a [`Utf8PathBuf`] because it cannot be represented as UTF-8.
    #[error("non-UTF-8 path encountered: {path:?}")]
    NonUtf8Path { path: std::path::PathBuf },

    /// A query had more terms than the limit allows.
    #[error("query has too many terms: {count} > {limit}")]
    TooManyTerms { count: usize, limit: usize },
}

/// The crate-wide result type.
///
/// Errors propagate to the caller.
pub type Result<T> = std::result::Result<T, MonoklError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn real_ignore_error() -> ignore::Error {
        ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err()
    }

    fn real_grep_regex_error() -> grep_regex::Error {
        grep_regex::RegexMatcherBuilder::new().build("(").unwrap_err()
    }

    #[test]
    fn io_display_is_transparent() {
        let err: MonoklError = FileIoError::read("/x", std::io::Error::other("boom")).into();
        assert_eq!(err.to_string(), "failed to read /x");
    }

    #[test]
    fn walk_display() {
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: real_ignore_error() };
        assert_eq!(err.to_string(), "walker error at /x");
    }

    #[test]
    fn regex_build_display() {
        let err: MonoklError = real_grep_regex_error().into();
        assert_eq!(err.to_string(), "regex compilation failed");
    }

    #[test]
    fn non_utf8_path_display() {
        let err = MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/bad") };
        assert_eq!(err.to_string(), "non-UTF-8 path encountered: \"/bad\"");
    }

    #[test]
    fn too_many_terms_display() {
        let err = MonoklError::TooManyTerms { count: 12, limit: 8 };
        assert_eq!(err.to_string(), "query has too many terms: 12 > 8");
    }

    #[cfg(feature = "miette")]
    #[test]
    fn variants_have_no_diagnostic_metadata() {
        use miette::Diagnostic;
        let variants: Vec<MonoklError> = vec![
            FileIoError::read("/x", std::io::Error::other("boom")).into(),
            MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: real_ignore_error() },
            real_grep_regex_error().into(),
            MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/bad") },
            MonoklError::TooManyTerms { count: 12, limit: 8 },
        ];
        assert_eq!(variants.len(), 5);
        for v in &variants {
            assert!(v.code().is_none());
            assert!(v.help().is_none());
            assert!(v.url().is_none());
        }
    }
}
