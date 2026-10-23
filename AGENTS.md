# monokl

AST-aware code search for TypeScript, JavaScript and Rust, built for coding agents. Part of [Orin DX](https://orin-dx.com) (github.com/orin-dx). Agent-facing output goes through [michi](https://github.com/orin-dx/michi) (`michi-core` on crates.io); don't hand-roll TOON lists, structured errors or hints that michi provides.

Vendor-neutral guidance. [CLAUDE.md](CLAUDE.md) adds Claude Code specifics.

## Read first

1. [`docs/spec/README.md`](docs/spec/README.md): which spec file covers what. The spec is the reconstruction source for unbuilt modules.
2. [`docs/architecture/README.md`](docs/architecture/README.md): which topic doc to read for the area you're changing.
3. [`docs/adr/README.md`](docs/adr/README.md) and [`docs/spec/05-research-and-decisions.md`](docs/spec/05-research-and-decisions.md): decisions already made. If something looks inconsistent, check here before "fixing" it.
4. [PRINCIPLES.md](PRINCIPLES.md): non-goals. No embedding reranker, no persistent trigram index, no tree-sitter TS/JS parser.

## Commands

```bash
just setup      # proto toolchain + moon git hooks
just ci         # everything CI runs
just test       # nextest + doctests
just lint       # fmt check + clippy, all features and no default features
```

Run one cargo command at a time; parallel cargo processes serialize on the `target/` lock. Never `git checkout Cargo.lock`.

## Current state

One crate, `crates/monokl`, with the error types, core data types, query language and michi output adapter. Modules are declared in `lib.rs` as they are implemented, not ahead of time, and a dependency is listed only once code uses it. [`.claude/plans/ACTIVE.md`](.claude/plans/ACTIVE.md) tracks what's next.

## Non-negotiables

- No `unwrap()`/`expect()`/`panic!()`/`unreachable!()`/`unimplemented!()` outside `#[cfg(test)]`. The workspace lint table denies them.
- One `thiserror` error enum, `MonoklError`. Compose failure modes with `#[from]`/`#[source]`; no `String`-message variant when a structured one is possible. File I/O failures are `FileIoError` ([ADR 0001](docs/adr/0001-file-io-error-lives-in-monokl.md)).
- `LanguageAnalyzer` implementations return fully owned data. No arena-borrowed AST node or parser-allocator lifetime in any field of `FileAnalysis`.
- `camino::Utf8Path`/`Utf8PathBuf` wherever a path appears.
- `FxHashMap`/`FxHashSet` for internal hot-path maps; `BTreeMap` for anything serialized, because output order must be deterministic.
- Core result types are not `Serialize`; wire shapes live in the output layer. See [`08-library-session-api.md`](docs/spec/08-library-session-api.md#9-plumbing-and-porcelain-two-crates).
- No caps or pre-rendered truncation strings in core. Counts only (`items_returned`/`items_total`); the output layer renders the marker.
- `unsafe_code` is denied workspace-wide.

## Rust style

Skills in [`.agents/skills/`](.agents/skills/) hold the detail: `rust-style` (control flow, matching, naming), `rust-types` (public type checklist, "wire or don't define"), `rustdoc` (doc comment format), `adr`.

- `pub(crate)` for implementation modules; `pub` only for the consumer API.
- No wildcard `_` arm on enums this crate owns; a compile error on a new variant is the point.
- Code never cites spec, criterion or plan IDs (`SPEC-001`, `AC-007`, `T-009`). Specs change; the code says what it guarantees in its own words, and tests name the behavior they prove.

## Testing

- `cargo nextest` plus `cargo test --doc`. `insta` for snapshots: accept a snapshot change only after confirming the new output is correct, not just different.
- Assert content, not presence: `assert_eq!` on the exact value over `contains`/`is_some`.
- Sibling-path parity: a function that mirrors an already-tested one gets a test asserting the same contract.
- CI gates line coverage at 90%.

## Fixing bugs

Look for structurally identical siblings and fix them in the same change. If one is deliberately left, say where and why.

When the spec itself is wrong, fix the spec file and add an entry to [`05-research-and-decisions.md`](docs/spec/05-research-and-decisions.md) explaining what was wrong and why the fix is right. A decision that departs from the spec with a credible alternative gets an ADR.

## Writing and commits

- Succinct: short sentences, bullets, tables. No backstory, hedging or filler. Banned words are in [`docs/adr/README.md`](docs/adr/README.md).
- Never hard-wrap Markdown, commit messages or PR descriptions.
- Comments explain only the non-obvious why, carry no history (the commit records it), and fill to 120 columns.
- Commits: Conventional Commits, imperative subject, at most one or two sentences of why. Skip the body when the subject says it all.
