# Principles

What monokl believes, and why. See [ARCHITECTURE.md](ARCHITECTURE.md) for how these get built.

## Design tenets

**Parse once.** Each file is parsed once per run, by one `LanguageAnalyzer`. Later stages read its `FileAnalysis` and never re-parse.

**Every stage is bounded.** File size, line length, nesting, file count and time each have a stated limit, and search keeps no state between runs ([ADR 0004](docs/adr/0004-search-stays-stateless-and-every-stage-has-a-stated-bound.md)).

**Token budget is non-negotiable.** `--max-tokens` defaults to 20,000. When output is cut, a `help` line says how many rows were cut. An agent consuming monokl's output should never have to guess whether it's seeing everything.

**Determinism.** Ties in score break on file path, line range, kind and code, so the order never depends on the walk or the thread schedule. Scores round to 6 decimal places to remove floating-point divergence between x86-64 and ARM64. The same query against the same code prints the same output on any machine. The exception is a partial result after a timeout.

**Arena lifetime containment.** No AST node born from the parser's `Allocator` escapes a per-file `analyze()` call. Every field on `FileAnalysis` is owned. This isn't a style preference. It's what lets a `FileAnalysis` cross threads.

**Graceful degradation.** A file with syntax errors still contributes whatever parsed. A file that is too large, too deep or unreadable is reported in a `help` line and does not abort the search. An unsupported `--language` prints a `help` line, not a crash and not silence.

**Planned: aggregate views, not re-scans.** An import graph and a symbol index folded from per-file analyses by a `WorkspaceEnricher`, with `symbols`, `refs` and `definition` on top. Not built. When built, tier 1 (`LanguageAnalyzer`, per file) produces and tier 2 (`WorkspaceEnricher`, per workspace) folds, and neither does the other's job.

**Planned: content hash is authoritative.** If a parse cache is built, a `blake3` hash of file content is its key, and `mtime` and size are only a fast path. Not built.

## Non-goals

Deliberately not built, and why:

- **A persistent on-disk search index** (tantivy-style). Search is stateless today, and a planned parse cache would remember parses, not maintain a standing inverted index. Simpler, and the numbers so far say it's fast enough without one below ~50k files. See [`docs/spec/05-research-and-decisions.md`](docs/spec/05-research-and-decisions.md).
- **A full [Salsa](https://github.com/salsa-rs/salsa) incremental-computation framework.** Real precedent exists for using Salsa standalone (Astral's `ty`, Biome), decoupled from rust-analyzer's own query set — this is an open investigation track, not a closed door.
- **Embedding-based retrieval by default.** No code-specific evidence it beats lexical + structural signals for single-codebase search, and it would mean either bundling a model or calling a cloud API — both in tension with staying local-first and low-latency. Gate behind an explicit opt-in if it's ever added; don't make it the default posture.
- **Stemming.** Code identifiers aren't prose; stemming rules built for English text don't transfer.
- **An in-memory trigram index.** Tried, measured, rejected — 600MB–1.2GB resident at scale in an earlier design. The planned `WorkspaceIndex` design lands at roughly 90–140MB at 50k files by comparison.
- **Tree-sitter for TypeScript/JavaScript.** OXC is the native, arena-owned, actively-maintained alternative already chosen — no reason to trade that for a generic multi-language parser framework on the one language that has a better option.
- **A running agent loop.** monokl is a tool an agent calls, not an agent itself.

## Language coverage isn't uniform, and shouldn't pretend to be

A language gets a native parser when it earns that investment — not by default, and not on a fixed roadmap regardless of what the parser ecosystem actually supports.

- **TypeScript/JS and Rust** — built. Real, mature native-Rust parsers (OXC, `ra_ap_syntax`).
- **Python and Java** — planned, not built. No viable native option exists, checked directly against crates.io, so they would use tree-sitter.
- **Go** — planned, not built. `gosyn` is a real, actively-maintained native option.

Planned: precision per operation declared through a `CapabilityPrecision` model, so no language is silently assumed to match another.
