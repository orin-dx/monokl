---
monokl: patch
---

Deeply nested or deliberately hostile files can no longer crash `monokl search`.

- **Other results survive.** One such file used to abort the whole search with no output; the rest of the results are now returned.
- **What you see.** The file is listed as degraded, with the reason.
