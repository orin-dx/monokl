---
monokl: minor
---

Removes `--max-bytes`, the `miette` cargo feature and the output-layer projection trait.

- **`--max-bytes` is gone.** `--max-tokens` counts one token per three bytes, so the 2 MiB byte cap only bound above about 700,000 tokens. Passing `--max-bytes` is now a usage error (exit code 2). `--max-tokens` and `--max-results` are unchanged, and output is byte-identical otherwise.
- **Library.** `Budget` has no `max_bytes` field, and `budget::apply_budget` takes the printed row of each block as a slice instead of a function. `output::agent` has `render_search` and `search_row_texts` in place of the `ToonProjection` trait, `render_projection` and `search_row_text`; `SymbolKind::as_str` and `DiagnosticKind::verb` replace `symbol_kind_str` and the match in the diagnostics renderer.
- **No `miette` feature.** `MonoklError` and `FileIoError` no longer derive `miette::Diagnostic`; they carried no diagnostic metadata and the CLI prints errors through michi. `miette` and its dependencies are no longer built.

Breaking: `--max-bytes` is removed and passing it is now a usage error (exit 2); use `--max-tokens` or `--max-results`.
