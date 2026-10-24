# 3. One crate with core and agent modules, not two crates

**Status:** Accepted **Date:** 2026-10-23

## Context

`08-library-session-api.md` §9.1 splits monokl into `monokl-core` (plumbing: analysis, indices, ranked uncapped results, no serde on result types, no budget) and `monokl-agent` (porcelain: budget, tiktoken, wire types, michi, CLI). The split's purpose is a boundary, not a second artifact: Wisp should depend on core without the CLI layer, and core result types must never become wire types. callisto merged its crates down to four because each crate adds versioning, release and dependency overhead.

## Decision

monokl stays one crate, `crates/monokl`, with the boundary as two top-level modules: `core` for what §9.1 puts in `monokl-core`, and `agent` for what it puts in `monokl-agent`. An architecture test enforces it: `core` never imports from `agent`, never derives `Serialize` on a result type, and never depends on tiktoken or michi. The `agent` module and its dependencies sit behind the `cli` feature, so a library consumer builds core alone.

## Consequences

- One version, one changeset stream, one release; no inter-crate version pins.
- The boundary is a test, not the compiler. A violation fails CI rather than failing to build.
- Wisp depends on `monokl` with default features off, rather than on a separate `monokl-core` crate.
- Existing modules move under `core` or `agent` when that layout lands; until then the architecture test names today's module paths.
- If a real reason appears (a different license, a separate publish, Wisp needing a smaller dependency graph than features allow), the modules become crates along the same line, and a new ADR records it.

## Alternatives considered

- **Two crates now, per §9.1.** The compiler enforces the boundary, at the cost of a second crate to version and release before anything outside monokl consumes core.
