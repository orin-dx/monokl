//! Agent-facing output: TOON projections of command results and conversion of [`MonoklError`] into michi errors.
use michi::toon::Value;

use crate::error::MonoklError;
use crate::types::{SymbolKind, Visibility};

/// The CLI command currently executing, so a recovery hint can name the right tool to retry.
pub struct ErrorContext<'a> {
    pub command: &'a str,
}

/// Converts a string into a TOON cell value.
///
/// The string is passed through unchanged: michi quotes and escapes control characters when it renders the row, so
/// escaping here as well would double-escape them.
pub fn cell(s: &str) -> Value {
    Value::from(s)
}

/// Like [`cell`], but maps `None` to [`Value::Null`].
pub fn opt_cell(s: Option<&str>) -> Value {
    match s {
        None => Value::Null,
        Some(s) => cell(s),
    }
}

#[cfg(test)]
mod cell_tests {
    use super::*;

    #[test]
    fn cell_plain_string_passes_through() {
        assert_eq!(cell("hello"), Value::from("hello"));
    }

    #[test]
    fn cell_leaves_line_breaks_for_michi_to_escape() {
        assert_eq!(cell("line1\r\nline2"), Value::from("line1\r\nline2"));
    }

    #[test]
    fn opt_cell_none_is_null() {
        assert_eq!(opt_cell(None), Value::Null);
    }

    #[test]
    fn opt_cell_some_empty_is_not_null() {
        assert_eq!(opt_cell(Some("")), Value::from(""));
        assert_ne!(opt_cell(Some("")), Value::Null);
    }

    // `cell` relies on michi escaping line breaks; if michi stops, multi-line code would break TOON rows.
    #[test]
    fn michi_escape_value_escapes_line_breaks() {
        let escaped = michi::toon::escape_value("// comment\r\nfn body() {\n}");
        assert_eq!(escaped, "\"// comment\\r\\nfn body() {\\n}\"");
    }
}

/// Returns the same `camelCase` name that serde produces for a [`SymbolKind`].
pub fn symbol_kind_str(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Constructor => "constructor",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "typeAlias",
        SymbolKind::Property => "property",
        SymbolKind::Field => "field",
        SymbolKind::Variable => "variable",
        SymbolKind::Module => "module",
        SymbolKind::Impl => "impl",
        SymbolKind::Macro => "macro",
        SymbolKind::Other => "other",
    }
}

/// Returns the same `camelCase` name that serde produces for a [`Visibility`].
pub fn visibility_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Crate => "crate",
        Visibility::Module => "module",
        Visibility::Private => "private",
    }
}

#[cfg(test)]
mod kind_str_tests {
    use super::*;

    #[test]
    fn symbol_kind_str_matches_serde_camel_case_form() {
        assert_eq!(symbol_kind_str(SymbolKind::Function), "function");
        assert_eq!(symbol_kind_str(SymbolKind::Method), "method");
        assert_eq!(symbol_kind_str(SymbolKind::Constructor), "constructor");
        assert_eq!(symbol_kind_str(SymbolKind::Class), "class");
        assert_eq!(symbol_kind_str(SymbolKind::Struct), "struct");
        assert_eq!(symbol_kind_str(SymbolKind::Enum), "enum");
        assert_eq!(symbol_kind_str(SymbolKind::Interface), "interface");
        assert_eq!(symbol_kind_str(SymbolKind::TypeAlias), "typeAlias");
        assert_eq!(symbol_kind_str(SymbolKind::Property), "property");
        assert_eq!(symbol_kind_str(SymbolKind::Field), "field");
        assert_eq!(symbol_kind_str(SymbolKind::Variable), "variable");
        assert_eq!(symbol_kind_str(SymbolKind::Module), "module");
        assert_eq!(symbol_kind_str(SymbolKind::Impl), "impl");
        assert_eq!(symbol_kind_str(SymbolKind::Macro), "macro");
        assert_eq!(symbol_kind_str(SymbolKind::Other), "other");
    }

    #[test]
    fn visibility_str_matches_serde_camel_case_form() {
        assert_eq!(visibility_str(Visibility::Public), "public");
        assert_eq!(visibility_str(Visibility::Crate), "crate");
        assert_eq!(visibility_str(Visibility::Module), "module");
        assert_eq!(visibility_str(Visibility::Private), "private");
    }
}

/// A command result that renders as a TOON table with `N` columns.
///
/// The column count is a const generic so a row with the wrong arity cannot be constructed.
pub trait ToonProjection<const N: usize> {
    /// The table name.
    const TYPE_NAME: &'static str;
    /// The column names.
    const FIELDS: [&'static str; N];
    /// Returns one row per item.
    fn toon_rows(&self) -> Vec<[Value; N]>;
    /// Returns the total item count to report, if any.
    fn toon_total_count(&self) -> Option<usize> {
        None
    }
    /// Returns hints rendered after the table.
    fn toon_hints(&self) -> Vec<michi::Hint> {
        Vec::new()
    }
}
impl ToonProjection<9> for crate::types::SymbolsResult {
    const TYPE_NAME: &'static str = "symbols";
    const FIELDS: [&'static str; 9] =
        ["file", "name", "kind", "line", "signature", "owner", "traitImpl", "visibility", "kindDetail"];

    fn toon_rows(&self) -> Vec<[Value; 9]> {
        let mut rows = Vec::new();
        for (file, entries) in &self.files {
            for e in entries {
                rows.push([
                    cell(file.as_str()),
                    cell(&e.name),
                    cell(symbol_kind_str(e.kind)),
                    Value::from(e.line),
                    opt_cell(e.signature.as_deref()),
                    opt_cell(e.owner.as_deref()),
                    opt_cell(e.trait_impl.as_deref()),
                    opt_cell(e.visibility.map(visibility_str)),
                    opt_cell(e.kind_detail.as_deref()),
                ]);
            }
        }
        rows
    }
}
impl ToonProjection<7> for crate::types::SearchResponse {
    const TYPE_NAME: &'static str = "search";
    const FIELDS: [&'static str; 7] =
        ["file", "lineStart", "lineEnd", "nodeKind", "symbolSignature", "rank", "finalScore"];

    fn toon_rows(&self) -> Vec<[Value; 7]> {
        self.results
            .iter()
            .map(|block| {
                [
                    cell(block.block.file.as_str()),
                    Value::from(block.block.line_start),
                    Value::from(block.block.line_end),
                    cell(symbol_kind_str(block.block.node_kind)),
                    opt_cell(block.block.symbol_signature.as_deref()),
                    Value::from(block.rank),
                    Value::from(block.final_score),
                ]
            })
            .collect()
    }

    fn toon_hints(&self) -> Vec<michi::Hint> {
        vec![michi::Hint::new("use extract with file/lineStart/lineEnd to retrieve a row's actual code content")]
    }
}

/// Converts a TOON validation failure into a [`michi::DomainError`].
///
/// It bypasses [`ToDomainError`] because this is not a [`MonoklError`]: it means this module built an invalid row or
/// field, not that the caller supplied bad input.
pub fn toon_validation_error(e: &michi::toon::ToonError) -> michi::DomainError {
    michi::DomainError::new(michi::ErrorCode::ExternalFailure, format!("toon_render_invalid: {e}")).retryable(false)
}

/// Renders a [`ToonProjection`] as a TOON document.
///
/// # Errors
///
/// Returns a [`toon_validation_error`] if the projection's table fails michi's validation.
#[expect(clippy::result_large_err, reason = "the error type is michi's DomainError, which agent output is built from")]
pub fn render_projection<const N: usize, T: ToonProjection<N>>(value: &T) -> Result<String, michi::DomainError> {
    let rows: Vec<Vec<Value>> = value.toon_rows().into_iter().map(|row| row.to_vec()).collect();
    let fields: Vec<String> = T::FIELDS.iter().map(|s| (*s).to_string()).collect();

    let opts = michi::toon::ToonOptions::new(T::TYPE_NAME, fields, rows.clone()).total_count(value.toon_total_count());
    if let Err(e) = opts.validate() {
        return Err(toon_validation_error(&e));
    }

    let mut resp = michi::AgentResponse::new(T::TYPE_NAME).items(rows, T::FIELDS.as_slice()).hints(value.toon_hints());
    if let Some(n) = value.toon_total_count() {
        resp = resp.total_count(n);
    }
    Ok(resp.render_toon())
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn render_projection_symbols_result_end_to_end() {
        use crate::types::{SymbolEntry, SymbolKind, SymbolsResult};
        use camino::Utf8PathBuf;
        use std::collections::BTreeMap;

        let mut files = BTreeMap::new();
        files.insert(
            Utf8PathBuf::from("src/lib.rs"),
            vec![SymbolEntry {
                name: "foo".to_string(),
                kind: SymbolKind::Function,
                line: 10,
                // The newline must render as the escape `\n` inside a quoted cell, neither breaking the row nor vanishing.
                signature: Some("fn foo(x)\nsecond line".to_string()),
                // Distinct values in every optional column, so a hardcoded `Value::Null` for any of them shows up.
                owner: Some("MyOwner".to_string()),
                trait_impl: Some("MyTrait".to_string()),
                visibility: Some(crate::types::Visibility::Public),
                kind_detail: Some("static".to_string()),
            }],
        );
        let result = SymbolsResult { files, total_symbol_count: 1, truncation_marker: None, diagnostics: Vec::new() };

        let rendered = render_projection(&result).expect("valid projection must render");
        assert!(
            rendered.starts_with("symbols[1]{file,name,kind,line,signature,owner,traitImpl,visibility,kindDetail}:\n"),
            "got: {rendered}"
        );
        assert!(
            rendered.contains("src/lib.rs,foo,function,10,\"fn foo(x)\\nsecond line\",MyOwner,MyTrait,public,static"),
            "got: {rendered}"
        );
    }

    #[test]
    fn render_projection_search_response_includes_extract_hint() {
        use crate::types::{CodeBlock, RankedBlock, SearchResponse, SymbolKind};
        use camino::Utf8PathBuf;

        let response = SearchResponse {
            results: vec![RankedBlock {
                block: CodeBlock {
                    file: Utf8PathBuf::from("src/lib.rs"),
                    // Line numbers, rank, and score are all distinct so a swapped column is detectable.
                    line_start: 5,
                    line_end: 9,
                    node_kind: SymbolKind::Function,
                    code: "fn foo() {}".to_string(),
                    symbol_signature: Some("fn foo(x)\nsecond line".to_string()),
                    matched_lines: vec![1],
                    matched_keywords: vec!["foo".to_string()],
                },
                bm25_score: 0.75,
                coverage_boost: 0.1,
                node_type_boost: 0.05,
                final_score: 0.92,
                rank: 2,
                parent_context: None,
            }],
            total_blocks_before_truncation: 1,
            truncated: false,
            truncation_marker: None,
            total_bytes: 100,
            total_tokens: 20,
            diagnostics: Vec::new(),
        };

        let rendered = render_projection(&response).expect("valid projection must render");
        assert!(
            rendered.contains("search[1]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:\n"),
            "got: {rendered}"
        );
        assert!(rendered.contains("src/lib.rs,5,9,function,\"fn foo(x)\\nsecond line\",2,0.92"), "got: {rendered}");
        assert!(!rendered.contains("fn foo() {}"), "code field must never appear in TOON output, got: {rendered}");
        assert!(
            rendered.ends_with(
                "\nhelp[1]: use extract with file/lineStart/lineEnd to retrieve a row's actual code content"
            ),
            "got: {rendered}"
        );
    }

    // Built by hand because `ToonProjection` cannot express an arity mismatch.
    #[test]
    fn toon_validation_error_shape() {
        let opts = michi::toon::ToonOptions::new(
            "t".to_string(),
            vec!["a".to_string()],
            vec![vec![Value::from("x"), Value::from("y")]],
        );
        let err = opts.validate().unwrap_err();
        let domain_err = toon_validation_error(&err);
        assert_eq!(domain_err.code, michi::ErrorCode::ExternalFailure);
        assert!(!domain_err.retryable);
        assert!(domain_err.message.starts_with("toon_render_invalid: "), "got: {}", domain_err.message);
    }

    // Drives `render_projection`'s own validation guard end to end. A repeated field name is invalid TOON, so michi
    // rejects it.
    struct InvalidFieldNameProjection;
    impl ToonProjection<2> for InvalidFieldNameProjection {
        const TYPE_NAME: &'static str = "invalid";
        const FIELDS: [&'static str; 2] = ["a", "a"];
        fn toon_rows(&self) -> Vec<[Value; 2]> {
            vec![[Value::from("x"), Value::from("y")]]
        }
    }

    #[test]
    fn render_projection_reports_validation_error() {
        let err =
            render_projection(&InvalidFieldNameProjection).expect_err("invalid FIELDS entry must fail validate()");
        assert_eq!(err.code, michi::ErrorCode::ExternalFailure);
        assert!(!err.retryable);
        assert!(err.message.starts_with("toon_render_invalid: "), "got: {}", err.message);
    }

    // `opt_cell` maps `None` to `Value::Null` so an agent can tell an absent field from an empty string.
    #[test]
    fn null_and_empty_string_render_distinctly() {
        let opts_null =
            michi::toon::ToonOptions::new("t".to_string(), vec!["a".to_string()], vec![vec![opt_cell(None)]]);
        let opts_empty =
            michi::toon::ToonOptions::new("t".to_string(), vec!["a".to_string()], vec![vec![opt_cell(Some(""))]]);
        assert_eq!(michi::toon::render_toon(&opts_null).unwrap(), "t[1]{a}:\n  null");
        assert_eq!(michi::toon::render_toon(&opts_empty).unwrap(), "t[1]{a}:\n  \"\"");
    }

    struct DummyProjection;
    impl ToonProjection<1> for DummyProjection {
        const TYPE_NAME: &'static str = "dummy";
        const FIELDS: [&'static str; 1] = ["x"];
        fn toon_rows(&self) -> Vec<[Value; 1]> {
            vec![[Value::from("x")]]
        }
    }

    #[test]
    fn default_toon_total_count_and_hints_are_empty() {
        let d = DummyProjection;
        assert_eq!(d.toon_total_count(), None);
        assert!(d.toon_hints().is_empty());
    }

    // Every other projection here uses the `None` default, so this is the only coverage of the total-count path.
    struct DummyProjectionWithTotal;
    impl ToonProjection<1> for DummyProjectionWithTotal {
        const TYPE_NAME: &'static str = "dummy";
        const FIELDS: [&'static str; 1] = ["x"];
        fn toon_rows(&self) -> Vec<[Value; 1]> {
            vec![[Value::from("x")]]
        }
        fn toon_total_count(&self) -> Option<usize> {
            Some(42)
        }
    }

    #[test]
    fn render_projection_wires_toon_total_count_into_output() {
        let rendered = render_projection(&DummyProjectionWithTotal).expect("valid projection must render");
        assert!(rendered.contains("totalCount: 42"), "got: {rendered}");
    }

    // The other projection tests render one row each, which would not catch a projection that drops later rows.
    #[test]
    fn multi_row_flattening_symbols_and_search() {
        use crate::types::{CodeBlock, RankedBlock, SearchResponse, SymbolEntry, SymbolsResult};
        use camino::Utf8PathBuf;
        use std::collections::BTreeMap;

        fn entry(name: &str, line: usize) -> SymbolEntry {
            SymbolEntry {
                name: name.to_string(),
                kind: SymbolKind::Function,
                line,
                signature: None,
                owner: None,
                trait_impl: None,
                visibility: None,
                kind_detail: None,
            }
        }

        fn block(file: &str, line_start: usize, line_end: usize, rank: usize, final_score: f64) -> RankedBlock {
            RankedBlock {
                block: CodeBlock {
                    file: Utf8PathBuf::from(file),
                    line_start,
                    line_end,
                    node_kind: SymbolKind::Function,
                    code: String::new(),
                    symbol_signature: None,
                    matched_lines: Vec::new(),
                    matched_keywords: Vec::new(),
                },
                bm25_score: 0.0,
                coverage_boost: 0.0,
                node_type_boost: 0.0,
                final_score,
                rank,
                parent_context: None,
            }
        }

        let mut files = BTreeMap::new();
        // Inserted out of key order to show that key order, not insertion order, decides row order.
        files.insert(Utf8PathBuf::from("src/z.rs"), vec![entry("z1", 1), entry("z2", 2)]);
        files.insert(Utf8PathBuf::from("src/a.rs"), vec![entry("a1", 3)]);
        let symbols = SymbolsResult { files, total_symbol_count: 3, truncation_marker: None, diagnostics: Vec::new() };

        assert_eq!(
            symbols.toon_rows().len(),
            3,
            "all (file, entry) pairs across every BTreeMap key must flatten into rows"
        );

        let rendered = render_projection(&symbols).expect("valid projection must render");
        assert!(
            rendered.starts_with("symbols[3]{file,name,kind,line,signature,owner,traitImpl,visibility,kindDetail}:\n"),
            "got: {rendered}"
        );
        let a1_pos = rendered.find("src/a.rs,a1,").expect("a1 row must be present");
        let z1_pos = rendered.find("src/z.rs,z1,").expect("z1 row must be present");
        let z2_pos = rendered.find("src/z.rs,z2,").expect("z2 row must be present");
        assert!(
            a1_pos < z1_pos && z1_pos < z2_pos,
            "rows must render in BTreeMap key order (src/a.rs before src/z.rs), then Vec order within a key: got {rendered}"
        );

        let search = SearchResponse {
            results: vec![
                block("src/one.rs", 1, 2, 1, 0.10),
                block("src/two.rs", 3, 4, 2, 0.20),
                block("src/three.rs", 5, 6, 3, 0.30),
            ],
            total_blocks_before_truncation: 3,
            truncated: false,
            truncation_marker: None,
            total_bytes: 0,
            total_tokens: 0,
            diagnostics: Vec::new(),
        };

        assert_eq!(search.toon_rows().len(), 3, "every RankedBlock in `results` must flatten into a row");

        let rendered = render_projection(&search).expect("valid projection must render");
        assert!(
            rendered.starts_with("search[3]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:\n"),
            "got: {rendered}"
        );
        let one_pos = rendered.find("src/one.rs,1,2,").expect("first result row must be present");
        let two_pos = rendered.find("src/two.rs,3,4,").expect("second result row must be present");
        let three_pos = rendered.find("src/three.rs,5,6,").expect("third result row must be present");
        assert!(
            one_pos < two_pos && two_pos < three_pos,
            "rows must render in `results`' own input order: got {rendered}"
        );
    }
}
fn io_error_code(kind: std::io::ErrorKind) -> michi::ErrorCode {
    use michi::ErrorCode;
    match kind {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::Forbidden,
        _ => ErrorCode::ExternalFailure,
    }
}

/// Classifies a walk error by its innermost cause.
///
/// `ignore::Error` does not expose its wrapped error through `source()`, so this unwraps the wrapper variants, and a
/// `Partial` holding exactly one error, by recursing on them.
fn walk_error_code(source: &ignore::Error) -> michi::ErrorCode {
    use michi::ErrorCode;
    match source {
        ignore::Error::WithLineNumber { err, .. }
        | ignore::Error::WithPath { err, .. }
        | ignore::Error::WithDepth { err, .. } => walk_error_code(err),
        ignore::Error::Partial(errs) if errs.len() == 1 => walk_error_code(&errs[0]),
        ignore::Error::Io(io_err) => io_error_code(io_err.kind()),
        ignore::Error::Glob { .. } | ignore::Error::UnrecognizedFileType(_) | ignore::Error::InvalidDefinition => {
            ErrorCode::InvalidInput
        }
        ignore::Error::Loop { .. } | ignore::Error::Partial(_) => ErrorCode::ExternalFailure,
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    #[test]
    fn walk_error_code_direct_glob_and_loop() {
        let glob_err = ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err();
        assert_eq!(walk_error_code(&glob_err), michi::ErrorCode::InvalidInput);
        let loop_err =
            ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        assert_eq!(walk_error_code(&loop_err), michi::ErrorCode::ExternalFailure);
    }

    #[test]
    fn walk_error_code_unwraps_nested_wrappers() {
        let inner = ignore::Error::InvalidDefinition;
        let wrapped = ignore::Error::WithDepth {
            depth: 2,
            err: Box::new(ignore::Error::WithPath {
                path: std::path::PathBuf::from("x"),
                err: Box::new(ignore::Error::WithLineNumber { line: 1, err: Box::new(inner) }),
            }),
        };
        assert_eq!(walk_error_code(&wrapped), michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_error_code_unrecognized_file_type_maps_to_invalid_input() {
        let err = ignore::Error::UnrecognizedFileType("bogus".to_string());
        assert_eq!(walk_error_code(&err), michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_error_code_invalid_definition_maps_to_invalid_input() {
        assert_eq!(walk_error_code(&ignore::Error::InvalidDefinition), michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_error_code_partial_non_single_maps_to_external_failure() {
        assert_eq!(walk_error_code(&ignore::Error::Partial(vec![])), michi::ErrorCode::ExternalFailure);
        let two = ignore::Error::Partial(vec![ignore::Error::InvalidDefinition, ignore::Error::InvalidDefinition]);
        assert_eq!(walk_error_code(&two), michi::ErrorCode::ExternalFailure);
    }

    #[test]
    fn walk_error_code_io_other_kind_maps_to_external_failure() {
        let io_err = ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists"));
        assert_eq!(walk_error_code(&io_err), michi::ErrorCode::ExternalFailure);
    }

    // An unrecursed `Partial` would classify as `ExternalFailure`, so `InvalidInput` proves the recursion.
    #[test]
    fn walk_error_code_partial_single_recurses() {
        let partial = ignore::Error::Partial(vec![ignore::Error::InvalidDefinition]);
        assert_eq!(walk_error_code(&partial), michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_error_code_partial_single_recurses_through_wrappers() {
        let partial = ignore::Error::Partial(vec![ignore::Error::WithPath {
            path: std::path::PathBuf::from(".gitignore"),
            err: Box::new(ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"))),
        }]);
        assert_eq!(walk_error_code(&partial), michi::ErrorCode::NotFound);
    }

    #[test]
    fn walk_error_code_io_permission_denied_maps_to_forbidden() {
        let io_err = ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"));
        assert_eq!(walk_error_code(&io_err), michi::ErrorCode::Forbidden);
    }
}
/// Converts an error into a [`michi::DomainError`] for agent output.
pub trait ToDomainError {
    /// Returns a short `snake_case` code that is distinct for each error kind.
    fn monokl_code(&self) -> &'static str;
    /// Converts the error, prefixing the message with [`monokl_code`](Self::monokl_code) and adding the rest of its
    /// display text and its whole source chain as hints.
    fn to_domain_error(&self, ctx: ErrorContext<'_>) -> michi::DomainError;
}

impl ToDomainError for MonoklError {
    fn monokl_code(&self) -> &'static str {
        match self {
            MonoklError::Io(_) => "io",
            MonoklError::Walk { .. } => "walk",
            MonoklError::RegexBuild(_) => "regex_build",
            MonoklError::NonUtf8Path { .. } => "non_utf8_path",
            MonoklError::TooManyTerms { .. } => "too_many_terms",
            MonoklError::TokenizerInit => "tokenizer_init",
            MonoklError::Json(_) => "json",
            MonoklError::PathOutsideRoot { .. } => "path_outside_root",
            MonoklError::StaleDiskCache => "stale_disk_cache",
            MonoklError::InvalidGitRef { .. } => "invalid_git_ref",
            MonoklError::Git { .. } => "git",
            MonoklError::SymlinkRejected { .. } => "symlink_rejected",
            MonoklError::FileTooLarge { .. } => "file_too_large",
            MonoklError::LockPoisoned { .. } => "lock_poisoned",
        }
    }

    #[expect(clippy::cast_possible_wrap, reason = "limits and caps are far below i64::MAX; KvValue::Int takes an i64")]
    fn to_domain_error(&self, ctx: ErrorContext<'_>) -> michi::DomainError {
        use michi::ErrorCode;

        let code = match self {
            MonoklError::RegexBuild(_)
            | MonoklError::NonUtf8Path { .. }
            | MonoklError::TooManyTerms { .. }
            | MonoklError::InvalidGitRef { .. }
            | MonoklError::FileTooLarge { .. } => ErrorCode::InvalidInput,
            MonoklError::PathOutsideRoot { .. } | MonoklError::SymlinkRejected { .. } => ErrorCode::Forbidden,
            MonoklError::StaleDiskCache => ErrorCode::Conflict,
            MonoklError::TokenizerInit
            | MonoklError::Json(_)
            | MonoklError::Git { .. }
            | MonoklError::LockPoisoned { .. } => ErrorCode::ExternalFailure,
            MonoklError::Io(err) => io_error_code(err.io_kind()),
            MonoklError::Walk { source, .. } => walk_error_code(source),
        };

        let mut self_lines = split_display_lines(&self.to_string()).into_iter();
        let head = self_lines.next().unwrap_or_default();

        let mut hints: Vec<String> = self_lines.collect();
        let mut cause: Option<&dyn std::error::Error> = std::error::Error::source(self);
        while let Some(err) = cause {
            for line in split_display_lines(&err.to_string()) {
                hints.push(line);
            }
            cause = err.source();
        }

        let mut domain_err = michi::DomainError::new(code, format!("{}: {head}", self.monokl_code())).retryable(false);
        for line in hints {
            domain_err = domain_err.hint(line);
        }

        domain_err = match self {
            MonoklError::TooManyTerms { count, limit } => {
                domain_err.hint(format!("you supplied {count} terms; the limit is {limit}")).recovery(
                    michi::RecoveryHint::new(ctx.command)
                        .param("maxTerms", michi::KvValue::Int(*limit as i64))
                        .reason("re-run with a shorter query"),
                )
            }
            MonoklError::FileTooLarge { path, size, cap } => {
                domain_err.hint(format!("{path} is {size} bytes; the cap is {cap} bytes")).recovery(
                    michi::RecoveryHint::new(ctx.command)
                        .param("maxBytes", michi::KvValue::Int(*cap as i64))
                        .reason("the file exceeds this workspace's size cap"),
                )
            }
            MonoklError::InvalidGitRef { ref_, reason } => {
                domain_err.hint(format!("{ref_:?} is not a valid git ref: {reason}"))
            }
            _ => domain_err,
        };

        domain_err
    }
}

fn split_display_lines(s: &str) -> Vec<String> {
    s.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(ToString::to_string).collect()
}

#[cfg(test)]
mod error_tests {
    use super::*;
    use camino::Utf8PathBuf;

    fn ctx() -> ErrorContext<'static> {
        ErrorContext { command: "search" }
    }

    #[test]
    fn monokl_code_is_distinct_and_nonempty_per_variant() {
        let errs: Vec<MonoklError> = vec![
            MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: ignore::Error::InvalidDefinition },
            grep_regex::RegexMatcherBuilder::new().build("(").unwrap_err().into(),
            MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/bad") },
            MonoklError::TooManyTerms { count: 1, limit: 1 },
            MonoklError::TokenizerInit,
            serde_json::from_str::<serde_json::Value>("{not valid").unwrap_err().into(),
            MonoklError::PathOutsideRoot { path: Utf8PathBuf::from("/x") },
            MonoklError::StaleDiskCache,
            MonoklError::InvalidGitRef { ref_: "-x".to_string(), reason: "bad" },
            MonoklError::Git { operation: "show", message: "e".to_string() },
            MonoklError::SymlinkRejected { path: Utf8PathBuf::from("/l") },
            MonoklError::FileTooLarge { path: Utf8PathBuf::from("/big"), size: 1, cap: 1 },
            MonoklError::LockPoisoned { context: "ctx" },
        ];
        assert_eq!(errs.len(), 13);
        let codes: Vec<&str> = errs.iter().map(ToDomainError::monokl_code).collect();
        assert_eq!(
            codes,
            vec![
                "walk",
                "regex_build",
                "non_utf8_path",
                "too_many_terms",
                "tokenizer_init",
                "json",
                "path_outside_root",
                "stale_disk_cache",
                "invalid_git_ref",
                "git",
                "symlink_rejected",
                "file_too_large",
                "lock_poisoned",
            ]
        );
        let mut dedup = codes.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), 13, "all monokl_code() values must be distinct");
        for c in &codes {
            assert!(!c.is_empty() && !c.contains(' '), "code must be non-empty with no whitespace: {c:?}");
        }
    }

    #[test]
    fn error_code_mapping() {
        assert_eq!(
            MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/x") }.to_domain_error(ctx()).code,
            michi::ErrorCode::InvalidInput
        );
        assert_eq!(
            MonoklError::PathOutsideRoot { path: Utf8PathBuf::from("/x") }.to_domain_error(ctx()).code,
            michi::ErrorCode::Forbidden
        );
        assert_eq!(MonoklError::StaleDiskCache.to_domain_error(ctx()).code, michi::ErrorCode::Conflict);
        assert_eq!(MonoklError::TokenizerInit.to_domain_error(ctx()).code, michi::ErrorCode::ExternalFailure);
        assert_eq!(
            MonoklError::LockPoisoned { context: "ctx" }.to_domain_error(ctx()).code,
            michi::ErrorCode::ExternalFailure
        );

        // `Walk` is covered by the walk-specific tests.
        let regex_err: MonoklError = grep_regex::RegexMatcherBuilder::new().build("(").unwrap_err().into();
        assert_eq!(regex_err.to_domain_error(ctx()).code, michi::ErrorCode::InvalidInput);
        assert_eq!(
            MonoklError::TooManyTerms { count: 1, limit: 1 }.to_domain_error(ctx()).code,
            michi::ErrorCode::InvalidInput
        );
        assert_eq!(
            MonoklError::InvalidGitRef { ref_: "-x".to_string(), reason: "bad" }.to_domain_error(ctx()).code,
            michi::ErrorCode::InvalidInput
        );
        assert_eq!(
            MonoklError::FileTooLarge { path: Utf8PathBuf::from("/big"), size: 1, cap: 1 }.to_domain_error(ctx()).code,
            michi::ErrorCode::InvalidInput
        );
        assert_eq!(
            MonoklError::SymlinkRejected { path: Utf8PathBuf::from("/l") }.to_domain_error(ctx()).code,
            michi::ErrorCode::Forbidden
        );
        let json_err: MonoklError = serde_json::from_str::<serde_json::Value>("{not valid").unwrap_err().into();
        assert_eq!(json_err.to_domain_error(ctx()).code, michi::ErrorCode::ExternalFailure);
        assert_eq!(
            MonoklError::Git { operation: "show", message: "e".to_string() }.to_domain_error(ctx()).code,
            michi::ErrorCode::ExternalFailure
        );
    }

    #[test]
    fn retryable_always_false_even_for_external_failure_codes() {
        let err = MonoklError::LockPoisoned { context: "ctx" };
        let domain_err = err.to_domain_error(ctx());
        assert_eq!(domain_err.code, michi::ErrorCode::ExternalFailure);
        assert!(!domain_err.retryable, "monokl errors must never be retryable");
        let michi_err = michi::Error::Domain(domain_err);
        assert_eq!(michi_err.class(), michi::ErrorClass::Internal, "must not classify as Transient");
        assert!(!michi_err.is_retryable());
    }

    #[test]
    fn head_is_the_variants_own_display_not_its_sources() {
        // `path` appears only in `Walk`'s own display text, never in the wrapped error's.
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/some/root"), source: ignore::Error::InvalidDefinition };
        let domain_err = err.to_domain_error(ctx());
        assert!(domain_err.message.contains("/some/root"), "got: {}", domain_err.message);
        assert!(domain_err.message.starts_with("walk: walker error at /some/root"), "got: {}", domain_err.message);
    }

    // A raw newline in the message would add unprefixed lines to the rendered error.
    #[test]
    fn multiline_self_display_head_is_first_line_only() {
        let err = MonoklError::Git { operation: "show", message: "line one\nline two\nline three".to_string() };
        let domain_err = err.to_domain_error(ctx());

        assert_eq!(domain_err.message, "git: git show failed: line one");
        assert!(!domain_err.message.contains('\n'), "got: {:?}", domain_err.message);
        let hint_strs: Vec<&str> = domain_err.hints.iter().map(michi::Hint::as_str).collect();
        assert_eq!(hint_strs, vec!["line two", "line three"]);
    }

    // The error's own extra lines come before the source chain's. `ignore::Error` has no `source()`, so the chain stops
    // after one multi-line link.
    #[test]
    fn self_and_source_multiline_hints_ordered() {
        let inner_io = std::io::Error::other("io line one\nio line two");
        let source = ignore::Error::Io(inner_io);
        let err = MonoklError::Walk { path: Utf8PathBuf::from("root line one\nroot line two"), source };
        let domain_err = err.to_domain_error(ctx());

        assert_eq!(domain_err.message, "walk: walker error at root line one");
        let hint_strs: Vec<&str> = domain_err.hints.iter().map(michi::Hint::as_str).collect();
        assert_eq!(hint_strs, vec!["root line two", "io line one", "io line two"]);
    }

    #[test]
    fn too_many_terms_enrichment() {
        let err = MonoklError::TooManyTerms { count: 12, limit: 8 };
        let domain_err = err.to_domain_error(ctx());
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "you supplied 12 terms; the limit is 8"),
            "got: {:?}",
            domain_err.hints
        );
        let recovery = domain_err.recovery.expect("recovery must be set");
        assert_eq!(recovery.tool, "search");
        assert_eq!(recovery.params, vec![("maxTerms".to_string(), michi::KvValue::Int(8))]);
        assert_eq!(recovery.reason.as_deref(), Some("re-run with a shorter query"));
    }

    #[test]
    fn file_too_large_enrichment() {
        let err = MonoklError::FileTooLarge { path: Utf8PathBuf::from("/big"), size: 100, cap: 50 };
        let domain_err = err.to_domain_error(ctx());
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "/big is 100 bytes; the cap is 50 bytes"),
            "got: {:?}",
            domain_err.hints
        );
        let recovery = domain_err.recovery.expect("recovery must be set");
        assert_eq!(recovery.tool, "search");
        assert_eq!(recovery.params, vec![("maxBytes".to_string(), michi::KvValue::Int(50))]);
        assert_eq!(recovery.reason.as_deref(), Some("the file exceeds this workspace's size cap"));
    }

    #[test]
    fn invalid_git_ref_enrichment_has_no_recovery() {
        let err = MonoklError::InvalidGitRef { ref_: "-x".to_string(), reason: "bad ref" };
        let domain_err = err.to_domain_error(ctx());
        // An exact match pins the quotes that `{ref_:?}` adds.
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "\"-x\" is not a valid git ref: bad ref"),
            "got: {:?}",
            domain_err.hints
        );
        assert!(domain_err.recovery.is_none());
    }

    #[test]
    fn remaining_variants_get_no_enrichment_beyond_head() {
        let err = MonoklError::LockPoisoned { context: "ctx" };
        let domain_err = err.to_domain_error(ctx());
        assert!(domain_err.hints.is_empty(), "got: {:?}", domain_err.hints);
        assert!(domain_err.recovery.is_none());
    }

    #[test]
    fn walk_glob_maps_to_invalid_input() {
        let glob_err = ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err();
        assert!(matches!(glob_err, ignore::Error::Glob { .. }), "expected a bare Glob error, got: {glob_err:?}");
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: glob_err };
        assert_eq!(err.to_domain_error(ctx()).code, michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_io_not_found_maps_to_not_found() {
        let io_err = ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: io_err };
        assert_eq!(err.to_domain_error(ctx()).code, michi::ErrorCode::NotFound);
    }

    #[test]
    fn walk_nested_glob_inside_with_path_inside_with_line_number() {
        // The shape a malformed `.gitignore` line produces.
        let nested = ignore::Error::WithLineNumber {
            line: 3,
            err: Box::new(ignore::Error::WithPath {
                path: std::path::PathBuf::from(".gitignore"),
                err: Box::new(ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err()),
            }),
        };
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: nested };
        assert_eq!(err.to_domain_error(ctx()).code, michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_loop_maps_to_external_failure() {
        let loop_err =
            ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: loop_err };
        assert_eq!(err.to_domain_error(ctx()).code, michi::ErrorCode::ExternalFailure);
    }

    #[test]
    fn split_display_lines_handles_crlf_lf_cr_and_plain() {
        assert_eq!(split_display_lines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_display_lines("single"), vec!["single"]);
        assert_eq!(split_display_lines(""), vec![""]);
    }
}
