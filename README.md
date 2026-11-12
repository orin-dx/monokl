<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/hero-dark.png">
    <img src="assets/brand/hero-light.png" alt="monokl, an Orin DX tool" width="360">
  </picture>
</p>

<p align="center">
  <b>AST-aware code search for TypeScript, JavaScript and Rust.</b><br />
  <i>Built for AI coding agents, not humans reading a terminal.</i>
</p>

> **Status: early.** `monokl search` works today for TypeScript, JavaScript and Rust. Symbol lookup, references, definitions and dependents are designed (see [`docs/spec/`](docs/spec/README.md)) but not built yet.

---

## What you get

Grep finds text. Embeddings find _vibes_. monokl parses your code with a compiler-grade parser ([OXC](https://oxc.rs) for TypeScript and JavaScript, [rust-analyzer's syntax crate](https://crates.io/crates/ra_ap_syntax) for Rust), so a search returns the **function, class, struct, trait or type** that matches, not a line number to open and read.

- **Whole code units, best first.** Results are ranked, with functions, classes and methods ahead of imports and re-exports. A search for `format price` puts `formatCurrency` and `validatePrice` first, because names are split on `camelCase`, `snake_case` and acronyms before matching. A block whose own name holds query words ranks above one that only uses them in its body.
- **A small query language.** Plain words, `"quoted phrases"`, `+required` and `-excluded` terms, and `regex:` patterns. See [Writing a query](#writing-a-query).
- **Your project's rules apply.** Files that `.gitignore` or `.ignore` exclude, hidden files, test files, Rust items under `#[cfg(test)]` or `#[test]` and binary files are skipped by default, and each has a flag to include it.
- **Output sized for an agent's context.** Results are a compact table with paths relative to where you ran it, capped by default at the limits in [Options](#options). When something is cut, the output says how many more matched and suggests raising the cap or narrowing the query.
- **Predictable.** The same search on the same files gives the same output, in the same order, every time. A bad pattern fails the whole search with a clear error instead of being quietly dropped.
- **Hard to break.** A file with syntax errors is searched for whatever parsed. A pathologically deep file is skipped instead of crashing the search. An unreadable directory doesn't stop the rest of the search.

## Install

**Prebuilt binary**, attached to each [GitHub release](https://github.com/orin-dx/monokl/releases) (tag `monokl@<version>`):

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
search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:
  src/utils/format.ts,1,3,function,null,"function formatCurrency(amount: number, currency: string = 'USD'): string","2: return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);",1,2.950647
  src/utils/validation.ts,8,12,function,null,"function validatePrice(price: number): ValidationResult","9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\n10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };",2,2.07557
  src/index.ts,3,3,other,null,null,"3: export { formatCurrency, truncateString } from './utils/format';",3,1.345138
totalCount: 10
help[1]: showing 3 of 10; raise the cap or refine the query with +term or -term
```

Each row is one match: where it is (`file`, `lineStart`, `lineEnd`), what it is (`nodeKind`: function, class, method, interface, type alias and so on), what holds it (`parent`), its signature, the lines in it that matched (`snippet`), and its `rank` and `finalScore`. Output uses [TOON](https://github.com/toon-format/spec), a table format that costs fewer tokens than JSON.

A method, a constructor, or a class property holding an arrow function or a function expression gets a row of its own when it spans three lines or more and its class is declared at the top level of the file or exported. So does a method, getter, setter, or a property holding an arrow function or a function expression, of an object literal that is assigned to a top-level `const`, `let` or `var`, or is the `export default` value, also through `as`, `satisfies` or parentheses. Its `parent` is `class Foo` for a class member and `variable api` for the members of `const api = { ... }` (`other default` for `export default { ... }`), and `nodeKind` is `method`, `constructor` or `property`; `parent` is `null` on every other row. A line that matched inside a member is shown on the member's row only, and the class's or object's row appears when it has a match outside its members, such as its name. Shorter members, members of an anonymous class, of an interface, of an object literal anywhere else (inside a function, a call or `module.exports = { ... }`) and functions nested inside a member are still reached through the row that holds them.

The `snippet` is the lines of the block that matched, each with its line number and cut at 160 characters. It shows up to three, then counts the rest (`...and 2 more matching lines`). A block's first line is left out when the signature already shows all of it, so the snippet is `null` when that line is the only match. A first line that says more than the signature stays in, such as a decorator or the start of an arrow function's body. To read the whole block, open the file at `lineStart`–`lineEnd`. Skipped or unreadable files, files read lossily because they are not valid UTF-8, syntax errors and an empty query are reported as `help` lines under the table.

## Writing a query

| You write | It means |
| --- | --- |
| `format price` | Code that mentions either word, ranked by how well it matches |
| `"format price"` | The exact phrase |
| `+cart` | Files must mention `cart` |
| `-Button` | Skip files that mention `Button` |
| `regex:use[A-Z]\w+` | Lines matching a regular expression |
| `regex:"a pattern with spaces"` | A regular expression with whitespace, quoted. `+` and `-` work in front of `regex:` too |
| `--exact` | Treat the whole query as one literal, operators and quotes included |

Matching ignores case. A `-term` removes whole files, not single results. If one query mixes terms, `+cart -Button` finds code in files that mention `cart` but never `Button`:

```
$ monokl search "+cart -Button" .
search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:
  src/hooks/useCart.ts,11,25,function,null,function useCart(),"12: const [items, setItems] = useState<CartItem[]>([]);\n14: const addItem = (item: CartItem): ValidationResult => {",1,0.453107
  src/hooks/useCart.ts,5,9,interface,null,null,"5: export interface CartItem {",2,0.30166
```

## Options

| Flag | Default | What it does |
| --- | --- | --- |
| `--max-results N` | 50 | Most results to print |
| `--max-tokens N` | 20,000 | Most estimated tokens (one per three bytes) across the printed rows. The cut is a prefix in rank order and the output says how many rows were cut |
| `--tests` | off | Also search test files: anything under `test/`, `tests/`, `__tests__/` or `spec/`, and `*.test.*` and `*.spec.*` in any searchable extension (`ts`, `tsx`, `js`, `jsx`, `mjs`, `cjs`, `mts`, `cts`). Also searches Rust files named `tests.rs`, `test.rs`, `*_tests.rs` or `*_test.rs` (the usual out-of-line `mod tests;`) and Rust items under `#[cfg(test)]` or `#[test]` |
| `--no-ignore` | off | Also search hidden files and files that `.gitignore` or `.ignore` exclude, in the search path or its parent directories |
| `--language L` | all | Search one language only: `typescript`, `javascript` or `rust`, chosen by file extension: `.ts .tsx .mts .cts`, `.js .jsx .mjs .cjs` or `.rs` (`python` is accepted but matches nothing yet) |

## Errors and exit codes

- **0:** the search ran, even if nothing matched (you get `search: []`).
- **1:** the search failed, for example an invalid regular expression or a directory that doesn't exist. The error is printed on stderr in the same compact format, with the cause.
- **2:** the command line itself was wrong.

## Language support

| Language | Status |
| --- | --- |
| TypeScript, JavaScript (`.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`, `.mts`, `.cts`; not `.d.ts`) | Search works today |
| Rust (`.rs`) | Search works today. Functions, structs, unions, enums, traits, type aliases, consts and statics, `macro_rules!` macros and inline modules are blocks, as are `impl` blocks and the methods in them and in traits (when 3 lines or more), item-position macro calls (`thread_local! { ... }`, `lazy_static! { ... }`) and `extern` blocks; modules declared in another file, `use` declarations and `extern crate` are not |
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
