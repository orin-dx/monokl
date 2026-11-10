# Analysis Fidelity Improvements (v0.7.x)

Closes semantic blind spots in the TypeScript and Rust analyzers: AST-based Rust symbol extraction (real visibility, `impl_owner`, `trait_impl`) and normalized signatures for variable-bound TS callables (arrows, `memo(forwardRef(...))` wrappers). No new commands — pure output-fidelity work. Includes forward-looking notes on Python's own fidelity challenges for v1.1, not yet implemented.

Part of the [monokl spec](./README.md). Builds on [03-multi-language-platform.md](./03-multi-language-platform.md).

---

# Analysis Fidelity Slice (0.7.x — feat/monokl-analysis-fidelity-v2)

> Added after Part 3. Closes the main semantic blind spots in the TypeScript and Rust analyzers. No new commands; all changes are in `symbols` output fidelity.

## Overview

Two categories of work:

1. **TypeScript signature normalization for variable-bound callables.** Arrow functions, function expressions, and call-wrapped React exports assigned to a `const` now produce normalized multi-line signatures instead of `None`.

2. **Rust AST-based symbol extraction.** When `lang-rust` is enabled, `extract_symbols` uses the `ra-ap-syntax` parse tree instead of the line scanner. The AST version adds normalized signatures, correct visibility, `impl_owner`, and `trait_impl` fields.

---

## 1. Rust: AST-based `extract_symbols` (`#[cfg(feature = "lang-rust")]`)

### What changed

The `#[cfg(not(feature = "lang-rust"))]` line scanner (Part 3 §4) is now the fallback only. When parser support is compiled in, a parallel function takes the already-parsed `SourceFile` tree produced by `parse_with_profile`:

```rust
#[cfg(feature = "lang-rust")]
fn extract_symbols(source: &str, tree: &SourceFile) -> Vec<SymbolEntry> {
    let mut symbols = Vec::new();
    for item in tree.items() {
        extract_item_symbols(source, &item, &mut symbols);
    }
    symbols
}
```

`extract_item_symbols` dispatches to `top_level_item_symbol` for named top-level items and, for `ast::Item::Impl`, also recurses into `extract_impl_symbols`.

### `top_level_item_symbol`

Handles: `Fn`, `Struct`, `Enum`, `Trait`, `Module`, `Const`, `Static`, `TypeAlias`, `MacroRules`.

New item: `ast::Item::Static` emits `kind_detail = "rust-static"` (not present in the line scanner).

Each arm calls:

```rust
ast_symbol(
    source,
    node.syntax(),
    node.name().map(|n| n.text().to_string()),
    SymbolKind::Function,          // or Struct, Enum, etc.
    "rust-function",               // or "rust-struct", etc.
    rust_visibility(node.syntax()),
    rust_signature(source, node.syntax(), Some(SyntaxKind::L_CURLY)),
)
```

**`rust_visibility(node)`** — walks the node's children for a `Visibility` token and maps `pub`, `pub(crate)`, `pub(super)`, `pub(self)` to `Visibility::{Public, Crate, Super, Module}`. Defaults to `Visibility::Private` when absent.

**`rust_signature(source, node, stop_before)`** — slices the raw source from the node's start offset to just before the first occurrence of `stop_before` token kind (`L_CURLY` for fn/struct/enum/trait/mod/impl, `EQ` for const/static/type). Multi-line slices are flattened by collapsing inner whitespace runs to a single space. Leading attribute tokens (`#[...]`) are excluded by advancing past the last attribute child before the slice.

### `extract_impl_symbols`

`impl` blocks emit one `SymbolEntry` for the `impl` itself plus one per associated `fn`. The impl-level symbol carries:

- `name` — the self type as text (e.g., `Widget<T>`).
- `kind_detail` — `"rust-impl"`.
- `impl_owner` — `Some(self_ty_text)`.
- `trait_impl` — `Some(trait_name)` when the `impl` has a `for` clause (e.g., `impl Render for Widget<T>` → `trait_impl = Some("Render")`).

Each associated `fn` inside the impl carries:

- `impl_owner` — the parent impl's self type (propagated down).
- `trait_impl` — the parent impl's trait name if present.
- `kind_detail` — `"rust-function"`.

### New `kind_detail` values

| `kind_detail`   | Source item                       | New in v2 |
| --------------- | --------------------------------- | --------- |
| `"rust-impl"`   | `impl Foo` / `impl Trait for Foo` | ✅        |
| `"rust-static"` | `static FOO: T = ...`             | ✅        |

All others (`"rust-struct"`, `"rust-enum"`, `"rust-trait"`, `"rust-function"`, `"rust-type"`, `"rust-const"`, `"rust-module"`) existed in the line scanner. The AST version adds correct signatures and visibility to them.

### Tests added

```rust
#[cfg(feature = "lang-rust")]
#[test]
fn rust_ast_symbols_include_signatures_impl_owner_and_trait_impl() {
    // Source: pub(crate) struct Widget<T>, pub trait Render,
    //   impl<T> Widget<T> { pub fn new }, impl<T: Display> Render for Widget<T> { fn render }
    // Asserts:
    //   Widget → visibility = Crate, signature = "pub(crate) struct Widget<T>"
    //   new → impl_owner = Some("Widget<T>"), trait_impl = None
    //   render → impl_owner = Some("Widget<T>"), trait_impl = Some("Render")
}
```

---

## 2. TypeScript: variable-bound callable signatures

### What changed

`extract_stmt` and `extract_export_decl` previously emitted `signature: None` for `VariableDeclarator` nodes where the initializer was a function-like expression. Both now call:

```rust
signature: declarator
    .init
    .as_ref()
    .and_then(|init| variable_function_signature(source, declarator.span.start, init)),
```

### `variable_function_signature`

```rust
fn variable_function_signature(source: &str, start: u32, init: &Expression<'_>) -> Option<String> {
    match init {
        Expression::ArrowFunctionExpression(arrow) => {
            normalize_signature(&source[start as usize..arrow.body.span().start as usize])
        }
        Expression::FunctionExpression(func) => function_signature(
            source,
            start,
            func.body.as_ref().map(|body| body.span.start),
            init.span().end,
        ),
        Expression::CallExpression(call) => {
            let body_start = call
                .arguments
                .first()
                .and_then(|arg| arg.as_expression())
                .and_then(wrapped_function_body_start)?;
            normalize_signature(&source[start as usize..body_start as usize])
        }
        _ => None,
    }
}
```

`start` is `declarator.span.start` — the slice covers the full `const Widget = …` up to the body brace (or arrow body), so the signature includes the LHS name.

### `wrapped_function_body_start`

Recursively unwraps nested call expressions to find the innermost function-like argument:

```rust
fn wrapped_function_body_start(expr: &Expression<'_>) -> Option<u32> {
    match expr {
        Expression::ArrowFunctionExpression(arrow) => Some(arrow.body.span().start),
        Expression::FunctionExpression(func) => func.body.as_ref().map(|body| body.span.start),
        Expression::CallExpression(call) => call
            .arguments
            .first()
            .and_then(|arg| arg.as_expression())
            .and_then(wrapped_function_body_start),
        _ => None,
    }
}
```

This handles arbitrarily nested wrappers: `memo(forwardRef(connect(mapState)(fn)))` would recurse through each call's first argument until it hits a function expression or arrow.

### Shapes covered

| Input shape | Normalized signature |
| --- | --- |
| `const Widget = (props: P): JSX.Element => {` | `Widget = (props: P): JSX.Element =>` |
| `const Widget = function(props: P): JSX.Element {` | `Widget = function(props: P): JSX.Element` |
| `const Widget = memo(forwardRef<Div, P>((props, ref): JSX.Element => {` | `Widget = memo(forwardRef<Div, P>((props, ref): JSX.Element =>` |

Multi-line forms collapse inner whitespace to a single space via `normalize_signature` (already used for `FunctionDeclaration` signatures — same normalization path).

### Shapes NOT covered (intentional)

`export default function() {}` / `export default () =>` / `export default class extends C {}` — anonymous default exports. These are future work requiring an explicit output-contract decision on synthetic naming. See strategic-roadmap.md §Analysis Fidelity.

### Tests added (in `integration_new_commands.rs`)

Three new black-box tests against the compiled binary:

- `symbols_multiline_arrow_function_variable_signature_is_normalized` — arrow function form.
- `symbols_multiline_function_expression_variable_signature_is_normalized` — function expression form.
- `symbols_wrapped_call_expression_variable_signature_is_normalized` — `memo(forwardRef(...))` form.

---

## 3. Python — fidelity considerations for v1.1

Python is slated for v1.1 (`lang-py`, tree-sitter-python, pyright). Its fidelity challenges differ substantially from TypeScript and Rust.

### Non-problems (Python eliminates these)

- **Anonymous default exports** — Python has no `export default`. `def` and `class` always bind a name. The synthetic-naming decision (§TS above) does not apply.
- **Multiline variable-bound lambdas** — Python `lambda` is syntactically single-expression. Multi-line callable definitions always use `def`, which always produces a named symbol.
- **Call-wrapped anonymous exports** — extremely rare at module level. `def` preserves the name through any decoration context.

### Python-specific fidelity problems that DO matter

**1. `__init__.py` re-export attribution.** The common "package as API" pattern:

```python
# mylib/__init__.py
from .button import Button
from .input import Input
__all__ = ["Button", "Input"]
```

`mnkl symbols mylib/__init__.py` should attribute `Button` and `Input` to their origin files, not emit them as new symbols defined here. Analogous to `pub use` re-export attribution in Rust.

**2. Decorator effects on signature.** Decorators change the effective calling signature:

```python
@lru_cache(maxsize=128)
def expensive(x: int) -> str: ...   # effective signature unchanged here

@property
def host(self) -> str: ...          # becomes a getter descriptor

@app.route("/users")
async def list_users(req): ...      # becomes a route handler
```

The symbol should carry both the declared signature and a `decorator` field listing applied decorators, so consumers can reason about the effective calling shape. Analogous to `trait_impl` on Rust symbols marking behavioral enrichment.

**3. `__all__`-based visibility.** Python has no enforced access control. The effective public API is:

- Names in `__all__` (when present) — explicitly exported.
- Names not prefixed with `_` (when `__all__` absent) — conventionally public.
- Names prefixed with `_` — conventionally private. `visibility` in Python `symbols` output should map: `__all__` member → `Public`, `_`-prefixed → `Private`, otherwise `Module` (file-scoped public convention).

**4. Protocol implementations.**

```python
class Foo(Protocol):    # defines a protocol — analogous to trait
    def bar(self) -> int: ...

class Widget(Foo):      # structural impl — analogous to impl Trait for Foo
    def bar(self) -> int: return 42
```

`symbols` should carry `protocol_impl: Option<String>` on implementors, parallel to `trait_impl` on Rust symbols.

**5. Type annotation resolution.** `from __future__ import annotations` makes all annotations lazy strings at runtime. Getting correct signatures without a type-checker (pyright/mypy) integration requires resolving forward references, which is approximate. The spec for `lang-py` should declare this as `CapabilityPrecision::Heuristic` on `inspect_detail` until pyright integration lands. (Corrected from an earlier draft's `Approximate`, which is not a variant of the real enum — see `CapabilityPrecision` in §11: `Unsupported < Heuristic < Structural < Exact`.)

**6. `@property` / `@classmethod` / `@staticmethod` tagging.** These three decorators change calling semantics enough that `mnkl symbols` should tag them explicitly — `kind_detail` values: `"py-property"`, `"py-classmethod"`, `"py-staticmethod"`. Plain methods are `"py-method"`. Module-level functions are `"py-function"`.

### Proposed Python `kind_detail` values

| `kind_detail`         | Python item                                |
| --------------------- | ------------------------------------------ |
| `"py-function"`       | module-level `def`                         |
| `"py-async-function"` | module-level `async def`                   |
| `"py-class"`          | `class`                                    |
| `"py-method"`         | method inside a class                      |
| `"py-classmethod"`    | `@classmethod` method                      |
| `"py-staticmethod"`   | `@staticmethod` method                     |
| `"py-property"`       | `@property` getter                         |
| `"py-const"`          | module-level name in ALL_CAPS (convention) |

These are forward-planning values, not yet implemented. They should be locked in the spec before the `lang-py` feature branch starts to avoid symbol output breaking changes mid-PR.

---
