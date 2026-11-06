---
monokl: minor
---

The library no longer exposes types for commands that do not exist yet, and its core result types no longer derive `Serialize`; the `monokl search` output is unchanged.

- **Result types.** `CodeBlock`, `RankedBlock`, `SearchResponse`, `Diagnostic`, `SymbolEntry` and the other `monokl::types` structs and enums lose their `Serialize` and `Deserialize` derives. `SearchResponse` loses `truncation_marker`; read `truncated` and `total_blocks_before_truncation`.
- **Removed types.** `SearchOptions`, `SearchLimits` (and `Budget::from(&SearchLimits)`), `SymbolsResult`, `DependentsResult`, `ExtractRequest`, `WorkspaceOptions`, `TsconfigMode`, `RustPathAnchor`, `DependencyTarget::RustPath` and `Namespace`, `BindingKind::NamespaceWide`, and the `Rust`, `Python`, `Go` and `Java` variants of `LangData`. `LangData::ts` now returns `&TsData`. `output::agent::visibility_str` is gone too.
- **Removed errors.** `MonoklError` keeps `Io`, `Walk`, `RegexBuild`, `NonUtf8Path` and `TooManyTerms`; `FileIoError` keeps `Read`. `ToDomainError::to_domain_error` no longer takes an `ErrorContext`.
