# CLAUDE.md

@AGENTS.md

This file adds only what's specific to Claude Code.

## Skills

- **`/guide:fix`** before proposing a fix for a test failure or unexpected behavior. It starts from a test that fails on the original bug.
- **`/guide:build`** before writing implementation code for a command or module. The spec's fixtures (`tests/fixtures/small-ts/`) drive its tests.
- **`/guide:explore`** before any design change not already covered by `docs/adr/` or `docs/spec/05-research-and-decisions.md`.

## Verification

Before claiming a command or module works, run it against the fixtures in `docs/spec/01-core-architecture.md`'s "Test fixtures" section, not just `cargo check`.

<!-- entire-agent:begin -->
<!-- Entire agent instructions are inherited through AGENTS.md. -->
<!-- entire-agent:end -->
