---
monokl: minor
---

`monokl search` now returns the functions of an object literal as rows of their own, so a query for `loadUser` shows that method, not the whole `const api = { ... }` around it.

- **New rows.** Each method, getter and setter, and each property holding an arrow function or a function expression, of an object literal assigned to a top-level `const`, `let` or `var` or used as the `export default` value, gets a row when it spans three lines or more. A literal wrapped in `as`, `satisfies` or parentheses counts. Its `nodeKind` is `method` for methods, getters and setters and `property` for the others. Object literals inside functions, calls and `module.exports = { ... }` are not split out yet.
- **`parent` column.** It reads `variable api` on a member of `const api = { ... }` and `other default` on a member of `export default { ... }`.
- **One owner per line.** A line that matched inside a member is credited to the member only. The object keeps its row when it matches outside its members, such as on its name, and drops out when every match is inside a member.
