---
monokl: minor
---

`monokl search` bounds what it reads and how long it runs, and says when it skipped or cut something.

- **Size.** A file over 4 MiB is no longer read; the output lists it in one line, `2 files skipped: over 4 MiB: a.ts, b.ts`. Before, such a file was read whole and then reported as too large to parse, so 40 files of 5 MB reached 479 MB of memory and one 300 MB line 752 MB; both now stay at 5 MB.
- **Lines.** Only the first 4,096 bytes of a matching line are kept for matching terms against the line. The output counts the lines it cut. A `+required` term that appears only past that point in a line is not seen.
- **`--timeout SECONDS`.** Stops after that long (default 30, `0` for no limit, fractions allowed) and prints what was found, with `partial result: 30 s timeout expired after analyzing 120 of 900 files`. The exit code stays 0. A partial result is the only output that can differ between runs.
- **Unchanged.** Rows, ranks and diagnostics for every file under 4 MiB with lines under 4,096 bytes. Output was byte-identical on 172 queries over the pinned repositories.
- **Library.** `TextSearchOptions` gains `max_file_bytes`, `max_line_bytes` and `deadline`, and `TextSearchResult` gains `oversize`, `truncated_lines` and `incomplete`. New `pipeline::search_with_timeout`; `search` runs without a deadline.
