//! Agent-facing output: TOON projections of command results and conversion of [`MonoklError`] into michi errors.
use std::collections::BTreeMap;

use camino::Utf8Path;
use michi::toon::Value;

use crate::error::MonoklError;
use crate::types::{Diagnostic, DiagnosticKind, SymbolKind};

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

/// Returns the `camelCase` name of a [`SymbolKind`].
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

#[cfg(test)]
mod kind_str_tests {
    use super::*;

    #[test]
    fn symbol_kind_str_is_camel_case() {
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
const TRUNCATION_GUIDANCE: &str = "raise the cap or refine the query with +term or -term";

/// The most matching lines a row's snippet shows.
const SNIPPET_LINES: usize = 3;
/// The most characters of one matching line a snippet shows.
const SNIPPET_LINE_CHARS: usize = 160;

/// The lines of `block` that matched the query, numbered, so a reader often needs no second read of the file.
///
/// The block's first line is left out when the signature column already shows all of it. A variable or type alias has
/// no signature, so its first line is the best thing to show; so is a first line that says more than the signature,
/// such as a decorator or the start of an arrow function's body. At most [`SNIPPET_LINES`] lines appear, each cut to
/// [`SNIPPET_LINE_CHARS`] characters, and the lines beyond that are counted, not dropped silently.
fn snippet(block: &crate::types::CodeBlock) -> Option<String> {
    let lines: Vec<&str> = block.code.split('\n').collect();
    let first_line_in_signature = match (block.symbol_signature.as_deref(), lines.first()) {
        (Some(signature), Some(line)) => signature_shows(signature, line),
        _ => false,
    };
    let first = if first_line_in_signature { block.line_start + 1 } else { block.line_start };
    let mut shown: Vec<usize> = block
        .matched_lines
        .iter()
        .copied()
        .filter(|&line| line >= first && line - block.line_start < lines.len())
        .collect();
    shown.sort_unstable();
    shown.dedup();
    let hidden = shown.len().saturating_sub(SNIPPET_LINES);
    let mut out: Vec<String> = shown
        .iter()
        .take(SNIPPET_LINES)
        .map(|&line| {
            let text = lines[line - block.line_start].trim();
            let mut chars = text.chars();
            let head: String = chars.by_ref().take(SNIPPET_LINE_CHARS).collect();
            let ellipsis = if chars.next().is_some() { "…" } else { "" };
            format!("{line}: {head}{ellipsis}")
        })
        .collect();
    if out.is_empty() {
        return None;
    }
    if hidden > 0 {
        out.push(format!("...and {hidden} more matching {}", if hidden == 1 { "line" } else { "lines" }));
    }
    Some(out.join("\n"))
}

/// Whether `signature` already shows everything on the block's first `line`: the line is the signature plus an opening
/// brace, or the signature runs on past the line. Whitespace and leading modifiers (`export`, `async` and the like),
/// which signatures drop, do not count.
fn signature_shows(signature: &str, line: &str) -> bool {
    const MODIFIERS: [&str; 10] =
        ["export", "default", "declare", "async", "abstract", "static", "public", "private", "protected", "override"];
    let normalized = |text: &str| -> String {
        let mut words = text.split_whitespace().peekable();
        while words.next_if(|word| MODIFIERS.contains(word)).is_some() {}
        words.collect::<Vec<_>>().join(" ")
    };
    let (signature, line) = (normalized(signature), normalized(line));
    let line_core = line.trim_end_matches(['{', '}', ' ', ';', ',']);
    signature.contains(line_core)
        || line
            .strip_prefix(signature.as_str())
            .is_some_and(|rest| rest.trim_matches(['{', '}', ' ', ';', ',']).is_empty())
}

/// The `parent` cell: the kind and name of the block's parent, such as `class Foo` for a method; `null` for a
/// top-level block.
fn parent_cell(parent: Option<&crate::types::ParentContext>) -> Value {
    opt_cell(parent.map(|p| format!("{} {}", symbol_kind_str(p.kind), p.name)).as_deref())
}

fn search_row(block: &crate::types::RankedBlock) -> [Value; 9] {
    [
        cell(block.block.file.as_str()),
        Value::from(block.block.line_start),
        Value::from(block.block.line_end),
        cell(symbol_kind_str(block.block.node_kind)),
        parent_cell(block.parent_context.as_ref()),
        opt_cell(block.block.symbol_signature.as_deref()),
        opt_cell(snippet(&block.block).as_deref()),
        Value::from(block.rank),
        Value::from(block.final_score),
    ]
}

/// The line `monokl search` prints for `block`, indentation included and the newline left out.
///
/// The search budget measures this text, so a cap counts what the reader receives.
pub fn search_row_text(block: &crate::types::RankedBlock) -> String {
    let fields = <crate::types::SearchResponse as ToonProjection<9>>::FIELDS;
    let row = search_row(block).to_vec();
    let rendered = michi::AgentResponse::new("search").items(vec![row], fields.as_slice()).render_toon();
    rendered.lines().nth(1).unwrap_or_default().to_owned()
}

impl ToonProjection<9> for crate::types::SearchResponse {
    const TYPE_NAME: &'static str = "search";
    const FIELDS: [&'static str; 9] =
        ["file", "lineStart", "lineEnd", "nodeKind", "parent", "symbolSignature", "snippet", "rank", "finalScore"];

    fn toon_rows(&self) -> Vec<[Value; 9]> {
        self.results.iter().map(search_row).collect()
    }

    fn toon_total_count(&self) -> Option<usize> {
        self.truncated.then_some(self.total_blocks_before_truncation)
    }

    fn toon_hints(&self) -> Vec<michi::Hint> {
        let shown = michi::TruncationOutcome::new(self.results.iter().collect(), self.total_blocks_before_truncation);
        let mut hints: Vec<michi::Hint> = shown.hint(TRUNCATION_GUIDANCE).into_iter().collect();
        hints.extend(diagnostic_lines(&self.diagnostics).into_iter().map(michi::Hint::new));
        if self.results.is_empty() {
            hints.push(michi::Hint::new(
                "only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg",
            ));
        }
        hints
    }
}

/// One line per distinct diagnostic message; a message tied to files is counted, says what happened to them and names
/// the first few.
fn diagnostic_lines(diagnostics: &[Diagnostic]) -> Vec<String> {
    const LISTED_FILES: usize = 3;
    let mut groups: BTreeMap<(&str, Option<&'static str>), Vec<&str>> = BTreeMap::new();
    for d in diagnostics {
        let verb = d.path.as_ref().map(|_| match d.kind {
            DiagnosticKind::Degraded => "degraded",
            DiagnosticKind::Skipped => "skipped",
            DiagnosticKind::Warning => "warned",
            DiagnosticKind::LossyRead => "read lossily",
        });
        let paths = groups.entry((d.message.as_str(), verb)).or_default();
        paths.extend(d.path.as_deref().map(Utf8Path::as_str));
    }
    groups
        .into_iter()
        .map(|((message, verb), mut paths)| match verb {
            Some(verb) => {
                paths.sort_unstable();
                let n = paths.len();
                let listed = paths[..n.min(LISTED_FILES)].join(", ");
                let more = if n > LISTED_FILES { format!(", ...and {} more", n - LISTED_FILES) } else { String::new() };
                format!("{n} {} {verb}: {message}: {listed}{more}", if n == 1 { "file" } else { "files" })
            }
            None => message.to_owned(),
        })
        .collect()
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
mod snippet_tests {
    use super::*;
    use crate::types::CodeBlock;
    use camino::Utf8PathBuf;

    fn block(line_start: usize, code: &str, matched_lines: &[usize]) -> CodeBlock {
        CodeBlock {
            file: Utf8PathBuf::from("src/a.ts"),
            line_start,
            line_end: line_start + code.split('\n').count() - 1,
            node_kind: SymbolKind::Function,
            code: code.to_owned(),
            // What the analyzer would give: the first line without its opening brace.
            symbol_signature: Some(code.lines().next().unwrap_or("").trim_end_matches(['{', ' ']).to_owned()),
            matched_lines: matched_lines.to_vec(),
            matched_keywords: Vec::new(),
            parent: None,
        }
    }

    #[test]
    fn a_snippet_shows_matching_lines_with_their_numbers_but_not_the_signature_line() {
        let b = block(10, "function f() {\n  const price = 1;\n  return price;\n}", &[10, 11, 12]);
        assert_eq!(snippet(&b), Some("11: const price = 1;\n12: return price;".to_owned()));
    }

    #[test]
    fn a_block_matching_only_on_its_first_line_has_no_snippet() {
        let b = block(10, "function price() {\n  return 1;\n}", &[10]);
        assert_eq!(snippet(&b), None);
    }

    #[test]
    fn a_block_without_a_signature_shows_its_first_line_when_it_matched() {
        let mut b = block(3, "export const price = 1;", &[3]);
        b.symbol_signature = None;
        assert_eq!(snippet(&b), Some("3: export const price = 1;".to_owned()));
    }

    #[test]
    fn a_block_with_a_signature_does_not_repeat_it_in_the_snippet() {
        let mut b = block(3, "function price() {\n  return price;\n}", &[3, 4]);
        b.symbol_signature = Some("function price()".to_owned());
        assert_eq!(snippet(&b), Some("4: return price;".to_owned()));
    }

    #[test]
    fn a_decorator_line_the_signature_leaves_out_is_shown_when_it_matched() {
        let mut b = block(2, "@Dec('needle')\nmulti(a: number): void {\n}", &[2]);
        b.symbol_signature = Some("multi(a: number): void".to_owned());
        assert_eq!(snippet(&b), Some("2: @Dec('needle')".to_owned()));
    }

    #[test]
    fn a_decorator_sharing_its_line_with_the_member_is_shown_when_it_matched() {
        let mut b = block(9, "@Dec2() prop2 = () => {\n  return 1;\n}", &[9]);
        b.symbol_signature = Some("prop2 = () =>".to_owned());
        assert_eq!(snippet(&b), Some("9: @Dec2() prop2 = () => {".to_owned()));
    }

    #[test]
    fn a_first_line_that_says_more_than_the_signature_is_shown() {
        let mut b = block(2, "load = () => needle(\n  1,\n);", &[2]);
        b.symbol_signature = Some("load = () =>".to_owned());
        assert_eq!(snippet(&b), Some("2: load = () => needle(".to_owned()));
    }

    #[test]
    fn an_export_keyword_does_not_make_a_covered_first_line_look_new() {
        let mut b = block(1, "export async function f(x) {\n  return x;\n}", &[1]);
        b.symbol_signature = Some("function f(x)".to_owned());
        assert_eq!(snippet(&b), None);
    }

    #[test]
    fn a_signature_that_spans_the_first_line_covers_it() {
        let mut b = block(1, "multi(\n  a: number,\n): void {\n}", &[1]);
        b.symbol_signature = Some("multi( a: number, ): void".to_owned());
        assert_eq!(snippet(&b), None);
    }

    #[test]
    fn a_block_with_no_matched_lines_has_no_snippet() {
        assert_eq!(snippet(&block(1, "a\nb", &[])), None);
    }

    #[test]
    fn a_snippet_shows_three_lines_and_counts_the_rest() {
        let b = block(1, "head\nx1\nx2\nx3\nx4\nx5", &[2, 3, 4, 5, 6]);
        assert_eq!(snippet(&b), Some("2: x1\n3: x2\n4: x3\n...and 2 more matching lines".to_owned()));
    }

    #[test]
    fn a_snippet_counts_one_more_line_in_the_singular() {
        let b = block(1, "head\nx1\nx2\nx3\nx4", &[2, 3, 4, 5]);
        assert_eq!(snippet(&b), Some("2: x1\n3: x2\n4: x3\n...and 1 more matching line".to_owned()));
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let long = "é".repeat(200);
        let b = block(1, &format!("head\n  {long}"), &[2]);
        assert_eq!(snippet(&b), Some(format!("2: {}…", "é".repeat(SNIPPET_LINE_CHARS))));
    }

    #[test]
    fn a_line_at_the_limit_is_not_cut() {
        let exact = "x".repeat(SNIPPET_LINE_CHARS);
        let b = block(1, &format!("head\n{exact}"), &[2]);
        assert_eq!(snippet(&b), Some(format!("2: {exact}")));
    }

    #[test]
    fn carriage_returns_and_indentation_are_trimmed() {
        let b = block(1, "head\r\n    return 1;\r\n", &[2]);
        assert_eq!(snippet(&b), Some("2: return 1;".to_owned()));
    }

    #[test]
    fn matched_lines_outside_the_block_are_ignored() {
        let b = block(5, "head\nbody", &[1, 6, 99]);
        assert_eq!(snippet(&b), Some("6: body".to_owned()));
    }

    #[test]
    fn a_line_matched_twice_is_shown_once() {
        let b = block(1, "head\nbody", &[2, 2]);
        assert_eq!(snippet(&b), Some("2: body".to_owned()));
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn render_projection_search_response_shows_a_bounded_snippet_not_the_block() {
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
                    code: "fn foo(x) {\n    let y = foo(x);\n    unrelated();\n    y\n}".to_string(),
                    symbol_signature: Some("fn foo(x)\nsecond line".to_string()),
                    matched_lines: vec![5, 6],
                    matched_keywords: vec!["foo".to_string()],
                    parent: None,
                },
                final_score: 0.92,
                rank: 2,
                parent_context: None,
            }],
            total_blocks_before_truncation: 1,
            truncated: false,
            diagnostics: Vec::new(),
        };

        let rendered = render_projection(&response).expect("valid projection must render");
        assert_eq!(
            rendered,
            "search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n  \
             src/lib.rs,5,9,function,null,\"fn foo(x)\\nsecond line\",\"6: let y = foo(x);\",2,0.92"
        );
    }

    #[test]
    fn a_member_row_names_its_parent_and_a_top_level_row_has_none() {
        use crate::types::{CodeBlock, ParentContext, RankedBlock, SearchResponse, SymbolKind};
        use camino::Utf8PathBuf;

        let ranked = |line_start, node_kind, parent_context, rank| RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from("src/a.ts"),
                line_start,
                line_end: line_start + 2,
                node_kind,
                code: String::new(),
                symbol_signature: None,
                matched_lines: Vec::new(),
                matched_keywords: Vec::new(),
                parent: None,
            },
            final_score: 0.5,
            rank,
            parent_context,
        };
        let class = |name: &str| ParentContext { kind: SymbolKind::Class, name: name.to_owned(), line: 1 };
        let response = SearchResponse {
            results: vec![
                ranked(3, SymbolKind::Method, Some(class("Foo")), 1),
                ranked(7, SymbolKind::Constructor, Some(class("Bar Baz")), 2),
                ranked(1, SymbolKind::Class, None, 3),
            ],
            total_blocks_before_truncation: 3,
            truncated: false,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            render_projection(&response).expect("valid projection must render"),
            "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n  \
             src/a.ts,3,5,method,class Foo,null,null,1,0.5\n  \
             src/a.ts,7,9,constructor,class Bar Baz,null,null,2,0.5\n  \
             src/a.ts,1,3,class,null,null,null,3,0.5"
        );
    }

    #[test]
    fn render_projection_empty_search_response_says_what_went_wrong() {
        use crate::types::SearchResponse;
        use camino::Utf8PathBuf;

        let diagnostic = |kind, path: Option<&str>, message: &str| Diagnostic {
            kind,
            path: path.map(Utf8PathBuf::from),
            message: message.to_owned(),
        };
        let response = SearchResponse {
            results: Vec::new(),
            total_blocks_before_truncation: 0,
            truncated: false,
            diagnostics: vec![
                diagnostic(DiagnosticKind::Skipped, Some("a.ts"), "unreadable"),
                diagnostic(DiagnosticKind::Skipped, Some("b.ts"), "unreadable"),
                diagnostic(DiagnosticKind::Degraded, Some("c.ts"), "parse errors"),
                diagnostic(DiagnosticKind::Skipped, None, "query has no positive terms"),
            ],
        };

        assert_eq!(
            render_projection(&response).expect("must render"),
            "search: []\n\
             help[4]: \"1 file degraded: parse errors: c.ts\",query has no positive terms,\
             \"2 files skipped: unreadable: a.ts, b.ts\",\
             \"only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg\""
        );
    }

    #[test]
    fn diagnostic_lines_name_one_file() {
        let diagnostics = [Diagnostic {
            kind: DiagnosticKind::Degraded,
            path: Some("src/big.ts".into()),
            message: "file too large to parse (300000 bytes, over the cap of 260096)".to_owned(),
        }];
        assert_eq!(
            diagnostic_lines(&diagnostics),
            ["1 file degraded: file too large to parse (300000 bytes, over the cap of 260096): src/big.ts"]
        );
    }

    #[test]
    fn diagnostic_lines_name_three_files_and_count_the_rest() {
        let diagnostics: Vec<Diagnostic> = ["e.ts", "b.ts", "d.ts", "a.ts", "c.ts"]
            .into_iter()
            .map(|path| Diagnostic {
                kind: DiagnosticKind::Skipped,
                path: Some(path.into()),
                message: "unreadable".to_owned(),
            })
            .collect();
        assert_eq!(diagnostic_lines(&diagnostics), ["5 files skipped: unreadable: a.ts, b.ts, c.ts, ...and 2 more"]);
    }

    #[test]
    fn diagnostic_lines_count_files_read_lossily_and_name_the_first_three() {
        let diagnostics: Vec<Diagnostic> = ["d.ts", "b.ts", "a.ts", "c.ts"]
            .into_iter()
            .map(|path| Diagnostic {
                kind: DiagnosticKind::LossyRead,
                path: Some(path.into()),
                message: "invalid UTF-8 replaced with U+FFFD".to_owned(),
            })
            .collect();
        assert_eq!(
            diagnostic_lines(&diagnostics),
            ["4 files read lossily: invalid UTF-8 replaced with U+FFFD: a.ts, b.ts, c.ts, ...and 1 more"]
        );
        assert_eq!(
            diagnostic_lines(&diagnostics[..1]),
            ["1 file read lossily: invalid UTF-8 replaced with U+FFFD: d.ts"]
        );
    }

    #[test]
    fn render_projection_budgeted_search_response_reports_the_cut() {
        use crate::budget::{Budget, apply_budget};
        use crate::types::{CodeBlock, RankedBlock, SymbolKind};
        use camino::Utf8PathBuf;

        let ranked: Vec<RankedBlock> = (1..=3)
            .map(|rank| RankedBlock {
                block: CodeBlock {
                    file: Utf8PathBuf::from(format!("src/f{rank}.rs")),
                    line_start: rank,
                    line_end: rank + 1,
                    node_kind: SymbolKind::Function,
                    code: "fn a() {}".to_string(),
                    symbol_signature: None,
                    matched_lines: Vec::new(),
                    matched_keywords: Vec::new(),
                    parent: None,
                },
                final_score: 0.5,
                rank,
                parent_context: None,
            })
            .collect();
        let budget = Budget { max_results: 2, ..Budget::default() };

        let rendered =
            render_projection(&apply_budget(ranked, 3, &budget, search_row_text, Vec::new())).expect("must render");
        assert_eq!(
            rendered,
            "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n  \
             src/f1.rs,1,2,function,null,null,null,1,0.5\n  \
             src/f2.rs,2,3,function,null,null,null,2,0.5\n\
             totalCount: 3\n\
             help[1]: showing 2 of 3; raise the cap or refine the query with +term or -term"
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
    fn multi_row_flattening_search() {
        use crate::types::{CodeBlock, RankedBlock, SearchResponse};
        use camino::Utf8PathBuf;
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
                    parent: None,
                },
                final_score,
                rank,
                parent_context: None,
            }
        }

        let search = SearchResponse {
            results: vec![
                block("src/one.rs", 1, 2, 1, 0.10),
                block("src/two.rs", 3, 4, 2, 0.20),
                block("src/three.rs", 5, 6, 3, 0.30),
            ],
            total_blocks_before_truncation: 3,
            truncated: false,
            diagnostics: Vec::new(),
        };

        assert_eq!(search.toon_rows().len(), 3, "every RankedBlock in `results` must flatten into a row");

        let rendered = render_projection(&search).expect("valid projection must render");
        assert!(
            rendered.starts_with(
                "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n"
            ),
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
        std::io::ErrorKind::NotADirectory => ErrorCode::InvalidInput,
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
    /// Converts the error, prefixing the message with [`monokl_code`](Self::monokl_code) and carrying its source chain
    /// as causes.
    fn to_domain_error(&self) -> michi::DomainError;
}

impl ToDomainError for MonoklError {
    fn monokl_code(&self) -> &'static str {
        match self {
            MonoklError::Io(_) => "io",
            MonoklError::Walk { .. } => "walk",
            MonoklError::RegexBuild(_) => "regex_build",
            MonoklError::NonUtf8Path { .. } => "non_utf8_path",
            MonoklError::TooManyTerms { .. } => "too_many_terms",
        }
    }

    fn to_domain_error(&self) -> michi::DomainError {
        use michi::ErrorCode;

        let code = match self {
            MonoklError::RegexBuild(_) | MonoklError::NonUtf8Path { .. } | MonoklError::TooManyTerms { .. } => {
                ErrorCode::InvalidInput
            }
            MonoklError::Io(err) => io_error_code(err.io_kind()),
            MonoklError::Walk { source, .. } => walk_error_code(source),
        };

        let mut domain_err = michi::DomainError::from_error(code, self).retryable(false);
        // Prefixed after from_error, which compares each cause against the unprefixed message to skip repeats.
        domain_err.message = format!("{}: {}", self.monokl_code(), domain_err.message);

        domain_err = match self {
            MonoklError::TooManyTerms { count, limit } => {
                domain_err.hint(format!("you supplied {count} terms; the limit is {limit}"))
            }
            MonoklError::Io(_)
            | MonoklError::Walk { .. }
            | MonoklError::RegexBuild(_)
            | MonoklError::NonUtf8Path { .. } => domain_err,
        };

        domain_err
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;
    use camino::Utf8PathBuf;

    #[test]
    fn monokl_code_is_distinct_and_nonempty_per_variant() {
        let errs: Vec<MonoklError> = vec![
            MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: ignore::Error::InvalidDefinition },
            grep_regex::RegexMatcherBuilder::new().build("(").unwrap_err().into(),
            MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/bad") },
            MonoklError::TooManyTerms { count: 1, limit: 1 },
        ];
        assert_eq!(errs.len(), 4);
        let codes: Vec<&str> = errs.iter().map(ToDomainError::monokl_code).collect();
        assert_eq!(codes, vec!["walk", "regex_build", "non_utf8_path", "too_many_terms"]);
        let mut dedup = codes.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), 4, "all monokl_code() values must be distinct");
        for c in &codes {
            assert!(!c.is_empty() && !c.contains(' '), "code must be non-empty with no whitespace: {c:?}");
        }
    }

    #[test]
    fn error_code_mapping() {
        assert_eq!(
            MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/x") }.to_domain_error().code,
            michi::ErrorCode::InvalidInput
        );

        // `Walk` is covered by the walk-specific tests.
        let regex_err: MonoklError = grep_regex::RegexMatcherBuilder::new().build("(").unwrap_err().into();
        assert_eq!(regex_err.to_domain_error().code, michi::ErrorCode::InvalidInput);
        assert_eq!(
            MonoklError::TooManyTerms { count: 1, limit: 1 }.to_domain_error().code,
            michi::ErrorCode::InvalidInput
        );
    }

    #[test]
    fn retryable_always_false_even_for_external_failure_codes() {
        let loop_err =
            ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: loop_err };
        let domain_err = err.to_domain_error();
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
        let domain_err = err.to_domain_error();
        assert!(domain_err.message.contains("/some/root"), "got: {}", domain_err.message);
        assert!(domain_err.message.starts_with("walk: walker error at /some/root"), "got: {}", domain_err.message);
    }

    // michi escapes line breaks when it renders, so a multi-line message stays whole and adds no rendered lines.
    #[test]
    fn multiline_message_is_kept_whole_and_renders_on_one_line() {
        let err = MonoklError::Walk {
            path: Utf8PathBuf::from("line one\nline two"),
            source: ignore::Error::InvalidDefinition,
        };
        let domain_err = err.to_domain_error();

        assert_eq!(domain_err.message, "walk: walker error at line one\nline two");
        assert_eq!(domain_err.causes, vec!["invalid definition (format is type:glob, e.g., html:*.html)"]);
        assert!(domain_err.hints.is_empty(), "got: {:?}", domain_err.hints);
        assert_eq!(domain_err.render().lines().count(), 4, "got: {}", domain_err.render());
    }

    // `ignore::Error` has no `source()`, so the chain is the one wrapped walker error.
    #[test]
    fn source_chain_becomes_causes_not_hints() {
        let source = ignore::Error::Io(std::io::Error::other("io line one\nio line two"));
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source };
        let domain_err = err.to_domain_error();

        assert_eq!(domain_err.message, "walk: walker error at /x");
        assert_eq!(domain_err.causes, vec!["io line one\nio line two"]);
        assert!(domain_err.hints.is_empty(), "got: {:?}", domain_err.hints);
    }

    // `Io` is transparent, so its message already is `FileIoError`'s; the cause is the `io::Error` beneath it, once.
    #[test]
    fn io_cause_is_the_wrapped_io_error_once() {
        let err: MonoklError = crate::file_io::FileIoError::read("/x", std::io::Error::other("boom")).into();
        let domain_err = err.to_domain_error();

        assert_eq!(domain_err.message, "io: failed to read /x");
        assert_eq!(domain_err.causes, vec!["boom"]);
    }

    #[test]
    fn too_many_terms_enrichment() {
        let err = MonoklError::TooManyTerms { count: 12, limit: 8 };
        let domain_err = err.to_domain_error();
        assert!(
            domain_err.hints.iter().any(|h| h.as_str() == "you supplied 12 terms; the limit is 8"),
            "got: {:?}",
            domain_err.hints
        );
        assert!(domain_err.recovery.is_none(), "search has no maxTerms flag to recover with");
    }

    #[test]
    fn a_file_as_the_search_root_is_invalid_input() {
        let io_err = ignore::Error::Io(std::io::Error::from(std::io::ErrorKind::NotADirectory));
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x/a.ts"), source: io_err };
        let domain_err = err.to_domain_error();
        assert_eq!(domain_err.code, michi::ErrorCode::InvalidInput);
        assert!(domain_err.message.contains("/x/a.ts"), "got: {}", domain_err.message);
    }

    #[test]
    fn remaining_variants_get_no_enrichment_beyond_head() {
        let err = MonoklError::NonUtf8Path { path: std::path::PathBuf::from("/bad") };
        let domain_err = err.to_domain_error();
        assert!(domain_err.hints.is_empty(), "got: {:?}", domain_err.hints);
        assert!(domain_err.causes.is_empty(), "got: {:?}", domain_err.causes);
        assert!(domain_err.recovery.is_none());
    }

    #[test]
    fn walk_glob_maps_to_invalid_input() {
        let glob_err = ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err();
        assert!(matches!(glob_err, ignore::Error::Glob { .. }), "expected a bare Glob error, got: {glob_err:?}");
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: glob_err };
        assert_eq!(err.to_domain_error().code, michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_io_not_found_maps_to_not_found() {
        let io_err = ignore::Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: io_err };
        assert_eq!(err.to_domain_error().code, michi::ErrorCode::NotFound);
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
        assert_eq!(err.to_domain_error().code, michi::ErrorCode::InvalidInput);
    }

    #[test]
    fn walk_loop_maps_to_external_failure() {
        let loop_err =
            ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: loop_err };
        assert_eq!(err.to_domain_error().code, michi::ErrorCode::ExternalFailure);
    }
}
