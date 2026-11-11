# 4. Search stays stateless and every stage has a stated bound

**Status:** Accepted **Date:** 2026-11-10

## Context

Performance problems kept recurring in different forms:

- The tokenizer vocabulary cost 50 ms on every search with a match. A byte estimate replaced it.
- Dedup was quadratic: 30 ms of 51 ms on broad queries. It is now indexed per file.
- The parallel search kept every file's hits past the file ceiling, so memory was unbounded. It now folds ordered, bounded batches.
- The per-file parse thread sizes its stack by input: 8 MiB + 4 KiB per source byte, capped at 1 GiB for TypeScript and 4 GiB for Rust. Very large generated files return nothing, and a failed reservation degrades output by host.
- `ra_ap_syntax` is quadratic in nesting depth. On a 100 KB file, 20k nested parentheses take 1.06 s and 50k take 4.2 s.
- Nothing has a deadline.
- Memory is bounded by file count, not bytes. 40 files of 5 MB reached 213 MB RSS. One 300 MB line reached 736 MB.
- Text search read every file under the directory, including non-source files, and dropped them afterwards. A path-restricted search over 54,009 files (18,171 of them source) took 2,179 ms against rg's 184 ms. A PR that filters by language before reading fixed it.

Each fix was local. The class of problem was not addressed: a stage whose cost depends on an input property nobody bounded.

## Decision

Search stays stateless: no persistent index, no on-disk cache, no daemon, for now.

Every stage states its bound in its doc comment. Work and memory are bounded by output or by bytes, not by input surprises:

1. No thread stack is sized by input. Parse on a fixed-size stack and reject deep nesting with an O(n) pre-scan.
2. Check file size before reading, and cap hit text length.
3. A global `--timeout` stops analysis early and reports what it skipped in a diagnostic.
4. No step is super-linear in match count. A scaling test in CI asserts the time ratio between two input sizes.
5. Results are deterministic regardless of thread timing: fold in walk order.
6. A file no analyzer owns is never opened.

## Consequences

Rule 1 is done. Every parse runs on one 256 MiB stack, and a file whose nesting could pass it is rejected by a linear scan before it is parsed (`TooDeep`). Size is capped by measured parse time and memory (4 MiB), not by the stack.

Stages that violate a rule today are follow-up work, not done:

- Rule 2: size is not checked before reading, so memory still grows with bytes.
- Rule 3: there is no `--timeout`, so no deadline.
- Rule 4: the per-file dedup fix removed the known quadratic step, but the CI scaling test is follow-up work.
- Rule 5 is what the ordered batch fold already does.
- Rule 6 holds once the language filter before reading merges (PR 79); until then non-source files are still opened.

The trade-off: a stateless search pays the cold cost on every run, and the bounds reject or skip some inputs (deeply nested or oversized files) that analysis would otherwise attempt. Each skip is reported, not silent.

## Alternatives considered

- **Two-phase search.** Score files cheaply from hit counts and path, then analyze only the top 100. Prototyped and rejected. Across 181 queries on three repos, the top-50 rows changed for 20, including rank 1 (`type`, `default`), and 11 changed inside the top 10. On the TypeScript repository, 4 of 30 queries changed at rank 1. Speedups were 1.25x, 0.88x, 0.91x, 1.04x and 1.7x on the 20,000-file set, because text search and the walk dominate (rg alone takes 364 ms there). A safe stopping bound is not cheap to compute: BM25 statistics and the name bonus depend on the whole analyzed set.
- **On-disk analysis cache keyed by content hash.** It helps only warm repeats, and a fresh checkout is cold. It adds writes, invalidation and read-only sandbox problems. Revisit only if warm repeat latency becomes the measured complaint.
- **Long-lived daemon.** A later step, not needed for current numbers. On main, medians on zod and axios are within a few ms of rg, except zod broad queries at 27.7 ms against 18.4 ms, which print 12 KB against rg's 864 KB.
