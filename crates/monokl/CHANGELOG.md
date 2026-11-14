# monokl

## 0.6.0

_2026-11-14 · [compare](https://github.com/orin-dx/monokl/compare/monokl@0.5.0...monokl@0.6.0)_

### Minor Changes

- Blocks carry the name their analyzer read from the syntax tree, and the name boost in `monokl search` uses it instead of cutting the signature text.

  - **Name from the tree.** `CodeBlock::name` is the declared identifier of a function, class, interface, type alias, enum, namespace, method, property or constant, and the target of an `impl` (`Trait for Target` for a trait impl). It is `None` for a re-export, an anonymous default export, a destructuring declaration, a macro call and an `extern` block, which get no name boost.
  - **Scores can change.** `finalScore` differs only where the old text head held a word the name does not, such as a modifier (`get`, `static`) or a variable's type, and for `export default Button;`, which declares no name. No column is added and the harness scores are unchanged: identifier 58/59, questions 17/18, members 15/16, Rust fd 22/24, held-out 23/24 and 12/12.
  - **Library.** `CodeBlock` has a new public field, so code that builds one must set `name`. `rank::name_match_boost` reads `block.name`, not the signature or the first line of `code`.

  Breaking: construct `CodeBlock` with a `name` field.

## 0.5.0

_2026-11-12 · [compare](https://github.com/orin-dx/monokl/compare/monokl@0.4.0...monokl@0.5.0)_

### Minor Changes

- `monokl search` bounds what it reads and how long it runs, and says when it skipped or cut something.

  - **Size.** A file over 4 MiB is no longer read; the output lists it in one line, `2 files skipped: over 4 MiB: a.ts, b.ts`. Before, such a file was read whole and then reported as too large to parse, so 40 files of 5 MB reached 479 MB of memory and one 300 MB line 752 MB; both now stay at 5 MB.
  - **Lines.** Only the first 4,096 bytes of a matching line are kept for matching terms against the line. The output counts the lines it cut. A `+required` term that appears only past that point in a line is not seen.
  - **`--timeout SECONDS`.** Stops after that long (default 30, `0` for no limit, fractions allowed) and prints what was found, with `partial result: 30 s timeout expired after analyzing 120 of 900 files`. The exit code stays 0. A partial result is the only output that can differ between runs.
  - **Unchanged.** Rows, ranks and diagnostics for every file under 4 MiB with lines under 4,096 bytes. Output was byte-identical on 172 queries over the pinned repositories.
  - **Library.** `TextSearchOptions` gains `max_file_bytes`, `max_line_bytes` and `deadline`, and `TextSearchResult` gains `oversize`, `truncated_lines` and `incomplete`. New `pipeline::search_with_timeout`; `search` runs without a deadline.
- `--max-tokens` and `--max-bytes` now count the rows `monokl search` prints, not the code blocks behind them.

  - **Before.** The caps summed each kept block's whole code, so a long function used up the budget even though its row prints only a signature and a few matching lines.
  - **Now.** Each row's printed line is measured, so a cap bounds the output an agent receives. The cut is unchanged: a prefix in rank order, with a `showing N of M` hint, and a first row larger than the cap leaves an empty, truncated result.
  - **Defaults.** Unchanged at 20,000 tokens and 2 MiB. On the pinned repositories the largest output was 11 KB, so the defaults are now a backstop and `--max-results` is the cap that usually applies.
  - **Library.** `budget::apply_budget` takes a function that returns each block's printed row text.
- `monokl search` now returns a class's methods as rows of their own, so a query for a method name shows that method, not the whole class around it.

  - **New rows.** Each method, constructor, getter and setter, and each class property holding an arrow function or a function expression, gets a row when it spans three lines or more and its class is declared at the top level of the file or exported. Its `nodeKind` is `method`, `constructor` or `property`. Members of an anonymous class, interface members and object-literal members are not split out yet.
  - **New `parent` column.** It sits after `nodeKind` and reads `class Foo` on a member row, `null` on every other. Scripts that read rows by position must account for the extra column.
  - **One owner per line.** A line that matched inside a member is credited to the member only. The class keeps its row when it matches outside its members, such as on its name, and drops out when every match is inside a member.
  - **Deduplication.** A member is no longer dropped because the class around it overlaps it; two members, or two blocks without a parent, still dedupe by overlap.
  - **Empty results.** The note now says that top-level declarations and class members are searched.
  - **Library.** `CodeBlock` has a new `parent` field, and `RankedBlock::parent_context` is filled from it.

  Action: a script that reads `monokl search` rows by position must account for the new `parent` column after `nodeKind`.
- The library no longer builds analyzer output that search never read: symbols, dependencies, exports, JSX elements, the blake3 `Digest` (`monokl::digest` is gone, along with the `blake3` and `serde` dependencies), analysis profiles and capabilities. `LanguageAnalyzer` is `languages`, `supports` and `analyze(path, text)`; `FileAnalysis` holds `blocks`, `test_blocks`, `had_parse_errors` and `degradation`. `CodeBlock::matched_keywords`, `ParentContext::line` and `RankedBlock::parent_context` (read `block.parent`) are removed. `monokl search` output is unchanged.

  Breaking: `monokl::digest`, the `blake3` and `serde` dependencies and the analyzer output that search never read (symbols, dependencies, exports, JSX elements, profiles, capabilities) are removed from the library.
- The library no longer exposes types for commands that do not exist yet, and its core result types no longer derive `Serialize`; the `monokl search` output is unchanged.

  - **Result types.** `CodeBlock`, `RankedBlock`, `SearchResponse`, `Diagnostic`, `SymbolEntry` and the other `monokl::types` structs and enums lose their `Serialize` and `Deserialize` derives. `SearchResponse` loses `truncation_marker`; read `truncated` and `total_blocks_before_truncation`.
  - **Removed types.** `SearchOptions`, `SearchLimits` (and `Budget::from(&SearchLimits)`), `SymbolsResult`, `DependentsResult`, `ExtractRequest`, `WorkspaceOptions`, `TsconfigMode`, `RustPathAnchor`, `DependencyTarget::RustPath` and `Namespace`, `BindingKind::NamespaceWide`, and the `Rust`, `Python`, `Go` and `Java` variants of `LangData`. `LangData::ts` now returns `&TsData`. `output::agent::visibility_str` is gone too.
  - **Removed errors.** `MonoklError` keeps `Io`, `Walk`, `RegexBuild`, `NonUtf8Path` and `TooManyTerms`; `FileIoError` keeps `Read`. `ToDomainError::to_domain_error` no longer takes an `ErrorContext`.

  Breaking: the core result types lose `Serialize` and `Deserialize`, and `SearchOptions`, `SearchLimits`, `SymbolsResult`, `DependentsResult`, `ExtractRequest`, `WorkspaceOptions` and the unbuilt-language variants are removed.
- `monokl search` now ranks a block above others when its declared name holds the query words, so a question like `build full path base url` lists `buildFullPath` before code that merely calls it.

  - **Name match.** The name is the block's signature, or for a block without one its first line that is not a comment or decorator, cut before the first `(`, `<`, `{`, `=` or `:`. The score is multiplied by `1 + 2 * matched / query terms`, from 1.0 to 3.0. Parameters, return types and the body do not count.
  - **Scores and order change.** `finalScore` values are higher for blocks that match by name, and rows can reorder. On the pinned-repo harness the answer file of a multi-word question is in the first three files for 17 of 18 questions (was 14), and a declaration is in the first three files for 58 of 59 identifiers (was 56). The median output of a question falls from 9,876 to 9,633 bytes.
  - **Library.** `rank::name_match_boost` is new, and `rank_blocks` multiplies it into `final_score`.
- `monokl search` now returns the functions of an object literal as rows of their own, so a query for `loadUser` shows that method, not the whole `const api = { ... }` around it.

  - **New rows.** Each method, getter and setter, and each property holding an arrow function or a function expression, of an object literal assigned to a top-level `const`, `let` or `var` or used as the `export default` value, gets a row when it spans three lines or more. A literal wrapped in `as`, `satisfies` or parentheses counts. Its `nodeKind` is `method` for methods, getters and setters and `property` for the others. Object literals inside functions, calls and `module.exports = { ... }` are not split out yet.
  - **`parent` column.** It reads `variable api` on a member of `const api = { ... }` and `other default` on a member of `export default { ... }`.
  - **One owner per line.** A line that matched inside a member is credited to the member only. The object keeps its row when it matches outside its members, such as on its name, and drops out when every match is inside a member.
  - **Empty results.** The note now says that top-level declarations and the members of their classes and objects are searched.
- `--language python` is now a usage error. It exits 2 and lists the valid values (`typescript`, `javascript`, `rust`). Before, it exited 0 with `language Python not supported in v1` and no results, because no Python analyzer exists.

  - **One definition.** Which extensions belong to a language and which files are tests is defined once, in `analysis/lang.rs`. Before, it was copied into text search, the pipeline's `--language` filter, both analyzers, ranking and the comparison scripts. Search output is unchanged: stdout, stderr and exit code were identical on 235 queries over the pinned repositories, against the previous build.
  - **Library.** `analysis::Language` replaces both `types::Language` and `analysis::LanguageId`, and has no `Python` variant. It gains `Language::of(path)`, `rules()`, `is_test_file(path)` and `ALL`; `analysis::lang::is_test_path` is the test-file rule text search applies. `LanguageAnalyzer::languages` returns `&[Language]`, and `TsAnalyzer` reports `TypeScript` and `JavaScript`.
  - **Scripts.** `scripts/rg-compare.py` and `scripts/bench.py` read the rules from the hidden `monokl language-rules` command instead of mirroring them.

  Breaking: `--language python` is now a usage error (exit 2); use `typescript`, `javascript` or `rust`.
- The `monokl::query` module is no longer public. It held the query lexer, parser and plan, which only the search pipeline uses; `monokl search` output is unchanged.

  Breaking: the `monokl::query` module is no longer public, so code that imports it must stop.
- `finalScore` in `monokl search` rows no longer includes a constant 1.0, and `RankedBlock` and `SearchResponse` lose fields nothing read.

  - **`finalScore` drops by exactly 1.0.** Every printed block holds a matching line, so the coverage term was always 1.0. Row order is unchanged.
  - **Library.** Removed `rank::coverage_boost`, `RankedBlock::{bm25_score, coverage_boost, node_type_boost}` and `SearchResponse::{total_bytes, total_tokens}`. `SymbolKind` now derives `PartialOrd` and `Ord` in declaration order, which ranking uses as its tie-break.

  Breaking: `finalScore` is now 1.0 lower for every row, so a script that compares it with a fixed threshold must lower the threshold by 1.0.
- `monokl search` now searches Rust (`.rs`) files as AST blocks, like TypeScript and JavaScript.

  - **Blocks.** Functions, structs, unions, enums, traits, type aliases, consts, statics, `macro_rules!` macros, item-position macro calls (`thread_local! { ... }`), `extern` blocks and inline modules, with their attributes and doc comments. `impl` blocks and the methods in them and in traits get their own rows when a method is 3 lines or more, with the `impl` or trait as the parent; a match outside every method is credited to the `impl`. Modules declared in another file, `use` and `extern crate` are not blocks.
  - **Test code.** Items under `#[cfg(test)]` or `#[test]`, including test methods inside a regular `impl` or trait (even a one-line one), and files named `tests.rs`, `test.rs`, `*_tests.rs` or `*_test.rs` are left out unless `--tests` is passed. A match inside a test method is never credited to the enclosing `impl` or trait.
  - **`--language rust`** now selects `.rs` files; before, it matched nothing. A build without the `lang-rust` feature reports `language Rust has no analyzer in this build` instead of an empty result.
  - **Library.** The new `lang-rust` cargo feature (on by default) builds `analysis::rust::RustAnalyzer` on `ra_ap_syntax`. `LangData` gains a `Rust` variant, `FileAnalysis` a `test_blocks` field and `Degradation` a `TooDeep` variant; `SearchRequest::allow_tests` also covers test code inside Rust files.
- `monokl search` rows now show the lines that matched, so you often don't need a second read of the file to see why a block matched.

  - **New `snippet` column.** It sits after `symbolSignature` and holds the matching lines of the block, each with its line number and cut at 160 characters. Scripts that read rows by position must account for the extra column.
  - **Bounded.** It shows up to three lines, then says how many more matched (`...and 2 more matching lines`). It is `null` when nothing beyond the signature matched.
  - **No repeat of the signature.** A block with a signature leaves out its first line; a variable or type alias, which has no signature, shows it.

  Action: a script that reads `monokl search` rows by position must account for the new `snippet` column after `symbolSignature`.
- Removes `--max-bytes`, the `miette` cargo feature and the output-layer projection trait.

  - **`--max-bytes` is gone.** `--max-tokens` counts one token per three bytes, so the 2 MiB byte cap only bound above about 700,000 tokens. Passing `--max-bytes` is now a usage error (exit code 2). `--max-tokens` and `--max-results` are unchanged, and output is byte-identical otherwise.
  - **Library.** `Budget` has no `max_bytes` field, and `budget::apply_budget` takes the printed row of each block as a slice instead of a function. `output::agent` has `render_search` and `search_row_texts` in place of the `ToonProjection` trait, `render_projection` and `search_row_text`; `SymbolKind::as_str` and `DiagnosticKind::verb` replace `symbol_kind_str` and the match in the diagnostics renderer.
  - **No `miette` feature.** `MonoklError` and `FileIoError` no longer derive `miette::Diagnostic`; they carried no diagnostic metadata and the CLI prints errors through michi. `miette` and its dependencies are no longer built.

  Breaking: `--max-bytes` is removed and passing it is now a usage error (exit 2); use `--max-tokens` or `--max-results`.

### Patch Changes

- A `monokl search` row now shows a matching first line in `snippet` whenever the signature leaves part of it out. Before, a hit on `@HostListener('click')`, on `@Dec() prop = () => {` or on `load = () => needle(` left `snippet` null and the matching line appeared nowhere.
- Broad `monokl search` queries are about 30 ms faster because duplicate blocks are found per file instead of against every kept block. Results are unchanged.
- `export default function X() {}`, `export default async function X() {}`, `export default class X {}` and `export default interface X {}` are now ranked as a function, class or interface instead of as an unclassified export, so blocks such as axios `buildFullPath` and `dispatchRequest` rank higher. Default exports of expressions (`export default () => {}`, `export default X`) and `export { x as default }` keep the unclassified kind.
- `monokl search` now finds code in files that are not valid UTF-8 instead of dropping them.

  - **Invalid bytes are replaced.** Each invalid byte sequence is read as U+FFFD, so the file is searched and its blocks are returned as usual.
  - **The output says so.** A `help` line counts the files read this way, for example `1 file read lossily: invalid UTF-8 replaced with U+FFFD: src/legacy.ts`, and names up to three of them.
- `monokl search` now honors the `.gitignore` and `.ignore` files of parent directories when the search path is inside a git repository.

  - **Scope.** Directories from the repository root (the nearest ancestor with a `.git` entry) down to the search path's parent are read, and nothing above the repository root. Rules inside the search path win over parent rules, as in ripgrep.
  - **No repository.** Without a `.git` ancestor no parent ignore file is read, as before.
  - **Opt out.** `--no-ignore` skips parent ignore files too.
- `monokl search` now parses every file on one fixed stack and rejects deeply nested files before parsing them, so what a file costs no longer depends on its contents or on how much memory the machine has.

  - **Large generated files are searched.** The size cap rose from 254 KiB (TypeScript) and 1 MiB (Rust) to 4 MiB. A 3.4 MB generated TypeScript file now returns rows instead of `search: []`.
  - **Deep nesting is reported, not parsed.** A file nested more than 16,384 levels (TypeScript) or 256 levels (Rust) is skipped with a `file nests too deeply to analyze` line. A file of 100,000 nested arrays took 188 MB and 20,000 nested Rust parentheses took a second; both now take under 10 ms.
- `monokl search` is faster on large repositories: files are now searched and analyzed on several threads. Output is unchanged. On zod a search for `ZodType` fell from 35 ms to 21 ms.
- `monokl search` is about 40 ms faster whenever it finds a match, because it no longer loads a tokenizer vocabulary before printing.

  - **`--max-tokens` is an estimate.** It counts one token per three bytes of the printed rows, rounded up, which reads about a tenth above `o200k_base` on real output. The default of 20,000 is unchanged.
  - **Smaller install.** The tokenizer and its vocabulary are no longer part of the binary.
- `monokl search` is faster when the search root holds many files that are not TypeScript or JavaScript: it no longer opens them. On the TypeScript compiler's `tests/baselines/reference` (54,009 files, 18,171 of them source) a path-restricted search reads a third of the files.

  - **File counts.** `files_total` and the file ceiling now count only files an analyzer owns, and with `--language` only files of that language. Before, a Markdown, JSON or `.d.ts` file with a match, or a TypeScript file under `--language javascript`, was counted and could use up the ceiling, so the "files matched but hits were kept for only the first N" warning could appear when the source files alone were under it. It now appears only when they are over it.
  - **Diagnostics.** A file no analyzer owns that could not be read, or whose path is not UTF-8, is no longer reported, because it was never going to be searched.
  - **Unchanged.** Rows, ranks, `--tests` and test-path rules, `.d.ts`, `.d.mts` and `.d.cts` exclusion, and `-term` exclusion (a file no analyzer owns was never a candidate for it).
  - **Library.** New `text_search::search_files_where` takes a predicate asked before a file is opened. `search_files` is unchanged.

## 0.4.0

- `monokl search` now tells you what it skipped instead of printing a bare empty result.

  - **Skipped files are counted.** Unreadable files, files with a line that is not valid UTF-8 and the 100,000-file limit each add a line to the output; files with syntax errors are still searched for what parsed and are reported as partly searched.
  - **Unsupported languages and empty queries say why.** `--language rust` and queries with no positive term (`-foo`, `regex:`, `+`) explain themselves instead of returning `search: []`.
  - **Empty results point you elsewhere.** The note says only top-level declarations are searched, so text in tests, calls and config needs `rg`.
- Deeply nested or deliberately hostile files can no longer crash `monokl search`.

  - **Other results survive.** One such file used to abort the whole search with no output; the rest of the results are now returned.
  - **What you see.** The file is listed as degraded, with the reason.
- File filters now treat every TypeScript and JavaScript extension alike.

  - **Declaration files.** `.d.mts` and `.d.cts` are skipped, like `.d.ts`.
  - **Test files.** `.test` and `.spec` files with `.jsx`, `.mjs`, `.cjs`, `.mts` and `.cts` extensions are now recognised as tests, so `--tests` controls them.
- Files larger than about 254 KiB are no longer parsed, and the output names them.

  - **Why.** It keeps parsing within a safe stack size. About 0.5% of TypeScript and JavaScript files on one developer machine are over the limit, mostly bundler output.
  - **What you see.** The file is listed as degraded with its path, size and the limit, so you can search it with `rg`.
- An empty quoted phrase (`""`) or empty `regex:""` pattern no longer matches every line; a query made only of one gets the usual "no positive terms" note.
- Search output no longer points to commands and options that do not exist.

  - **No `extract` hint.** Every result used to end by suggesting an `extract` command that is not built yet.
  - **Fewer wrong retries.** The error for too many query terms no longer suggests a `maxTerms` option that does not exist.
- `--language javascript` and `--language typescript` now search different files.

  - **TypeScript.** `.ts`, `.tsx`, `.mts` and `.cts`.
  - **JavaScript.** `.js`, `.jsx`, `.mjs` and `.cjs`.
  - **Before.** Both flags searched every file.
- `monokl search` now exits with an error when it cannot write its results, for example to a full disk; a closed pipe such as `| head` still exits 0.
- `+regex:` and `-regex:` now work like `+` and `-` on any term.

  - **`-regex:foo` excludes.** It used to rank the files containing `foo` first; it now drops them.
  - **`+regex:foo` requires.** A file without a match is no longer returned.
  - **Patterns with spaces.** Quote them: `regex:"function use[A-Z]"`.

## 0.3.0

- Prebuilt `monokl` binaries are now attached to each GitHub release, so you can install without a Rust toolchain.

  - **Four targets.** macOS on Apple Silicon and Intel, and Linux on ARM64 and x86-64 (GNU).
  - **Verifiable.** Each archive carries build provenance, and its SHA-256 is published with the release.
  - **Install from source still works.** `cargo install monokl` is unchanged.

## 0.2.0

- First usable release. `monokl search` finds TypeScript and JavaScript code by structure and ranks it for AI coding agents.
  
  - **Search returns whole code units.** Each result is the function, class, interface, type or export that matches, with its signature and line range, not a bare line.
  - **Ranked, best first.** Functions, classes and methods rank above imports and re-exports, and names are split on `camelCase`, `snake_case` and acronyms, so `format price` finds `formatCurrency` and `validatePrice`.
  - **A small query language.** Words, `"quoted phrases"`, `+required`, `-excluded` and `regex:` terms, plus `--exact` for a literal query.
  - **Your project's rules apply.** `.gitignore`, `.ignore`, hidden, test and binary files are skipped by default, with `--tests` and `--no-ignore` to include them.
  - **Sized for an agent's context.** Compact table output with paths relative to where you ran it, capped at 50 results, 20,000 tokens and 2 MiB (`--max-results`, `--max-tokens`, `--max-bytes`). When results are cut, the output says how many matched.
  - **Predictable and robust.** The same search always returns the same output. An invalid pattern fails the whole search with a clear error. Files with syntax errors are still searched for what parsed, very deeply nested files are skipped safely, and an unreadable directory does not stop the search.
  
  TypeScript and JavaScript only for now; the `extract`, `symbols`, `refs`, `definition` and `dependents` commands are not built yet.

