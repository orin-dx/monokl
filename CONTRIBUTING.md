# Contributing

By participating in this project, you agree to abide by the [Code of Conduct](./CODE_OF_CONDUCT.md). Found a security issue? See [SECURITY.md](./SECURITY.md) instead of opening a public issue.

## Setup

[proto](https://moonrepo.dev/proto) pins the toolchain in `.prototools`; [just](https://just.systems) runs every task.

```bash
just setup          # install the pinned Rust and moon, install git hooks
just install-tools  # nextest, llvm-cov, machete, mutants, deny, typos, zizmor
```

## Recipes

| Recipe | What it does |
| --- | --- |
| `just ci` | Everything CI runs |
| `just test` | nextest plus doctests |
| `just lint` | `cargo fmt --check` and clippy with `-D warnings`, all features and no default features |
| `just doc-check` | `cargo doc` with warnings as errors |
| `just typos` | Spell check (part of `just lint`) |
| `just coverage` | Line coverage, failing below the justfile's threshold |
| `just zizmor` | Workflow audit for permission, credential and supply-chain issues |
| `just check-drift` | Fails when a duplicated fact (the Callisto pin) disagrees with its owner |
| `just deny` | Licenses, advisories, bans and sources |
| `just machete` | Unused dependencies |
| `just span-invariants` | Block span and parent invariants over the `rg-compare` repositories or `MONOKL_SPAN_CORPUS`; skips with a message when neither exists (on demand) |
| `just mutants` | Mutation testing, on demand only |
| `just changeset <bump> "<summary>"` | Record a pending change for the next release (Callisto) |
| `just changesets` | Check Callisto config, pending changesets and that every package changed since `origin/main` has one |
| `just bench [--repo NAME] [--json PATH]` | p50/p95 wall time, peak RSS and output bytes of `monokl search` against `rg` for six query classes on pinned repos (one large) plus a synthetic stress set; warm cache only, no gates (on demand, needs network) |
| `just rg-compare [--terms N] [--min-recall R]` | Whether `monokl search` puts a symbol's declaration in the top files, and how much output it costs against `rg -n`, on pinned public repos, plus 18 multi-word questions with known answers (`scripts/rg-compare.questions`, gated by `--min-question-recall`), 36 held-out questions on fastify, trpc and date-fns scored apart (`--min-heldout-recall`, `--min-heldout-member-recall`, both 0.9), and Rust rows on pinned `fd` (24 pooled, gated by `--min-rust-recall`, 0.9) (on demand, needs network) |
| `just agent-cost [--detail] [--json PATH]` | Bytes, estimated tokens and turns of one `monokl search` against a fixed rg-only agent policy on the `rg-compare` rows, and whether the answer is in the first 3 rows; deterministic, report only, no model (on demand, needs network) |
| `just verify-release <version>` | Check a published release: tag, assets, crates.io, attestation, binary |

## Where things stand

See [AGENTS.md](AGENTS.md#current-state).

## Proposing a change

- A spec correction: fix the spec file and add a dated entry to `docs/spec/05-research-and-decisions.md`.
- A decision with a credible alternative that is expensive to reverse: write an ADR (`docs/adr/0000-template.md`).
- Everything else: a PR with a Conventional Commits title.

Conventions are in [AGENTS.md](AGENTS.md). Highlights: no `unwrap()`/`expect()`/`panic!()` outside tests, one `thiserror` error enum, owned data out of every parse, `camino` paths, 120-column formatting, and no spec IDs in code.

## Pull requests

- CI's aggregate `CI` check must pass: lint, tests on Linux and macOS, MSRV, line coverage, audit and changesets. The same checks run on the merge queue's combined commit and again on every push to main; a red main opens or updates a `ci-red` issue.
- Keep a PR to one topic.
- A change users can see needs a changeset. A change to the crate users cannot see (tests, refactors) needs one too, with `just changeset none "<summary>"` (severity `none`: no release); CI and docs changes outside the crate need none. CI fails a PR that changes the crate without one.

## Writing a changeset

Each changeset becomes one bullet in the changelog. Each version lists its bullets under `Major Changes`, `Minor Changes` and `Patch Changes` headings, below the release date and a compare link. The release PR titles each change by its first line and collapses the rest.

- **One topic per file** in `.changeset/`, with `monokl: minor` for a new capability or `monokl: patch` for a fix.
- **Entries appear in filename order within their bump heading.** Prefix a filename with `0-` (then `1-`) to make an entry lead its heading.
- **Put identifiers, flags and syntax in backticks.**
- `just changesets` validates them. The rendered text only shows in the release PR, so read it there before merging.

Changesets become the project's changelog, so write them for the people who use it, not for the implementer. Write for the user of the CLI first; a Rust library user reads the last sentence.

- Open with a bold lead-in naming the change as the user sees it, then one line of value in plain words: what they can now do or what changed for them.
- Then at most 4 short bullets, one idea each, in this order: why it matters, how to adopt it (commands, configuration, migration), limits they will hit. No sub-bullets, no `#` headings (the changelog supplies those).
- Use plain words. No repository-internal terms: the harness, decision-log section numbers, script names, parameter grids, rejected alternatives. Those belong in `docs/spec/05-research-and-decisions.md`.
- At most one headline number, with how it was measured in half a sentence ("about 9% fewer tokens per search, measured on 222 sample questions"). Other measurements go in the decision log.
- Put a breaking change in its own paragraph starting `**Breaking:**` (old behavior, then new behavior) and a step users must take in one starting `**Action:**` (the exact command, flag or before/after). Each is at most 3 sentences and never inside a bullet: the release PR lists them under "Read before merging" only as paragraphs. A removed or renamed flag, a removed or non-public library item, a changed output shape or value and a library signature change are breaking.
- Put library API notes (new public fields, changed signatures) last, in one sentence of `Breaking:` or `Action:` that starts `For library users:`, so a CLI user can stop reading before it.
- An internal-only change (a refactor, a new library field nobody calls, a decision-log edit) gets `monokl: patch` or no changeset, and never a headline.

`just changesets` rejects repository-internal terms and more than 4 bullets in `.changeset/*.md` (`scripts/check-changesets.py`). It cannot judge wording, so the rest is review.

Before merging a release PR, read the rendered changelog as a whole, as a user upgrading would. Is there a one-line what's-new? Are related entries grouped? Is a measurement repeated? Does any entry need more than 10 seconds to scan? Fix the changesets, not the generated `CHANGELOG.md`.

Before and after, for the `lean-columns` entry:

```markdown
<!-- before: 5 dense bullets, harness numbers first, library types mixed in -->
**A search row drops `rank` and shows `relevance` in place of `finalScore`.** The columns are now `file,...,relevance`. On the 222 rows of the harness the total output falls from 118,601 to 107,918 `o200k_base` tokens (9.0%), the median from 448.5 to 405.5 tokens and the p90 from 1,120 to 1,033; in bytes ...

- **Rejected, with the measurements.** Merging `lineStart` and `lineEnd` breaks `scripts/agent-cost.py` (206 to 0 answerable) ...

**Breaking:** ... `SearchResponse` has a new public field, `top_score` ... `SearchView` has a new public field, `min_score`, and is no longer `Eq`. ...

<!-- after: value first, one number, library note last -->
**A search row drops `rank` and shows `relevance` instead of `finalScore`.** Each `monokl search` answer costs about 9% fewer tokens (measured on 222 sample questions) with the same results.

- `rank` is gone. Rows are in rank order, so a row's position is its rank.
- `relevance` is the row's score as a whole percent of the top row (top row 100).

**Breaking:** A row had 9 cells and now has 8. `rank` and `finalScore` are gone, and `relevance` is last.

**Action:** Read cells by header name, not by position. For library users: set `top_score` on a `SearchResponse` you build.
```

## Filing an issue

Say what you expected, what happened, and, if it's a spec-vs-reality mismatch, which file in `docs/spec/` you were reading.
