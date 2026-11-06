---
monokl: patch
---

`monokl search` now honors the `.gitignore` and `.ignore` files of parent directories when the search path is inside a git repository.

- **Scope.** Directories from the repository root (the nearest ancestor with a `.git` entry) down to the search path's parent are read, and nothing above the repository root. Rules inside the search path win over parent rules, as in ripgrep.
- **No repository.** Without a `.git` ancestor no parent ignore file is read, as before.
- **Opt out.** `--no-ignore` skips parent ignore files too.
