# Principles

What monokl believes, and why. See [ARCHITECTURE.md](ARCHITECTURE.md) for how these get built.

## Design tenets

**Parse once, use everywhere.** A single `FileAnalysis` cache feeds every command — `search`, `symbols`, `refs`, `definition`, all of it. No command re-parses a file another command already analyzed in the same run.

**Aggregate views, not re-scans.** The import graph and symbol index are folded from cached per-file data by `WorkspaceEnricher`s. Building workspace-wide structure never means re-walking the filesystem or re-reading files that are already cached.

**Token budget is non-negotiable.** `max_bytes` defaults to 2 MiB. When output gets truncated, the response says so explicitly — a truncation marker, not silent data loss. An agent consuming monokl's output should never have to guess whether it's seeing everything.

**Determinism.** Serialized output uses `BTreeMap` for stable key order. Ties break on `(file, line_start)`. BM25 scores round to 6 decimal places specifically to eliminate floating-point divergence between x86-64 and ARM64 — the same query against the same code should produce byte-identical JSON regardless of what machine ran it.

**Arena lifetime containment.** No AST node born from the parser's `Allocator` escapes a per-file `analyze()` call. Every field on `FileAnalysis` is owned. This isn't a style preference — it's what makes caching and cross-thread sharing possible at all.

**Graceful degradation.** A broken file sets `had_parse_errors = true` and still contributes whatever data survived — it doesn't abort the whole operation. An unsupported language emits a `Skipped` diagnostic — it doesn't crash or silently return nothing.

**Tier separation.** Level 1 (`LanguageAnalyzer`, per-file) produces. Level 2 (`WorkspaceEnricher`, per-workspace) folds. Neither layer does the other's job.

**Content hash is authoritative.** `blake3` over file content is the real cache key. `mtime`/ `size` is the cheap fast-path check that avoids reading files that haven't changed — never the source of truth on its own.

## Non-goals

Deliberately not built, and why:

- **A persistent on-disk search index** (tantivy-style). The cache remembers parses; it doesn't maintain a standing inverted index. Simpler, and the numbers so far say it's fast enough without one below ~50k files — see [`docs/spec/05-research-and-decisions.md`](docs/spec/05-research-and-decisions.md).
- **A full [Salsa](https://github.com/salsa-rs/salsa) incremental-computation framework.** Real precedent exists for using Salsa standalone (Astral's `ty`, Biome), decoupled from rust-analyzer's own query set — this is an open investigation track, not a closed door.
- **Embedding-based retrieval by default.** No code-specific evidence it beats lexical + structural signals for single-codebase search, and it would mean either bundling a model or calling a cloud API — both in tension with staying local-first and low-latency. Gate behind an explicit opt-in if it's ever added; don't make it the default posture.
- **Stemming.** Code identifiers aren't prose; stemming rules built for English text don't transfer.
- **An in-memory trigram index.** Tried, measured, rejected — 600MB–1.2GB resident at scale in an earlier design. The current `WorkspaceIndex` design lands at roughly 90–140MB at 50k files by comparison.
- **Tree-sitter for TypeScript/JavaScript.** OXC is the native, arena-owned, actively-maintained alternative already chosen — no reason to trade that for a generic multi-language parser framework on the one language that has a better option.
- **A running agent loop.** monokl is a tool an agent calls, not an agent itself.

## Language coverage isn't uniform, and shouldn't pretend to be

A language gets a native parser and full `Structural`/`Exact` precision when it earns that investment — not by default, and not on a fixed roadmap regardless of what the parser ecosystem actually supports.

- **TypeScript/JS and Rust** — real, mature native-Rust parsers (OXC, `ra_ap_syntax`).
- **Python and Java** — no viable native option exists, checked directly against crates.io, so they use tree-sitter instead.
- **Go** — `gosyn` is a real, actively-maintained native option.

Precision targets follow from what's achievable per language, declared explicitly via the `CapabilityPrecision` model — never silently assumed uniform across languages that aren't.
