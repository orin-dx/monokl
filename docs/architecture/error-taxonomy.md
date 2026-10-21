# Error Taxonomy — Semantic Model

Source: `docs/spec/01-core-architecture.md` §3 (error.rs), verbatim enum at lines 190–252.
No code exists yet — this file is currently **normative**, not descriptive. When `error.rs`
is implemented, re-point this citation at the source and reconcile via `canon/drift`; until
then, deviating from the enum below is a spec change, not an implementation detail.

The enum shown is already the **post-research-correction** state: `InvalidGitRef`/`Git`/
`SymlinkRejected`/`FileTooLarge` were added late (Part 3 constructs all four but an earlier
draft never declared them — a compile-breaking gap). See "Removed variants" below for what
was cut in the same pass.

## `MonoklError`

```rust
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "miette", derive(miette::Diagnostic))]
#[non_exhaustive]
pub enum MonoklError {
    Io(#[from] FileIoError),                                        // transparent
    Walk { path: Utf8PathBuf, #[source] source: ignore::Error },
    RegexBuild(#[from] grep_regex::Error),
    NonUtf8Path { path: std::path::PathBuf },
    TooManyTerms { count: usize, limit: usize },
    TokenizerInit,
    Json(#[from] serde_json::Error),
    PathOutsideRoot { path: Utf8PathBuf },
    StaleDiskCache,
    InvalidGitRef { ref_: String, reason: &'static str },
    Git { operation: &'static str, message: String },
    SymlinkRejected { path: Utf8PathBuf },
    FileTooLarge { path: Utf8PathBuf, size: u64, cap: u64 },
    LockPoisoned { context: &'static str },
}

pub type Result<T> = std::result::Result<T, MonoklError>;
```

`#[non_exhaustive]` — every downstream `match` needs a wildcard arm; adding variants later is
non-breaking. `miette::Diagnostic` derive is present whenever the `miette` feature is on, which
`cli` always pulls in (`cli = ["dep:clap", "miette"]`) — i.e. every CLI build has it, whether or
not anything actually renders it (see gap 5).

`FileIoError` (`io_errors` crate, §"types.rs" cross-ref, 01-core-architecture.md:2959) has its
own `#[non_exhaustive]` enum: `Read`/`Write`/`CreateDir`/`Remove { path: Utf8PathBuf, #[source]
source: io::Error }`. `MonoklError::Io` wraps it transparently via `#[from]`.

## When each variant fires

| Variant | Fires when | Construction site(s) |
|---|---|---|
| `Io` | Any `FileIoError::{read,write,create_dir,remove}` — filesystem op failed | Root canonicalization, cache load/write, `read_to_string_capped`'s underlying read; auto via `#[from]` on any `?` |
| `Walk` | `ignore::WalkBuilder` iterator yields `Err` mid-traversal (perm denied, loop, etc.) | `text_search::search_files` (01-core:1005), index-build walker (01-core:2206), 03-multi:2033. `path` is the walk root, not the failing entry |
| `RegexBuild` | `RegexMatcherBuilder::build()` fails to compile the **combined** OR'd pattern from all query terms | `text_search::search_files` stage (01-core:998), auto via `#[from]`/`?`. NOT used for the per-term highlighting regex — see gap 3 |
| `NonUtf8Path` | `Utf8PathBuf::from_path_buf` fails at a canonicalization boundary | `search` stage 3 (root canon.), `dependents` (file+root canon.), `pipeline::session::canonicalize_utf8`/`current_dir` fallback (03-multi:1289-1292). NOT raised mid-walk — see gap 4 |
| `TooManyTerms` | Query term count exceeds `LIMIT: usize = 64` | Query parse, `parse(&opts.query)` (01-core:831) |
| `TokenizerInit` | `tiktoken_rs::o200k_base()` fails to build the BPE tokenizer | `budget`/token counting (01-core:1240). No fields — unparameterized init failure |
| `Json` | `serde_json::{from_str,to_string}` fails | `load_cache`'s `serde_json::from_str::<CacheFile>(&content)` (01-core:1602) when `cache.json` isn't valid JSON. Auto via `#[from]`. Hard-fail path in gap 1 |
| `PathOutsideRoot` | Canonicalized file path doesn't `starts_with` the canonicalized workspace root | `dependents`: `!abs_file.starts_with(&abs_root)` (01-core:2332). Enforced per-command, not centralized in `io_safety.rs` (03-multi:2131) |
| `StaleDiskCache` | `cache.version != CACHE_VERSION \|\| cache.config_hash != config_hash` | `load_cache` (01-core:1603-1604). Caught by `init()`'s own match arm (01-core:1572) when reached via `init()` — but `persist::flush`'s fallback also calls `load_cache` (01-core:1657) with no interception at all, so this variant is NOT unconditionally caller-invisible (see gap 2; SPEC-001 AC-013B) |
| `InvalidGitRef` | `validate_git_ref`: ref is empty, starts with `-` (git-option-injection guard), or contains a char outside the allowed safe set | `git_scope::validate_git_ref` (03-multi:1365-1385), three `reason` strings |
| `Git` | Any `gix` operation fails, tagged with a static `operation` label | `merge_base` (`operation: "merge-base"`, 03-multi:1397), `blob_at_ref_in_repo` (03-multi:1540), diff `changes()` (03-multi:1428) |
| `SymlinkRejected` | `symlink_metadata(path).file_type().is_symlink()` is true | `io_safety::read_to_string_capped` (03-multi:2113-2114) is the only place this variant is ever constructed — and that helper has zero call sites anywhere in the corpus (SPEC-001 AC-016, verified against 07-edge-cases Part 3 findings #2 and #16). The Part 3 prose claiming it guards search/inspect/extract/refs/definition (03-multi:2131) is refuted, not just unconfirmed: `extract` and the `refs`/`definition` path use raw `fs::read_to_string` instead. This variant is currently unconstructible via any shown command path, not merely narrowly reachable |
| `FileTooLarge` | `meta.len() > MAX_INSPECTABLE_FILE_SIZE` (50 MiB) | Same helper as `SymlinkRejected`, same total unreachability (03-multi:2117-2122; SPEC-001 AC-017) |
| `LockPoisoned` | `Mutex::lock()` returns `Err` — a prior panic left the lock's data possibly torn | `analysis::persist::CacheState` (01-core:1560), `pipeline::session::WorkspaceSession::standard_index` (03-multi:1248). Deliberately NOT auto-recovered via `PoisonError::into_inner()` — comment at 01-core:1556-1559 explains recovering would launder torn state back into use |

## Known contract gaps — must be resolved, not ported as-is

These are audit findings (`docs/spec/07-edge-cases-and-failure-modes.md`, Part 1) where the
spec text produces two different outcomes for the same underlying failure condition. Do not
silently pick one when implementing — spec@1 for this subsystem must state the choice and why.

1. **Corrupted `cache.json` — self-heal or hard-fail? (07#5)** `load_cache`'s
   `serde_json::from_str::<CacheFile>(&content)?` (01-core:1602) runs *before* the staleness
   check, and `init()`'s match (01-core:1567-1584) only special-cases `MonoklError::StaleDiskCache`
   (line 1572) — not `Json`. A syntactically-complete-but-malformed file (e.g. a disk-full
   truncated write that lands before the paired atomic rename) hard-fails the whole workspace
   build. Does `Json` on cache-load deserve `StaleDiskCache`'s self-healing treatment (catch,
   warn, rebuild from empty), or is hard-fail intentional because corruption might signal
   something worth surfacing that routine staleness doesn't? If self-healing wins, decide
   whether it covers only `load_cache` or every cache-touching `Json` error.

   **Status: deliberately still open.** An early SPEC-001 draft tried to resolve this inline;
   two independent review rounds found the resulting criteria kept failing (asserted an
   interception no code showed, invented unsourced log text). Deferred to a follow-on track
   (SPEC-002) gated on `superpowers:brainstorming`, per this project's own process for new
   design decisions — not something a spec@1 judgment call should settle unilaterally.

2. **`StaleDiskCache`'s message promises a CLI subcommand that doesn't exist (07#6).** The
   `#[error(...)]` text says `run \`mnkl init --rebuild\``, but no `Subcmd` variant — in any of
   the corpus's three successive enums (the original 5-variant one at §21, an intermediate
   16-variant revision at 02-inspection-and-analysis.md:54-58, or the final 20-variant one at
   03-multi:2212-2342) — has an `init` subcommand. Originally thought to be doubly moot because `init()`'s match arm
   (01-core:1572) "always" intercepts `StaleDiskCache` before it reaches a caller — that framing
   was itself wrong: `persist::flush`'s fallback also calls `load_cache` (01-core:1657) with no
   interception, so `StaleDiskCache` (and `Json`) CAN reach a caller unmediated. The message
   needed fixing regardless of whether the variant is ever caller-visible.

   **Status: message text resolved by SPEC-001 (AC-020)** — corrected to "disk cache is stale
   (version or config hash mismatch)" with the command reference removed outright. The larger
   design question — should `flush`'s fallback get the same interception `init` has, making
   `StaleDiskCache` truly caller-invisible — is still open, tracked as SPEC-001 AC-013B
   (documented, not fixed; fixing requires editing `persist.rs`'s verbatim `flush()` body).

3. **Bad `regex:` term: hard error in one search stage, silently dropped in the next (07#7).**
   Candidate-search stage builds one OR'd pattern from all terms via
   `RegexMatcherBuilder::build(&combined)?` (01-core:998) → `RegexBuild`, hard-failing the whole
   `search` call. The later per-term highlighting stage builds `regex::Regex` per term and
   "silently drop[s] regex compile failures" (01-core:2321) — no error, no `Diagnostic`, term
   just excluded from `matched_keywords`. Same malformed `regex:pattern` term, opposite outcomes
   within one invocation. Should both stages hard-fail (consistent, but a query that would
   otherwise partially work now fails outright)? Both degrade via a `Diagnostic` (consistent,
   matches the doc's own graceful-degradation tenet)? Or is the asymmetry actually correct
   because stage 1's regex gates *candidate selection* (a bad pattern really does invalidate
   the result set) while the later stage's gates *cosmetic highlighting* only? Pick one and, if
   degrade-not-fail, route it through a `Diagnostic` instead of disappearing.

4. **Non-UTF-8 path: hard error at the root, silent skip mid-walk (07#12).** Root/entry-point
   canonicalization (`search` stage 3, `dependents`, `canonicalize_utf8`/`current_dir` fallback)
   raises `NonUtf8Path` and hard-fails. `text_search::search_files`'s per-entry walk does
   `Utf8PathBuf::from_path_buf(entry.into_path()) { Err(_non_utf8) => continue }`
   (01-core:1010-1013) — silent, not even `tracing::warn!`. Same condition, opposite outcomes
   depending on where in the pipeline it's hit. Should a mid-walk non-UTF-8 entry surface as a
   `Diagnostic` (Skipped/Warning) the way other per-file skips are supposed to, or is silent skip
   intentional (non-UTF-8 paths mid-tree assumed irrelevant, e.g. vendored binaries)? If a
   `Diagnostic` is added, note `NonUtf8Path` itself only carries `std::path::PathBuf` (not a
   lossy string) — decide how a diagnostic would represent a path that by definition can't
   round-trip through the rest of the UTF-8-only type system.

5. **Two contradictory error-output mechanisms both defined; only one wired up (07#19).**
   `output.rs::render_error(err: &MonoklError) -> !` (01-core:2436) prints
   `{"error": "{err}"}` to stdout and `exit(1)` — JSON-on-stdout, matching `render_output`'s own
   convention. `main()`'s `run()` (01-core:2453-2515) never calls it: every fallible step is
   `.into_diagnostic().wrap_err(...)?` against `miette::Result`, so failures print via miette's
   `fancy` stderr renderer instead. `render_error` is unreachable dead code as specified. Which
   is the real contract? If JSON-on-stdout (matching the "agent-parseable output" value
   proposition the audit itself invokes), `main()` must call `render_error`, not miette. If
   miette-on-stderr, delete `render_error` and correct any doc/agent-facing promise of parseable
   stdout errors. This also decides whether `#[error(...)]` strings are the real user-facing
   surface (miette path) or need a distinct structured JSON shape (render_error path) — they
   can't cleanly serve both at once.

## Removed variants — caller contract

An earlier draft also declared `Parse { path, details: String }` and
`Query { input, message: String }`. Both were removed — nothing in the spec ever constructs
either: parse failures surface via `FileAnalysis::had_parse_errors` + `tracing::warn!` (§17),
and query failures only ever raise `TooManyTerms`. Removal was a clean cut, not a breaking one,
because `#[non_exhaustive]` already forces external `match` arms to carry a wildcard.

**Contract:** don't reintroduce a variant speculatively. A new `MonoklError` variant earns its
place only when spec/implementation text shows an actual `return Err(MonoklError::Variant {
.. })` construction site — not because the failure mode is merely imaginable.
