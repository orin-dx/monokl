# CLAUDE.md

@AGENTS.md

This file adds only what's specific to Claude Code.

## Skills

- **`superpowers:test-driven-development`** before writing implementation code for any command or module. The spec's fixtures (`tests/fixtures/small-ts/`) exist to drive this.
- **`superpowers:systematic-debugging`** before proposing a fix for a test failure or unexpected behavior.
- **`superpowers:brainstorming`** before any design change not already covered by `docs/adr/` or `docs/spec/05-research-and-decisions.md`.

## Verification

Before claiming a command or module works, run it against the fixtures in `docs/spec/01-core-architecture.md`'s "Test fixtures" section, not just `cargo check`.

<!-- entire-agent:begin -->
<!-- Entire agent instructions are inherited through AGENTS.md. -->
<!-- entire-agent:end -->
