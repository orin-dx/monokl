---
monokl: minor
---

The library no longer builds analyzer output that search never read: symbols, dependencies, exports, JSX elements, the blake3 `Digest` (`monokl::digest` is gone, along with the `blake3` and `serde` dependencies), analysis profiles and capabilities. `LanguageAnalyzer` is `languages`, `supports` and `analyze(path, text)`; `FileAnalysis` holds `blocks`, `test_blocks`, `had_parse_errors` and `degradation`. `CodeBlock::matched_keywords`, `ParentContext::line` and `RankedBlock::parent_context` (read `block.parent`) are removed. `monokl search` output is unchanged.
