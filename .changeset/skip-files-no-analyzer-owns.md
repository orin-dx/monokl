---
monokl: patch
---

`monokl search` is faster when the search root holds many files that are not TypeScript or JavaScript: it no longer opens them. On the TypeScript compiler's `tests/baselines/reference` (54,009 files, 18,171 of them source) a path-restricted search reads a third of the files.

- **File counts.** `files_total` and the file ceiling now count only files an analyzer owns, and with `--language` only files of that language. Before, a Markdown, JSON or `.d.ts` file with a match, or a TypeScript file under `--language javascript`, was counted and could use up the ceiling, so the "files matched but hits were kept for only the first N" warning could appear when the source files alone were under it. It now appears only when they are over it.
- **Diagnostics.** A file no analyzer owns that could not be read, or whose path is not UTF-8, is no longer reported, because it was never going to be searched.
- **Unchanged.** Rows, ranks, `--tests` and test-path rules, `.d.ts`, `.d.mts` and `.d.cts` exclusion, and `-term` exclusion (a file no analyzer owns was never a candidate for it).
- **Library.** New `text_search::search_files_where` takes a predicate asked before a file is opened. `search_files` is unchanged.
