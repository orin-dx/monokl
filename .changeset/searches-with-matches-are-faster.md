---
monokl: patch
---

`monokl search` is about 40 ms faster whenever it finds a match, because it no longer loads a tokenizer vocabulary before printing.

- **`--max-tokens` is an estimate.** It counts one token per three bytes of the printed rows, rounded up, which reads about a tenth above `o200k_base` on real output. The default of 20,000 is unchanged.
- **Smaller install.** The tokenizer and its vocabulary are no longer part of the binary.
