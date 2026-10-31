<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/hero-dark.png">
    <img src="assets/brand/hero-light.png" alt="monokl, an Orin DX tool" width="360">
  </picture>
</p>

<p align="center">
  <b>AST-aware code search for TypeScript and JavaScript.</b><br />
  <i>Built for AI coding agents, not humans reading a terminal.</i>
</p>

> **Status: early.** `monokl search` works today for TypeScript and JavaScript. Symbol lookup, references, definitions and dependents are designed (see [`docs/spec/`](docs/spec/README.md)) but not built yet.

---

## What you get

Grep finds text. Embeddings find _vibes_. monokl parses your code with a compiler-grade parser ([OXC](https://oxc.rs)), so a search returns the **function, class, interface or type** that matches, not a line number to open and read.

- **Whole code units, best first.** Results are ranked, with functions, classes and methods ahead of imports and re-exports. A search for `format price` puts `formatCurrency` and `validatePrice` first, because names are split on `camelCase`, `snake_case` and acronyms before matching.
- **A small query language.** Plain words, `"quoted phrases"`, `+required` and `-excluded` terms, and `regex:` patterns. See [Writing a query](#writing-a-query).
- **Your project's rules apply.** Files that `.gitignore` or `.ignore` exclude, hidden files, test files and binary files are skipped by default, and each has a flag to include it.
- **Output sized for an agent's context.** Results are a compact table with paths relative to where you ran it, capped at 50 results, 20,000 tokens and 2 MiB by default. When something is cut, the output says how many more matched and suggests narrowing the query.
- **Predictable.** The same search on the same files gives the same output, in the same order, every time. A bad pattern fails the whole search with a clear error instead of being quietly dropped.
- **Hard to break.** A file with syntax errors is searched for whatever parsed. A pathologically deep file is skipped instead of crashing the search. An unreadable directory doesn't stop the rest of the search.

## Try it

## Install

**Prebuilt binary** (from 0.3.0), from [GitHub Releases](https://github.com/orin-dx/monokl/releases) (tag `monokl@<version>`):

```
curl -sL "https://github.com/orin-dx/monokl/releases/download/monokl@<version>/monokl-<target>.tar.gz" | tar -xz
```

Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`.

**cargo**, built from source:

```
cargo install monokl
```

## Try it

Search a directory (the default is the current one):

```
$ monokl search "format price" src/ --max-results 3
search[3]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:
  src/utils/format.ts,1,3,function,"function formatCurrency(amount: number, currency: string = 'USD'): string",1,3.950647
  src/utils/validation.ts,8,12,function,"function validatePrice(price: number): ValidationResult",2,3.07557
  src/index.ts,3,3,other,null,3,2.345138
totalCount: 10
help[2]: showing 3 of 10; refine the query with +term or -term,use extract with file/lineStart/lineEnd to retrieve a row's actual code content
```

Each row is one match: where it is (`file`, `lineStart`, `lineEnd`), what it is (`nodeKind`: function, class, interface, type alias and so on), its signature, and its `rank` and `finalScore`. Output uses [TOON](https://github.com/toon-format/spec), a table format that costs fewer tokens than JSON.

The last line suggests an `extract` command to read a row's code. That command is planned but not built yet; for now, open the file at `lineStart`–`lineEnd`.

## Writing a query

| You write | It means |
| --- | --- |
| `format price` | Code that mentions either word, ranked by how well it matches |
| `"format price"` | The exact phrase |
| `+cart` | Files must mention `cart` |
| `-Button` | Skip files that mention `Button` |
| `regex:use[A-Z]\w+` | Lines matching a regular expression |
| `--exact` | Treat the whole query as one literal, operators and quotes included |

Matching ignores case. A `-term` removes whole files, not single results. If one query mixes terms, `+cart -Button` finds code in files that mention `cart` but never `Button`:

```
$ monokl search "+cart -Button" .
search[2]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:
  src/hooks/useCart.ts,11,25,function,function useCart(),1,1.453107
  src/hooks/useCart.ts,5,9,interface,null,2,1.30166
```

## Options

| Flag | Default | What it does |
| --- | --- | --- |
| `--max-results N` | 50 | Most results to print |
| `--max-tokens N` | 20,000 | Most tokens of code to print |
| `--max-bytes N` | 2 MiB | Most bytes of code to print |
| `--tests` | off | Also search test files: anything under `test/`, `tests/`, `__tests__/` or `spec/`, and `*.test.ts`, `*.test.tsx`, `*.test.js` and the `.spec` equivalents |
| `--no-ignore` | off | Also search hidden files and files that `.gitignore` or `.ignore` exclude |
| `--language L` | all | Search one language only: `typescript`, `javascript` (`rust` and `python` are accepted but not supported yet) |

## Errors and exit codes

- **0:** the search ran, even if nothing matched (you get `search: []`).
- **1:** the search failed, for example an invalid regular expression or a directory that doesn't exist. The error is printed on stderr in the same compact format, with the cause.
- **2:** the command line itself was wrong.

## Language support

| Language | Status |
| --- | --- |
| TypeScript, JavaScript (`.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`, `.mts`, `.cts`; not `.d.ts`) | Search works today |
| Rust | Designed, not built |
| Python, Go, Java | Designed, not built |

## Why

Existing code-search-for-agents tools are either fast but fuzzy (grep, embeddings) or precise but heavy (a full language server, a persistent daemon). monokl aims at the middle: real AST parsing without a language server, and no persistent index to keep warm. See [ARCHITECTURE.md](ARCHITECTURE.md) for how, and [PRINCIPLES.md](PRINCIPLES.md) for why.

## Docs

| File | Covers |
| --- | --- |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How it's built, in prose |
| [PRINCIPLES.md](PRINCIPLES.md) | Design tenets and non-goals |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to work on this repo |
| [`docs/spec/`](docs/spec/README.md) | The full specification, including what's still to build |
| [AGENTS.md](AGENTS.md) | Guidance for AI coding agents working in this repo |

## Name

Monokl (MON-oh-kl) — from _monocle_, a single precision lens for close examination. The binary is `monokl`.

## License

FSL-1.1-MIT — Functional Source License, converting to MIT two years after each release. See [`LICENSE`](LICENSE).
