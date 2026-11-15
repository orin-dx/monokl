---
monokl: minor
---

**Two blocks that share a line are no longer taken for one.** `monokl search` removes a block that repeats a better one by the bytes the two share, not the lines, and decides what a snippet leaves out by where the signature ends, not by comparing text.

- **Why it matters.** `const first = 1; const second = 2;` and `fn a() {} fn b() {}` each used to give one row, because both blocks claimed line 1 and the second counted as a copy of the first. Each block that holds a match now gets its row. A block inside another, or equal to it, is still dropped as before.
- **Snippets.** A matching line is left out when the signature column already shows all of it, wherever that line is in the block: the lines of a multi-line signature, and the signature line of a Rust item below its attributes or doc comments, no longer repeat the signature in the snippet. A line that says more than the signature stays: a decorator, the start of an arrow function's expression body, or a body on the same line.
- **Output.** Same columns and the same order. A search can return more rows where blocks shared a line, and fewer snippet lines where the signature spans several.

**Breaking:** `CodeBlock` has a new public field, `signature_span`, the bytes of the declaration's head that `symbol_signature` is the one-line form of. Code that builds a `CodeBlock` must set it, with `None` for a block that has no signature.

**Breaking:** `dedup_blocks` compares `CodeBlock::span`, not `line_start` and `line_end`. A caller that built blocks with an empty span (`0..0`) now has every one of them kept; set each block's real span.
