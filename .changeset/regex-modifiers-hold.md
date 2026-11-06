---
monokl: patch
---

`+regex:` and `-regex:` now work like `+` and `-` on any term.

- **`-regex:foo` excludes.** It used to rank the files containing `foo` first; it now drops them.
- **`+regex:foo` requires.** A file without a match is no longer returned.
- **Patterns with spaces.** Quote them: `regex:"function use[A-Z]"`.
