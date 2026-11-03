# monokl

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

