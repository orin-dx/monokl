# 5. Hits and blocks are identified by byte spans, and blocks form a tree

**Status:** Accepted **Date:** 2026-11-11

## Context

A block is a line range plus a copy of its text (`CodeBlock { line_start, line_end, code, .. }`, `types.rs`) and a hit is a whole line (`LineHit { line_number, text }`). Both analyzers hold the exact span when they build a block (`push_block` in `analysis/ts/parse.rs`, `record` in `analysis/rust/parse.rs`) and drop it after cutting lines and `code`. The tree is dropped too: `parent` is a `ParentContext { kind, name }`, not a link to a block.

Later stages re-derive structure from lines or text. Non-test code mentions `line_start`, `line_end`, `matched_lines`, `line_number` or `code` on 87 lines in 11 files. The guessing stages:

- `owned_hits` (`pipeline/mod.rs`) picks a hit's owner by line range, own-text matching, and `outer.code.contains(inner.code)` for equal ranges.
- `snippet` and `signature_shows` (`output/agent.rs`) decide by text whether the signature already shows the first line.
- `encloses_member` and `overlaps_significantly` (`dedup.rs`) compare line ranges, so two disjoint blocks on one line overlap fully.
- `name_match_boost` (`rank/boost.rs`) cuts a signature string at `(`, `<`, `{`, `=`, `:` to recover a name.

The same gap produced 7 fix or test commits in 3 PRs (#66: 3, #68: 1, #77: 3). Ownership was rewritten three times (earliest of equal size, innermost by range, own text on the line) for `} b() {`, `}, // note` and `const a = 1; const needle = 2;`. A test function and its module with the same range were credited twice. A first-line snippet repeated or hid the signature. `pub(crate) fn` was cut to `pub`. `pipeline/proptests.rs` re-implements the old rule as its reference, so it pins the rule, not a contract.

## Decision

We recommend identifying hits and blocks by byte spans and linking blocks into a forest.

- A hit carries its line and the byte range of each match.
- A block carries a byte span and its parent's index in the file's block list, built once by the analyzer.
- The AST supplies `name` and `kind` as typed fields.
- A hit belongs to the innermost block whose span contains the hit's start byte. Equal spans resolve by parent depth, not text.
- Snippets are cut from spans and dedup compares spans.

## Consequences

Deleted: own-text matching and the equal-range rule in `owned_hits`, `signature_shows`, the head-cutting in `name_match_boost` with its `unrestricted` special case, and the proptest's reference reimplementation.

Costs:

- `LineHit` grows a list of match ranges. The searcher does not give them: `SinkMatch` in `grep-searcher` 0.1.17 (`sink.rs:379-423`) exposes only `bytes()`, `absolute_byte_offset()` and `line_number()` for the whole line. In-line offsets need `Matcher::find_iter` (`grep-matcher` 0.1.9, `lib.rs:629`) over each matching line. `LineCollector::text_matches` (`text_search.rs`) already makes that second pass for CRLF lines.
- Offsets must mean the same bytes on both sides. The searcher sees raw file bytes and the analyzer sees the lossy-decoded `String`, which differ after any invalid UTF-8.
- Each block gains a span (16 bytes) and a parent index (4 to 8 bytes). `code` stays, so nested blocks keep duplicating text.
- Dedup compares bytes instead of lines, the one stage whose output can change beyond the old bugs.

## Alternatives considered

Keep line ranges and patch each case, as we did. It costs a commit per new line shape, and each patch adds a rule the next must respect.

## Order of work

Three PRs. After each, `scripts/rg-compare.py` must not drop: identifier 58/59, questions 17/18, members 15/16, Rust fd 22/24, held-out 23/24 and 12/12. Output is byte-identical except the cases the old rules got wrong, which each PR lists.

1. Analyzers emit spans, parent indices, `name` and `kind`. Nothing reads them yet. Tests assert spans slice to `code`.
2. The sink reports match ranges and `owned_hits` becomes innermost-containing. Test-first: the three line shapes above and the same-range test function and module.
3. `snippet`, `name_match_boost` and `dedup_blocks` read spans and typed fields.

## Not measured

Memory and time of the second regex pass and the spans on a large repo. How many harness queries change under byte-based dedup. Whether mapping lossy offsets beats searching decoded text. How many of the 87 sites would change.
