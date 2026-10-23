//! The crate-wide [`MonoklError`] type and its [`Result`] alias.
//!
//! Every fallible `pub` and `pub(crate)` function returns [`Result`]. The one exception is
//! `git_scope::blob_at_ref_in_repo`, whose bare `String` error is mapped into [`MonoklError::Git`] at its only call
//! site.
use camino::Utf8PathBuf;

use crate::file_io::FileIoError;

/// Errors produced by monokl.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum MonoklError {
    /// A file read or write failed.
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

    /// The tokenizer failed to initialize.
    #[error("tokenizer init failed")]
    TokenizerInit,

    /// JSON parsing or serialization failed.
    ///
    /// The message mentions parsing because loading the disk cache deserializes JSON.
    #[error("json parsing or serialization failed")]
    Json(#[from] serde_json::Error),

    /// A target path resolved outside the workspace root.
    #[error("path outside workspace root: {path}")]
    PathOutsideRoot { path: Utf8PathBuf },

    /// The on-disk cache's version or config hash does not match the current build.
    ///
    /// `persist::init` intercepts this variant, logs a warning, and starts from an empty cache; `persist::flush`'s
    /// fallback load propagates it like any other error.
    #[error("disk cache is stale (version or config hash mismatch)")]
    StaleDiskCache,

    /// A git ref failed validation before being passed to git.
    ///
    /// `reason` says whether the ref is empty, starts with `-` (and could be parsed as a git option), or contains
    /// characters outside `A-Za-z0-9_/.@{}^~:+-`.
    #[error("invalid git ref {ref_:?}: {reason}")]
    InvalidGitRef { ref_: String, reason: &'static str },

    /// A git subprocess failed.
    #[error("git {operation} failed: {message}")]
    Git { operation: &'static str, message: String },

    /// A read was refused because the path is a symlink.
    ///
    /// The symlink check runs before the size check, so an oversized symlink reports this variant rather than
    /// [`MonoklError::FileTooLarge`].
    #[error("refusing to read symlink: {path}")]
    SymlinkRejected { path: Utf8PathBuf },

    /// A read was refused because the file exceeds the size cap.
    #[error("file too large: {path} is {size} bytes, cap is {cap}")]
    FileTooLarge { path: Utf8PathBuf, size: u64, cap: u64 },

    /// A mutex was poisoned by an earlier panic.
    ///
    /// It is deliberately not recovered with `PoisonError::into_inner`, which would put possibly torn shared state
    /// back into use.
    #[error(
        "internal lock poisoned in {context} — a prior panic left shared state possibly inconsistent; this is a bug, please report"
    )]
    LockPoisoned { context: &'static str },
}

/// The crate-wide result type.
///
/// Errors propagate to the caller by default; the only one handled internally is [`MonoklError::StaleDiskCache`],
/// inside `persist::init`.
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

    fn real_serde_json_error() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{not valid json").unwrap_err()
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

    #[test]
    fn tokenizer_init_display() {
        let err = MonoklError::TokenizerInit;
        assert_eq!(err.to_string(), "tokenizer init failed");
    }

    #[test]
    fn json_display() {
        let err: MonoklError = real_serde_json_error().into();
        assert_eq!(err.to_string(), "json parsing or serialization failed");
    }

    #[test]
    fn path_outside_root_display() {
        let err = MonoklError::PathOutsideRoot { path: Utf8PathBuf::from("/root/outside") };
        assert_eq!(err.to_string(), "path outside workspace root: /root/outside");
    }

    #[test]
    fn stale_disk_cache_display() {
        let err = MonoklError::StaleDiskCache;
        assert_eq!(err.to_string(), "disk cache is stale (version or config hash mismatch)");
    }

    #[test]
    fn invalid_git_ref_display() {
        let err = MonoklError::InvalidGitRef {
            ref_: "-x".to_string(),
            reason: "starts with '-' — refusing as it could be parsed as a git option",
        };
        assert_eq!(
            err.to_string(),
            "invalid git ref \"-x\": starts with '-' — refusing as it could be parsed as a git option"
        );
    }

    #[test]
    fn git_display() {
        let err = MonoklError::Git { operation: "show", message: "not found".to_string() };
        assert_eq!(err.to_string(), "git show failed: not found");
    }

    #[test]
    fn symlink_rejected_display() {
        let err = MonoklError::SymlinkRejected { path: Utf8PathBuf::from("/l") };
        assert_eq!(err.to_string(), "refusing to read symlink: /l");
    }

    #[test]
    fn file_too_large_display() {
        let err = MonoklError::FileTooLarge { path: Utf8PathBuf::from("/big"), size: 100, cap: 50 };
        assert_eq!(err.to_string(), "file too large: /big is 100 bytes, cap is 50");
    }

    #[test]
    fn lock_poisoned_display() {
        let err = MonoklError::LockPoisoned { context: "ctx" };
        assert_eq!(
            err.to_string(),
            "internal lock poisoned in ctx — a prior panic left shared state possibly inconsistent; this is a bug, please report"
        );
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
            MonoklError::TokenizerInit,
            real_serde_json_error().into(),
            MonoklError::PathOutsideRoot { path: Utf8PathBuf::from("/root/outside") },
            MonoklError::StaleDiskCache,
            MonoklError::InvalidGitRef {
                ref_: "-x".to_string(),
                reason: "starts with '-' — refusing as it could be parsed as a git option",
            },
            MonoklError::Git { operation: "show", message: "not found".to_string() },
            MonoklError::SymlinkRejected { path: Utf8PathBuf::from("/l") },
            MonoklError::FileTooLarge { path: Utf8PathBuf::from("/big"), size: 100, cap: 50 },
            MonoklError::LockPoisoned { context: "ctx" },
        ];
        assert_eq!(variants.len(), 14);
        for v in &variants {
            assert!(v.code().is_none());
            assert!(v.help().is_none());
            assert!(v.url().is_none());
        }
    }
}
