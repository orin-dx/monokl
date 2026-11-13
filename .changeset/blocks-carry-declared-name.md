---
monokl: minor
---

Blocks carry the name their analyzer read from the syntax tree, and the name boost in `monokl search` uses it instead of cutting the signature text.

- **Name from the tree.** `CodeBlock::name` is the declared identifier of a function, class, interface, type alias, enum, namespace, method, property or constant, and the target of an `impl` (`Trait for Target` for a trait impl). It is `None` for a re-export, an anonymous default export, a destructuring declaration, a macro call and an `extern` block, which get no name boost.
- **Scores can change.** `finalScore` differs only where the old text head held a word the name does not, such as a modifier (`get`, `static`) or a variable's type, and for `export default Button;`, which declares no name. No column is added and the harness scores are unchanged: identifier 58/59, questions 17/18, members 15/16, Rust fd 22/24, held-out 23/24 and 12/12.
- **Library.** `CodeBlock` has a new public field, so code that builds one must set `name`. `rank::name_match_boost` reads `block.name`, not the signature or the first line of `code`.

Breaking: construct `CodeBlock` with a `name` field.
