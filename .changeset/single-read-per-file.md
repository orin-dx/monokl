---
monokl: patch
---

**`monokl search` opens each matching source file once.** Text search keeps the text of a matching file of at most 256 KiB, up to 16 MiB over all files, and analysis uses it instead of opening the file again. On a tree of 20,000 small matching files the search takes about a quarter less time, and peak memory rises by at most that 16 MiB. The output is byte-identical. A file over 256 KiB, or past the 16 MiB, is opened again as before.
