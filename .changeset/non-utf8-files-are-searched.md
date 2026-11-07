---
monokl: patch
---

`monokl search` now finds code in files that are not valid UTF-8 instead of dropping them.

- **Invalid bytes are replaced.** Each invalid byte sequence is read as U+FFFD, so the file is searched and its blocks are returned as usual.
- **The output says so.** A `help` line counts the files read this way, for example `1 file read lossily: invalid UTF-8 replaced with U+FFFD: src/legacy.ts`, and names up to three of them.
