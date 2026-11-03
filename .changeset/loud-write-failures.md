---
monokl: patch
---

`monokl search` now exits with an error when it cannot write its results, for example to a full disk; a closed pipe such as `| head` still exits 0.
