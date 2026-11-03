---
monokl: minor
---

`monokl search` now tells you what it skipped instead of printing a bare empty result.

- **Skipped files are counted.** Unreadable files, files with a line that is not valid UTF-8, files that fail to parse and the 100,000-file limit each add a line to the output.
- **Unsupported languages and empty queries say why.** `--language rust` and queries with no positive term (`-foo`, `regex:`, `+`) explain themselves instead of returning `search: []`.
- **Empty results point you elsewhere.** The note says only top-level declarations are searched, so text in tests, calls and config needs `rg`.
