---
monokl: minor
---

`monokl search` now searches Rust (`.rs`) files as AST blocks, like TypeScript and JavaScript.

- **Blocks.** Functions, structs, unions, enums, traits, type aliases, consts, statics, `macro_rules!` macros and inline modules, with their attributes and doc comments. `impl` blocks and the methods in them and in traits get their own rows when a method is 3 lines or more, with the `impl` or trait as the parent; a match outside every method is credited to the `impl`.
- **Test code.** Items under `#[cfg(test)]` or `#[test]` are left out unless `--tests` is passed.
- **`--language rust`** now selects `.rs` files; before, it matched nothing.
- **Library.** The new `lang-rust` cargo feature (on by default) builds `analysis::rust::RustAnalyzer` on `ra_ap_syntax`. `LangData` gains a `Rust` variant, `FileAnalysis` a `test_blocks` field and `Degradation` a `TooDeep` variant; `SearchRequest::allow_tests` also covers test code inside Rust files.
