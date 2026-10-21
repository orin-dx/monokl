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
| `just coverage 90` | Line coverage, failing below the threshold |
| `just deny` | Licenses, advisories, bans and sources |
| `just machete` | Unused dependencies |
| `just mutants` | Mutation testing, on demand only |

## Where things stand

One crate, `crates/monokl`, with the error types, core data types, query language and michi output adapter. The rest of the design is in [`docs/spec/`](docs/spec/README.md); start at its README. [`docs/architecture/`](docs/architecture/README.md) has topic docs per subsystem, and [`docs/adr/`](docs/adr/README.md) records decisions made since the spec.

## Proposing a change

- A spec correction: fix the spec file and add a dated entry to `docs/spec/05-research-and-decisions.md`.
- A decision with a credible alternative that is expensive to reverse: write an ADR (`docs/adr/0000-template.md`).
- Everything else: a PR with a Conventional Commits title.

Conventions are in [AGENTS.md](AGENTS.md). Highlights: no `unwrap()`/`expect()`/`panic!()` outside tests, one `thiserror` error enum, owned data out of every parse, `camino` paths, 120-column formatting, and no spec IDs in code.

## Pull requests

- CI's aggregate `CI` check must pass: lint, tests on Linux and macOS, MSRV, 90% line coverage, and audit.
- Keep a PR to one topic.

## Filing an issue

Say what you expected, what happened, and, if it's a spec-vs-reality mismatch, which file in `docs/spec/` you were reading.
