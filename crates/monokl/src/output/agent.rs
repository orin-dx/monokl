//! Agent-facing output: TOON projections of command results and conversion of [`MonoklError`] into michi errors.
use std::collections::BTreeMap;

use camino::Utf8Path;
use michi::toon::Value;

use crate::error::MonoklError;
use crate::types::{Diagnostic, RankedBlock, SearchResponse};

/// Maps `None` to [`Value::Null`], so a reader can tell an absent field from an empty string.
fn opt_cell(s: Option<&str>) -> Value {
    s.map_or(Value::Null, Value::from)
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
    opt_cell(parent.map(|p| format!("{} {}", p.kind.as_str(), p.name)).as_deref())
}

fn search_row(block: &crate::types::RankedBlock) -> [Value; 9] {
    [
        Value::from(block.block.file.as_str()),
        Value::from(block.block.line_start),
        Value::from(block.block.line_end),
        Value::from(block.block.node_kind.as_str()),
        parent_cell(block.block.parent.as_ref()),
        opt_cell(block.block.symbol_signature.as_deref()),
        opt_cell(snippet(&block.block).as_deref()),
        Value::from(block.rank),
        Value::from(block.final_score),
    ]
}

const SEARCH_FIELDS: [&str; 9] =
    ["file", "lineStart", "lineEnd", "nodeKind", "parent", "symbolSignature", "snippet", "rank", "finalScore"];

/// The line `monokl search` prints for each of `blocks`, indentation included and the newline left out.
///
/// The search budget measures these lines, so a cap counts what the reader receives. Rendering them in one table keeps
/// michi's cell quoting and truncation as the only definition of a row.
pub fn search_row_texts(blocks: &[RankedBlock]) -> Vec<String> {
    let rows: Vec<Vec<Value>> = blocks.iter().map(|block| search_row(block).to_vec()).collect();
    let rendered = michi::AgentResponse::new("search").items(rows, SEARCH_FIELDS.as_slice()).render_toon();
    rendered.lines().skip(1).map(str::to_owned).collect()
}

/// Renders a search response as a TOON table of its rows, followed by the truncation count and hints.
pub fn render_search(response: &SearchResponse) -> String {
    let rows: Vec<Vec<Value>> = response.results.iter().map(|block| search_row(block).to_vec()).collect();
    let mut rendered =
        michi::AgentResponse::new("search").items(rows, SEARCH_FIELDS.as_slice()).hints(search_hints(response));
    if response.truncated {
        rendered = rendered.total_count(response.total_blocks_before_truncation);
    }
    rendered.render_toon()
}

fn search_hints(response: &SearchResponse) -> Vec<michi::Hint> {
    let shown =
        michi::TruncationOutcome::new(response.results.iter().collect(), response.total_blocks_before_truncation);
    let mut hints: Vec<michi::Hint> = shown.hint(TRUNCATION_GUIDANCE).into_iter().collect();
    hints.extend(diagnostic_lines(&response.diagnostics).into_iter().map(michi::Hint::new));
    if response.results.is_empty() {
        hints.push(michi::Hint::new(
            "only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg",
        ));
    }
    hints
}

/// One line per distinct diagnostic message; a message tied to files is counted, says what happened to them and names
/// the first few.
fn diagnostic_lines(diagnostics: &[Diagnostic]) -> Vec<String> {
    const LISTED_FILES: usize = 3;
    let mut groups: BTreeMap<(&str, Option<&'static str>), Vec<&str>> = BTreeMap::new();
    for d in diagnostics {
        let verb = d.path.as_ref().map(|_| d.kind.verb());
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

#[cfg(test)]
mod snippet_tests {
    use super::*;
    use crate::types::{CodeBlock, SymbolKind};
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
            name: None,
            matched_lines: matched_lines.to_vec(),
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
    use crate::types::DiagnosticKind;

    #[test]
    fn a_search_row_shows_a_bounded_snippet_not_the_block() {
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
                    name: None,
                    matched_lines: vec![5, 6],
                    parent: None,
                },
                final_score: 0.92,
                rank: 2,
            }],
            total_blocks_before_truncation: 1,
            truncated: false,
            diagnostics: Vec::new(),
        };

        let rendered = render_search(&response);
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

        let ranked = |line_start, node_kind, parent, rank| RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from("src/a.ts"),
                line_start,
                line_end: line_start + 2,
                node_kind,
                code: String::new(),
                symbol_signature: None,
                name: None,
                matched_lines: Vec::new(),
                parent,
            },
            final_score: 0.5,
            rank,
        };
        let class = |name: &str| ParentContext { kind: SymbolKind::Class, name: name.to_owned() };
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
            render_search(&response),
            "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n  \
             src/a.ts,3,5,method,class Foo,null,null,1,0.5\n  \
             src/a.ts,7,9,constructor,class Bar Baz,null,null,2,0.5\n  \
             src/a.ts,1,3,class,null,null,null,3,0.5"
        );
    }

    #[test]
    fn an_empty_search_says_what_went_wrong() {
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
            render_search(&response),
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
    fn a_budgeted_search_reports_the_cut() {
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
                    name: None,
                    matched_lines: Vec::new(),
                    parent: None,
                },
                final_score: 0.5,
                rank,
            })
            .collect();
        let budget = Budget { max_results: 2, ..Budget::default() };
        let texts = search_row_texts(&ranked);

        let rendered = render_search(&apply_budget(ranked, 3, &budget, &texts, Vec::new()));
        assert_eq!(
            rendered,
            "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n  \
             src/f1.rs,1,2,function,null,null,null,1,0.5\n  \
             src/f2.rs,2,3,function,null,null,null,2,0.5\n\
             totalCount: 3\n\
             help[1]: showing 2 of 3; raise the cap or refine the query with +term or -term"
        );
    }

    // The other projection tests render one row each, which would not catch a projection that drops later rows.
    #[test]
    fn multi_row_flattening_search() {
        use crate::types::{CodeBlock, SearchResponse, SymbolKind};
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
                    name: None,
                    matched_lines: Vec::new(),
                    parent: None,
                },
                final_score,
                rank,
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

        let rendered = render_search(&search);
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

/// Classifies a walk error: an I/O failure by its kind, a bad glob as invalid input, anything else as external.
fn walk_error_code(source: &ignore::Error) -> michi::ErrorCode {
    match source.io_error() {
        Some(io_err) => io_error_code(io_err.kind()),
        None if matches!(source, ignore::Error::Glob { .. }) => michi::ErrorCode::InvalidInput,
        None => michi::ErrorCode::ExternalFailure,
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    fn io(kind: std::io::ErrorKind) -> ignore::Error {
        ignore::Error::Io(std::io::Error::new(kind, "x"))
    }

    #[test]
    fn a_glob_is_invalid_input_and_a_loop_is_an_external_failure() {
        let glob_err = ignore::overrides::OverrideBuilder::new(".").add("[").unwrap_err();
        assert_eq!(walk_error_code(&glob_err), michi::ErrorCode::InvalidInput);
        let loop_err =
            ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        assert_eq!(walk_error_code(&loop_err), michi::ErrorCode::ExternalFailure);
    }

    #[test]
    fn an_io_error_maps_by_its_kind() {
        assert_eq!(walk_error_code(&io(std::io::ErrorKind::NotFound)), michi::ErrorCode::NotFound);
        assert_eq!(walk_error_code(&io(std::io::ErrorKind::PermissionDenied)), michi::ErrorCode::Forbidden);
        assert_eq!(walk_error_code(&io(std::io::ErrorKind::NotADirectory)), michi::ErrorCode::InvalidInput);
        assert_eq!(walk_error_code(&io(std::io::ErrorKind::AlreadyExists)), michi::ErrorCode::ExternalFailure);
    }

    #[test]
    fn a_wrapped_io_error_maps_by_its_kind() {
        let wrapped = ignore::Error::WithPath {
            path: std::path::PathBuf::from("x"),
            err: Box::new(io(std::io::ErrorKind::NotFound)),
        };
        assert_eq!(walk_error_code(&wrapped), michi::ErrorCode::NotFound);
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
    fn walk_loop_maps_to_external_failure() {
        let loop_err =
            ignore::Error::Loop { ancestor: std::path::PathBuf::from("/a"), child: std::path::PathBuf::from("/a/b") };
        let err = MonoklError::Walk { path: Utf8PathBuf::from("/x"), source: loop_err };
        assert_eq!(err.to_domain_error().code, michi::ErrorCode::ExternalFailure);
    }
}
