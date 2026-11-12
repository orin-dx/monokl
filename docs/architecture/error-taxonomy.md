# Error Taxonomy — Semantic Model

Source: `docs/spec/01-core-architecture.md` §3 (error.rs), verbatim enum at lines 190–252.
`crates/monokl/src/error.rs` and `file_io.rs` hold the enum below, which is the part of the spec's enum that code
constructs. The variants the spec declares for unbuilt modules (cache, git scope, I/O safety) are not in the code; see
"Removed variants" below. The code is authoritative; the spec text for the missing variants is the design to build from
when the module lands.

## `MonoklError`

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MonoklError {
    Io(#[from] FileIoError),                                        // transparent
    Walk { path: Utf8PathBuf, #[source] source: ignore::Error },
    RegexBuild(#[from] grep_regex::Error),
    NonUtf8Path { path: std::path::PathBuf },
    TooManyTerms { count: usize, limit: usize },
}

pub type Result<T> = std::result::Result<T, MonoklError>;
```

`#[non_exhaustive]` — every downstream `match` needs a wildcard arm; adding variants later is
non-breaking. The CLI prints a failed search as a structured error on stderr through michi and exits 1.

`FileIoError` (`file_io.rs`, ADR 0001) has its own `#[non_exhaustive]` enum with one variant today:
`Read { path: Utf8PathBuf, #[source] source: io::Error }`. `MonoklError::Io` wraps it transparently via `#[from]`.

## When each variant fires

| Variant | Fires when | Construction site(s) |
|---|---|---|
| `Io` | `FileIoError::read` — a file read failed | The text searcher and the analyzer's file read; auto via `#[from]` on any `?` |
| `Walk` | `ignore::WalkBuilder` iterator yields `Err` mid-traversal (perm denied, loop, etc.) | `text_search::search_files` raises it only for the root itself (missing, not a directory, cannot be listed); errors below the root go in `TextSearchResult::unreadable`/`walk_errors_without_path` (05 §18). Index-build walker (01-core:2206), 03-multi:2033. `path` is the walk root, not the failing entry |
| `RegexBuild` | `RegexMatcherBuilder::build()` fails to compile a query pattern | `text_search::search_files` compiles each pattern alone, then all together, before walking; auto via `#[from]`/`?`. The per-term highlighting compile must fail the same way (05 §18, #7) — see gap 3 |
| `NonUtf8Path` | `Utf8PathBuf::from_path_buf` fails at a canonicalization boundary | The search root's canonicalization in `pipeline`. NOT raised mid-walk — see gap 4 |
| `TooManyTerms` | Query term count exceeds `LIMIT: usize = 64` | Query parse, `parse(&opts.query)` (01-core:831) |

## Known contract gaps — must be resolved, not ported as-is

Gaps 1 and 2 concern the disk cache, which is planned and not built: `Json` and `StaleDiskCache` are not in the enum above, and
they return with `persist.rs`.

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

2. **`StaleDiskCache`'s message promised a CLI subcommand that doesn't exist (07#6).** The
   `#[error(...)]` text said `run \`mnkl init --rebuild\``, but no `Subcmd` variant — in any of
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
   **Resolved (05 §18):** hard-fail in both stages.

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
   **Partly resolved (05 §18):** `search_files` no longer drops them silently; it counts them in `TextSearchResult::non_utf8_paths`, and the search pipeline turns a non-zero count into one diagnostic.

5. **Error output (07#19). Resolved:** a failed search prints one structured error through michi on stderr
   (`error`, `message`, `causes`, `exit_code`) and exits 1; a bad command line exits 2. There is no `render_error` and no
   miette.

## Removed variants — caller contract

Two earlier spec drafts declared `Parse { path, details: String }` and `Query { input, message: String }`; both were
removed because nothing constructs them: parse failures surface via `FileAnalysis::had_parse_errors` +
`tracing::warn!` (§17), and query failures only ever raise `TooManyTerms`.

The code also lacks the variants whose only construction sites are in unbuilt modules: `Json`, `StaleDiskCache` and
`LockPoisoned` (cache), `InvalidGitRef` and `Git` (git-scoped queries), `SymlinkRejected` and `FileTooLarge` (I/O
safety; search reports an oversize file as a `Skipped` diagnostic instead), `PathOutsideRoot` (`dependents`) and
`TokenizerInit` (the token count is a byte estimate, with no tokenizer), and
`FileIoError`'s `Write`, `CreateDir` and `Remove`. The spec text for each stays the design. Removal is a clean cut,
because `#[non_exhaustive]` already forces external `match` arms to carry a wildcard.

**Contract:** don't reintroduce a variant speculatively. A new `MonoklError` variant earns its
place only when spec/implementation text shows an actual `return Err(MonoklError::Variant {
.. })` construction site — not because the failure mode is merely imaginable.
