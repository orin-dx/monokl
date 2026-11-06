---
monokl: patch
---

Search output no longer points to commands and options that do not exist.

- **No `extract` hint.** Every result used to end by suggesting an `extract` command that is not built yet.
- **Fewer wrong retries.** The error for too many query terms no longer suggests a `maxTerms` option that does not exist.
