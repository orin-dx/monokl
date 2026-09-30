<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/hero-dark.png">
    <img src="assets/brand/hero-light.png" alt="monokl, an Orin DX tool" width="360">
  </picture>
</p>

<p align="center">
  <b>AST-aware semantic code search for TypeScript, JavaScript, and Rust.</b><br />
  <i>Built for AI coding agents, not humans reading a terminal.</i>
</p>

> **Status: spec-stage — no runnable binary.** No code exists yet. The full design lives in [`docs/spec/`](docs/spec/README.md); this repo is where that spec gets implemented. See [CONTRIBUTING.md](CONTRIBUTING.md) for where things stand.

---

## What it does

Grep finds text. Embeddings find _vibes_. monokl parses code with a compiler-grade parser ([OXC](https://oxc.rs) for TS/JS, [`ra_ap_syntax`](https://crates.io/crates/ra_ap_syntax) for Rust) and answers structural questions:

```
mnkl search "validatePrice" --path src/
mnkl refs Button --root . --include-tests
mnkl definition useCart --root .
mnkl dependents src/utils/format.ts --root .
mnkl explain src/components/Button.tsx
```

- **Text search** ranks by BM25 and node-type relevance, not string distance.
- **`refs`/`definition`** resolve real bindings, not name matches.
- **`dependents`** walks an actual import graph.
- **Output** is structured JSON by default, with a JSON Schema per result type — a tool an agent calls, not a report a human scrolls through.

## Why

Existing code-search-for-agents tools fall into two camps: fast-but-fuzzy (grep, embeddings) or precise-but-heavyweight (a full language server, a persistent daemon). monokl aims at the middle — real AST parsing without the cost of running a language server, no persistent index to keep warm, cache-friendly enough that repeat queries in one session don't re-pay the parse cost. See [ARCHITECTURE.md](ARCHITECTURE.md) for how, and [PRINCIPLES.md](PRINCIPLES.md) for why it's built this way instead of the alternatives.

## Language support

| Language | Parser | Status |
| --- | --- | --- |
| TypeScript / JavaScript | OXC | Fully specified |
| Rust | `ra_ap_syntax` | Fully specified |
| Python | tree-sitter-python | Specified, not implemented |
| Go | [`gosyn`](https://github.com/chikaku/gosyn) | Specified, not implemented |
| Java | tree-sitter-java | Specified, not implemented |

Unsupported operations degrade explicitly — a `Skipped`/`Warning` diagnostic in the response — rather than failing silently or crashing. See the capability model in [ARCHITECTURE.md](ARCHITECTURE.md).

## Docs

| File | Covers |
| --- | --- |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How it's built, in prose |
| [PRINCIPLES.md](PRINCIPLES.md) | Design tenets and non-goals |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Project status, how to work on this repo |
| [`docs/spec/`](docs/spec/README.md) | The full reconstruction-grade specification |
| [AGENTS.md](AGENTS.md) | Guidance for AI coding agents working in this repo |

## Name

Monokl (MON-oh-kl) — from _monocle_, a single precision lens for close examination. CLI binary is `monokl`, short alias `mnkl`.

## License

FSL-1.1-MIT — Functional Source License, converting to MIT two years after each release. See [`LICENSE`](LICENSE).
