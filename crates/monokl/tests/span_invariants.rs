//! Safety net for block spans and parent links, independent of the unit tests that pin each analyzer.
//!
//! Ranking, hit ownership and snippets read `CodeBlock::span` and `CodeBlock::parent_index` and trust them. This file
//! runs both analyzers over code nobody wrote for the test and checks, for every block of every file, what that trust
//! needs:
//!
//! - the span is inside the lossy-decoded source, on character boundaries, and slices to the block's `code`;
//! - the span starts at the block's declaration, after any attributes, decorators and comments the block carries;
//! - `parent_index` is set exactly when `parent` is, points at an earlier block of `blocks` followed by `test_blocks`,
//!   and that block's span strictly encloses the child's;
//! - following parents never loops;
//! - no two blocks partially overlap, and a block's parent is the innermost block that encloses it;
//! - the declared `name` occurs in the span;
//! - `signature_span` is set exactly when `symbol_signature` is, lies inside the block's span, and its text, on one line,
//!   holds the signature.
//!
//! The corpora are the checked-in fixtures and this crate's own source, a test over the pinned harness repositories
//! when they are on disk, and generated TypeScript and Rust. The checker is itself tested by breaking one invariant at
//! a time in a real analysis and expecting that invariant to be reported.
//!
//! The harness test is `#[ignore]`d and never clones. It reads `target/rg-compare`, which `scripts/rg-compare.py` fills,
//! or the directories in `MONOKL_SPAN_CORPUS` (separated like `PATH`), and says so and passes when neither exists.
#![cfg(all(feature = "lang-ts", feature = "lang-rust"))]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a failing assertion should panic in a test")]
#![allow(clippy::print_stderr, reason = "an ignored test says why it did nothing")]

use std::cmp::Reverse;
use std::fmt::Write as _;
use std::fs;
use std::ops::Range;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::WalkBuilder;
use monokl::analysis::lang::Language;
use monokl::analysis::rust::RustAnalyzer;
use monokl::analysis::ts::TsAnalyzer;
use monokl::analysis::{FileAnalysis, LanguageAnalyzer};
use monokl::types::{CodeBlock, SymbolKind};
use rayon::prelude::*;

/// The invariants this file checks, so a test can name the one it broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Invariant {
    SpanInBounds,
    SpanOnCharBoundaries,
    SpanSlicesToCode,
    StartsAtDeclaration,
    NameInSpan,
    SignatureSpanMatchesSignature,
    SignatureSpanInBlock,
    SignatureSpanHoldsSignature,
    ParentIndexMatchesParent,
    ParentIndexInRange,
    ParentPrecedesChild,
    ParentEnclosesChild,
    ParentChainEnds,
    NoPartialOverlap,
    ParentIsInnermost,
}

#[derive(Debug)]
struct Violation {
    invariant: Invariant,
    file: Utf8PathBuf,
    /// Position in `blocks` followed by `test_blocks`.
    block: usize,
    note: String,
    excerpt: String,
}

fn excerpt(source: &str, span: &Range<usize>) -> String {
    let end = span.end.min(source.len());
    let start = span.start.min(end);
    let text = String::from_utf8_lossy(&source.as_bytes()[start..end]);
    text.chars().take(80).collect::<String>().escape_debug().to_string()
}

fn analyzer_for(path: &Utf8Path) -> Option<&'static dyn LanguageAnalyzer> {
    match Language::of(path)? {
        Language::Rust => Some(&RustAnalyzer),
        Language::TypeScript | Language::JavaScript => Some(&TsAnalyzer),
        // `Language` is non-exhaustive for code outside the crate; a new language needs an analyzer here.
        _ => None,
    }
}

fn analyze(path: &Utf8Path, source: &str) -> Arc<FileAnalysis> {
    analyzer_for(path).unwrap().analyze(path, source.to_owned()).unwrap()
}

/// `text` on one line, the way the Rust analyzer writes an `impl`'s name.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The end of the raw string (`r"..."`, `r#"..."#`) whose opening quote is at `quote` in `text`, if one opens there.
fn raw_string_end(text: &str, quote: usize) -> Option<usize> {
    let before = &text[..quote];
    let hashes = before.len() - before.trim_end_matches('#').len();
    let before = &before[..before.len() - hashes];
    let boundary =
        before.strip_suffix('r').is_some_and(|rest| !rest.ends_with(|c: char| c.is_alphanumeric() || c == '_'));
    let closing = format!("\"{}", "#".repeat(hashes));
    boundary.then(|| text[quote + 1..].find(&closing).map(|at| quote + 1 + at + closing.len())).flatten()
}

/// Skips the balanced `open ... close` group at the start of `text`, looking past quoted text, and returns what follows.
fn skip_group<'a>(text: &'a str, open: char, close: char, quotes: &[char], raw_strings: bool) -> &'a str {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut resume = 0;
    for (offset, c) in text.char_indices() {
        if offset < resume {
            continue;
        }
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
        } else if raw_strings
            && c == '"'
            && let Some(end) = raw_string_end(text, offset)
        {
            resume = end;
        } else if quotes.contains(&c) {
            quote = Some(c);
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return &text[offset + c.len_utf8()..];
            }
        }
    }
    ""
}

/// What a block's text may start with before its declaration: whitespace, comments, and attributes (`#[...]`) or
/// decorators (`@dec(...)`).
fn strip_prelude(language: Language, mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        if let Some(rest) = text.strip_prefix("//") {
            text = rest.split_once('\n').map_or("", |(_, after)| after);
        } else if let Some(rest) = text.strip_prefix("/*") {
            text = rest.split_once("*/").map_or("", |(_, after)| after);
        } else if language == Language::Rust && text.starts_with('#') {
            // Generated code spaces its tokens, `# [doc = ...]`.
            let rest = text[1..].trim_start().trim_start_matches('!').trim_start();
            if !rest.starts_with('[') {
                return text;
            }
            text = skip_group(rest, '[', ']', &['"'], true);
        } else if language != Language::Rust && text.starts_with('@') {
            let rest = text[1..].trim_start_matches(|c: char| c.is_alphanumeric() || matches!(c, '_' | '$' | '.'));
            text = if rest.starts_with('(') { skip_group(rest, '(', ')', &['"', '\'', '`'], false) } else { rest };
        } else {
            return text;
        }
    }
}

fn first_word(text: &str) -> &str {
    let end = text.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(text.len());
    &text[..end]
}

const RUST_ITEM_STARTS: &[&str] = &[
    "pub",
    "fn",
    "const",
    "async",
    "unsafe",
    "extern",
    "struct",
    "enum",
    "union",
    "trait",
    "type",
    "static",
    "mod",
    "impl",
    "macro_rules",
    "macro",
    "default",
    "safe",
    "gen",
    "auto",
];
const TS_STATEMENT_STARTS: &[&str] = &[
    "export",
    "declare",
    "abstract",
    "async",
    "function",
    "class",
    "interface",
    "type",
    "enum",
    "const",
    "let",
    "var",
    "namespace",
    "module",
    "global",
    "import",
    "using",
];
const TS_MEMBER_MODIFIERS: &[&str] = &[
    "static",
    "async",
    "public",
    "private",
    "protected",
    "readonly",
    "override",
    "get",
    "set",
    "abstract",
    "declare",
    "accessor",
];

/// Whether `text`, a block's code, starts at what the block declares, once the prelude is skipped.
fn starts_at_declaration(language: Language, block: &CodeBlock, text: &str) -> bool {
    let mut rest = strip_prelude(language, text);
    if language == Language::Rust {
        // A macro call item is a path, then `!`.
        let macro_call = block.node_kind == SymbolKind::Macro
            && rest.split(['(', '{', '[']).next().is_some_and(|path| path.contains('!'));
        return RUST_ITEM_STARTS.contains(&first_word(rest)) || macro_call;
    }
    if block.parent.is_none() {
        return TS_STATEMENT_STARTS.contains(&first_word(rest));
    }
    // A member starts with modifiers, then its key: the name, a string, a computed key or a private name.
    loop {
        if block.name.as_deref().is_none_or(|name| rest.starts_with(name))
            || rest.starts_with(['[', '\'', '"', '`', '#'])
        {
            return true;
        }
        if let Some(after) = rest.strip_prefix('*') {
            rest = after.trim_start();
            continue;
        }
        let word = first_word(rest);
        if !TS_MEMBER_MODIFIERS.contains(&word) {
            return false;
        }
        rest = rest[word.len()..].trim_start();
    }
}

/// Every invariant violation of one analyzed file.
fn violations(path: &Utf8Path, source: &str, analysis: &FileAnalysis) -> Vec<Violation> {
    let language = Language::of(path).unwrap();
    let all: Vec<&CodeBlock> = analysis.blocks.iter().chain(&analysis.test_blocks).collect();
    let mut found = Vec::new();
    let mut report = |invariant, block: usize, note: String| {
        let excerpt = excerpt(source, &all[block].span);
        found.push(Violation { invariant, file: path.to_owned(), block, note, excerpt });
    };

    // Which spans are sound enough to compare against each other.
    let mut usable = vec![false; all.len()];
    for (index, block) in all.iter().enumerate() {
        let Range { start, end } = block.span;
        if start >= end || end > source.len() {
            report(Invariant::SpanInBounds, index, format!("span {start}..{end} of {} bytes", source.len()));
            continue;
        }
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            report(Invariant::SpanOnCharBoundaries, index, format!("span {start}..{end}"));
            continue;
        }
        usable[index] = true;
        let text = &source[start..end];
        if text != block.code {
            report(Invariant::SpanSlicesToCode, index, format!("span {start}..{end} is not the block's code"));
        }
        if !starts_at_declaration(language, block, text) {
            report(Invariant::StartsAtDeclaration, index, format!("{:?} {:?}", block.node_kind, block.name));
        }
        if let Some(name) = &block.name
            && !text.contains(name.as_str())
            && !collapse(text).contains(&collapse(name))
        {
            report(Invariant::NameInSpan, index, format!("name {name:?}"));
        }
        if block.signature_span.is_some() != block.symbol_signature.is_some() {
            let note = format!("signature_span {:?}, signature {:?}", block.signature_span, block.symbol_signature);
            report(Invariant::SignatureSpanMatchesSignature, index, note);
        }
        let Some(head) = &block.signature_span else { continue };
        if head.start < start
            || head.end > end
            || !source.is_char_boundary(head.start)
            || !source.is_char_boundary(head.end)
        {
            report(Invariant::SignatureSpanInBlock, index, format!("head {head:?} of span {start}..{end}"));
        } else if let Some(signature) = &block.symbol_signature
            && !collapse(&source[head.clone()]).contains(signature.as_str())
        {
            report(Invariant::SignatureSpanHoldsSignature, index, format!("head {head:?} lacks {signature:?}"));
        }
    }

    for (index, block) in all.iter().enumerate() {
        if block.parent_index.is_some() != block.parent.is_some() {
            report(
                Invariant::ParentIndexMatchesParent,
                index,
                format!("parent_index {:?}, parent {:?}", block.parent_index, block.parent),
            );
        }
        let Some(parent) = block.parent_index else { continue };
        if parent >= all.len() {
            report(Invariant::ParentIndexInRange, index, format!("parent_index {parent} of {} blocks", all.len()));
            continue;
        }
        if parent >= index {
            report(Invariant::ParentPrecedesChild, index, format!("parent_index {parent}"));
        }
        let (outer, inner) = (&all[parent].span, &block.span);
        if usable[parent] && usable[index] {
            let encloses = outer.start <= inner.start && inner.end <= outer.end && outer != inner;
            if !encloses {
                report(
                    Invariant::ParentEnclosesChild,
                    index,
                    format!("parent {parent} spans {outer:?}, child {inner:?}"),
                );
            }
        }
        // Walk up without trusting the order, so a cycle is reported even when `ParentPrecedesChild` is.
        let (mut at, mut steps) = (index, 0);
        while let Some(next) = all[at].parent_index.filter(|&next| next < all.len()) {
            steps += 1;
            if steps > all.len() {
                report(Invariant::ParentChainEnds, index, "parents never reach a root".to_owned());
                break;
            }
            at = next;
        }
    }

    // One pass over the blocks in source order, a holder before what it holds, keeping the blocks still open: the top
    // is the innermost one enclosing the next block, and a block that starts inside it and ends after it is a partial
    // overlap.
    let mut order: Vec<usize> = (0..all.len()).filter(|&index| usable[index]).collect();
    order.sort_by_key(|&index| (all[index].span.start, Reverse(all[index].span.end), index));
    let mut open: Vec<usize> = Vec::new();
    for index in order {
        let span = &all[index].span;
        while open.last().is_some_and(|&top| all[top].span.end <= span.start) {
            open.pop();
        }
        let innermost = open.last().copied();
        if let Some(top) = innermost
            && all[top].span.end < span.end
        {
            report(Invariant::NoPartialOverlap, index, format!("{span:?} overlaps block {top} at {:?}", all[top].span));
        } else if all[index].parent_index.is_none_or(|parent| parent < all.len())
            && all[index].parent_index != innermost
        {
            report(
                Invariant::ParentIsInnermost,
                index,
                format!("parent_index {:?}, innermost enclosing block {innermost:?}", all[index].parent_index),
            );
        }
        open.push(index);
    }
    found
}

fn kinds(found: &[Violation]) -> Vec<Invariant> {
    let mut kinds: Vec<Invariant> = found.iter().map(|violation| violation.invariant).collect();
    kinds.dedup();
    kinds
}

fn render(found: &[Violation]) -> String {
    let mut text = format!("{} invariant violations\n", found.len());
    for violation in found.iter().take(25) {
        writeln!(
            text,
            "  {} block {} [{:?}] {}\n      {}",
            violation.file, violation.block, violation.invariant, violation.note, violation.excerpt
        )
        .unwrap();
    }
    text
}

/// What a corpus run saw, so a run that checked nothing does not pass.
#[derive(Debug, Default)]
struct Totals {
    typescript_files: usize,
    rust_files: usize,
    blocks: usize,
    nested_blocks: usize,
}

/// Analyzes and checks one file as the pipeline reads it: bytes decoded lossily.
fn check_file(path: &Utf8Path, bytes: &[u8]) -> (Totals, Vec<Violation>) {
    let source = String::from_utf8_lossy(bytes);
    let analysis = analyze(path, &source);
    let all = || analysis.blocks.iter().chain(&analysis.test_blocks);
    let rust = Language::of(path) == Some(Language::Rust);
    let totals = Totals {
        typescript_files: usize::from(!rust),
        rust_files: usize::from(rust),
        blocks: all().count(),
        nested_blocks: all().filter(|block| block.parent_index.is_some()).count(),
    };
    (totals, violations(path, &source, &analysis))
}

fn check_tree(root: &Utf8Path) -> (Totals, Vec<Violation>) {
    let walk = WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some(".git" | "node_modules" | "target")))
        .build();
    let mut files: Vec<Utf8PathBuf> = walk
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.into_path()).ok())
        .filter(|path| Language::of(path).is_some())
        .collect();
    files.sort();
    let results: Vec<_> = files.par_iter().map(|path| check_file(path, &fs::read(path).unwrap())).collect();
    let mut totals = Totals::default();
    let mut found = Vec::new();
    for (file_totals, file_found) in results {
        totals.typescript_files += file_totals.typescript_files;
        totals.rust_files += file_totals.rust_files;
        totals.blocks += file_totals.blocks;
        totals.nested_blocks += file_totals.nested_blocks;
        found.extend(file_found);
    }
    (totals, found)
}

fn manifest_dir() -> &'static Utf8Path {
    Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_checked_in_fixtures_and_this_crates_source_hold_every_invariant() {
    let (fixtures, mut found) = check_tree(&manifest_dir().join("tests/fixtures"));
    let (source, in_source) = check_tree(&manifest_dir().join("src"));
    found.extend(in_source);
    assert!(found.is_empty(), "{}", render(&found));
    // The corpora are not empty: the fixtures have no members, so the nesting is the source's.
    assert!(fixtures.typescript_files >= 5 && fixtures.blocks >= 10, "{fixtures:?}");
    assert!(source.rust_files >= 30 && source.nested_blocks > 100, "{source:?}");
}

/// The directories of the harness repositories on this machine.
fn harness_roots() -> Vec<Utf8PathBuf> {
    if let Some(paths) = std::env::var_os("MONOKL_SPAN_CORPUS") {
        return std::env::split_paths(&paths).filter_map(|path| Utf8PathBuf::from_path_buf(path).ok()).collect();
    }
    let cache = manifest_dir().ancestors().nth(2).unwrap().join("target/rg-compare");
    let Ok(entries) = fs::read_dir(&cache) else { return Vec::new() };
    let mut roots: Vec<Utf8PathBuf> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.path()).ok())
        .filter(|path| path.is_dir())
        .collect();
    roots.sort();
    roots
}

#[test]
#[ignore = "walks the harness repositories under target/rg-compare or MONOKL_SPAN_CORPUS; `just span-invariants`"]
fn the_harness_repositories_hold_every_invariant() {
    let roots: Vec<_> = harness_roots().into_iter().filter(|root| root.is_dir()).collect();
    if roots.is_empty() {
        eprintln!(
            "skipped: no harness repositories. Run `python3 -I scripts/rg-compare.py` to fill target/rg-compare, or \
             set MONOKL_SPAN_CORPUS to directories of source."
        );
        return;
    }
    let mut found = Vec::new();
    for root in &roots {
        let (totals, in_root) = check_tree(root);
        eprintln!("{root}: {totals:?}, {} violations", in_root.len());
        found.extend(in_root);
    }
    assert!(found.is_empty(), "{}", render(&found));
}

// The checker catches what it is for: each test breaks one invariant in a real analysis.

const TS_SAMPLE: &str = "\
// Café
export class Café {
  load(id: string) {
    return id;
  }
  save = (id: string) => {
    return id;
  };
}
export const api = {
  fetch() {
    return 1;
  },
};
";

const RUST_SAMPLE: &str = "\
mod outer {
    impl Foo {
        fn first(&self) {
            let _ = 1;
        }
        fn second(&self) {
            let _ = 2;
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn works() {}
}
";

/// The analysis of a sample with `tamper` applied to its `blocks`, checked against the sample.
fn tampered(path: &str, source: &str, tamper: impl FnOnce(&mut Vec<CodeBlock>)) -> Vec<Violation> {
    let path = Utf8Path::new(path);
    let mut analysis = (*analyze(path, source)).clone();
    tamper(&mut analysis.blocks);
    violations(path, source, &analysis)
}

fn position(blocks: &[CodeBlock], name: &str) -> usize {
    blocks.iter().position(|block| block.name.as_deref() == Some(name)).unwrap()
}

#[test]
fn untouched_samples_have_no_violations_and_the_shape_the_tests_below_rely_on() {
    for (path, source) in [("a.ts", TS_SAMPLE), ("a.rs", RUST_SAMPLE)] {
        let analysis = analyze(Utf8Path::new(path), source);
        let found = violations(Utf8Path::new(path), source, &analysis);
        assert!(found.is_empty(), "{}", render(&found));
    }
    let ts = analyze(Utf8Path::new("a.ts"), TS_SAMPLE);
    let shape: Vec<_> = ts.blocks.iter().map(|b| (b.name.as_deref().unwrap(), b.parent_index)).collect();
    assert_eq!(shape, [("Café", None), ("load", Some(0)), ("save", Some(0)), ("api", None), ("fetch", Some(3))]);
    let rs = analyze(Utf8Path::new("a.rs"), RUST_SAMPLE);
    let shape: Vec<_> =
        rs.blocks.iter().chain(&rs.test_blocks).map(|b| (b.name.as_deref().unwrap(), b.parent_index)).collect();
    assert_eq!(
        shape,
        [
            ("outer", None),
            ("Foo", Some(0)),
            ("first", Some(1)),
            ("second", Some(1)),
            ("tests", None),
            ("works", Some(4))
        ]
    );
}

#[test]
fn a_span_past_the_source_is_out_of_bounds() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[0].span.end = TS_SAMPLE.len() + 1);
    assert!(kinds(&found).contains(&Invariant::SpanInBounds), "{}", render(&found));
}

#[test]
fn an_empty_span_is_out_of_bounds() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].span = 40..40);
    assert!(kinds(&found).contains(&Invariant::SpanInBounds), "{}", render(&found));
}

#[test]
fn a_span_inside_a_character_is_off_the_boundaries() {
    let inside = TS_SAMPLE.rfind('é').unwrap() + 1;
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].span.start = inside);
    assert!(kinds(&found).contains(&Invariant::SpanOnCharBoundaries), "{}", render(&found));
}

#[test]
fn a_span_that_slices_other_text_than_the_code_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[3].span.start += 1);
    assert!(kinds(&found).contains(&Invariant::SpanSlicesToCode), "{}", render(&found));
}

#[test]
fn a_span_that_starts_inside_the_declaration_is_reported() {
    let found = tampered("a.rs", RUST_SAMPLE, |blocks| {
        let block = &mut blocks[2];
        block.span.start += 3;
        block.code = RUST_SAMPLE[block.span.clone()].to_owned();
    });
    assert!(kinds(&found).contains(&Invariant::StartsAtDeclaration), "{}", render(&found));
    let found = tampered("a.ts", TS_SAMPLE, |blocks| {
        let block = &mut blocks[1];
        block.span.start += 1;
        block.code = TS_SAMPLE[block.span.clone()].to_owned();
    });
    assert!(kinds(&found).contains(&Invariant::StartsAtDeclaration), "{}", render(&found));
}

#[test]
fn a_name_the_span_does_not_contain_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].name = Some("unload".to_owned()));
    assert!(kinds(&found).contains(&Invariant::NameInSpan), "{}", render(&found));
}

#[test]
fn a_signature_span_without_a_signature_or_a_signature_without_a_span_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].signature_span = None);
    assert!(kinds(&found).contains(&Invariant::SignatureSpanMatchesSignature), "{}", render(&found));
    let found = tampered("a.rs", RUST_SAMPLE, |blocks| blocks[2].symbol_signature = None);
    assert!(kinds(&found).contains(&Invariant::SignatureSpanMatchesSignature), "{}", render(&found));
}

#[test]
fn a_signature_span_outside_its_block_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].signature_span = Some(0..5));
    assert!(kinds(&found).contains(&Invariant::SignatureSpanInBlock), "{}", render(&found));
    let found = tampered("a.ts", TS_SAMPLE, |blocks| {
        let end = blocks[1].span.end;
        blocks[1].signature_span = Some(blocks[1].span.start..end + 1);
    });
    assert!(kinds(&found).contains(&Invariant::SignatureSpanInBlock), "{}", render(&found));
}

#[test]
fn a_signature_span_whose_text_lacks_the_signature_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| {
        let head = blocks[1].signature_span.clone().unwrap();
        blocks[1].signature_span = Some(head.start + 2..head.end);
    });
    assert!(kinds(&found).contains(&Invariant::SignatureSpanHoldsSignature), "{}", render(&found));
    let found = tampered("a.rs", RUST_SAMPLE, |blocks| {
        let head = blocks[2].signature_span.clone().unwrap();
        blocks[2].signature_span = Some(head.start..head.end - 4);
    });
    assert!(kinds(&found).contains(&Invariant::SignatureSpanHoldsSignature), "{}", render(&found));
}

#[test]
fn a_parent_index_without_a_parent_or_a_parent_without_an_index_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].parent_index = None);
    assert!(kinds(&found).contains(&Invariant::ParentIndexMatchesParent), "{}", render(&found));
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[3].parent_index = Some(0));
    assert!(kinds(&found).contains(&Invariant::ParentIndexMatchesParent), "{}", render(&found));
}

#[test]
fn a_parent_index_past_the_block_list_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].parent_index = Some(99));
    assert!(kinds(&found).contains(&Invariant::ParentIndexInRange), "{}", render(&found));
}

#[test]
fn a_parent_that_does_not_come_first_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].parent_index = Some(1));
    assert!(kinds(&found).contains(&Invariant::ParentPrecedesChild), "{}", render(&found));
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[1].parent_index = Some(2));
    assert!(kinds(&found).contains(&Invariant::ParentPrecedesChild), "{}", render(&found));
}

#[test]
fn a_parent_cycle_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| {
        blocks[0].parent_index = Some(1);
        blocks[0].parent = blocks[1].parent.clone();
    });
    assert!(kinds(&found).contains(&Invariant::ParentChainEnds), "{}", render(&found));
}

#[test]
fn a_parent_that_does_not_enclose_its_child_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| blocks[4].parent_index = Some(2));
    assert!(kinds(&found).contains(&Invariant::ParentEnclosesChild), "{}", render(&found));
}

#[test]
fn a_parent_with_the_same_span_as_its_child_is_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| {
        blocks[1].span = blocks[0].span.clone();
        blocks[1].code = blocks[0].code.clone();
    });
    assert!(kinds(&found).contains(&Invariant::ParentEnclosesChild), "{}", render(&found));
}

#[test]
fn blocks_that_partly_overlap_are_reported() {
    let found = tampered("a.ts", TS_SAMPLE, |blocks| {
        let next = blocks[2].span.start;
        blocks[1].span.end = next + 10;
        blocks[1].code = TS_SAMPLE[blocks[1].span.clone()].to_owned();
    });
    assert!(kinds(&found).contains(&Invariant::NoPartialOverlap), "{}", render(&found));
}

#[test]
fn a_parent_above_the_innermost_enclosing_block_is_reported() {
    let found = tampered("a.rs", RUST_SAMPLE, |blocks| {
        let method = position(blocks, "first");
        blocks[method].parent_index = Some(0);
    });
    assert!(kinds(&found).contains(&Invariant::ParentIsInnermost), "{}", render(&found));
}

#[test]
fn a_root_that_an_earlier_block_encloses_is_reported() {
    let found = tampered("a.rs", RUST_SAMPLE, |blocks| {
        let method = position(blocks, "first");
        blocks[method].parent_index = None;
        blocks[method].parent = None;
    });
    assert!(kinds(&found).contains(&Invariant::ParentIsInnermost), "{}", render(&found));
}

#[test]
fn the_prelude_skips_comments_attributes_and_decorators_and_nothing_else() {
    let rust = |text| strip_prelude(Language::Rust, text);
    assert_eq!(rust("/// doc\n#[a(\"]\")]\n  // x\n/* y */ pub fn f() {}"), "pub fn f() {}");
    assert_eq!(rust("#[test_case(r\"\\c\"; \"a]\")]\nfn f() {}"), "fn f() {}");
    assert_eq!(rust("#[test_case(r#\"a\"]\"#)] fn f() {}"), "fn f() {}");
    assert_eq!(rust("# [doc = \"x\"] # [allow (a)] impl A {}"), "impl A {}");
    assert_eq!(rust("#![cfg(test)] fn f() {}"), "fn f() {}");
    assert_eq!(rust("#nope"), "#nope");
    let ts = |text| strip_prelude(Language::TypeScript, text);
    assert_eq!(ts("@dec\n@a.b({ x: ')' }) /* c */ load() {}"), "load() {}");
    assert_eq!(ts("static @x"), "static @x");
}

// Generated sources: nested classes, objects, impls and modules, with comments, attributes, multi-byte text and CRLF.

mod generated {
    use proptest::prelude::*;
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

    use super::*;

    const NAMES: &[&str] = &["alpha", "beta", "gamma", "load", "save", "épsilon", "日本語", "$ref", "_priv", "Self_"];
    const TEXT: &[&str] = &["plain", "naïve", "日本 😀", "tab\\t", "a{b}", "(open", "// not a comment", "'q'"];

    fn pick(options: &'static [&'static str]) -> prop::sample::Select<&'static str> {
        prop::sample::select(options)
    }

    fn name() -> impl Strategy<Value = String> {
        pick(NAMES).prop_map(str::to_owned)
    }

    fn text() -> impl Strategy<Value = &'static str> {
        pick(TEXT)
    }

    fn gap() -> impl Strategy<Value = &'static str> {
        pick(&["\n", "\n\n", "\n// gap é 😀\n", "\n/** doc */\n", "\n\n\n", "\n/* a\n b */\n"])
    }

    fn indent(body: &str, by: &str) -> String {
        body.lines().map(|line| format!("{by}{line}")).collect::<Vec<_>>().join("\n")
    }

    fn concat(items: Vec<(String, &'static str)>) -> String {
        items.into_iter().fold(String::new(), |mut text, (item, gap)| {
            text.push_str(&item);
            text.push_str(gap);
            text
        })
    }

    /// Items one after another with a gap after each, in CRLF or LF.
    fn join(items: Vec<(String, &'static str)>, crlf: bool) -> String {
        let source = concat(items);
        if crlf { source.replace('\n', "\r\n") } else { source }
    }

    // TypeScript

    fn ts_statement() -> impl Strategy<Value = String> {
        prop_oneof![
            text().prop_map(|t| format!("const s = \"{t}\";")),
            text().prop_map(|t| format!("// {t}")),
            text().prop_map(|t| format!("/* {t} */")),
            text().prop_map(|t| format!("const t = `{t} ${{x}}`;")),
            Just("if (a) { b(); }".to_owned()),
            Just("const o = { k: () => {\n  1;\n} };".to_owned()),
            Just("return { a: 1 };".to_owned()),
        ]
    }

    fn ts_body() -> impl Strategy<Value = String> {
        prop::collection::vec(ts_statement(), 0..4).prop_map(|lines| indent(&lines.join("\n"), "  "))
    }

    fn ts_member() -> impl Strategy<Value = String> {
        let modifier = pick(&["", "static ", "async ", "public ", "private ", "static async ", "get ", "set "]);
        let decorator = pick(&["", "@dec\n", "@dec({ a: '}' })\n", "/* c */ @a.b() "]);
        let methods = prop_oneof![
            (decorator.clone(), modifier, name(), ts_body())
                .prop_map(|(d, m, n, body)| format!("{d}{m}{n}(a: string) {{\n{body}\n}}")),
            (decorator, name(), ts_body()).prop_map(|(d, n, body)| format!("{d}{n} = (a) => {{\n{body}\n}};")),
            (name(), ts_body()).prop_map(|(n, body)| format!("static {n} = function () {{\n{body}\n}};")),
            ts_body().prop_map(|body| format!("constructor(private x: number) {{\n{body}\n}}")),
            ts_body().prop_map(|body| format!("'quoted-key'() {{\n{body}\n}}")),
            ts_body().prop_map(|body| format!("[Symbol.iterator]() {{\n{body}\n}}")),
            ts_body().prop_map(|body| format!("#hidden() {{\n{body}\n}}")),
        ];
        let others = prop_oneof![
            name().prop_map(|n| format!("{n}: string = 'x';")),
            name().prop_map(|n| format!("{n}(a: string): void;")),
            (name(), ts_body())
                .prop_map(|(n, body)| format!("static Inner = class Nested {{\n  m() {{\n{body}\n  }}\n}};\n// {n}")),
        ];
        prop_oneof![methods, others]
    }

    fn ts_members() -> impl Strategy<Value = String> {
        prop::collection::vec(ts_member(), 0..5).prop_map(|members| indent(&members.join("\n"), "  "))
    }

    fn ts_object_member() -> impl Strategy<Value = String> {
        prop_oneof![
            (name(), ts_body()).prop_map(|(n, body)| format!("{n}() {{\n{body}\n}},")),
            (name(), ts_body()).prop_map(|(n, body)| format!("async {n}() {{\n{body}\n}},")),
            (name(), ts_body()).prop_map(|(n, body)| format!("{n}: () => {{\n{body}\n}},")),
            (name(), ts_body()).prop_map(|(n, body)| format!("get {n}() {{\n{body}\n}},")),
            (name(), ts_body()).prop_map(|(n, body)| format!("{n}: {{\n  inner() {{\n{body}\n  }},\n}},")),
            name().prop_map(|n| format!("{n}: 1,")),
            name().prop_map(|n| format!("...{n},")),
        ]
    }

    fn ts_object() -> impl Strategy<Value = String> {
        prop::collection::vec(ts_object_member(), 0..5)
            .prop_map(|members| format!("{{\n{}\n}}", indent(&members.join("\n"), "  ")))
    }

    fn ts_declaration() -> impl Strategy<Value = String> {
        let export = pick(&["", "export ", "export default ", "declare ", "export declare "]);
        let decorator = pick(&["", "@dec\n", "@dec({ a: '}' })\n"]);
        prop_oneof![
            (decorator, export.clone(), pick(&["", "abstract "]), name(), ts_members()).prop_map(
                |(d, e, a, n, members)| format!("{d}{e}{a}class {n} extends Base<{{ a: 1 }}> {{\n{members}\n}}")
            ),
            (export.clone(), name(), ts_object(), pick(&["", " as const", " satisfies T"]))
                .prop_map(|(e, n, object, tail)| format!("{}const {n} = {object}{tail};", e.replace("default ", ""))),
            ts_object().prop_map(|object| format!("export default {object};")),
            (name(), ts_object(), name(), ts_object()).prop_map(|(a, x, b, y)| format!("const {a} = {x}, {b} = {y};")),
            (export, pick(&["", "async "]), pick(&["", "*"]), name(), ts_body())
                .prop_map(|(e, a, g, n, body)| format!("{e}{a}function{g} {n}(a: string): string {{\n{body}\n}}")),
            (name(), ts_body()).prop_map(|(n, body)| format!("export default function {n}() {{\n{body}\n}}")),
            (name(), ts_members()).prop_map(|(n, members)| format!("export default class {n} {{\n{members}\n}}")),
        ]
    }

    fn ts_other_statement() -> impl Strategy<Value = String> {
        prop_oneof![
            (name(), ts_members()).prop_map(|(n, members)| format!("export interface {n} {{\n{members}\n}}")),
            name().prop_map(|n| format!("export type {n}<T> = {{\n  a: T;\n}};")),
            name().prop_map(|n| format!("export enum {n} {{\n  A = 1,\n  B,\n}}")),
            (name(), ts_body())
                .prop_map(|(n, body)| format!("export namespace {n} {{\n  export function f() {{\n{body}\n  }}\n}}")),
            ts_body().prop_map(|body| format!("declare module \"some-mod\" {{\n{body}\n}}")),
            ts_body().prop_map(|body| format!("declare global {{\n{body}\n}}")),
            name().prop_map(|n| format!("import {{ {n} }} from './{n}';")),
            name().prop_map(|n| format!("export {{ {n} }} from './{n}';")),
            name().prop_map(|n| format!("const {{ {n}, ...rest }} = require('x');")),
        ]
    }

    fn ts_source() -> impl Strategy<Value = String> {
        let item = prop_oneof![ts_declaration(), ts_other_statement()];
        (prop::collection::vec((item, gap()), 0..6), any::<bool>()).prop_map(|(items, crlf)| join(items, crlf))
    }

    // Rust

    fn rs_statement() -> impl Strategy<Value = String> {
        prop_oneof![
            text().prop_map(|t| format!("let s = \"{t}\";")),
            text().prop_map(|t| format!("// {t}")),
            text().prop_map(|t| format!("/* {t} */")),
            Just("if a { b(); }".to_owned()),
            Just("match x { _ => {} }".to_owned()),
            Just("let f = |a| { a + 1 };".to_owned()),
            Just("let c = 'x';".to_owned()),
        ]
    }

    fn rs_body() -> impl Strategy<Value = String> {
        prop::collection::vec(rs_statement(), 0..4).prop_map(|lines| indent(&lines.join("\n"), "    "))
    }

    fn rs_attrs() -> impl Strategy<Value = &'static str> {
        pick(&[
            "",
            "#[inline]\n",
            "/// doc é\n",
            "#[derive(Debug)]\n/// doc\n",
            "#[allow(x, reason = \"]\")]\n",
            "// plain\n",
        ])
    }

    fn rs_fn() -> impl Strategy<Value = String> {
        let qualifier = pick(&["", "async ", "const ", "unsafe "]);
        (rs_attrs(), pick(&["", "pub ", "pub(crate) "]), qualifier, name(), rs_body()).prop_map(
            |(attrs, vis, q, n, body)| format!("{attrs}{vis}{q}fn {n}<T>(a: T) -> T where T: Clone {{\n{body}\n}}"),
        )
    }

    fn rs_member() -> impl Strategy<Value = String> {
        prop_oneof![
            rs_fn(),
            (name(), rs_body()).prop_map(|(n, body)| format!("#[test]\nfn {n}() {{\n{body}\n}}")),
            name().prop_map(|n| format!("#[test]\nfn {n}() {{}}")),
            name().prop_map(|n| format!("pub fn {n}(&self) -> u8 {{ 1 }}")),
            name().prop_map(|n| format!("fn {n}(&self);")),
            name().prop_map(|n| format!("const {n}: u8 = 1;")),
            name().prop_map(|n| format!("#[cfg(test)]\nconst {n}: u8 = 1;")),
            name().prop_map(|n| format!("type {n} = u8;")),
            Just("some_macro!(a, b);".to_owned()),
            Just("#[cfg(test)]\nsome_macro! { a }".to_owned()),
        ]
    }

    fn rs_members() -> impl Strategy<Value = String> {
        prop::collection::vec(rs_member(), 0..5).prop_map(|members| indent(&members.join("\n"), "    "))
    }

    fn rs_leaf() -> impl Strategy<Value = String> {
        let impl_head = pick(&[
            "impl Foo",
            "impl<T> Foo<T>",
            "impl<T> From<T>   for   Foo<T> where T: Clone",
            "unsafe impl Send for Foo",
            "impl dyn Foo",
        ]);
        let types = prop_oneof![
            (rs_attrs(), pick(&["", "pub "]), name())
                .prop_map(|(a, v, n)| format!("{a}{v}struct {n} {{\n    a: u8,\n}}")),
            (rs_attrs(), name()).prop_map(|(a, n)| format!("{a}struct {n}(u8);")),
            (rs_attrs(), name()).prop_map(|(a, n)| format!("{a}pub enum {n} {{\n    A,\n    B(u8),\n}}")),
            name().prop_map(|n| format!("union {n} {{\n    a: u8,\n}}")),
            (rs_attrs(), name(), rs_members()).prop_map(|(a, n, m)| format!("{a}pub trait {n}: Sized {{\n{m}\n}}")),
            (rs_attrs(), impl_head, rs_members()).prop_map(|(a, h, m)| format!("{a}{h} {{\n{m}\n}}")),
        ];
        let others = prop_oneof![
            name().prop_map(|n| format!("macro_rules! {n} {{\n    () => {{}};\n}}")),
            name().prop_map(|n| format!("pub const {n}: &str = \"日本\";")),
            name().prop_map(|n| format!("static {n}: u8 = 1;")),
            name().prop_map(|n| format!("pub type {n} = Vec<u8>;")),
            Just("thread_local! {\n    static X: u8 = 1;\n}".to_owned()),
            Just("extern \"C\" {\n    fn f();\n}".to_owned()),
            name().prop_map(|n| format!("mod {n};")),
            name().prop_map(|n| format!("use std::{n};")),
            name().prop_map(|n| format!("extern crate {n};")),
        ];
        prop_oneof![rs_fn(), types, others]
    }

    fn rs_item() -> impl Strategy<Value = String> {
        rs_leaf().prop_recursive(3, 24, 4, |inner| {
            let attrs = pick(&[
                "",
                "#[cfg(test)]\n",
                "#[cfg(all(test, unix))]\n",
                "#[cfg(not(test))]\n",
                "#[cfg(any(test, unix))]\n",
                "pub ",
            ]);
            (attrs, name(), prop::collection::vec((inner, gap()), 0..4)).prop_map(|(attrs, n, items)| {
                let body = concat(items);
                format!("{attrs}mod {n} {{\n{}\n}}", indent(&body, "    "))
            })
        })
    }

    fn rs_source() -> impl Strategy<Value = String> {
        let inner_attr = pick(&["", "", "", "#![allow(dead_code)]\n", "#![cfg(test)]\n", "//! crate doc é\n"]);
        (inner_attr, prop::collection::vec((rs_item(), gap()), 0..6), any::<bool>())
            .prop_map(|(inner, items, crlf)| format!("{inner}{}", join(items, crlf)))
    }

    /// A runner whose seed is fixed, so a failure on one machine fails on every machine and in CI.
    fn runner() -> TestRunner {
        let config = Config { cases: 400, failure_persistence: None, max_shrink_iters: 4096, ..Config::default() };
        TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha))
    }

    fn check_generated(path: &str, source: &str) -> Result<(), TestCaseError> {
        let found = check_file(Utf8Path::new(path), source.as_bytes()).1;
        prop_assert!(found.is_empty(), "{path}: {}", render(&found));
        Ok(())
    }

    #[test]
    fn generated_typescript_holds_every_invariant_as_typescript_and_as_javascript() {
        let result = runner().run(&ts_source(), |source| {
            check_generated("a.ts", &source)?;
            // TypeScript syntax in a `.js` file is a parse error the parser recovers from.
            check_generated("a.js", &source)
        });
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn generated_rust_holds_every_invariant() {
        let result = runner().run(&rs_source(), |source| check_generated("a.rs", &source));
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn the_generators_make_nested_blocks_in_both_languages() {
        // A fixed-seed draw of sources, counting what the properties above would check.
        let mut runner = runner();
        let (mut ts_nested, mut rs_nested, mut rs_test, mut multibyte) = (0, 0, 0, 0);
        for _ in 0..60 {
            let ts = ts_source().new_tree(&mut runner).unwrap().current();
            let rs = rs_source().new_tree(&mut runner).unwrap().current();
            multibyte += usize::from(!ts.is_ascii()) + usize::from(!rs.is_ascii());
            let analysis = analyze(Utf8Path::new("a.ts"), &ts);
            ts_nested += analysis.blocks.iter().filter(|block| block.parent_index.is_some()).count();
            let analysis = analyze(Utf8Path::new("a.rs"), &rs);
            rs_nested += analysis.blocks.iter().filter(|block| block.parent_index.is_some()).count();
            rs_test += analysis.test_blocks.len();
        }
        assert!(
            ts_nested > 20 && rs_nested > 50 && rs_test > 20 && multibyte > 20,
            "{ts_nested} {rs_nested} {rs_test} {multibyte}"
        );
    }
}
