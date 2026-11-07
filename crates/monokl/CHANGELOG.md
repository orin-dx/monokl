# monokl

## 0.5.0

- The library no longer exposes types for commands that do not exist yet, and its core result types no longer derive `Serialize`; the `monokl search` output is unchanged.

  - **Result types.** `CodeBlock`, `RankedBlock`, `SearchResponse`, `Diagnostic`, `SymbolEntry` and the other `monokl::types` structs and enums lose their `Serialize` and `Deserialize` derives. `SearchResponse` loses `truncation_marker`; read `truncated` and `total_blocks_before_truncation`.
  - **Removed types.** `SearchOptions`, `SearchLimits` (and `Budget::from(&SearchLimits)`), `SymbolsResult`, `DependentsResult`, `ExtractRequest`, `WorkspaceOptions`, `TsconfigMode`, `RustPathAnchor`, `DependencyTarget::RustPath` and `Namespace`, `BindingKind::NamespaceWide`, and the `Rust`, `Python`, `Go` and `Java` variants of `LangData`. `LangData::ts` now returns `&TsData`. `output::agent::visibility_str` is gone too.
  - **Removed errors.** `MonoklError` keeps `Io`, `Walk`, `RegexBuild`, `NonUtf8Path` and `TooManyTerms`; `FileIoError` keeps `Read`. `ToDomainError::to_domain_error` no longer takes an `ErrorContext`.
- `monokl search` now finds code in files that are not valid UTF-8 instead of dropping them.

  - **Invalid bytes are replaced.** Each invalid byte sequence is read as U+FFFD, so the file is searched and its blocks are returned as usual.
  - **The output says so.** A `help` line counts the files read this way, for example `1 file read lossily: invalid UTF-8 replaced with U+FFFD: src/legacy.ts`, and names up to three of them.
- `monokl search` now honors the `.gitignore` and `.ignore` files of parent directories when the search path is inside a git repository.

  - **Scope.** Directories from the repository root (the nearest ancestor with a `.git` entry) down to the search path's parent are read, and nothing above the repository root. Rules inside the search path win over parent rules, as in ripgrep.
  - **No repository.** Without a `.git` ancestor no parent ignore file is read, as before.
  - **Opt out.** `--no-ignore` skips parent ignore files too.

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

