---
monokl: patch
---

`monokl search` is faster on trees with many matching files: 20,000 one-line files take 545 ms instead of 759 ms (about 28% less), and a broad query over zod 31 ms instead of 37 ms. Output is byte-identical.

- **One parse thread per worker, not per file.** Analysis used to start a thread with a 256 MiB stack for every file, which cost 3.5 s of CPU against 35 ms of parsing over 20,000 files. A search now starts its analysis threads once with that stack. Each parse still runs on a stack of exactly that size, behind the same nesting scan and 4 MiB cap, and a parse made outside a search still gets a thread of its own.
- **Ranking runs in parallel.** Scoring and sorting spent 42 ms of that run on one thread and now take 6 ms. `rank::rank_blocks` needs its hasher to be `Sync`, which every standard and `FxHash` hasher is.
- **Larger batches.** A search hands out 64 files per thread to text search and 32 to analysis, up from 16 and 8, so threads wait less for the slowest file of a batch. After the first batch a `--timeout` is checked every 576 files on an 18-core machine instead of every 144.
