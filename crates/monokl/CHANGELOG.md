# monokl

## 0.8.0

_2026-11-21 · [compare](https://github.com/orin-dx/monokl/compare/monokl@0.7.0...monokl@0.8.0)_

### Minor Changes

- **A search row drops `rank` and shows `relevance` instead of `finalScore`.** Each `monokl search` answer costs about 9% fewer tokens (measured on 222 sample questions) with the same results.

  - The row has 8 columns, ending in `relevance`: `file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance`.
  - `rank` is gone. Rows are in rank order, so a row's position is its rank.
  - `relevance` is the row's score as a whole percent of the top row (top row 100). Compare rows within one query, not across queries. With `--offset` it still measures against the top row of the whole result, so the first row of page 2 is not 100.
  - It lines up with `--min-score`: the paging hint now reads `--min-score 0 for the 2 rows below relevance 30`.

  **Breaking:** A row had 9 cells and now has 8. `rank` and `finalScore` (a float) are gone, and `relevance` (an integer from 0 to 100) is last.

  **Action:** Read cells by header name, not by position, and use row order where you used `rank`. The header changes from `…,snippet,rank,finalScore` to `…,snippet,relevance`, and there is no flag for the old columns. For library users: set `top_score` on a `SearchResponse` you build (`apply_budget` does) and `min_score` on a `SearchView` (`SearchView::default()` has it); `SearchView` is no longer `Eq`.
- **A cut cell now ends in `…` and is listed in a `truncated` block.** You can tell a cut cell from a whole one and see how much was lost. Only cells over 200 characters are cut, in practice a long `symbolSignature`: about 1 search in 10 on 222 sample questions.

  - The cell keeps 199 characters and ends in `…`. It used to end in `(N more chars)`.
  - After the rows, `truncated[N]{row,field,removed}:` lists each cut cell: `row` is 0-based, `field` is the column, `removed` is the number of characters cut.
  - Rows without a cut cell are unchanged.

  **Breaking:** A cut cell ends in `…`, not `(N more chars)`, and a `truncated[N]{row,field,removed}:` block follows the rows when a cell was cut.

  **Action:** Read the `truncated` block instead of parsing `(N more chars)`. If you take every indented line of the output as a row, read only the indented lines directly under the `search[N]{...}:` header, because the `truncated` block's lines are indented too. For library users: `ToDomainError::to_domain_error` returns the `DomainError` of `michi-core` 0.10.0, so code that names michi types needs that version.

## 0.7.0

_2026-11-16 · [compare](https://github.com/orin-dx/monokl/compare/monokl@0.6.0...monokl@0.7.0)_

### Minor Changes

- **`monokl search` prints less by default.** The default output is the rows that matter, with one snippet line each, so an answer costs about a fifth of the bytes. On the 222 rows of the harness the median output falls from 10,012 to 1,567 bytes, the 90th percentile from 13,351 to 3,997 and the total from 1,714,350 to 420,946, and every harness score stays at its baseline.

  - **Relevance cut.** A row is kept when its `finalScore` is at least 0.3 times the top row's (`--min-score FRACTION`, `0` turns the cut off). The first 8 rows are always kept when that many exist. `--max-results` caps the result on top of that, now 20 by default (was 50), and it wins over the floor of 8. `--min-score` takes 0 to 1; anything else is a usage error (exit 2).
  - **Snippet.** Each row shows its first matching line and counts the rest (`...and 2 more matching lines`). `--snippet-lines N` shows 0 to 3 lines (3 is the old snippet). `0` prints no snippet, so a variable or type row, which has no signature, shows no declaration text.
  - **Help line.** When rows were cut the help names the flag that brings them back: `showing 9 of 37; --min-score 0 or --max-results N for more, or refine with +term/-term`. It names `--max-tokens N` when the tokens ran out first. There is no `--offset` yet.
  - **Parameters.** Chosen from a grid of 18 settings (floor 3, 5 or 8; fraction 0.15, 0.2 or 0.3; cap 15 or 20) as the smallest total output that loses no harness question against the baseline. The first choice, floor 3 and cap 15, lost a member and a Rust question.
  - **Not measured.** Whether a model finds the answer in the leaner rows, and what snippet lines 2 and 3 are worth when an agent reads for context instead of answering in one turn.

  **Breaking:** The default output changed: at most 20 rows (was 50), none below 0.3 times the top score except the first 8, and one snippet line per row (was up to 3). `SearchResponse` has a new public field, `blocks_cut_by_relevance`, `Budget` has a new public field, `min_score` (and no longer implements `Eq`), `render_search` takes a `&SearchView` and `search_row_texts` a snippet length. The truncation help line is reworded.

  **Action:** To get the earlier output, run `monokl search <query> --min-score 0 --max-results 50 --snippet-lines 3`. Code that builds a `SearchResponse` or a `Budget`, or calls `render_search` or `search_row_texts`, must pass the new values: `Budget::default()` carries the new defaults, and `SearchView::default()` the new snippet length.
- **A Rust row costs fewer bytes.** On the 222 rows of the harness the total output falls from 420,764 to 383,506 bytes (8.9%). The fd Rust median falls from 1,463 to 1,322 bytes (p90 3,140 to 2,696) and the held-out Rust median from 3,258 to 2,792 (p90 4,709 to 4,058). Every harness score and every one-turn answer count stays at its baseline.

  - **Snippet.** A matched code line is shown before a comment or attribute line: a line starting with `//`, `/*`, `*/`, `* `, `#[` or `#![` (a Rust doc comment, an attribute, a JSDoc line, a comment in a body) is shown only when too few code lines matched or it is the only match. 59% of the Rust rows with a snippet showed a doc comment or an attribute. The shown lines keep their order in the block. TypeScript follows the same rule.
  - **Rust signature.** `symbolSignature` ends before the `where` clause and says `where …` in its place, so `impl<L, M, S, F> Debug for WithGracefulShutdown<L, M, S, F> where L: Debug + 'static, M: Debug, ...` becomes `impl<L, M, S, F> Debug for WithGracefulShutdown<L, M, S, F> where …`. A line of the clause that matched is a snippet line, after the code lines. The cut is where the analyzer's syntax tree puts the clause, so a `where` in a comment or a string is not one. Ranking still reads the whole signature.
  - **More lines marker.** The count of snippet lines past the shown ones reads `+3 more` (was `...and 3 more matching lines`).
  - **Not changed, with the numbers.** No signature starts with an attribute or a doc comment already. The `parent` column is `kind name` and costs 5% of the bytes. Test-path rows are 1.4% of the rows. Capping the signature length and rounding `finalScore` were measured and not taken (docs/spec/05-research-and-decisions.md, section 48).
  - **Still above the rg policy.** The fd Rust median is 1,322 bytes against 731. What remains is the floor of 8 rows.

  **Breaking:** `CodeBlock` has a new public field, `where_start`, the offset of the `where` keyword of a Rust declaration's where clause (`None` when there is none, and for TypeScript and JavaScript); code that builds a `CodeBlock` must set it. The text of a row changed. A Rust row's `symbolSignature` drops the where clause and ends in ` where …`; the snippet line is a code line where one matched, and for a Rust item a where-clause line can now be a snippet line; the marker is `+N more` instead of `...and N more matching line(s)`.

  **Action:** Set `where_start: None` where you build a `CodeBlock`. Code that parses the marker, or reads a bound out of a Rust `symbolSignature`, must change: the marker is `+N more`, and a where clause is in the file at `lineStart`–`lineEnd`. `search_row_texts` and `render_search` keep their signatures. There is no flag for the old rows.
- **Search ranks a declaration by its abbreviated name, and ranks `impl` blocks lower.** Scores and row order change; no API or wire shape changes.

  - **Abbreviated names.** A name token of at least 3 characters now matches a query term that starts with it, so `ListenerExt` holds `listener` and `extension`, and `reqIdGenFactory` holds `request`, `id`, `generator` and `factory`. A token never matches a term it is longer than (`parser` is not `parse`), and a token of 1 or 2 characters has to equal the term. A prefix match counts as much as an equal one in the name boost, including the bonus for a name that is exactly the query. Measured alone: answerable in the first three rows 204 of 222 to 206, held-out Rust 34 of 36 to 35 (axum `listener extension` from absent to rank 2), no harness set loses a hit.
  - **Impl prior.** The multiplier for a Rust `impl` block falls from 1.1 to 0.9, so the struct or method an `impl` wraps outranks the block around it. Measured alone it changes no harness count; it puts `OwnerFilter` and `is_within` ahead of their `impl` blocks and, with the prefix rule, removes a regression the prefix rule causes on fd `directory entry stripped path`. Dropping it was tried and measured worse: the prefix rule alone changes 6 first rows (3 better, 3 worse), both together change 8 (5 better, 1 equal, 2 worse).
  - **Tradeoff.** The prefix rule that fixes `ListenerExt` also matches where it should not. `RequestPartsExt` now matches `extractor` through `ext` and scores as exactly the query, so axum `extractor from request parts` no longer has `FromRequestParts` in the first five rows, and zod `global registry metadata` is worse (`meta` starts `metadata`). 47 of the 48 queries of the earlier blind review keep their first row.
- **`monokl search --offset N` prints the next page of rows.** It skips the first N ranked rows and prints the next page under the same cut and caps, so the rows the lean default hides are reachable without paying for the first ones again. The relevance cut stays relative to the top row of the whole result, so page 1 plus page 2 equals one run with a larger `--max-results`. `--max-tokens` bounds the page that is returned, not the rows skipped. An offset past the last row prints no rows and the `totalCount`, and exits 0; a non-number is a usage error (exit 2).

  - **Help line.** When rows remain it names the next value: `showing 8 of 31; next: --offset 8, or refine with +term/-term`. It replaces the `--max-results N` and `--max-tokens N` advice, which re-paid for the rows already seen; `--min-score 0 for the 20 rows below the cut` is named when the relevance cut dropped rows (the `of 31` counts those rows too), and offsets keep counting the same ranking with it. An empty page the caps caused (a row larger than the token cap) still names the cap.
  - **Library.** `Budget` and `SearchResponse` each have a new public field, `offset`, so code that builds them must set it (`Budget::default()` has 0).
  - **Stateless.** Every call re-runs the search. Pages are exact for an unchanged tree.

### Patch Changes

- **`monokl search` opens each matching source file once.** Text search keeps the text of a matching file of at most 256 KiB, up to 16 MiB over all files, and analysis uses it instead of opening the file again. On a tree of 20,000 small matching files the search takes about a quarter less time, and peak memory rises by at most that 16 MiB. The output is byte-identical. A file over 256 KiB, or past the 16 MiB, is opened again as before.

## 0.6.0

_2026-11-15 · [compare](https://github.com/orin-dx/monokl/compare/monokl@0.5.0...monokl@0.6.0)_

### Minor Changes

- Blocks carry the byte span they cover and the position of the block that encloses them, so later stages can tell blocks apart by where they are and not by their text.

  - **Span.** `CodeBlock::span` is the byte range of the analyzed source the block covers, so `source[span] == code`. Offsets count bytes of the text the analyzer received, which is the lossy UTF-8 decoding of a file with invalid bytes.
  - **Parent.** `CodeBlock::parent_index` is the position of the enclosing block in `FileAnalysis::blocks` followed by `FileAnalysis::test_blocks`. It is set exactly when `parent` is, for the members of a class, object literal, `impl`, trait or inline module.
  - **No change to results.** Nothing reads either field yet: `monokl search` output and the harness scores are unchanged.

  **Breaking:** `CodeBlock` has two new public fields. Code that builds one must set `span` and `parent_index`.
- Blocks carry the name their analyzer read from the syntax tree, and the name boost in `monokl search` uses it instead of cutting the signature text.

  - **Name from the tree.** `CodeBlock::name` is the declared identifier of a function, class, interface, type alias, enum, namespace, method, property or constant, and the target of an `impl` (`Trait for Target` for a trait impl). It is `None` for a re-export, an anonymous default export, a destructuring declaration, a macro call and an `extern` block, which get no name boost.
  - **Scores can change.** `finalScore` differs only where the old text head held a word the name does not, such as a modifier (`get`, `static`) or a variable's type, and for `export default Button;`, which declares no name. No column is added and the harness scores are unchanged: identifier 58/59, questions 17/18, members 15/16, Rust fd 22/24, held-out 23/24 and 12/12.
  - **Library.** `CodeBlock` has a new public field, so code that builds one must set `name`. `rank::name_match_boost` reads `block.name`, not the signature or the first line of `code`.

  Breaking: construct `CodeBlock` with a `name` field.
- **A match is credited to the block it sits in.** `monokl search` decides which block holds a matching line by where in the line each match starts, not by comparing the text of the line, so a match is never given to a block that only shares its line, and a line a block covers is never lost.

  - **Why it matters.** A class or object keeps a match that is its own even when a method on the same line matches another term, and blocks that share a line (`} b() {`, `}, // note`, `const a = 1; const needle = 2;`) are told apart by the byte where the match begins. A test function and the module that holds it are never both credited.
  - **Terms are searched one by one.** `regex:class.* m` finds the method `m` that starts on the line `class.*` matches; the terms used to be joined, and the first match hid the second.
  - **End of a declaration.** `regex:$` and other matches at the end of a declaration belong to it.
  - **Output.** Same columns and the same order. On the pinned repositories the shown rows are identical, `totalCount` is 1 to 3 lower in a few queries, and `finalScore` moves in its later digits where fewer blocks change the document frequencies. Harness scores are unchanged.
  - **A shrunken file.** Matching lines past the end of a file that changed after it was searched are counted in a warning.

  **Breaking:** `LineHit` has a new public field, `match_starts`, the offsets in the hit's text at which a match starts. Code that builds a `LineHit` must set it, and `text_search` fills it.
- `monokl search` now ranks the declaration a query names above longer names that merely contain its words, so `use watch` style questions find `useWatch` and `create form control` finds `createFormControl`.

  - **Name match.** The name boost compares the query words with the words of the block's declared name, with stop words such as `use` and `from` left out of the name. It rises with the share of the query the name holds, falls with the share of the name the query does not ask for, and is 7.0 when the two are the same words. Before it was `1 + 2 * share of the query in the name`, 1.0 to 3.0; it is now 1.0 to 7.0.
  - **Scores and order change.** `finalScore` values and row order change on 110 of the 250 pinned-harness queries, 48 of them in the first row. The answer file of a multi-word question is in the first three files for 17 of 18 questions (unchanged), a declaration for 59 of 59 identifiers (was 58), and 14 of 14 in the report-only react-hook-form set (was 12). No earlier hit is lost.
  - **Library.** `rank::name_match_boost` returns 1.0 to 7.0 instead of 1.0 to 3.0; its signature is unchanged. This is a behavior change and not a breaking API change, so the bump is minor, as for the earlier name-boost change, and not patch.
- `monokl search` is faster on trees with many matching files: 20,000 one-line files take 545 ms instead of 759 ms (about 28% less), and a broad query over zod 31 ms instead of 37 ms. Output is byte-identical.

  - **One parse thread per worker, not per file.** Analysis used to start a thread with a 256 MiB stack for every file, which cost 3.5 s of CPU against 35 ms of parsing over 20,000 files. A search now starts its analysis threads once with that stack, and only as many as it has files to analyze: a query that matches nothing starts none. Each parse still runs on a stack of exactly that size, behind the same nesting scan and 4 MiB cap, and a parse made outside a search still gets a thread of its own.
  - **Ranking runs in parallel.** Scoring and sorting spent 42 ms of that run on one thread and now take 6 ms. `rank::rank_blocks` now requires its hasher `S` to be `Sync`; every standard and `FxHash` hasher is.
  - **Larger batches.** A search hands out 64 files per thread to text search and 32 to analysis, up from 16 and 8, so threads wait less for the slowest file of a batch. After the first batch a `--timeout` is checked every 576 files on an 18-core machine instead of every 144.

  **Breaking:** `rank::rank_blocks<S>` now requires `S: BuildHasher + Sync`. A caller that passes a `df` map with a hasher that is not `Sync` must switch to one that is.
- **Two blocks that share a line are no longer taken for one.** `monokl search` removes a block that repeats a better one by the bytes the two share, not the lines, and decides what a snippet leaves out by where the signature ends, not by comparing text.

  - **Why it matters.** `const first = 1; const second = 2;` and `fn a() {} fn b() {}` each used to give one row, because both blocks claimed line 1 and the second counted as a copy of the first. Each block that holds a match now gets its row. A block inside another, or equal to it, is still dropped as before.
  - **Snippets.** A matching line is left out when the signature column already shows all of it, wherever that line is in the block: the lines of a multi-line signature, and the signature line of a Rust item below its attributes or doc comments, no longer repeat the signature in the snippet. A line that says more than the signature stays: a decorator, the start of an arrow function's expression body, or a body on the same line.
  - **Output.** Same columns and the same order. A search can return more rows where blocks shared a line, and fewer snippet lines where the signature spans several.

  **Breaking:** `CodeBlock` has a new public field, `signature_span`, the bytes of the declaration's head that `symbol_signature` is the one-line form of. Code that builds a `CodeBlock` must set it, with `None` for a block that has no signature.

  **Breaking:** `dedup_blocks` compares `CodeBlock::span`, not `line_start` and `line_end`. A caller that built blocks with an empty span (`0..0`) now has every one of them kept; set each block's real span.

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

