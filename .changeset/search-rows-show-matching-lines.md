---
monokl: minor
---

`monokl search` rows now show the lines that matched, so you often don't need a second read of the file to see why a block matched.

- **New `snippet` column.** It sits after `symbolSignature` and holds the matching lines of the block, each with its line number and cut at 160 characters. Scripts that read rows by position must account for the extra column.
- **Bounded.** It shows up to three lines, then says how many more matched (`...and 2 more matching lines`). It is `null` when nothing beyond the signature matched.
- **No repeat of the signature.** A block with a signature leaves out its first line; a variable or type alias, which has no signature, shows it.
