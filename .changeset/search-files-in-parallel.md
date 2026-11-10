---
monokl: patch
---

`monokl search` is faster on large repositories: files are now searched and analyzed on several threads. Output is unchanged. On zod a search for `ZodType` fell from 35 ms to 21 ms.
