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
| `just rg-compare [--terms N] [--min-recall R]` | Whether `monokl search` puts a symbol's declaration in the top files, and how much output it costs against `rg -n`, on pinned public repos (on demand, needs network) |
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

Each changeset becomes one bullet in the changelog, which is a flat list per version.

- **One topic per file** in `.changeset/`, with `monokl: minor` for a new capability or `monokl: patch` for a fix.
- **Lead with what the user sees now,** in one sentence that stands alone. Say what changed, not that something was "improved" or "fixed".
- **Details go in sub-bullets,** one level deep, each starting with a bold lead-in that ends in a period.
- **Never use `#` headings** or start the summary with `- `: the changelog supplies the version heading and the bullet.
- **Put identifiers, flags and syntax in backticks,** and leave out function names, spec IDs and PR numbers.
- **Entries appear in filename order.** Prefix a filename with `0-` (then `1-`) to make an entry lead its release.
- `just changesets` validates them. The rendered text only shows in the release PR, so read it there before merging.

## Filing an issue

Say what you expected, what happened, and, if it's a spec-vs-reality mismatch, which file in `docs/spec/` you were reading.
