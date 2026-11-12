---
monokl: minor
---

The `monokl::query` module is no longer public. It held the query lexer, parser and plan, which only the search pipeline uses; `monokl search` output is unchanged.

Breaking: the `monokl::query` module is no longer public, so code that imports it must stop.
