---
monokl: patch
---

File filters now treat every TypeScript and JavaScript extension alike.

- **Declaration files.** `.d.mts` and `.d.cts` are skipped, like `.d.ts`.
- **Test files.** `.test` and `.spec` files with `.jsx`, `.mjs`, `.cjs`, `.mts` and `.cts` extensions are now recognised as tests, so `--tests` controls them.
