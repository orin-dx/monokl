use michi::toon::Value;

use crate::error::MonoklError;
use crate::types::{SymbolKind, Visibility};

/// Carries the name of the CLI (sub)command currently executing, so
/// `RecoveryHint::new()` can name the correct tool for an agent to retry.
pub struct ErrorContext<'a> {
    pub command: &'a str,
}

/// The only sanctioned way to convert a `&str` into a `michi::toon::Value`
/// inside this module. Pre-escapes `\n`/`\r` before handing the string to
/// michi, since michi-toon's own `escape_value` deletes them silently.
pub fn cell(s: &str) -> Value {
    if s.contains('\n') || s.contains('\r') {
        let mut out = String::with_capacity(s.len());
        for ch in s.chars() {
            match ch {
                '\r' => {}
                '\n' => out.push_str("\\n"),
                _ => out.push(ch),
            }
        }
        Value::from(out)
    } else {
        Value::from(s)
    }
}

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
    fn cell_replaces_lf_with_literal_backslash_n() {
        let v = cell("line1\nline2");
        assert_eq!(v, Value::from("line1\\nline2"));
    }

    #[test]
    fn cell_drops_cr_and_replaces_lf() {
        let v = cell("line1\r\nline2");
        assert_eq!(v, Value::from("line1\\nline2"));
    }

    #[test]
    fn cell_drops_bare_cr() {
        let v = cell("line1\rline2");
        assert_eq!(v, Value::from("line1line2"));
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

    /// AC-001: verifies michi-toon's own `escape_value` deletes `\n`/`\r`
    /// rather than escaping them -- the exact defect `cell()` exists to
    /// route around. If michi's pinned rev ever fixes this, this test fails
    /// loudly rather than the workaround silently going stale.
    #[test]
    fn ac001_michi_escape_value_deletes_newlines() {
        let raw = "// comment\nfn body() {\n    real_code();\n}";
        let escaped = michi::toon::escape_value(raw);
        assert!(!escaped.contains('\n'), "expected newline deletion, got: {escaped:?}");
        assert_eq!(escaped, "// commentfn body() {    real_code();}");
    }

}

/// `SymbolKind` has no `as_str()`/`Display` of its own (SPEC-003 AC-001);
/// this returns the same camelCase strings serde's `rename_all = "camelCase"`
/// already produces for it (SPEC-003 AC-002), via a direct, exhaustive
/// match with no default arm.
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

/// `Visibility` has no `as_str()`/`Display` of its own (SPEC-003 AC-003);
/// same rationale as `symbol_kind_str`.
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
        // B-005: all 15 variants pinned, not just 3.
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
        // B-005: all 4 variants pinned, not just 2.
        assert_eq!(visibility_str(Visibility::Public), "public");
        assert_eq!(visibility_str(Visibility::Crate), "crate");
        assert_eq!(visibility_str(Visibility::Module), "module");
        assert_eq!(visibility_str(Visibility::Private), "private");
    }
}

pub trait ToonProjection<const N: usize> {
    const TYPE_NAME: &'static str;
    const FIELDS: [&'static str; N];
    fn toon_rows(&self) -> Vec<[Value; N]>;
    fn toon_total_count(&self) -> Option<usize> {
        None
    }
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
        vec![michi::Hint::new(
            "use extract with file/lineStart/lineEnd to retrieve a row's actual code content",
        )]
    }
}

/// Constructs a `DomainError` directly from a `michi::toon::ToonError` --
/// deliberately not via `ToDomainError`, which is scoped only to
/// `MonoklError`'s closed `monokl_code()` set. Indicates a bug in this
/// module's own row/field construction, not a caller input error.
pub fn toon_validation_error(e: &michi::toon::ToonError) -> michi::DomainError {
    michi::DomainError::new(michi::ErrorCode::ExternalFailure, format!("toon_render_invalid: {e}")).retryable(false)
}

/// The sole function that converts a `ToonProjection<N>` implementation into
/// a rendered TOON string.
#[allow(clippy::result_large_err)] // AC-004B locks this exact signature verbatim against michi's own DomainError.
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
    fn ac004b_render_projection_symbols_result_end_to_end() {
        use crate::types::{SymbolEntry, SymbolKind, SymbolsResult};
        use camino::Utf8PathBuf;
        use std::collections::BTreeMap;

        let mut files = BTreeMap::new();
        files.insert(
            Utf8PathBuf::from("src/lib.rs"),
            vec![SymbolEntry {
                name: "foo".to_string(),
                kind: SymbolKind::Function,
                // B-008/AC-001: a literal embedded newline in a `cell()`-routed
                // field must survive as the two-character escape `\n`, not a
                // raw newline (which would corrupt the row) and not silently
                // vanish (michi-toon's own `escape_value` deletes it).
                line: 10,
                signature: Some("fn foo(x)\nsecond line".to_string()),
                // B-009/AC-005: all three `Option` columns populated with
                // distinct, recognizable values so a hardcoded `Value::Null`
                // substitution for any of them cannot survive undetected.
                owner: Some("MyOwner".to_string()),
                trait_impl: Some("MyTrait".to_string()),
                visibility: Some(crate::types::Visibility::Public),
                kind_detail: Some("static".to_string()),
            }],
        );
        let result =
            SymbolsResult { files, total_symbol_count: 1, truncation_marker: None, diagnostics: Vec::new() };

        let rendered = render_projection(&result).expect("valid projection must render");
        assert!(rendered.starts_with("symbols[1]{file,name,kind,line,signature,owner,traitImpl,visibility,kindDetail}:\n"), "got: {rendered}");
        assert!(
            rendered.contains("src/lib.rs,foo,function,10,fn foo(x)\\nsecond line,MyOwner,MyTrait,public,static"),
            "got: {rendered}"
        );
    }

    #[test]
    fn ac004b_render_projection_search_response_includes_extract_hint() {
        use crate::types::{CodeBlock, RankedBlock, SearchResponse, SymbolKind};
        use camino::Utf8PathBuf;

        let response = SearchResponse {
            results: vec![RankedBlock {
                block: CodeBlock {
                    file: Utf8PathBuf::from("src/lib.rs"),
                    // B-010/AC-006: line_start/line_end/rank/final_score are
                    // all distinct so a column swap between block.block's
                    // fields (line_start/line_end) and block's own fields
                    // (rank/final_score) is actually detectable -- with the
                    // prior degenerate all-1.0/all-1 values, such a swap
                    // would have rendered an identical row.
                    line_start: 5,
                    line_end: 9,
                    node_kind: SymbolKind::Function,
                    code: "fn foo() {}".to_string(),
                    // B-008/AC-001: a literal embedded newline in an
                    // `opt_cell()`-routed field must survive as the
                    // two-character escape `\n`, not corrupt the row and not
                    // silently vanish.
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
        assert!(rendered.contains("search[1]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:\n"), "got: {rendered}");
        assert!(
            rendered.contains("src/lib.rs,5,9,function,fn foo(x)\\nsecond line,2,0.92"),
            "got: {rendered}"
        );
        assert!(!rendered.contains("fn foo() {}"), "code field must never appear in TOON output, got: {rendered}");
        // B-011/AC-006/AC-012: the extract hint's exact literal text, not a
        // substring -- a mutant altering any word of this prose would
        // otherwise survive.
        assert!(
            rendered.contains(
                "help[1]:\n  use extract with file/lineStart/lineEnd to retrieve a row's actual code content\n"
            ),
            "got: {rendered}"
        );
    }

    /// AC-014: an invalid ToonOptions (deliberately mismatched arity, forced
    /// via a hand-built ToonOptions rather than a real ToonProjection impl,
    /// since ToonProjection<N> makes a real mismatch impossible to construct)
    /// is caught by render_projection's own explicit validate() call and
    /// converted via toon_validation_error, not silently rendered.
    #[test]
    fn ac014_toon_validation_error_shape() {
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

    /// B-013/AC-014: `render_projection`'s own explicit `validate()` guard
    /// (`if let Err(e) = opts.validate() { return Err(toon_validation_error(&e)); }`)
    /// has zero end-to-end coverage from `ac014_toon_validation_error_shape`
    /// above -- that test exercises `toon_validation_error()` in isolation via
    /// a hand-built `ToonOptions`, never through `render_projection` itself,
    /// so deleting the guard leaves every existing test green. This drives a
    /// real `ToonProjection<1>` impl with a deliberately invalid `FIELDS`
    /// entry (a comma, one of michi-toon's own structural chars -- verified
    /// against `ToonOptions::validate()`'s real source, crates/michi-toon/src/lib.rs,
    /// which rejects any field name containing a `STRUCTURAL` char via
    /// `Err(ToonError::InvalidFieldName)`) through `render_projection` itself.
    struct InvalidFieldNameProjection;
    impl ToonProjection<1> for InvalidFieldNameProjection {
        const TYPE_NAME: &'static str = "invalid";
        const FIELDS: [&'static str; 1] = ["a,b"];
        fn toon_rows(&self) -> Vec<[Value; 1]> {
            vec![[Value::from("x")]]
        }
    }

    #[test]
    fn ac014_render_projection_end_to_end_validation_error() {
        let err = render_projection(&InvalidFieldNameProjection).expect_err("invalid FIELDS entry must fail validate()");
        assert_eq!(err.code, michi::ErrorCode::ExternalFailure);
        assert!(!err.retryable);
        assert!(err.message.starts_with("toon_render_invalid: "), "got: {}", err.message);
    }

    /// AC-013: michi::toon::Value::Null and Value::from("") render
    /// identically in a full TOON document -- there is no way for a reader
    /// to distinguish an absent field from a present empty-string field.
    #[test]
    fn ac013_null_and_empty_string_render_identically() {
        let opts_null = michi::toon::ToonOptions::new(
            "t".to_string(),
            vec!["a".to_string()],
            vec![vec![Value::Null]],
        );
        let opts_empty = michi::toon::ToonOptions::new(
            "t".to_string(),
            vec!["a".to_string()],
            vec![vec![Value::from("")]],
        );
        assert_eq!(michi::toon::render_toon(&opts_null), michi::toon::render_toon(&opts_empty));
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

    /// B-007 (AC-004B step 4): a `ToonProjection` whose `toon_total_count()`
    /// returns `Some(n)` must have that `n` reach the rendered output as a
    /// literal `totalCount: {n}` line -- proving `render_projection`'s own
    /// `if let Some(n) = value.toon_total_count() { resp = resp.total_count(n); }`
    /// step is actually exercised, not dead code every other test happens to
    /// never trigger (every other `ToonProjection` impl in this module's own
    /// tests relies on the trait's `None` default).
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
    fn b007_render_projection_wires_toon_total_count_into_output() {
        let rendered = render_projection(&DummyProjectionWithTotal).expect("valid projection must render");
        assert!(rendered.contains("totalCount: 42"), "got: {rendered}");
    }

    /// B-012/AC-005 + B-014/AC-006: every prior projection test used exactly one
    /// row, so the multi-row flattening loop itself (`SymbolsResult`'s nested
    /// `for (file, entries) in &self.files { for e in entries { .. } }`, and
    /// `SearchResponse`'s `self.results.iter().map(..)`) was unproven -- a
    /// `.take(1)` on either outer iterator left every existing test green.
    #[test]
    fn ac005_ac006_multi_row_flattening_symbols_and_search() {
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
        // Inserted out of key order -- src/z.rs before src/a.rs -- so a passing
        // test proves BTreeMap's own key-sorted iteration governs render order,
        // not insertion order.
        files.insert(Utf8PathBuf::from("src/z.rs"), vec![entry("z1", 1), entry("z2", 2)]);
        files.insert(Utf8PathBuf::from("src/a.rs"), vec![entry("a1", 3)]);
        let symbols = SymbolsResult { files, total_symbol_count: 3, truncation_marker: None, diagnostics: Vec::new() };

        assert_eq!(symbols.toon_rows().len(), 3, "all (file, entry) pairs across every BTreeMap key must flatten into rows");

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
/// AC-009 (Walk half only -- `MonoklError::Walk`'s wrapped `ignore::Error`).
/// `ignore::Error` never overrides `std::error::Error::source()`, so this
/// recurses through its own public wrapper variants directly instead. Match
/// arms are deliberately kept separate (not combined via `|`) to match
/// AC-009's own verbatim-locked, independently-compiled function text.
#[allow(clippy::match_same_arms)]
fn innermost_walk_error(e: &ignore::Error) -> &ignore::Error {
    match e {
        ignore::Error::WithLineNumber { err, .. } => innermost_walk_error(err),
        ignore::Error::WithPath { err, .. } => innermost_walk_error(err),
        ignore::Error::WithDepth { err, .. } => innermost_walk_error(err),
        ignore::Error::Partial(errs) if errs.len() == 1 => innermost_walk_error(&errs[0]),
        other => other,
    }
}

/// Match arms kept in AC-009's own verbatim-locked order (not reordered to
/// merge identical `InvalidInput` bodies) so this function stays a direct,
/// literal match against the independently-compiled text AC-009 quotes.
#[allow(clippy::match_same_arms)]
// AC-009 locks `unreachable!()` as the correct body for the WithLineNumber/WithPath/WithDepth
// wrapper arms below -- `innermost_walk_error` provably strips them before `leaf` can hold one.
#[allow(clippy::unreachable)]
fn io_error_code(kind: std::io::ErrorKind) -> michi::ErrorCode {
    use michi::ErrorCode;
    match kind {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::Forbidden,
        _ => ErrorCode::ExternalFailure,
    }
}

fn walk_error_code(source: &ignore::Error) -> michi::ErrorCode {
    use michi::ErrorCode;
    let leaf = innermost_walk_error(source);
    match leaf {
        ignore::Error::Io(io_err) => io_error_code(io_err.kind()),
        ignore::Error::Glob { .. } => ErrorCode::InvalidInput,
        ignore::Error::Loop { .. } => ErrorCode::ExternalFailure,
        ignore::Error::UnrecognizedFileType(_) => ErrorCode::InvalidInput,
        ignore::Error::InvalidDefinition => ErrorCode::InvalidInput,
        ignore::Error::Partial(errs) => {
            let _ = errs;
            ErrorCode::ExternalFailure
        }
        ignore::Error::WithLineNumber { .. } => unreachable!("innermost_walk_error strips WithLineNumber"),
        ignore::Error::WithPath { .. } => unreachable!("innermost_walk_error strips WithPath"),
        ignore::Error::WithDepth { .. } => unreachable!("innermost_walk_error strips WithDepth"),
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    #[test]
    fn ac009_walk_error_code_direct_glob_and_loop() {
        let glob_err = ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err();
        assert_eq!(walk_error_code(&glob_err), michi::ErrorCode::InvalidInput);
        let loop_err = ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        assert_eq!(walk_error_code(&loop_err), michi::ErrorCode::ExternalFailure);
    }

    #[test]
    fn ac009_innermost_walk_error_unwraps_nested_wrappers() {
        let inner = ignore::Error::InvalidDefinition;
        let wrapped = ignore::Error::WithDepth {
            depth: 2,
            err: Box::new(ignore::Error::WithPath {
                path: std::path::PathBuf::from("x"),
                err: Box::new(ignore::Error::WithLineNumber { line: 1, err: Box::new(inner) }),
            }),
        };
        assert!(matches!(innermost_walk_error(&wrapped), ignore::Error::InvalidDefinition));
        // B-003: also assert the resulting ErrorCode, not just leaf identity.
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

    #[test]
    fn innermost_walk_error_partial_single_recurses() {
        let inner = ignore::Error::InvalidDefinition;
        let partial = ignore::Error::Partial(vec![inner]);
        assert!(matches!(innermost_walk_error(&partial), ignore::Error::InvalidDefinition));
    }

    #[test]
    fn innermost_walk_error_partial_non_single_does_not_recurse() {
        let partial_empty = ignore::Error::Partial(vec![]);
        assert!(matches!(innermost_walk_error(&partial_empty), ignore::Error::Partial(v) if v.is_empty()));
        let partial_two = ignore::Error::Partial(vec![ignore::Error::InvalidDefinition, ignore::Error::InvalidDefinition]);
        assert!(matches!(innermost_walk_error(&partial_two), ignore::Error::Partial(v) if v.len() == 2));
    }

    #[test]
    fn walk_error_code_io_permission_denied_maps_to_forbidden() {
        let io_err = ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"));
        assert_eq!(walk_error_code(&io_err), michi::ErrorCode::Forbidden);
    }
}
/// Converts `Self` into a `michi::DomainError`, preserving classification
/// via a `monokl_code()` message prefix and the full `std::error::Error`
/// source chain.
pub trait ToDomainError {
    fn monokl_code(&self) -> &'static str;
    fn to_domain_error(&self, ctx: ErrorContext<'_>) -> michi::DomainError;
}

impl ToDomainError for MonoklError {
    /// One distinct `snake_case` string per variant.
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

    #[allow(clippy::cast_possible_wrap)] // count/limit/size/cap are always small in practice; KvValue::Int requires i64.
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
            MonoklError::TokenizerInit | MonoklError::Json(_) | MonoklError::Git { .. } | MonoklError::LockPoisoned { .. } => {
                ErrorCode::ExternalFailure
            }
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
            MonoklError::TooManyTerms { count, limit } => domain_err
                .hint(format!("you supplied {count} terms; the limit is {limit}"))
                .recovery(
                    michi::RecoveryHint::new(ctx.command)
                        .param("maxTerms", michi::KvValue::Int(*limit as i64))
                        .reason("re-run with a shorter query"),
                ),
            MonoklError::FileTooLarge { path, size, cap } => domain_err
                .hint(format!("{path} is {size} bytes; the cap is {cap} bytes"))
                .recovery(
                    michi::RecoveryHint::new(ctx.command)
                        .param("maxBytes", michi::KvValue::Int(*cap as i64))
                        .reason("the file exceeds this workspace's size cap"),
                ),
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
    fn ac007_monokl_code_covers_13_distinct_nonempty_codes() {
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
        let codes: Vec<&str> = errs.iter().map(|e| e.monokl_code()).collect();
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
    fn ac008_error_code_mapping() {
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

        // B-002: the remaining 7 non-Walk variants of the fixed ErrorCode
        // mapping table (Walk's own fan-out is covered separately by the
        // walk_error_code/AC-009 tests below).
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
    fn ac011_retryable_always_false_even_for_external_failure_codes() {
        let err = MonoklError::LockPoisoned { context: "ctx" };
        let domain_err = err.to_domain_error(ctx());
        assert_eq!(domain_err.code, michi::ErrorCode::ExternalFailure);
        assert!(!domain_err.retryable, "monokl errors must never be retryable");
        let michi_err = michi::Error::Domain(domain_err);
        assert_eq!(michi_err.class(), michi::ErrorClass::Internal, "must not classify as Transient");
        assert!(!michi_err.is_retryable());
    }

    #[test]
    fn ac010_head_is_selfs_own_display_not_sources() {
        // Walk's own Display carries `path`, which lives ONLY in Walk's own
        // Display text, never in the wrapped ignore::Error's Display.
        let err = MonoklError::Walk {
            path: Utf8PathBuf::from("/some/root"),
            source: ignore::Error::InvalidDefinition,
        };
        let domain_err = err.to_domain_error(ctx());
        assert!(domain_err.message.contains("/some/root"), "got: {}", domain_err.message);
        assert!(domain_err.message.starts_with("walk: walker error at /some/root"), "got: {}", domain_err.message);
    }

    /// B-001 regression: `head` must be exactly the FIRST LINE of self's own
    /// Display text -- reproduces the exact corruption case a real
    /// multi-line `Git { message }` produces before the fix (extra physical
    /// lines with no `message:`/`help[`/`  ` prefix at all). Before the
    /// fix, `head` was `self.to_string()` unsplit, so `domain_err.message`
    /// would contain all three lines joined by raw `\n`; after the fix,
    /// only the first line is in `message`, and the remaining lines are
    /// separate `.hint()` entries.
    #[test]
    fn ac010_b001_multiline_self_display_head_is_first_line_only() {
        let err = MonoklError::Git { operation: "show", message: "line one\nline two\nline three".to_string() };
        let domain_err = err.to_domain_error(ctx());

        assert_eq!(domain_err.message, "git: git show failed: line one");
        assert!(!domain_err.message.contains('\n'), "got: {:?}", domain_err.message);
        let hint_strs: Vec<&str> = domain_err.hints.iter().map(michi::Hint::as_str).collect();
        assert_eq!(hint_strs, vec!["line two", "line three"]);
    }

    /// B-004: covers both halves of AC-010 in one place -- self's own
    /// multi-line Display is split with element 0 as `head` and the rest
    /// pushed as hints (B-001's fix), AND those self-derived hints precede
    /// the hints produced by the separate source-chain-flattening walk
    /// (whose own link -- `ignore::Error::Io`'s Display -- is itself
    /// multi-line here, exercising that no source() call exists on
    /// `ignore::Error` per AC-009, so the walk terminates after exactly one
    /// link). Ordering: self's extra lines first, then the source walk's.
    #[test]
    fn ac010_b004_self_and_source_multiline_hints_ordered() {
        let inner_io = std::io::Error::other("io line one\nio line two");
        let source = ignore::Error::Io(inner_io);
        let err = MonoklError::Walk { path: Utf8PathBuf::from("root line one\nroot line two"), source };
        let domain_err = err.to_domain_error(ctx());

        assert_eq!(domain_err.message, "walk: walker error at root line one");
        let hint_strs: Vec<&str> = domain_err.hints.iter().map(michi::Hint::as_str).collect();
        assert_eq!(hint_strs, vec!["root line two", "io line one", "io line two"]);
    }

    #[test]
    fn ac012_too_many_terms_enrichment() {
        let err = MonoklError::TooManyTerms { count: 12, limit: 8 };
        let domain_err = err.to_domain_error(ctx());
        // B-011/AC-012: the exact `.hint(...)` literal, not a substring --
        // a mutant altering the tail text ("; the limit is {limit}") would
        // otherwise survive.
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "you supplied 12 terms; the limit is 8"),
            "got: {:?}",
            domain_err.hints
        );
        let recovery = domain_err.recovery.expect("recovery must be set");
        assert_eq!(recovery.tool, "search");
        assert_eq!(recovery.params, vec![("maxTerms".to_string(), michi::KvValue::Int(8))]);
        // B-005/AC-012: the exact `.reason(...)` literal is locked verbatim.
        assert_eq!(recovery.reason.as_deref(), Some("re-run with a shorter query"));
    }

    #[test]
    fn ac012_file_too_large_enrichment() {
        let err = MonoklError::FileTooLarge { path: Utf8PathBuf::from("/big"), size: 100, cap: 50 };
        let domain_err = err.to_domain_error(ctx());
        // B-011/AC-012: the exact `.hint(...)` literal, not a substring --
        // a mutant altering the tail text ("; the cap is {cap} bytes")
        // would otherwise survive.
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "/big is 100 bytes; the cap is 50 bytes"),
            "got: {:?}",
            domain_err.hints
        );
        let recovery = domain_err.recovery.expect("recovery must be set");
        // B-005: sibling of ac012_too_many_terms_enrichment's own .tool check.
        assert_eq!(recovery.tool, "search");
        assert_eq!(recovery.params, vec![("maxBytes".to_string(), michi::KvValue::Int(50))]);
        // B-005/AC-012: the exact `.reason(...)` literal is locked verbatim.
        assert_eq!(recovery.reason.as_deref(), Some("the file exceeds this workspace's size cap"));
    }

    #[test]
    fn ac012_invalid_git_ref_enrichment_has_no_recovery() {
        let err = MonoklError::InvalidGitRef { ref_: "-x".to_string(), reason: "bad ref" };
        let domain_err = err.to_domain_error(ctx());
        // B-011/AC-012: the exact `.hint(...)` literal, not a substring --
        // this pins the `{ref_:?}` debug-quoting specifically (a mutant
        // dropping the `:?` to plain `{ref_}` would drop the surrounding
        // quotes and would have survived a mere `.contains("bad ref")`
        // check, since "bad ref" appears either way).
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "\"-x\" is not a valid git ref: bad ref"),
            "got: {:?}",
            domain_err.hints
        );
        assert!(domain_err.recovery.is_none());
    }

    #[test]
    fn ac012_remaining_variants_get_no_enrichment_beyond_head() {
        let err = MonoklError::LockPoisoned { context: "ctx" };
        let domain_err = err.to_domain_error(ctx());
        assert!(domain_err.hints.is_empty(), "got: {:?}", domain_err.hints);
        assert!(domain_err.recovery.is_none());
    }

    #[test]
    fn ac009_walk_glob_maps_to_invalid_input() {
        let glob_err = ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err();
        assert!(matches!(glob_err, ignore::Error::Glob { .. }), "expected a bare Glob error, got: {glob_err:?}");
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: glob_err };
        assert_eq!(err.to_domain_error(ctx()).code, michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn ac009_walk_io_not_found_maps_to_not_found() {
        let io_err = ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: io_err };
        assert_eq!(err.to_domain_error(ctx()).code, michi::ErrorCode::NotFound);
    }

    #[test]
    fn ac009_walk_nested_glob_inside_with_path_inside_with_line_number() {
        // The real shape a malformed .gitignore line produces per AC-009's
        // own finding: a Glob wrapped in WithPath wrapped in WithLineNumber.
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
    fn ac009_walk_loop_maps_to_external_failure() {
        let loop_err = ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
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
