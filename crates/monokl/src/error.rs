//! monokl's single unified error type and crate-wide `Result` alias.
//!
//! Per SPEC-001 (`.claude/specs/SPEC-001.json`), `MonoklError` is the crate's
//! sole error enum: every fallible `pub`/`pub(crate)` function in this crate
//! returns `crate::error::Result<T>` for its own `T`, with exactly one named
//! exception: `git_scope::blob_at_ref_in_repo`, a `pub(crate)` helper that
//! returns a bare `std::result::Result<String, String>` because it is always
//! mapped into `MonoklError::Git { operation: "show", .. }` at its sole call
//! site, `git_scope::blob_at_ref` (AC-003). `git_scope`, `persist`, and
//! `io_safety` are separate, not-yet-implemented tracks; this module defines
//! only the error contract they must conform to, not their bodies.
use camino::Utf8PathBuf;

use crate::file_io::FileIoError;

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum MonoklError {
    #[error(transparent)]
    Io(#[from] FileIoError),

    /// AC-005: three construction sites in the spec corpus --
    /// `text_search::search_files`, `WorkspaceIndex::build`'s walk, and
    /// `source_scan`'s per-directory walk -- in all three, `path` is the
    /// root of the walk in progress, not the failing entry.
    #[error("walker error at {path}")]
    Walk {
        path: Utf8PathBuf,
        #[source]
        source: ignore::Error,
    },

    /// AC-006.
    #[error("regex compilation failed")]
    RegexBuild(#[from] grep_regex::Error),

    /// AC-007: `path` is deliberately `std::path::PathBuf`, not
    /// `Utf8PathBuf` -- the entire point of this variant is that the path
    /// is not valid UTF-8, so it cannot be represented as camino's
    /// UTF-8-guaranteed type.
    #[error("non-UTF-8 path encountered: {path:?}")]
    NonUtf8Path { path: std::path::PathBuf },

    /// AC-008.
    #[error("query has too many terms: {count} > {limit}")]
    TooManyTerms { count: usize, limit: usize },

    /// AC-009: constructed at
    /// `tiktoken_rs::o200k_base().map_err(|_| MonoklError::TokenizerInit)?`.
    #[error("tokenizer init failed")]
    TokenizerInit,

    /// AC-011: 8 raw, non-test call sites in 3 categories -- cache-deserialize
    /// (1 site, `persist::load_cache`), cache-serialize (3 sites, all in
    /// `persist.rs`), CLI/output-serialize (4 sites: `output::render_json`,
    /// `output::render_json_compact`, `projection::project_inspect_result`,
    /// `projection::project_search_response`). Display text corrected from
    /// "json serialization failed" to the text below, since the
    /// cache-deserialize site is a parse failure the old text
    /// mischaracterized as a serialization failure.
    #[error("json parsing or serialization failed")]
    Json(#[from] serde_json::Error),

    /// AC-010: fired by the `dependents` command when the canonicalized
    /// target file does not start with the canonicalized root argument.
    #[error("path outside workspace root: {path}")]
    PathOutsideRoot { path: Utf8PathBuf },

    /// AC-013: returned by `persist::load_cache` when
    /// `cache.version != CACHE_VERSION || cache.config_hash != config_hash`.
    /// `load_cache` has exactly two callers: `persist::init`, whose match
    /// arm intercepts `Err(MonoklError::StaleDiskCache)` only (logs
    /// AC-020B's `tracing::warn!` text below and substitutes a fresh empty
    /// `CacheFile`, never returning `StaleDiskCache` to its own caller) --
    /// every other `Err(e)`, including `Json` and `Io`, propagates
    /// unmodified; and AC-013B: `persist::flush`'s
    /// `state.cache_file.is_none()` fallback branch, which calls
    /// `load_cache(..)?` directly with no match arm and no interception of
    /// any kind -- any `Err`, including `StaleDiskCache` or `Json`,
    /// propagates unmodified to `flush`'s own caller. In every call chain
    /// shown in the spec corpus, `flush`'s sole caller
    /// (`WorkspaceIndex::build`) calls `persist::init(..)?` first, so this
    /// fallback branch is not exercised today -- but `flush` is
    /// `pub(crate)`, not gated behind `init` at the type level, so nothing
    /// in the source prevents a future in-crate caller from invoking it
    /// first. This asymmetry is documented, not resolved here (non_goals):
    /// fixing it means editing `persist.rs`'s verbatim `flush()` body, out
    /// of scope for this error-contract track.
    ///
    /// AC-020B: `persist::init`'s interception arm additionally logs, on
    /// the self-heal path only (never returned to any caller), this exact
    /// text: "on-disk cache is stale (version or config_hash mismatch); starting fresh".
    #[error("disk cache is stale (version or config hash mismatch)")]
    StaleDiskCache,

    /// AC-012: sole construction site `git_scope::validate_git_ref`.
    /// Exactly three `reason` literals are constructed anywhere in the spec
    /// corpus: "ref is empty" (empty input), "starts with '-' — refusing
    /// as it could be parsed as a git option" (git-option-injection guard),
    /// and "contains characters outside the safe set" (any char outside
    /// A-Za-z0-9_/.@{}^~:+-).
    #[error("invalid git ref {ref_:?}: {reason}")]
    InvalidGitRef { ref_: String, reason: &'static str },

    /// AC-015: exactly 2 of `git_scope.rs`'s construction sites spell out a
    /// literal `operation` value in the spec corpus --
    /// `effective_pr_range`'s merge_base call (`operation: "merge-base"`)
    /// and `blob_at_ref` (`operation: "show"`). `collect_changes` contains
    /// 3 additional fallible-git-operation sites whose `map_err` closures
    /// are elided in the source as shown; their operation literals (and,
    /// for 2 of the 3, even whether the constructed variant is `Git` at
    /// all) are undetermined by the spec corpus and are not invented here
    /// (non_goals).
    #[error("git {operation} failed: {message}")]
    Git {
        operation: &'static str,
        message: String,
    },

    /// AC-016: sole construction site `io_safety::read_to_string_capped`,
    /// fired when `symlink_metadata(path).file_type().is_symlink()` is
    /// true, checked before the size check (see `FileTooLarge`).
    /// `read_to_string_capped` has no call site anywhere in the spec
    /// corpus despite prose claiming it guards
    /// search/inspect/extract/refs/definition -- refuted by
    /// `07-edge-cases-and-failure-modes.md` Part 3 findings #2 and #16,
    /// which show `extract` and the refs/definition path use raw
    /// `fs::read_to_string` instead. This variant is therefore currently
    /// unconstructible via any shown command path; wiring
    /// `read_to_string_capped` in is `io_safety.rs` implementation work,
    /// out of scope here (non_goals).
    #[error("refusing to read symlink: {path}")]
    SymlinkRejected { path: Utf8PathBuf },

    /// AC-017: same sole construction site as `SymlinkRejected`, fired when
    /// `meta.len() > MAX_INSPECTABLE_FILE_SIZE` (50 * 1024 * 1024 =
    /// 52,428,800 bytes). Shares `SymlinkRejected`'s total unreachability
    /// in the current corpus.
    #[error("file too large: {path} is {size} bytes, cap is {cap}")]
    FileTooLarge {
        path: Utf8PathBuf,
        size: u64,
        cap: u64,
    },

    /// AC-018: deliberately NOT auto-recovered via
    /// `PoisonError::into_inner()` -- recovering would launder possibly-torn
    /// shared state back into use. Two confirmed construction sites:
    /// `analysis::persist::lock_state()`
    /// (`context: "analysis::persist::CacheState"`) and
    /// `pipeline::session::WorkspaceSession::standard_index`
    /// (`context: "pipeline::session::WorkspaceSession::standard_index"`).
    #[error("internal lock poisoned in {context} — a prior panic left shared state possibly inconsistent; this is a bug, please report")]
    LockPoisoned { context: &'static str },
}

/// AC-019: crate-wide alias, immediately after the enum definition.
///
/// AC-003: every `pub`/`pub(crate)` function in the spec corpus's Part 1
/// and Part 3 verbatim code blocks whose own signature returns some
/// `Result<_, _>` returns exactly this alias for its own `T`, with exactly
/// one named exception: `git_scope::blob_at_ref_in_repo(repo, git_ref,
/// path) -> std::result::Result<String, String>`, a `pub(crate)` helper
/// whose bare-`String` error is mapped into `MonoklError::Git { operation:
/// "show", .. }` at its sole call site, `git_scope::blob_at_ref`.
///
/// AC-021: within the scope of variants and call sites this spec resolves,
/// `MonoklError`'s default posture is propagate-to-caller via `?` (or an
/// explicit `map_err`/`Into::into` at the construction boundary, for the
/// AC-003 exception). The sole documented exception is `StaleDiskCache`,
/// and only when `load_cache` is reached via `persist::init` specifically
/// -- not via `persist::flush`'s fallback, where the identical variant
/// propagates uncaught (AC-013B).
pub type Result<T> = std::result::Result<T, MonoklError>;

#[cfg(test)]
mod tests {
    //! Real `assert_eq!` tests for every Display literal this module locks, for the 13
    //! variants that do not depend on the io-errors blocker (see T-009). This module is
    //! valid, correct Rust and ships with error.rs -- but it cannot be run via `cargo test`
    //! against the real monokl crate yet, because the crate as a whole cannot build (see
    //! this plan's `baseline_build_status` note: 14 of lib.rs's 15 declared modules, and
    //! Cargo.toml's `[[bin]]` target, have no file on disk, and
    //! crates/monokl/Cargo.toml's `registry = "orin-cargo"` dependency line fails cargo's
    //! manifest parse before dependency resolution even begins). T-010 (this task, in
    //! PLAN-001) proves every one of these exact assertions for real, using an isolated
    //! /tmp fixture crate that reproduces this enum without the io-errors/registry
    //! dependency at all -- the same isolation pattern SPEC-004/PLAN-004 already
    //! established for this exact class of blocker, and the real command output this
    //! plan's own steps quote was captured live against cargo 1.96.0, not guessed. Once
    //! the crate builds for real (io-errors blocker resolved), this module runs as-is
    //! inside the real crate with no changes needed. The Io variant has no test here and
    //! is not expected to: it does not exist in this enum yet (T-009).
    use super::*;

    fn real_ignore_error() -> ignore::Error {
        ignore::overrides::OverrideBuilder::new(".")
            .add("[")
            .unwrap_err()
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
        let err = MonoklError::InvalidGitRef { ref_: "-x".to_string(), reason: "starts with '-' — refusing as it could be parsed as a git option" };
        assert_eq!(err.to_string(), "invalid git ref \"-x\": starts with '-' — refusing as it could be parsed as a git option");
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
        assert_eq!(err.to_string(), "internal lock poisoned in ctx — a prior panic left shared state possibly inconsistent; this is a bug, please report");
    }

    /// AC-002B (non-Io half): for all 13 non-Io variants, <MonoklError as
    /// miette::Diagnostic>::code()/help()/url() all return None. The Io
    /// variant's own code()/help()/url() cannot be tested here or anywhere in
    /// this workspace today -- it does not exist in the enum yet (T-009) --
    /// and remains in this plan's deferred_criteria field, narrowed to just
    /// that half. This test is gated on the "miette" feature (unlike the
    /// other 13 Display-literal tests above, which need no feature) because
    /// it calls the miette::Diagnostic trait methods, and the enum's own
    /// `#[derive(miette::Diagnostic)]` is itself feature-gated via
    /// `#[cfg_attr(feature = "miette", ...)]` above -- crates/monokl/Cargo.toml
    /// enables "miette" by default (cli = ["dep:clap", "miette"], and "cli"
    /// is in the default feature set), so this test runs under a default
    /// `cargo test` once the crate builds; it is proven for real today via
    /// T-010's isolated /tmp fixture (steps 5-7), which is unaffected by the
    /// io-errors/registry blocker that keeps the real crate from building.
    #[cfg(feature = "miette")]
    #[test]
    fn all_non_io_variants_have_no_diagnostic_metadata() {
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
            MonoklError::InvalidGitRef { ref_: "-x".to_string(), reason: "starts with '-' — refusing as it could be parsed as a git option" },
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
