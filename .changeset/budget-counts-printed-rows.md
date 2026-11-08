---
monokl: minor
---

`--max-tokens` and `--max-bytes` now count the rows `monokl search` prints, not the code blocks behind them.

- **Before.** The caps summed each kept block's whole code, so a long function used up the budget even though its row prints only a signature and a few matching lines.
- **Now.** Each row's printed line is measured, so a cap bounds the output an agent receives. The cut is unchanged: a prefix in rank order, with a `showing N of M` hint, and a first row larger than the cap leaves an empty, truncated result.
- **Defaults.** Unchanged at 20,000 tokens and 2 MiB. On the pinned repositories the largest output was 11 KB, so the defaults are now a backstop and `--max-results` is the cap that usually applies.
- **Library.** `budget::apply_budget` takes a function that returns each block's printed row text.
