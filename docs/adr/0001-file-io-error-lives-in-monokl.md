# 1. Keep FileIoError in monokl instead of a shared io-errors crate

**Status:** Accepted **Date:** 2026-10-21

## Context

The spec (`01-core-architecture.md` §1 and §27) puts `FileIoError` in a separate `io-errors` crate, pulled from a private `orin-cargo` registry. That crate was never written or published, and no `.cargo/config.toml` defines the registry, so `cargo metadata` failed at manifest parse and monokl could not build or run a single test.

## Decision

`FileIoError` is the `monokl::file_io` module, with the spec's §27 contents unchanged. `MonoklError::Io` wraps it with `#[error(transparent)]` and `#[from]`. There is no private registry: dependencies come from crates.io, or by pinned git revision from an `orin-dx` repository allowed in `deny.toml`.

## Consequences

- monokl builds and tests with no out-of-repo setup.
- Another orin-dx tool that wants the same type cannot depend on it without depending on monokl. When a second tool needs it, it moves to a shared crate, under michi's own "second independent data point" rule.
- `docs/spec/01-core-architecture.md` §1 and §27 still describe the separate crate and need a matching correction.

## Alternatives considered

- **A new michi crate.** michi renders agent-facing responses; its principles keep domain logic out, and a file I/O error with a path is domain logic. One consumer is also one data point, which michi's principles say is not enough to add a primitive.
- **`crates/io-errors` in this workspace.** It would be publishable on its own, but it adds a crate to version and release for one 70-line enum with one consumer. callisto merged its crates down to four for the same overhead.
