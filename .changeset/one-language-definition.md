---
monokl: minor
---

`--language python` is now a usage error. It exits 2 and lists the valid values (`typescript`, `javascript`, `rust`). Before, it exited 0 with `language Python not supported in v1` and no results, because no Python analyzer exists.

- **One definition.** Which extensions belong to a language and which files are tests is defined once, in `analysis/lang.rs`. Before, it was copied into text search, the pipeline's `--language` filter, both analyzers, ranking and the comparison scripts. Search output is unchanged: stdout, stderr and exit code were identical on 235 queries over the pinned repositories, against the previous build.
- **Library.** `analysis::Language` replaces both `types::Language` and `analysis::LanguageId`, and has no `Python` variant. It gains `Language::of(path)`, `rules()`, `is_test_file(path)` and `ALL`; `analysis::lang::is_test_path` is the test-file rule text search applies. `LanguageAnalyzer::languages` returns `&[Language]`, and `TsAnalyzer` reports `TypeScript` and `JavaScript`.
- **Scripts.** `scripts/rg-compare.py` and `scripts/bench.py` read the rules from the hidden `monokl language-rules` command instead of mirroring them.

Breaking: `--language python` is now a usage error (exit 2); use `typescript`, `javascript` or `rust`.
