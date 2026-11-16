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

/// The most snippet lines a row can show, and the largest value `--snippet-lines` takes.
pub const MAX_SNIPPET_LINES: usize = 3;
/// The snippet lines a row shows unless `--snippet-lines` says otherwise.
pub const DEFAULT_SNIPPET_LINES: usize = 1;
/// The most characters of one matching line a snippet shows.
const SNIPPET_LINE_CHARS: usize = 160;

/// How a search response is shown: how long each row's snippet is, and the row cap and relevance cut the truncation
/// hint names.
///
/// # Examples
///
/// ```
/// use monokl::output::agent::SearchView;
///
/// let view = SearchView::default();
/// assert_eq!((view.snippet_lines, view.max_results, view.min_score), (1, 20, 0.3));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchView {
    /// The most matching lines a row's snippet shows, from 0 to [`MAX_SNIPPET_LINES`]. With 0 the column is `null` and a
    /// variable or type alias row, which has no signature, shows no declaration text.
    pub snippet_lines: usize,
    /// The row cap the response was budgeted with, which tells the hint whether the rows or the tokens ran out.
    pub max_results: usize,
    /// The relevance cut the response was budgeted with, which the hint names as a `relevance` value. The rows it
    /// dropped are those with a `relevance` below `100 * min_score`.
    pub min_score: f64,
}

impl Default for SearchView {
    fn default() -> Self {
        let budget = crate::budget::Budget::default();
        Self { snippet_lines: DEFAULT_SNIPPET_LINES, max_results: budget.max_results, min_score: budget.min_score }
    }
}

/// The lines of `block` that matched the query, numbered, so a reader often needs no second read of the file.
///
/// A line the signature column already shows is left out: one whose text lies inside the block's
/// [`signature_span`](crate::types::CodeBlock::signature_span), except that a Rust `where` clause is not shown in the
/// column (see [`signature_cell`]), so the lines of the clause after the declaration line stay. A variable or type
/// alias has no signature, so its first line is the best thing to show; so is a first line that says more than the
/// signature, such as a decorator or the start of an arrow function's body. A comment or attribute line (a Rust doc
/// comment above the declaration, a `#[...]` attribute, a comment in a body) and a where-clause line say less about the
/// code than a body line does, so a code line that matched is shown first and those fill the snippet only when there
/// are not enough code lines, or are the only match. At most `max_lines` lines appear, in the order they have in the block, each cut to [`SNIPPET_LINE_CHARS`]
/// characters, and the lines beyond that are counted, not dropped silently. With `max_lines` 0 there is no snippet.
fn snippet(block: &crate::types::CodeBlock, max_lines: usize) -> Option<String> {
    let mut lines = Vec::new();
    let mut start = block.span.start;
    for text in block.code.split('\n') {
        lines.push((start, text));
        start += text.len() + 1;
    }
    let where_at = block.where_start;
    let mut shown: Vec<usize> = block
        .matched_lines
        .iter()
        .copied()
        .filter(|&line| line >= block.line_start && line - block.line_start < lines.len())
        .filter(|&line| {
            let (start, text) = lines[line - block.line_start];
            !block.signature_span.as_ref().is_some_and(|head| inside(head, start, text))
                || where_at.is_some_and(|at| start + (text.len() - text.trim_start().len()) >= at)
        })
        .collect();
    shown.sort_unstable();
    shown.dedup();
    let hidden = shown.len().saturating_sub(max_lines);
    // Stable, so each kind of line stays in block order; the chosen lines are put back in block order.
    shown.sort_by_key(|&line| {
        let (start, text) = lines[line - block.line_start];
        let content_start = start + (text.len() - text.trim_start().len());
        let in_where_clause = where_at.is_some_and(|at| content_start >= at)
            && block.signature_span.as_ref().is_some_and(|head| content_start < head.end);
        is_comment_or_attribute(text) || in_where_clause
    });
    shown.truncate(max_lines);
    shown.sort_unstable();
    let mut out: Vec<String> = shown
        .iter()
        .map(|&line| {
            let text = lines[line - block.line_start].1.trim();
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
        out.push(format!("+{hidden} more"));
    }
    Some(out.join("\n"))
}

/// The `symbolSignature` cell. A Rust signature ends where its where clause starts, with ` where …` to say there is one:
/// the clause is a list of bounds that is often longer than the rest of the signature, and the row's snippet shows a
/// line of it when that line matched. The clause is where the analyzer recorded it in
/// [`where_start`](crate::types::CodeBlock::where_start), so a `where` inside a comment or a literal is not mistaken for
/// it; the ranking still reads the whole signature.
fn signature_cell(block: &crate::types::CodeBlock) -> Option<String> {
    let signature = block.symbol_signature.as_deref()?;
    let before_clause = block.signature_span.as_ref().zip(block.where_start).and_then(|(head, at)| {
        let offset = |position: usize| position.checked_sub(block.span.start);
        block.code.get(offset(head.start)?..offset(at)?)
    });
    Some(match before_clause {
        Some(text) => format!("{} where …", text.split_whitespace().collect::<Vec<_>>().join(" ")),
        None => signature.to_owned(),
    })
}

/// Whether a line looks like only a comment or an attribute: it starts with `//`, `/*`, `*/`, `* ` (the body of a block
/// comment), `#[` or `#![`. A line that starts with `*` and no space is code (a dereference), as is any other.
///
/// This reads the line's text, not the syntax tree, because it only orders the lines of a snippet: a line wrongly taken
/// for a comment (one that begins `//` inside a multi-line string) is shown after the other matched lines instead of
/// before them, and is never left out, so a wrong answer costs a less useful snippet line and no information.
fn is_comment_or_attribute(text: &str) -> bool {
    let text = text.trim_start();
    ["//", "/*", "*/", "#[", "#!["].iter().any(|prefix| text.starts_with(prefix))
        || text == "*"
        || text.starts_with("* ")
}

/// Whether the text of a line, which starts at byte `start`, holds something and lies wholly inside `head`.
fn inside(head: &std::ops::Range<usize>, start: usize, text: &str) -> bool {
    let content = text.trim();
    let content_start = start + (text.len() - text.trim_start().len());
    !content.is_empty() && head.start <= content_start && content_start + content.len() <= head.end
}

/// The `parent` cell: the kind and name of the block's parent, such as `class Foo` for a method; `null` for a
/// top-level block.
fn parent_cell(parent: Option<&crate::types::ParentContext>) -> Value {
    opt_cell(parent.map(|p| format!("{} {}", p.kind.as_str(), p.name)).as_deref())
}

/// The `relevance` cell: `score` as a whole percent of `top`, the best score of the whole ranking, so the top row is 100.
///
/// It is computed from the same float the ranking sorts by and the relevance cut compares, never from a printed score.
/// A row the cut keeps scores at least `min_score` times the top, so it shows at least `100 * min_score` (a row within
/// half a point below that line rounds up onto it). A `top` that is not positive leaves nothing to divide by; every row
/// then ties with the top under the cut, so each shows 100.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is rounded and clamped to 0..=100"
)]
fn relevance_percent(score: f64, top: f64) -> usize {
    if top > 0.0 { (score / top * 100.0).round().clamp(0.0, 100.0) as usize } else { 100 }
}

fn search_row(block: &crate::types::RankedBlock, top_score: f64, snippet_lines: usize) -> [Value; 8] {
    [
        Value::from(block.block.file.as_str()),
        Value::from(block.block.line_start),
        Value::from(block.block.line_end),
        Value::from(block.block.node_kind.as_str()),
        parent_cell(block.block.parent.as_ref()),
        opt_cell(signature_cell(&block.block).as_deref()),
        opt_cell(snippet(&block.block, snippet_lines).as_deref()),
        Value::from(relevance_percent(block.final_score, top_score)),
    ]
}

const SEARCH_FIELDS: [&str; 8] =
    ["file", "lineStart", "lineEnd", "nodeKind", "parent", "symbolSignature", "snippet", "relevance"];

/// The line `monokl search` prints for each of `blocks`, indentation included and the newline left out.
///
/// `blocks` is the whole ranking, best first, so its first row is the top that each `relevance` cell is measured
/// against. The search budget measures these lines, so a cap counts what the reader receives. Rendering them in one
/// table keeps michi's cell quoting and truncation as the only definition of a row.
pub fn search_row_texts(blocks: &[RankedBlock], snippet_lines: usize) -> Vec<String> {
    let top_score = blocks.first().map_or(0.0, |top| top.final_score);
    let rows: Vec<Vec<Value>> =
        blocks.iter().map(|block| search_row(block, top_score, snippet_lines).to_vec()).collect();
    let rendered =
        michi::AgentResponse::new("search").items(rows, SEARCH_FIELDS.as_slice()).render(michi::OutputFormat::Text);
    // Rows are the indented lines under the header. michi prints blocks that are not rows (`truncated[N]{..}:` for a
    // cut cell, `totalCount`) at column 0 after them, and those must not count as rows.
    rendered.lines().skip(1).take_while(|line| line.starts_with(' ')).map(str::to_owned).collect()
}

/// Renders a search response as a TOON table of its rows, followed by the truncation count and hints. Each row's
/// `relevance` is a percent of [`SearchResponse::top_score`], so a later page still measures against the first row.
pub fn render_search(response: &SearchResponse, view: &SearchView) -> String {
    let rows: Vec<Vec<Value>> = response
        .results
        .iter()
        .map(|block| search_row(block, response.top_score, view.snippet_lines).to_vec())
        .collect();
    let fields = SEARCH_FIELDS.iter().map(|field| (*field).to_owned()).collect();
    let mut section = michi::Section::toon("search", fields, rows);
    if response.truncated {
        section = section.total_count(Some(response.total_blocks_before_truncation));
    }
    michi::AgentResponse::new("search")
        .section(section)
        .hints(search_hints(response, view))
        .render(michi::OutputFormat::Text)
}

/// What the truncation hint tells a reader to do to see more.
///
/// Rows the caps left out are reached with `--offset`, which names the exact next value so the reader does not pay for
/// the rows already seen; rows the relevance cut dropped, those with a `relevance` below `100 * min_score`, are reached
/// with `--min-score 0`, which keeps the same order, so the same offsets still apply. An offset past the last kept row
/// says so and how many rows there are. The caps are named only when a page came back empty, because an offset would
/// not move past a row that does not fit.
fn truncation_guidance(response: &SearchResponse, view: &SearchView) -> String {
    let returned = response.results.len();
    let cut = response.blocks_cut_by_relevance;
    let relevant = response.total_blocks_before_truncation.saturating_sub(cut);
    let left = relevant.saturating_sub(response.offset + returned);
    let below_cut = (cut > 0).then(|| {
        format!(
            "--min-score 0 for the {cut} {} below relevance {:.0}",
            if cut == 1 { "row" } else { "rows" },
            view.min_score * 100.0
        )
    });
    let mut parts = Vec::new();
    if returned == 0 && response.offset >= relevant {
        parts.push(format!("offset {} is past the last of {relevant} rows", response.offset));
        parts.extend(below_cut);
        return parts.join("; ");
    }
    if left > 0 && returned > 0 {
        parts.push(format!("next: --offset {}", response.offset + returned));
    } else if left > 0 {
        parts.push(format!(
            "{} for more",
            if returned >= view.max_results { "--max-results N" } else { "--max-tokens N" }
        ));
    }
    parts.extend(below_cut);
    parts.push("refine with +term/-term".to_owned());
    let (last, rest) = parts.split_last().map_or(("", &[][..]), |(last, rest)| (last.as_str(), rest));
    if rest.is_empty() { last.to_owned() } else { format!("{}, or {last}", rest.join(", ")) }
}

fn search_hints(response: &SearchResponse, view: &SearchView) -> Vec<michi::Hint> {
    let shown =
        michi::TruncationOutcome::new(response.results.iter().collect(), response.total_blocks_before_truncation);
    let mut hints: Vec<michi::Hint> = shown.hint(truncation_guidance(response, view)).into_iter().collect();
    hints.extend(diagnostic_lines(&response.diagnostics).into_iter().map(michi::Hint::new));
    let past_the_end = response.offset > 0 && response.total_blocks_before_truncation > 0;
    if response.results.is_empty() && !past_the_end {
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

    /// Where the test blocks start in their file, so a span offset is not mistaken for an offset in the code.
    const START: usize = 100;

    /// The snippet at the longest length, which is what every row showed before `--snippet-lines` existed; the tests
    /// below that call this pin that contract unchanged.
    fn snippet(block: &CodeBlock) -> Option<String> {
        super::snippet(block, MAX_SNIPPET_LINES)
    }

    /// The snippet at an explicit length.
    fn snippet_of(block: &CodeBlock, lines: usize) -> Option<String> {
        super::snippet(block, lines)
    }

    /// A block whose head is its first line, as for a function whose body opens at the end of that line.
    fn block(line_start: usize, code: &str, matched_lines: &[usize]) -> CodeBlock {
        let first_line = code.split('\n').next().unwrap_or("");
        CodeBlock {
            file: Utf8PathBuf::from("src/a.ts"),
            line_start,
            line_end: line_start + code.split('\n').count() - 1,
            node_kind: SymbolKind::Function,
            span: START..START + code.len(),
            parent_index: None,
            code: code.to_owned(),
            symbol_signature: Some(first_line.trim_end_matches(['{', ' ']).to_owned()),
            signature_span: Some(START..START + first_line.len()),
            where_start: None,
            name: None,
            matched_lines: matched_lines.to_vec(),
            parent: None,
        }
    }

    /// `block` with the head set to the text `head`, which `code` must hold.
    fn headed(line_start: usize, code: &str, head: &str, matched_lines: &[usize]) -> CodeBlock {
        let at = START + code.find(head).expect("the code holds the head");
        CodeBlock { signature_span: Some(at..at + head.len()), ..block(line_start, code, matched_lines) }
    }

    /// `block` with no signature, as for a variable or a type alias.
    fn unsigned(line_start: usize, code: &str, matched_lines: &[usize]) -> CodeBlock {
        CodeBlock { symbol_signature: None, signature_span: None, ..block(line_start, code, matched_lines) }
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
        let b = unsigned(3, "export const price = 1;", &[3]);
        assert_eq!(snippet(&b), Some("3: export const price = 1;".to_owned()));
    }

    #[test]
    fn a_decorator_line_the_head_leaves_out_is_shown_when_it_matched() {
        let b = headed(2, "@Dec('needle')\nmulti(a: number): void {\n}", "\nmulti(a: number): void {", &[2]);
        assert_eq!(snippet(&b), Some("2: @Dec('needle')".to_owned()));
    }

    #[test]
    fn a_decorator_sharing_its_line_with_the_member_is_shown_when_it_matched() {
        let b = headed(9, "@Dec2() prop2 = () => {\n  return 1;\n}", " prop2 = () => {", &[9]);
        assert_eq!(snippet(&b), Some("9: @Dec2() prop2 = () => {".to_owned()));
    }

    #[test]
    fn a_first_line_that_says_more_than_the_head_is_shown() {
        let b = headed(2, "load = () => needle(\n  1,\n);", "load = () => ", &[2]);
        assert_eq!(snippet(&b), Some("2: load = () => needle(".to_owned()));
    }

    #[test]
    fn a_one_line_body_after_the_head_is_shown() {
        let b = headed(4, "function f() { return needle; }", "function f() {", &[4]);
        assert_eq!(snippet(&b), Some("4: function f() { return needle; }".to_owned()));
    }

    #[test]
    fn an_export_keyword_inside_the_head_does_not_make_a_covered_first_line_look_new() {
        let b = block(1, "export async function f(x) {\n  return x;\n}", &[1]);
        assert_eq!(snippet(&b), None);
    }

    #[test]
    fn every_line_of_a_head_that_spans_lines_is_covered_and_a_body_line_is_not() {
        let b = headed(
            1,
            "multi(\n  a: number,\n): void {\n  return;\n}",
            "multi(\n  a: number,\n): void {",
            &[1, 2, 3, 4],
        );
        assert_eq!(snippet(&b), Some("4: return;".to_owned()));
    }

    #[test]
    fn a_head_below_attributes_and_doc_comments_is_covered_and_they_are_not() {
        let code = "/// needle docs\n#[inline]\npub fn needle() {\n    needle();\n}";
        let b = headed(7, code, "pub fn needle() {", &[7, 8, 9, 10]);
        assert_eq!(snippet(&b), Some("7: /// needle docs\n8: #[inline]\n10: needle();".to_owned()));
    }

    #[test]
    fn a_blank_matched_line_is_shown_as_the_empty_line_it_is() {
        let b = block(1, "head\n\nbody", &[2]);
        assert_eq!(snippet(&b), Some("2: ".to_owned()));
    }

    #[test]
    fn a_block_with_no_matched_lines_has_no_snippet() {
        assert_eq!(snippet(&block(1, "a\nb", &[])), None);
    }

    #[test]
    fn a_code_line_is_shown_before_a_doc_comment_that_matched() {
        let code = "/// needle docs\npub fn f() {\n    needle();\n}";
        let b = headed(7, code, "pub fn f() {", &[7, 9]);
        assert_eq!(snippet_of(&b, 1), Some("9: needle();\n+1 more".to_owned()));
    }

    #[test]
    fn a_doc_comment_is_shown_when_it_is_the_only_line_that_matched() {
        let code = "/// needle docs\npub fn f() {\n    other();\n}";
        let b = headed(7, code, "pub fn f() {", &[7]);
        assert_eq!(snippet_of(&b, 1), Some("7: /// needle docs".to_owned()));
    }

    #[test]
    fn an_attribute_line_is_shown_after_the_code_lines_that_matched() {
        let code = "#[needle]\npub fn f() {\n    needle();\n}";
        let b = headed(3, code, "pub fn f() {", &[3, 5]);
        assert_eq!(snippet_of(&b, 1), Some("5: needle();\n+1 more".to_owned()));
        let only = headed(3, code, "pub fn f() {", &[3]);
        assert_eq!(snippet_of(&only, 1), Some("3: #[needle]".to_owned()));
    }

    #[test]
    fn every_comment_form_gives_way_to_a_code_line() {
        for comment in
            ["// needle", "//! needle", "/// needle", "/* needle */", "/** needle */", "* needle", "*/", "#![needle]"]
        {
            let code = format!("function f() {{\n  {comment}\n  needle();\n}}");
            let b = block(1, &code, &[2, 3]);
            assert_eq!(snippet_of(&b, 1), Some("3: needle();\n+1 more".to_owned()), "{comment}");
        }
    }

    #[test]
    fn a_dereference_or_a_multiplication_is_code_not_a_comment() {
        for line in ["*ptr = needle;", "**ptr = needle;", "#needle = 1;", "@Dec(needle)"] {
            let code = format!("function f() {{\n  {line}\n}}");
            let b = block(1, &code, &[2]);
            assert_eq!(snippet_of(&b, 1), Some(format!("2: {line}")), "{line}");
        }
        let b = block(1, "function f() {\n  // needle\n  *p = needle;\n}", &[2, 3]);
        assert_eq!(snippet_of(&b, 1), Some("3: *p = needle;\n+1 more".to_owned()));
    }

    #[test]
    fn the_lines_a_snippet_shows_keep_their_order_in_the_block_with_code_lines_chosen_first() {
        let b = block(1, "head\n// c1\nx1\n// c2\nx2\nx3", &[2, 3, 4, 5, 6]);
        assert_eq!(snippet_of(&b, 3), Some("3: x1\n5: x2\n6: x3\n+2 more".to_owned()));
        assert_eq!(snippet_of(&b, 2), Some("3: x1\n5: x2\n+3 more".to_owned()));
    }

    #[test]
    fn comment_lines_fill_the_snippet_after_the_code_lines_run_out() {
        let b = block(1, "head\n// c1\nx1\n// c2", &[2, 3, 4]);
        assert_eq!(snippet_of(&b, 3), Some("2: // c1\n3: x1\n4: // c2".to_owned()));
        assert_eq!(snippet_of(&b, 2), Some("2: // c1\n3: x1\n+1 more".to_owned()));
    }

    /// `block` with the head set to the text `head` of `code` and the where clause starting at the last `where` in the
    /// head, as the Rust analyzer records it.
    fn rust_headed(line_start: usize, code: &str, head: &str, matched_lines: &[usize]) -> CodeBlock {
        let at = code.find(head).expect("the code holds the head") + head.rfind("where").expect("the head has a where");
        CodeBlock { where_start: Some(START + at), ..headed(line_start, code, head, matched_lines) }
    }

    const WHERE_FN: &str = "pub fn f<T>(x: T) -> T\nwhere\n    T: needle,\n{\n    x\n}";
    const WHERE_HEAD: &str = "pub fn f<T>(x: T) -> T\nwhere\n    T: needle,\n{";

    #[test]
    fn a_matched_bound_in_a_rust_where_clause_is_shown_because_the_signature_column_leaves_the_clause_out() {
        let b = rust_headed(1, WHERE_FN, WHERE_HEAD, &[1, 3]);
        assert_eq!(snippet(&b), Some("3: T: needle,".to_owned()));
    }

    #[test]
    fn a_where_clause_line_gives_way_to_a_body_line_that_matched() {
        let code = "pub fn f<T>(x: T) -> T\nwhere\n    T: needle,\n{\n    needle(x)\n}";
        let b = rust_headed(1, code, WHERE_HEAD, &[3, 5]);
        assert_eq!(snippet_of(&b, 1), Some("5: needle(x)\n+1 more".to_owned()));
        assert_eq!(snippet_of(&b, 2), Some("3: T: needle,\n5: needle(x)".to_owned()));
    }

    #[test]
    fn the_where_keyword_line_is_shown_when_it_matched_and_the_line_before_it_is_not() {
        let b = rust_headed(1, WHERE_FN, WHERE_HEAD, &[1, 2]);
        assert_eq!(snippet(&b), Some("2: where".to_owned()));
    }

    #[test]
    fn a_where_clause_on_the_declaration_line_does_not_make_that_line_a_snippet() {
        let code = "pub fn f<T>(x: T) -> T where T: needle {\n    x\n}";
        let b = rust_headed(1, code, "pub fn f<T>(x: T) -> T where T: needle {", &[1]);
        assert_eq!(snippet(&b), None);
    }

    #[test]
    fn a_head_with_no_recorded_where_clause_stays_whole_in_the_head() {
        // The same text as a block the analyzer found no where clause in, as for TypeScript.
        let b = headed(1, WHERE_FN, WHERE_HEAD, &[1, 3]);
        assert_eq!(snippet(&b), None);
    }

    #[test]
    fn a_rust_signature_ends_before_the_recorded_where_clause_and_says_so() {
        let cell = |code: &str, head: &str| {
            let b =
                CodeBlock { symbol_signature: Some("whole signature".to_owned()), ..rust_headed(1, code, head, &[]) };
            signature_cell(&b)
        };
        assert_eq!(cell(WHERE_FN, WHERE_HEAD), Some("pub fn f<T>(x: T) -> T where …".to_owned()));
        let one_line = "pub fn f<T>(x: T) -> T where T: Copy {";
        assert_eq!(cell(one_line, one_line), Some("pub fn f<T>(x: T) -> T where …".to_owned()));
        let imp = "impl<L> X<L>\nwhere\n    L: Listener,\n    L::Addr: Debug,\n{";
        assert_eq!(cell(imp, imp), Some("impl<L> X<L> where …".to_owned()));
        // A `where` before the clause, in a comment, stays in the signature.
        let commented = "pub fn f() /* where */ -> T where T: A {";
        assert_eq!(cell(commented, commented), Some("pub fn f() /* where */ -> T where …".to_owned()));
    }

    #[test]
    fn a_signature_with_no_recorded_where_clause_is_shown_whole_whatever_it_spells() {
        for text in [
            "pub fn value() /* where values originate */ -> u64",
            "pub fn value<const S: &'static str = \"a where b\">() -> u64",
            "pub struct Foo<const N: usize = { r#\"\" } where \"#.len() }>;",
            "function f() where T: A",
        ] {
            assert_eq!(signature_cell(&block(1, text, &[])), Some(text.to_owned()), "{text}");
        }
        assert_eq!(signature_cell(&unsigned(1, "x", &[])), None);
    }

    #[test]
    fn a_snippet_shows_three_lines_and_counts_the_rest() {
        let b = block(1, "head\nx1\nx2\nx3\nx4\nx5", &[2, 3, 4, 5, 6]);
        assert_eq!(snippet(&b), Some("2: x1\n3: x2\n4: x3\n+2 more".to_owned()));
    }

    #[test]
    fn a_snippet_counts_one_more_line_in_the_singular() {
        let b = block(1, "head\nx1\nx2\nx3\nx4", &[2, 3, 4, 5]);
        assert_eq!(snippet(&b), Some("2: x1\n3: x2\n4: x3\n+1 more".to_owned()));
    }

    #[test]
    fn a_snippet_of_one_line_shows_the_first_matching_line_and_counts_the_rest() {
        let b = block(1, "head\nx1\nx2\nx3", &[2, 3, 4]);
        assert_eq!(snippet_of(&b, 1), Some("2: x1\n+2 more".to_owned()));
    }

    #[test]
    fn a_snippet_of_one_line_has_no_marker_when_one_line_matched() {
        let b = block(1, "head\nx1\nx2", &[2]);
        assert_eq!(snippet_of(&b, 1), Some("2: x1".to_owned()));
        assert_eq!(snippet_of(&b, 3), Some("2: x1".to_owned()));
    }

    #[test]
    fn a_snippet_of_one_line_counts_one_more_line_in_the_singular() {
        let b = block(1, "head\nx1\nx2", &[2, 3]);
        assert_eq!(snippet_of(&b, 1), Some("2: x1\n+1 more".to_owned()));
        assert_eq!(snippet_of(&b, 2), Some("2: x1\n3: x2".to_owned()));
    }

    #[test]
    fn a_snippet_of_one_line_still_shows_the_declaration_of_a_variable_or_type() {
        let b = unsigned(3, "export const price = 1;", &[3]);
        assert_eq!(snippet_of(&b, 1), Some("3: export const price = 1;".to_owned()));
    }

    #[test]
    fn a_snippet_of_no_lines_is_none_even_where_a_line_matched() {
        let b = unsigned(3, "export const price = 1;", &[3]);
        assert_eq!(snippet_of(&b, 0), None);
        assert_eq!(snippet_of(&block(1, "head\nx1\nx2", &[2, 3]), 0), None);
    }

    #[test]
    fn the_default_and_the_longest_snippet_are_one_and_three_lines() {
        assert_eq!((DEFAULT_SNIPPET_LINES, MAX_SNIPPET_LINES), (1, 3));
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

    #[test]
    fn a_head_is_read_in_the_offsets_of_the_file_not_of_the_code() {
        // The same head at offsets that do not hold this line leaves the line in the snippet.
        let mut b = block(1, "head\nbody", &[1]);
        b.signature_span = Some(0..4);
        assert_eq!(snippet(&b), Some("1: head".to_owned()));
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
                    // Line numbers and relevance are all distinct so a swapped column is detectable.
                    line_start: 5,
                    line_end: 9,
                    node_kind: SymbolKind::Function,
                    span: 0..56,
                    parent_index: None,
                    code: "fn foo(x) {\n    let y = foo(x);\n    unrelated();\n    y\n}".to_string(),
                    symbol_signature: Some("fn foo(x)\nsecond line".to_string()),
                    signature_span: Some(0..11),
                    where_start: None,
                    name: None,
                    matched_lines: vec![5, 6],
                    parent: None,
                },
                final_score: 0.92,
                rank: 2,
            }],
            top_score: 1.0,
            total_blocks_before_truncation: 1,
            blocks_cut_by_relevance: 0,
            offset: 0,
            truncated: false,
            diagnostics: Vec::new(),
        };

        let rendered = render_search(&response, &SearchView::default());
        assert_eq!(
            rendered,
            "search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n  \
             src/lib.rs,5,9,function,null,\"fn foo(x)\\nsecond line\",\"6: let y = foo(x);\",92"
        );
    }

    #[test]
    fn a_member_row_names_its_parent_and_a_top_level_row_has_none() {
        use crate::types::{CodeBlock, ParentContext, RankedBlock, SearchResponse, SymbolKind};
        use camino::Utf8PathBuf;

        let ranked = |line_start, node_kind, parent, final_score, rank| RankedBlock {
            block: CodeBlock {
                file: Utf8PathBuf::from("src/a.ts"),
                line_start,
                line_end: line_start + 2,
                node_kind,
                span: 0..0,
                parent_index: None,
                code: String::new(),
                symbol_signature: None,
                signature_span: None,
                where_start: None,
                name: None,
                matched_lines: Vec::new(),
                parent,
            },
            final_score,
            rank,
        };
        let class = |name: &str| ParentContext { kind: SymbolKind::Class, name: name.to_owned() };
        let response = SearchResponse {
            results: vec![
                ranked(3, SymbolKind::Method, Some(class("Foo")), 1.0, 1),
                ranked(7, SymbolKind::Constructor, Some(class("Bar Baz")), 0.5, 2),
                ranked(1, SymbolKind::Class, None, 0.25, 3),
            ],
            top_score: 1.0,
            total_blocks_before_truncation: 3,
            blocks_cut_by_relevance: 0,
            offset: 0,
            truncated: false,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            render_search(&response, &SearchView::default()),
            "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n  \
             src/a.ts,3,5,method,class Foo,null,null,100\n  \
             src/a.ts,7,9,constructor,class Bar Baz,null,null,50\n  \
             src/a.ts,1,3,class,null,null,null,25"
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
            top_score: 0.0,
            total_blocks_before_truncation: 0,
            blocks_cut_by_relevance: 0,
            offset: 0,
            truncated: false,
            diagnostics: vec![
                diagnostic(DiagnosticKind::Skipped, Some("a.ts"), "unreadable"),
                diagnostic(DiagnosticKind::Skipped, Some("b.ts"), "unreadable"),
                diagnostic(DiagnosticKind::Degraded, Some("c.ts"), "parse errors"),
                diagnostic(DiagnosticKind::Skipped, None, "query has no positive terms"),
            ],
        };

        assert_eq!(
            render_search(&response, &SearchView::default()),
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

    /// `scores.len()` ranked blocks in `src/f<rank>.rs`, scored as given, without a snippet.
    fn scored_blocks(scores: &[f64]) -> Vec<crate::types::RankedBlock> {
        use crate::types::{CodeBlock, RankedBlock, SymbolKind};
        use camino::Utf8PathBuf;

        scores
            .iter()
            .zip(1..)
            .map(|(&final_score, rank)| RankedBlock {
                block: CodeBlock {
                    file: Utf8PathBuf::from(format!("src/f{rank}.rs")),
                    line_start: rank,
                    line_end: rank + 1,
                    node_kind: SymbolKind::Function,
                    span: 0..9,
                    parent_index: None,
                    code: "fn a() {}".to_string(),
                    symbol_signature: None,
                    signature_span: None,
                    where_start: None,
                    name: None,
                    matched_lines: Vec::new(),
                    parent: None,
                },
                final_score,
                rank,
            })
            .collect()
    }

    /// `scores.len()` blocks as `scored_blocks` builds them, with the signature of each block in `long` replaced by one
    /// of `len` characters, which michi cuts at its 200-character cell cap.
    fn blocks_with_long_signatures(scores: &[f64], long: &[usize], len: usize) -> Vec<crate::types::RankedBlock> {
        let mut ranked = scored_blocks(scores);
        for &index in long {
            ranked[index].block.symbol_signature = Some("x".repeat(len));
        }
        ranked
    }

    #[test]
    fn row_texts_hold_one_line_per_block_when_a_cell_is_cut() {
        let ranked = blocks_with_long_signatures(&[1.0, 0.9, 0.8], &[1], 250);
        let texts = search_row_texts(&ranked, DEFAULT_SNIPPET_LINES);
        assert_eq!(
            texts,
            [
                "  src/f1.rs,1,2,function,null,null,null,100".to_owned(),
                format!("  src/f2.rs,2,3,function,null,{}…,null,90", "x".repeat(199)),
                "  src/f3.rs,3,4,function,null,null,null,80".to_owned(),
            ]
        );
    }

    #[test]
    fn row_texts_hold_one_line_per_block_when_several_cells_are_cut() {
        let ranked = blocks_with_long_signatures(&[1.0, 0.9, 0.8], &[0, 2], 300);
        let texts = search_row_texts(&ranked, DEFAULT_SNIPPET_LINES);
        assert_eq!(texts.len(), ranked.len());
        assert!(texts.iter().all(|text| text.starts_with("  src/f")), "{texts:?}");
    }

    #[test]
    fn the_truncated_block_is_listed_after_the_rows_and_is_not_a_row() {
        let ranked = blocks_with_long_signatures(&[1.0, 0.9], &[1], 250);
        let total = ranked.len();
        let view = SearchView::default();
        let texts = search_row_texts(&ranked, view.snippet_lines);
        assert_eq!(texts.len(), total);
        let response =
            crate::budget::apply_budget(ranked, total, &crate::budget::Budget::default(), &texts, Vec::new());
        assert_eq!((response.results.len(), response.truncated), (2, false));
        let rendered = render_search(&response, &view);
        let tail: Vec<&str> = rendered.lines().skip(1 + total).collect();
        assert_eq!(tail[0], "truncated[1]{row,field,removed}:");
        assert_eq!(tail[1], "  1,symbolSignature,51");
        assert!(texts.iter().all(|text| !tail.contains(&text.as_str())));
    }

    #[test]
    fn relevance_counts_the_rows_only_when_a_cell_is_cut_on_an_offset_page() {
        let ranked = blocks_with_long_signatures(&[1.0, 0.8, 0.6, 0.4], &[2], 250);
        let view = SearchView { min_score: 0.0, ..SearchView::default() };
        let texts = search_row_texts(&ranked, view.snippet_lines);
        assert_eq!(texts.len(), 4);
        let budget = crate::budget::Budget { offset: 2, min_score: 0.0, ..crate::budget::Budget::default() };
        let response = crate::budget::apply_budget(ranked, 4, &budget, &texts, Vec::new());
        let rendered = render_search(&response, &view);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:");
        assert_eq!(lines[1], format!("  src/f3.rs,3,4,function,null,{}…,null,60", "x".repeat(199)));
        assert_eq!(lines[2], "  src/f4.rs,4,5,function,null,null,null,40");
        assert_eq!(lines[3], "truncated[1]{row,field,removed}:");
        assert_eq!(lines[4], "  0,symbolSignature,51");
        assert_eq!(lines[5..], ["totalCount: 4", "help[1]: showing 2 of 4; refine with +term/-term"]);
    }

    /// The help line `scores` produce under `budget`, or `None` when the whole result is shown.
    fn help_line(scores: &[f64], budget: &crate::budget::Budget) -> Option<String> {
        let ranked = scored_blocks(scores);
        let total = ranked.len();
        let view = SearchView { max_results: budget.max_results, min_score: budget.min_score, ..SearchView::default() };
        let texts = search_row_texts(&ranked, view.snippet_lines);
        let rendered = render_search(&crate::budget::apply_budget(ranked, total, budget, &texts, Vec::new()), &view);
        rendered.lines().find_map(|line| {
            let text = line.strip_prefix("help[1]: ")?;
            Some(text.strip_prefix('"').and_then(|quoted| quoted.strip_suffix('"')).unwrap_or(text).to_owned())
        })
    }

    #[test]
    fn the_help_line_names_the_min_score_flag_when_the_relevance_cut_dropped_rows() {
        let scores = [vec![1.0; 9], vec![0.1, 0.05]].concat();
        assert_eq!(
            help_line(&scores, &crate::budget::Budget::default()).as_deref(),
            Some("showing 9 of 11; --min-score 0 for the 2 rows below relevance 30, or refine with +term/-term")
        );
    }

    #[test]
    fn the_help_line_names_the_next_offset_when_the_row_cap_dropped_rows() {
        let budget = crate::budget::Budget { max_results: 2, ..crate::budget::Budget::default() };
        assert_eq!(
            help_line(&[1.0, 0.9, 0.8], &budget).as_deref(),
            Some("showing 2 of 3; next: --offset 2, or refine with +term/-term")
        );
    }

    #[test]
    fn the_help_line_names_the_offset_and_the_min_score_flag_when_both_rules_dropped_rows() {
        let budget = crate::budget::Budget { max_results: 5, ..crate::budget::Budget::default() };
        let scores = [vec![1.0; 9], vec![0.1]].concat();
        assert_eq!(
            help_line(&scores, &budget).as_deref(),
            Some(
                "showing 5 of 10; next: --offset 5, --min-score 0 for the 1 row below relevance 30, or refine with +term/-term"
            )
        );
    }

    #[test]
    fn the_help_line_names_the_next_offset_when_the_tokens_ran_out_before_the_rows() {
        let budget = crate::budget::Budget { max_tokens: 20, ..crate::budget::Budget::default() };
        assert_eq!(
            help_line(&[1.0, 0.9, 0.8], &budget).as_deref(),
            Some("showing 1 of 3; next: --offset 1, or refine with +term/-term")
        );
    }

    #[test]
    fn the_help_line_names_the_max_tokens_flag_when_one_row_is_larger_than_the_whole_budget() {
        let budget = crate::budget::Budget { max_tokens: 1, ..crate::budget::Budget::default() };
        let ranked = scored_blocks(&[1.0, 0.9, 0.8]);
        let view = SearchView::default();
        let texts = search_row_texts(&ranked, view.snippet_lines);
        let rendered = render_search(&crate::budget::apply_budget(ranked, 3, &budget, &texts, Vec::new()), &view);
        assert!(
            rendered.contains("\"showing 0 of 3; --max-tokens N for more, or refine with +term/-term\""),
            "got: {rendered}"
        );
    }

    #[test]
    fn the_first_page_of_thirty_one_names_offset_eight() {
        let budget = crate::budget::Budget { max_results: 8, ..crate::budget::Budget::default() };
        assert_eq!(
            help_line(&[1.0; 31], &budget).as_deref(),
            Some("showing 8 of 31; next: --offset 8, or refine with +term/-term")
        );
    }

    #[test]
    fn a_middle_page_names_the_offset_after_its_last_row() {
        let budget = crate::budget::Budget { max_results: 8, offset: 8, ..crate::budget::Budget::default() };
        assert_eq!(
            help_line(&[1.0; 31], &budget).as_deref(),
            Some("showing 8 of 31; next: --offset 16, or refine with +term/-term")
        );
    }

    #[test]
    fn the_last_page_names_no_next_offset() {
        let budget = crate::budget::Budget { max_results: 8, offset: 24, ..crate::budget::Budget::default() };
        assert_eq!(help_line(&[1.0; 31], &budget).as_deref(), Some("showing 7 of 31; refine with +term/-term"));
    }

    #[test]
    fn the_last_page_of_the_kept_rows_still_names_the_min_score_flag() {
        let budget = crate::budget::Budget { max_results: 8, offset: 8, ..crate::budget::Budget::default() };
        let scores = [vec![1.0; 12], vec![0.1; 3]].concat();
        assert_eq!(
            help_line(&scores, &budget).as_deref(),
            Some("showing 4 of 15; --min-score 0 for the 3 rows below relevance 30, or refine with +term/-term")
        );
    }

    #[test]
    fn an_offset_past_the_last_row_says_so_with_the_count_and_prints_no_rows() {
        let budget = crate::budget::Budget { offset: 40, ..crate::budget::Budget::default() };
        let ranked = scored_blocks(&[1.0; 31]);
        let view = SearchView::default();
        let texts = search_row_texts(&ranked, view.snippet_lines);
        let rendered = render_search(&crate::budget::apply_budget(ranked, 31, &budget, &texts, Vec::new()), &view);
        assert_eq!(
            rendered,
            "search: []\n\
             totalCount: 31\n\
             help[1]: showing 0 of 31; offset 40 is past the last of 31 rows"
        );
    }

    #[test]
    fn an_offset_past_the_kept_rows_names_the_rows_below_the_cut() {
        let budget = crate::budget::Budget { offset: 9, ..crate::budget::Budget::default() };
        let scores = [vec![1.0; 9], vec![0.1; 2]].concat();
        assert_eq!(
            help_line(&scores, &budget).as_deref(),
            Some(
                "showing 0 of 11; offset 9 is past the last of 9 rows; --min-score 0 for the 2 rows below relevance 30"
            )
        );
    }

    #[test]
    fn the_hint_names_the_relevance_the_cut_was_set_at() {
        let scores = [vec![1.0; 9], vec![0.2]].concat();
        for (min_score, named) in [(0.5, "50"), (0.35, "35"), (1.0, "100")] {
            let budget = crate::budget::Budget { min_score, ..crate::budget::Budget::default() };
            assert_eq!(
                help_line(&scores, &budget),
                Some(format!(
                    "showing 9 of 10; --min-score 0 for the 1 row below relevance {named}, or refine with +term/-term"
                )),
                "min_score {min_score}"
            );
        }
    }

    #[test]
    fn the_help_line_does_not_name_min_score_once_the_relevance_cut_is_off() {
        let budget = crate::budget::Budget { max_results: 2, min_score: 0.0, ..crate::budget::Budget::default() };
        assert_eq!(
            help_line(&[1.0, 0.9, 0.1], &budget).as_deref(),
            Some("showing 2 of 3; next: --offset 2, or refine with +term/-term")
        );
    }

    #[test]
    fn a_result_nothing_was_cut_from_has_no_help_line() {
        assert_eq!(help_line(&[1.0, 0.9, 0.8], &crate::budget::Budget::default()), None);
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
                    span: 0..9,
                    parent_index: None,
                    code: "fn a() {}".to_string(),
                    symbol_signature: None,
                    signature_span: None,
                    where_start: None,
                    name: None,
                    matched_lines: Vec::new(),
                    parent: None,
                },
                final_score: 0.5,
                rank,
            })
            .collect();
        let budget = Budget { max_results: 2, ..Budget::default() };
        let view = SearchView { max_results: 2, ..SearchView::default() };
        let texts = search_row_texts(&ranked, view.snippet_lines);

        let rendered = render_search(&apply_budget(ranked, 3, &budget, &texts, Vec::new()), &view);
        assert_eq!(
            rendered,
            "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n  \
             src/f1.rs,1,2,function,null,null,null,100\n  \
             src/f2.rs,2,3,function,null,null,null,100\n\
             totalCount: 3\n\
             help[1]: \"showing 2 of 3; next: --offset 2, or refine with +term/-term\""
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
                    span: 0..0,
                    parent_index: None,
                    code: String::new(),
                    symbol_signature: None,
                    signature_span: None,
                    where_start: None,
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
            top_score: 0.30,
            total_blocks_before_truncation: 3,
            blocks_cut_by_relevance: 0,
            offset: 0,
            truncated: false,
            diagnostics: Vec::new(),
        };

        // The rows keep `results`' own order, though their scores rise.
        assert_eq!(
            render_search(&search, &SearchView::default()),
            "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n  \
             src/one.rs,1,2,function,null,null,null,33\n  \
             src/two.rs,3,4,function,null,null,null,67\n  \
             src/three.rs,5,6,function,null,null,null,100"
        );
    }

    #[test]
    fn relevance_is_a_percent_of_the_top_score_rounded_to_the_nearest_whole_number() {
        for (score, top, percent) in [
            (4.0, 4.0, 100),
            (2.0, 4.0, 50),
            (1.0, 3.0, 33),
            (2.0, 3.0, 67),
            (0.125, 1.0, 13),
            (0.124, 1.0, 12),
            (0.004, 1.0, 0),
            (0.3, 1.0, 30),
        ] {
            assert_eq!(relevance_percent(score, top), percent, "{score} of {top}");
        }
    }

    #[test]
    fn relevance_stays_within_zero_and_one_hundred() {
        assert_eq!(relevance_percent(5.0, 4.0), 100);
        assert_eq!(relevance_percent(-1.0, 4.0), 0);
    }

    #[test]
    fn relevance_is_one_hundred_for_every_row_when_the_top_score_is_not_positive() {
        for top in [0.0, -1.0, f64::NAN] {
            assert_eq!(relevance_percent(0.0, top), 100, "top {top}");
        }
        let response = SearchResponse {
            results: scored_blocks(&[0.0, 0.0]),
            top_score: 0.0,
            total_blocks_before_truncation: 2,
            blocks_cut_by_relevance: 0,
            offset: 0,
            truncated: false,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            render_search(&response, &SearchView::default()),
            "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n  \
             src/f1.rs,1,2,function,null,null,null,100\n  \
             src/f2.rs,2,3,function,null,null,null,100"
        );
    }

    #[test]
    fn a_later_page_measures_relevance_against_the_top_of_the_whole_result() {
        let ranked = scored_blocks(&[1.0, 0.8, 0.6, 0.4]);
        let view = SearchView::default();
        let texts = search_row_texts(&ranked, view.snippet_lines);
        let budget = crate::budget::Budget { offset: 2, min_score: 0.0, ..crate::budget::Budget::default() };
        let page = crate::budget::apply_budget(ranked, 4, &budget, &texts, Vec::new());
        assert_eq!(
            render_search(&page, &view).lines().skip(1).take(2).collect::<Vec<_>>(),
            ["  src/f3.rs,3,4,function,null,null,null,60", "  src/f4.rs,4,5,function,null,null,null,40"]
        );
    }

    #[test]
    fn the_budgeted_row_texts_are_the_rows_that_print() {
        let ranked = scored_blocks(&[0.9, 0.45, 0.3]);
        let view = SearchView::default();
        let texts = search_row_texts(&ranked, view.snippet_lines);
        assert_eq!(
            texts,
            [
                "  src/f1.rs,1,2,function,null,null,null,100",
                "  src/f2.rs,2,3,function,null,null,null,50",
                "  src/f3.rs,3,4,function,null,null,null,33",
            ]
        );
        let budget = crate::budget::Budget::default();
        let response = crate::budget::apply_budget(ranked, 3, &budget, &texts, Vec::new());
        let printed: Vec<String> = render_search(&response, &view).lines().skip(1).map(str::to_owned).collect();
        assert_eq!(printed, texts);
    }

    #[test]
    fn the_rows_the_cut_drops_show_a_relevance_below_the_cut() {
        let scores = [vec![1.0; 8], vec![0.31, 0.29, 0.1]].concat();
        let ranked = scored_blocks(&scores);
        let view = SearchView::default();
        let texts = search_row_texts(&ranked, view.snippet_lines);
        let response =
            crate::budget::apply_budget(ranked, scores.len(), &crate::budget::Budget::default(), &texts, Vec::new());
        let shown: Vec<&str> = texts.iter().map(|text| text.rsplit(',').next().unwrap_or("")).collect();
        assert_eq!(shown[8..], ["31", "29", "10"]);
        assert_eq!(response.results.len(), 9);
        assert_eq!(response.blocks_cut_by_relevance, 2);
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

        let mut domain_err = michi::DomainError::from_error_with_causes(code, self).retryable(false);
        // Prefixed after from_error_with_causes, which compares each cause against the unprefixed message to skip repeats.
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
