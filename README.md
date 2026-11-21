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
- **Hard to break.** A file with syntax errors is searched for whatever parsed. A pathologically deep file is reported as degraded instead of crashing the search. An unreadable directory doesn't stop the rest of the search.

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
$ monokl search "format price" src/
search[8]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:
  src/utils/format.ts,1,3,function,null,"function formatCurrency(amount: number, currency: string = 'USD'): string","2: return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);",100
  src/utils/validation.ts,8,12,function,null,"function validatePrice(price: number): ValidationResult","9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\n+1 more",70
  src/index.ts,3,3,other,null,null,"3: export { formatCurrency, truncateString } from './utils/format';",23
  src/utils/validation.ts,14,14,typeAlias,null,null,"14: export type PriceValidator = (price: number) => ValidationResult;",18
  src/components/Button.tsx,11,18,function,null,"function Button({ label, price, onClick, disabled }: ButtonProps)","12: const priceValid = price === undefined || validatePrice(price).valid;\n+1 more",13
  src/hooks/useCart.ts,11,25,function,null,function useCart(),"15: const result = validatePrice(item.price);",7
  src/hooks/useCart.ts,5,9,interface,null,null,"8: price: number;",7
  src/components/Button.tsx,4,9,interface,null,null,"6: price?: number;",7
totalCount: 10
help[1]: "showing 8 of 10; --min-score 0 for the 2 rows below relevance 30, or refine with +term/-term"
```

Each row is one match: where it is (`file`, `lineStart`, `lineEnd`), what it is (`nodeKind`: function, class, method, interface, type alias and so on), what holds it (`parent`), its signature, the lines in it that matched (`snippet`), and its `relevance`: the row's score as a whole percent of the top row's score, so the top row is 100 and a row at 50 scores half as well. Rows are in rank order, best first. `relevance` compares the rows of one query; it is not a number to compare across queries. Output uses [TOON](https://github.com/toon-format/spec), a table format that costs fewer tokens than JSON.

**How many rows.** The default output is lean. A row is kept when it scores at least 0.3 times the top row (`--min-score`), which is a `relevance` of 30, the first eight rows are always kept, and at most 20 are printed (`--max-results`), so the cap wins when it is lower than eight. When rows were cut, `totalCount` holds how many matched and the `help` line names the flag that brings them back: `showing 8 of 10; --min-score 0 for the 2 rows below relevance 30, or refine with +term/-term`. To page through rows the cap or the budget left out, add `--offset N`: it skips the first N rows of the same ranking and prints the next page under the same cut and caps, and a help line such as `showing 8 of 31; next: --offset 8, or refine with +term/-term` names the next value. To get the output of earlier versions, which printed up to 50 rows with up to three snippet lines each, run `--min-score 0 --max-results 50 --snippet-lines 3`.

A method, a constructor, or a class property holding an arrow function or a function expression gets a row of its own when it spans three lines or more and its class is declared at the top level of the file or exported. So does a method, getter, setter, or a property holding an arrow function or a function expression, of an object literal that is assigned to a top-level `const`, `let` or `var`, or is the `export default` value, also through `as`, `satisfies` or parentheses. Its `parent` is `class Foo` for a class member and `variable api` for the members of `const api = { ... }` (`other default` for `export default { ... }`), and `nodeKind` is `method`, `constructor` or `property`; `parent` is `null` on every other row. A line that matched inside a member is shown on the member's row only, and the class's or object's row appears when it has a match outside its members, such as its name. Shorter members, members of an anonymous class, of an interface, of an object literal anywhere else (inside a function, a call or `module.exports = { ... }`) and functions nested inside a member are still reached through the row that holds them.

The `snippet` is the lines of the block that matched, each with its line number and cut at 160 characters. It shows the first matching line by default, then counts the rest (`+1 more`); `--snippet-lines N` shows up to three. A line is left out when the signature column already shows all of it, wherever it is in the block (the lines of a signature that spans several, or the signature line below a Rust item's attributes or doc comments), so the snippet is `null` when the signature is the only match. A line that says more than the signature stays in, such as a decorator, the start of an arrow function's body or a body on the same line. A code line is shown before a comment or attribute line (a Rust doc comment, a `#[...]` attribute), which is shown only when no code line matched or the line limit leaves room. A Rust signature ends before its `where` clause and says `where …` in its place; a line of the clause that matched is a snippet line, after the code lines. To read the whole block, open the file at `lineStart`–`lineEnd`. Under the table, `help` lines report files that were skipped, degraded (syntax errors, too deep to parse) or read lossily (invalid UTF-8), a partial result, hit lines that were cut, and a query with no positive terms.

A cell over 200 characters, in practice a long signature, keeps 199 of them and ends in `…`, and a `truncated[N]{row,field,removed}:` block after the rows lists each cut: the 0-based row of the page, the column and how many characters were removed.

A Rust example, with `src/ledger.rs` holding `quote` (a `where` clause, a doc comment and an attribute) and `total`, and `src/long.rs` holding `settle_ledger`, a function with fourteen `u64` arguments:

```
$ monokl search "quote ledger" .
search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:
  src/ledger.rs,1,8,function,null,"pub fn quote<T>(ledger: T) -> u64 where …","7: ledger.total()\n+2 more",100
  src/long.rs,1,3,function,null,"pub fn settle_ledger(argument_0: u64, argument_1: u64, argument_2: u64, argument_3: u64, argument_4: u64, argument_5: u64, argument_6: u64, argument_7: u64, argument_8: u64, argument_9: u64, argument…",null,7
  src/ledger.rs,10,13,function,null,"pub fn total(ledger: &Ledger) -> u64","12: ledger.sum()\n+1 more",7
truncated[1]{row,field,removed}:
  1,symbolSignature,70
```

`quote` starts at its attribute and doc comment (line 1), its signature stops before `where`, and the snippet shows the matching body line, not the doc comment.

## For agents

1. Call `monokl search "<words from the task>" <dir>` once. Each row is a whole function, class or type with its file and line range, so the table usually answers without opening files.
2. Read `relevance`: the top row is 100 and a row at 20 scores a fifth as well. Rows far below the top are usually noise.
3. When `help` says `next: --offset N`, run the same query with `--offset N` for the next page. Pages of one query do not overlap.
4. When `help` says `--min-score 0 for the N rows below relevance P`, add `--min-score 0` to see those rows too. The order and the offsets stay the same.
5. Narrow with `+term`, `-term` or `--language`. Add `--tests` to search test files.
6. Open the file at `lineStart`-`lineEnd` only for the row you need. `search: []` with exit 0 means nothing matched.

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
search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:
  src/hooks/useCart.ts,11,25,function,null,function useCart(),"12: const [items, setItems] = useState<CartItem[]>([]);\n+1 more",100
  src/hooks/useCart.ts,5,9,interface,null,null,"5: export interface CartItem {",25
```

## Options

| Flag | Default | What it does |
| --- | --- | --- |
| `--min-score FRACTION` | 0.3 | Drop the rows scoring below this fraction of the top row's score, which are the rows whose `relevance` is below 100 times it (a row within half a point of the line rounds up onto it), but never one of the first eight. `0` turns the cut off; `1` keeps only the rows that tie with the top. Outside 0 to 1 is a usage error (exit 2) |
| `--max-results N` | 20 | Most results to print. It applies after `--min-score` and wins over its floor of eight rows |
| `--offset N` | 0 | Skip the first N rows that pass the cut and print the next page, still bounded by `--max-results` and `--max-tokens`. The cut is measured against the top row of the whole result, so pages of one query do not overlap or skip rows. An offset past the last row prints no rows and a `totalCount`, and exits 0; a non-number is a usage error (exit 2) |
| `--max-tokens N` | 20,000 | Most estimated tokens (one per three bytes) across the printed rows. The cut is a prefix in rank order and the output says how many rows were cut |
| `--snippet-lines N` | 1 | Most matching lines each row's `snippet` shows, 0 to 3; the rest are counted. `0` prints no snippet, so a variable or type row, which has no signature, shows no declaration text. Above 3 is a usage error (exit 2) |
| `--exact` | off | Treat the whole query as one literal term, operators and quotes included |
| `--tests` | off | Also search test files: anything under `test/`, `tests/`, `__tests__/` or `spec/`, and `*.test.*` and `*.spec.*` in any searchable extension (`ts`, `tsx`, `js`, `jsx`, `mjs`, `cjs`, `mts`, `cts`). Also searches Rust files named `tests.rs`, `test.rs`, `*_tests.rs` or `*_test.rs` (the usual out-of-line `mod tests;`) and Rust items under `#[cfg(test)]` or `#[test]` |
| `--no-ignore` | off | Also search hidden files and files that `.gitignore` or `.ignore` exclude, in the search path or its parent directories |
| `--language L` | all | Search one language only: `typescript`, `javascript` or `rust`, chosen by file extension: `.ts .tsx .mts .cts`, `.js .jsx .mjs .cjs` or `.rs`. Any other value is a usage error (exit 2) that lists these three |
| `--timeout SECONDS` | 30 | Stop after this many seconds and print what was found so far. `0` means no limit. Fractions are accepted (`0.5`) |

## Limits

- **Time.** When `--timeout` expires the output says `partial result: 30 s timeout expired after analyzing 120 of 900 files` (or `searching`, when the text search itself ran out of time). The exit code is still 0. A partial result is the only output that can differ between runs, because which files are reached depends on how fast the machine is. At least one batch of files is always processed, so a very small timeout still returns something. A `-term` exclusion applies only to the files that were reached. The directory walk that lists the files is not interrupted.
- **File size.** A file over 4 MiB is not read. The output says `2 files skipped: over 4 MiB: a.ts, b.ts`. Source that large is generated or bundled; reading it costs about 2.5 times its size in memory.
- **Nesting.** A file nested deeper than 16,384 levels (TypeScript, JavaScript) or 256 (Rust) is not parsed. The output reports it as degraded.
- **Files.** At most 100,000 matching files are examined. The output says how many matched and suggests narrowing the query or path.
- **Line length.** Only the first 4,096 bytes of a matching line are kept for matching terms against the line. A longer line still matches as a whole, and the output counts the lines it cut. A required term (`+term`) that appears only past that point in a line is not seen.

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
| [`docs/architecture/`](docs/architecture/README.md) | One topic doc per subsystem |
| [`docs/adr/`](docs/adr/README.md) | Architecture decision records |
| [`docs/spec/`](docs/spec/README.md) | The full specification, including what's still to build |
| [AGENTS.md](AGENTS.md) | Guidance for AI coding agents working in this repo |

## Name

Monokl (MON-oh-kl) — from _monocle_, a single precision lens for close examination. The binary is `monokl`.

## License

FSL-1.1-MIT — Functional Source License, converting to MIT two years after each release. See [`LICENSE`](LICENSE).
