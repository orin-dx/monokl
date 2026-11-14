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
| `just coverage` | Line coverage, failing below the justfile's threshold |
| `just check-drift` | Fails when a duplicated fact (the Callisto pin) disagrees with its owner |
| `just deny` | Licenses, advisories, bans and sources |
| `just machete` | Unused dependencies |
| `just mutants` | Mutation testing, on demand only |
| `just changeset <bump> "<summary>"` | Record a pending change for the next release (Callisto) |
| `just changesets` | Check Callisto config and pending changesets |
| `just bench [--repo NAME] [--json PATH]` | p50/p95 wall time, peak RSS and output bytes of `monokl search` against `rg` for six query classes on pinned repos (one large) plus a synthetic stress set; warm cache only, no gates (on demand, needs network) |
| `just rg-compare [--terms N] [--min-recall R]` | Whether `monokl search` puts a symbol's declaration in the top files, and how much output it costs against `rg -n`, on pinned public repos, plus 18 multi-word questions with known answers (`scripts/rg-compare.questions`, gated by `--min-question-recall`), 36 held-out questions on fastify, trpc and date-fns scored apart (`--min-heldout-recall`, `--min-heldout-member-recall`, both 0.9), and Rust rows on pinned `fd` (24 pooled, gated by `--min-rust-recall`, 0.9) (on demand, needs network) |
| `just verify-release <version>` | Check a published release: tag, assets, crates.io, attestation, binary |

## Where things stand

See [AGENTS.md](AGENTS.md#current-state).

## Proposing a change

- A spec correction: fix the spec file and add a dated entry to `docs/spec/05-research-and-decisions.md`.
- A decision with a credible alternative that is expensive to reverse: write an ADR (`docs/adr/0000-template.md`).
- Everything else: a PR with a Conventional Commits title.

Conventions are in [AGENTS.md](AGENTS.md). Highlights: no `unwrap()`/`expect()`/`panic!()` outside tests, one `thiserror` error enum, owned data out of every parse, `camino` paths, 120-column formatting, and no spec IDs in code.

## Pull requests

- CI's aggregate `CI` check must pass: lint, tests on Linux and macOS, MSRV, line coverage, and audit.
- Keep a PR to one topic.
- A change users can see needs a changeset. Internal changes (tests, CI, docs, refactors) do not.

## Writing a changeset

Each changeset becomes one bullet in the changelog. Each version lists its bullets under `Major Changes`, `Minor Changes` and `Patch Changes` headings, below the release date and a compare link. The release PR titles each change by its first line and collapses the rest.

- **One topic per file** in `.changeset/`, with `monokl: minor` for a new capability or `monokl: patch` for a fix.
- **Entries appear in filename order within their bump heading.** Prefix a filename with `0-` (then `1-`) to make an entry lead its heading.
- **Put identifiers, flags and syntax in backticks.**
- `just changesets` validates them. The rendered text only shows in the release PR, so read it there before merging.

Changesets become the project's changelog, so write them for the people who use it, not for the implementer.

- Open with a bold lead-in naming the change as the user sees it, then one line on its value: what they can now do or what changed for them.
- When there is more to say, add short bullets in this order: why it matters, how to adopt it (commands, configuration, migration), and limits they will hit. Use sub-bullets for detail.
- Group a longer entry under bold labels such as **Upgrading** or **Scripting**. Do not use `#` headings; the changelog supplies those.
- Put a breaking change in its own paragraph starting `**Breaking:**` (old behavior, new behavior, what to do), and a step users must take in one starting `**Action:**`. Never inside a bullet: the release PR lists them under "Read before merging" only as paragraphs. A removed or renamed flag, a removed or non-public library item, a changed output shape or value and a library signature change are breaking.
- One idea per bullet. Leave out internals unless the user must know them.

## Filing an issue

Say what you expected, what happened, and, if it's a spec-vs-reality mismatch, which file in `docs/spec/` you were reading.
