---
monokl: minor
---

Blocks carry the byte span they cover and the position of the block that encloses them, so later stages can tell blocks apart by where they are and not by their text.

- **Span.** `CodeBlock::span` is the byte range of the analyzed source the block covers, so `source[span] == code`. Offsets count bytes of the text the analyzer received, which is the lossy UTF-8 decoding of a file with invalid bytes.
- **Parent.** `CodeBlock::parent_index` is the position of the enclosing block in `FileAnalysis::blocks` followed by `FileAnalysis::test_blocks`. It is set exactly when `parent` is, for the members of a class, object literal, `impl`, trait or inline module.
- **No change to results.** Nothing reads either field yet: `monokl search` output and the harness scores are unchanged.

**Breaking:** `CodeBlock` has two new public fields. Code that builds one must set `span` and `parent_index`.
