---
monokl: patch
---

Files larger than about 254 KiB are no longer parsed, and the output names them.

- **Why.** It keeps parsing within a safe stack size. About 0.5% of TypeScript and JavaScript files on one developer machine are over the limit, mostly bundler output.
- **What you see.** The file is listed as degraded with its path, size and the limit, so you can search it with `rg`.
