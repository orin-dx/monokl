---
monokl: patch
---

`monokl search` now parses every file on one fixed stack and rejects deeply nested files before parsing them, so what a file costs no longer depends on its contents or on how much memory the machine has.

- **Large generated files are searched.** The size cap rose from 254 KiB (TypeScript) and 1 MiB (Rust) to 4 MiB. A 3.4 MB generated TypeScript file now returns rows instead of `search: []`.
- **Deep nesting is reported, not parsed.** A file nested more than 16,384 levels (TypeScript) or 256 levels (Rust) is skipped with a `file nests too deeply to analyze` line. A file of 100,000 nested arrays took 188 MB and 20,000 nested Rust parentheses took a second; both now take under 10 ms.
