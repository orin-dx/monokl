---
monokl: minor
---

`monokl search` now ranks the declaration a query names above longer names that merely contain its words, so `use watch` style questions find `useWatch` and `create form control` finds `createFormControl`.

- **Name match.** The name boost compares the query words with the words of the block's declared name, with stop words such as `use` and `from` left out of the name. It rises with the share of the query the name holds, falls with the share of the name the query does not ask for, and is 7.0 when the two are the same words. Before it was `1 + 2 * share of the query in the name`, 1.0 to 3.0; it is now 1.0 to 7.0.
- **Scores and order change.** `finalScore` values and row order change on 110 of the 250 pinned-harness queries, 48 of them in the first row. The answer file of a multi-word question is in the first three files for 17 of 18 questions (unchanged), a declaration for 59 of 59 identifiers (was 58), and 14 of 14 in the report-only react-hook-form set (was 12). No earlier hit is lost.
- **Library.** `rank::name_match_boost` returns 1.0 to 7.0 instead of 1.0 to 3.0; its signature is unchanged. This is a behavior change and not a breaking API change, so the bump is minor, as for the earlier name-boost change, and not patch.
