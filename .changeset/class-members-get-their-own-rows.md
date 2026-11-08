---
monokl: minor
---

`monokl search` now returns a class's methods as rows of their own, so a query for a method name shows that method, not the whole class around it.

- **New rows.** Each method, constructor, getter and setter, and each class property holding an arrow function or a function expression, gets a row when it spans three lines or more and its class is declared at the top level of the file or exported. Its `nodeKind` is `method`, `constructor` or `property`. Members of an anonymous class, interface members and object-literal members are not split out yet.
- **New `parent` column.** It sits after `nodeKind` and reads `class Foo` on a member row, `null` on every other. Scripts that read rows by position must account for the extra column.
- **One owner per line.** A line that matched inside a member is credited to the member only. The class keeps its row when it matches outside its members, such as on its name, and drops out when every match is inside a member.
- **Deduplication.** A member is no longer dropped because the class around it overlaps it; two members, or two blocks without a parent, still dedupe by overlap.
- **Empty results.** The note now says that top-level declarations and class members are searched.
- **Library.** `CodeBlock` has a new `parent` field, and `RankedBlock::parent_context` is filled from it.
