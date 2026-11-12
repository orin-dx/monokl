//! The core of `monokl search`: a query in, fully ranked and deduplicated blocks out.
//!
//! Nothing here caps, budgets, serializes or prints. Callers get every surviving block with the counts needed to
//! describe what was left out, and apply their own budget.

use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::analysis::LanguageId;
use crate::analysis::registry::AnalyzerRegistry;
use crate::dedup::dedup_blocks;
use crate::error::{MonoklError, Result};
use crate::file_io::FileIoError;
use crate::query::{Modifier, ParsedTerm, QueryPlan, parse};
use crate::rank::{rank_blocks, tokenize_block};
use crate::text_search::{
    DEFAULT_MAX_FILES, Incomplete, MAX_FILE_BYTES, MAX_LINE_BYTES, TextSearchOptions, TextSearchResult,
    search_files_where,
};
use crate::tokens::tokenize;
use crate::types::{CodeBlock, Diagnostic, DiagnosticKind, Language, LineHit, RankedBlock};

/// What to search for and where.
#[derive(Clone, Copy, Debug)]
pub struct SearchRequest<'a> {
    /// The directory to search; it is canonicalized first.
    pub root: &'a Utf8Path,
    /// The query text, in the syntax `parse` reads, or a literal when `exact` is set.
    pub query: &'a str,
    /// Treats the whole query as one literal term, operators and quotes included.
    pub exact: bool,
    /// Searches test files too, and test code inside source files: Rust items under `#[cfg(test)]` or `#[test]`.
    pub allow_tests: bool,
    /// Skips hidden files and what `.gitignore` and `.ignore` files exclude: inside the root and, within a git
    /// repository, in its parent directories up to the repository root.
    pub respect_ignore_files: bool,
    /// Restricts the search to one language; only TypeScript, JavaScript and Rust are supported.
    pub language: Option<Language>,
}

/// Everything a search found, best first and uncapped.
#[derive(Clone, Debug)]
pub struct SearchResult {
    /// Deduplicated blocks with 1-based, contiguous ranks.
    pub ranked: Vec<RankedBlock>,
    /// Files an analyzer owns with at least one line matching a positive term, kept or not.
    pub files_total: u64,
    /// Whether `files_total` exceeded the file ceiling, so some matching files were never examined.
    pub ceiling_hit: bool,
    /// What was skipped or only partly understood, in a deterministic order.
    pub diagnostics: Vec<Diagnostic>,
}

impl SearchResult {
    fn empty() -> Self {
        Self { ranked: Vec::new(), files_total: 0, ceiling_hit: false, diagnostics: Vec::new() }
    }

    fn skipped(message: impl Into<String>) -> Self {
        Self { diagnostics: vec![skipped(None, message)], ..Self::empty() }
    }
}

/// Runs a search over `request.root` and returns ranked blocks.
///
/// Text search finds the lines matching the query's positive terms; files with a line matching an excluded term, or
/// lacking a match for a required term, drop out. Each remaining file goes to the analyzer that owns it, and only
/// blocks containing a matching line are kept, ranked with BM25 and deduplicated. A matching line belongs to the
/// smallest block containing it, so a class or object literal keeps only the matches outside its members and is
/// dropped when it has none. Files no analyzer supports, or that `language` excludes, are never opened, counted or reported.
///
/// BM25 query terms are the lowercase [`tokenize`] output of each non-excluded literal term. Regex terms select
/// lines but add no BM25 terms. A query with only excluded terms has nothing to
/// search for and returns an empty result with one `Skipped` diagnostic.
///
/// # Errors
///
/// - [`MonoklError::TooManyTerms`] if the query has too many terms.
/// - [`MonoklError::RegexBuild`] if any term is not a valid regex; no file is read first.
/// - [`MonoklError::Io`] or [`MonoklError::NonUtf8Path`] if `request.root` cannot be canonicalized.
/// - [`MonoklError::Walk`] if the root is not a listable directory.
///
/// # Examples
///
/// ```
/// use camino::Utf8Path;
/// use monokl::analysis::registry::AnalyzerRegistry;
/// use monokl::pipeline::{SearchRequest, search};
///
/// let dir = tempfile::tempdir()?;
/// std::fs::write(dir.path().join("a.ts"), "const alpha = 1;\n")?;
/// let root = Utf8Path::from_path(dir.path()).ok_or("temp dir is not UTF-8")?;
/// let request = SearchRequest {
///     root,
///     query: "alpha",
///     exact: false,
///     allow_tests: false,
///     respect_ignore_files: true,
///     language: None,
/// };
///
/// // With no analyzer registered, no file is owned, so none is searched.
/// let result = search(&AnalyzerRegistry::new(Vec::new()), &request)?;
/// assert_eq!(result.files_total, 0);
/// assert!(result.ranked.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn search(registry: &AnalyzerRegistry, request: &SearchRequest<'_>) -> Result<SearchResult> {
    search_with_timeout(registry, request, None)
}

/// [`search`] that stops after `timeout` and returns what it has, or runs to completion when `timeout` is `None`.
///
/// The clock starts when the call does. It is checked between files while text search runs and between batches of files
/// while the matching files are analyzed. Both stages always finish their first batch, so even a zero timeout returns
/// the best blocks of the first files reached. A search that stopped early carries one `Warning` diagnostic with no path
/// saying how many of how many files were reached, so a partial result is never mistaken for a complete one. Partial
/// results are the only output that can differ between runs: which files are reached depends on speed. A `-term`
/// exclusion applies only to files text search reached.
///
/// Bounds: text search reads at most [`MAX_FILE_BYTES`] per file, and analysis reads no file larger than that either;
/// an oversize file is not read and is reported as one grouped `Skipped` diagnostic. The walk that lists the files is
/// not interrupted, and a batch in progress finishes.
///
/// # Errors
///
/// Those of [`search`].
pub fn search_with_timeout(
    registry: &AnalyzerRegistry,
    request: &SearchRequest<'_>,
    timeout: Option<Duration>,
) -> Result<SearchResult> {
    search_with_ceiling(registry, request, DEFAULT_MAX_FILES, timeout)
}

/// A block annotated with its matches, and its tokens.
type Candidate = (CodeBlock, Vec<String>);

/// How many files each thread analyzes per batch. The deadline is checked between batches, and a batch ends in a wait
/// for its slowest file.
const ANALYSIS_FILES_PER_THREAD: usize = 8;

/// [`search_with_timeout`] with the file ceiling as a parameter, so tests can reach it without 100,000 files.
fn search_with_ceiling(
    registry: &AnalyzerRegistry,
    request: &SearchRequest<'_>,
    max_files: usize,
    timeout: Option<Duration>,
) -> Result<SearchResult> {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let SearchRequest { root, query, exact, allow_tests, respect_ignore_files, language } = *request;
    if let Some(language) = language {
        match analyzer_language(language) {
            None => {
                return Ok(SearchResult::skipped(format!(
                    "language {language:?} not supported in v1; only TypeScript, JavaScript and Rust"
                )));
            }
            Some(id) if !registry.handles(id) => {
                return Ok(SearchResult::skipped(format!("language {language:?} has no analyzer in this build")));
            }
            Some(_) => {}
        }
    }

    let terms = if exact {
        if query.trim().is_empty() {
            Vec::new()
        } else {
            vec![ParsedTerm { modifier: Modifier::Optional, pattern: regex::escape(query), is_regex: false }]
        }
    } else {
        parse(query)?
    };
    let plan = QueryPlan::from_terms(terms);
    let patterns = plan.search_patterns();
    if patterns.is_empty() {
        return Ok(SearchResult::skipped("query has no positive terms; add a term without a leading `-`"));
    }

    // Every term, excluded ones too, compiles before any file is read, so an invalid one fails the whole query.
    let matchers = plan.terms.iter().map(|term| compile(&term.pattern)).collect::<Result<Vec<_>>>()?;
    let root = canonicalize(root)?;

    // A file no analyzer would read, or that `language` excludes, is never opened: it could not yield a block, and
    // counting it would let it use up the file ceiling.
    let wanted = |path: &Utf8Path| searchable(registry, language, path);
    let found = search_files_where(
        &root,
        &patterns,
        &TextSearchOptions { allow_tests, respect_ignore_files, max_files, deadline, ..TextSearchOptions::default() },
        &wanted,
    )?;
    let mut diagnostics = text_search_diagnostics(&found);
    let mut partial = found.incomplete.map(|reached| incomplete_diagnostic(timeout, "searching", reached, ""));

    // The ceiling must not apply here: an excluded file past it would wrongly survive.
    let excluded_files = if found.hits.is_empty() || plan.excluded.is_empty() {
        BTreeSet::new()
    } else {
        let excluded_patterns: Vec<&str> = plan.excluded.iter().map(|&i| plan.terms[i].pattern.as_str()).collect();
        let excluded = search_files_where(
            &root,
            &excluded_patterns,
            &TextSearchOptions {
                allow_tests,
                respect_ignore_files,
                max_files: usize::MAX,
                deadline,
                ..TextSearchOptions::default()
            },
            &wanted,
        )?;
        // The walk is the same, so only a file unreadable or oversize for the excluded patterns alone is new.
        for path in &excluded.unreadable {
            if !found.unreadable.contains(path) {
                diagnostics.push(unreadable_diagnostic(path));
            }
        }
        for path in &excluded.oversize {
            if !found.oversize.contains(path) {
                diagnostics.push(oversize_diagnostic(path, MAX_FILE_BYTES));
            }
        }
        if partial.is_none() {
            partial = excluded
                .incomplete
                .map(|reached| incomplete_diagnostic(timeout, "searching", reached, " for excluded terms"));
        }
        excluded.hits.into_keys().collect()
    };

    let survivors: Vec<(&Utf8PathBuf, &Vec<LineHit>)> = found
        .hits
        .iter()
        .filter(|(path, _)| !excluded_files.contains(*path))
        .filter(|(_, hits)| plan.required.iter().all(|&i| hits.iter().any(|hit| matches_text(&matchers[i], &hit.text))))
        .collect();
    // Files are analyzed on separate threads a batch at a time, but each result is kept in path order, so the output
    // never depends on which thread finished first. The deadline is checked between batches, never before the first.
    let ctx = FileContext {
        registry,
        plan: &plan,
        matchers: &matchers,
        language,
        allow_tests,
        max_file_bytes: MAX_FILE_BYTES,
    };
    let batch_size = rayon::current_num_threads() * ANALYSIS_FILES_PER_THREAD;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut analyzed = 0_usize;
    for batch in survivors.chunks(batch_size.max(1)) {
        if analyzed > 0 && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let reached = Incomplete { reached: analyzed as u64, total: survivors.len() as u64 };
            partial = partial.or_else(|| Some(incomplete_diagnostic(timeout, "analyzing", reached, "")));
            break;
        }
        let per_file: Vec<(Vec<Candidate>, Vec<Diagnostic>)> = batch
            .par_iter()
            .map(|&(path, hits)| {
                let (mut blocks, mut file_diagnostics) = (Vec::new(), Vec::new());
                retrieve_blocks(&ctx, path, hits, &read_source, &mut blocks, &mut file_diagnostics);
                (blocks, file_diagnostics)
            })
            .collect();
        for (blocks, file_diagnostics) in per_file {
            candidates.extend(blocks);
            diagnostics.extend(file_diagnostics);
        }
        analyzed += batch.len();
    }
    diagnostics.extend(partial);

    let bm25_terms = bm25_terms(&plan);
    let df = document_frequencies(&candidates, &bm25_terms);
    let bm25_terms: Vec<&str> = bm25_terms.iter().map(String::as_str).collect();
    let ranked = dedup_blocks(rank_blocks(candidates, &bm25_terms, &df));

    Ok(SearchResult { ranked, files_total: found.files_total, ceiling_hit: found.ceiling_hit, diagnostics })
}

/// The analyzer language that serves `language` in v1, or `None` when no analyzer path exists for it.
fn analyzer_language(language: Language) -> Option<LanguageId> {
    match language {
        Language::TypeScript | Language::JavaScript => Some(LanguageId::TypeScript),
        Language::Rust => Some(LanguageId::Rust),
        Language::Python => None,
    }
}

/// Compiles one term the way text search does, case-insensitively, so a term can never match differently here.
fn compile(pattern: &str) -> Result<RegexMatcher> {
    Ok(RegexMatcherBuilder::new().case_insensitive(true).build(pattern)?)
}

fn canonicalize(root: &Utf8Path) -> Result<Utf8PathBuf> {
    let path = root.as_std_path().canonicalize().map_err(|source| FileIoError::read(root, source))?;
    Utf8PathBuf::from_path_buf(path).map_err(|path| MonoklError::NonUtf8Path { path })
}

/// A file's text, and whether getting it meant replacing bytes that are not UTF-8.
struct Source {
    text: String,
    lossy: bool,
}

/// Reads `path`, replacing each invalid UTF-8 sequence with U+FFFD so the file is still searched and extracted.
fn read_source(path: &Utf8Path) -> io::Result<Source> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => Source { text, lossy: false },
        Err(err) => Source { text: String::from_utf8_lossy(err.as_bytes()).into_owned(), lossy: true },
    })
}

/// Whether `matcher` matches `text`. Matching in memory cannot fail, so an error counts as no match.
fn matches_text(matcher: &RegexMatcher, text: &str) -> bool {
    matches!(matcher.is_match(text.as_bytes()), Ok(true))
}

fn skipped(path: Option<&Utf8Path>, message: impl Into<String>) -> Diagnostic {
    Diagnostic { kind: DiagnosticKind::Skipped, path: path.map(Utf8Path::to_owned), message: message.into() }
}

fn unreadable_diagnostic(path: &Utf8Path) -> Diagnostic {
    skipped(Some(path), "file could not be read")
}

/// `4 MiB` for a whole number of mebibytes, bytes otherwise.
fn size_label(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes.is_multiple_of(MIB) { format!("{} MiB", bytes / MIB) } else { format!("{bytes} bytes") }
}

/// One per oversize file; the output layer groups them into one line naming the limit.
fn oversize_diagnostic(path: &Utf8Path, max_file_bytes: u64) -> Diagnostic {
    skipped(Some(path), format!("over {}", size_label(max_file_bytes)))
}

/// The one diagnostic of a search that hit its deadline: `doing` is what the stage was doing to its files, and `what`
/// qualifies it.
fn incomplete_diagnostic(timeout: Option<Duration>, doing: &str, reached: Incomplete, what: &str) -> Diagnostic {
    let limit = timeout.map_or_else(String::new, |timeout| format!("{} s ", timeout.as_secs_f64()));
    Diagnostic {
        kind: DiagnosticKind::Warning,
        path: None,
        message: format!(
            "partial result: {limit}timeout expired after {doing} {} of {} files{what}",
            reached.reached, reached.total
        ),
    }
}

/// Turns what text search could not read into diagnostics: one per unreadable path, then one per non-zero count.
fn text_search_diagnostics(found: &TextSearchResult) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = found.unreadable.iter().map(|path| unreadable_diagnostic(path)).collect();
    diagnostics.extend(found.oversize.iter().map(|path| oversize_diagnostic(path, MAX_FILE_BYTES)));
    if found.truncated_lines > 0 {
        let n = found.truncated_lines;
        let (lines, were) = if n == 1 { ("line", "was") } else { ("lines", "were") };
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Warning,
            path: None,
            message: format!(
                "{n} matching {lines} {were} cut to {MAX_LINE_BYTES} bytes; a term that matches only past the cut is not seen"
            ),
        });
    }
    if found.non_utf8_paths > 0 {
        diagnostics.push(skipped(None, format!("{} files skipped: path is not valid UTF-8", found.non_utf8_paths)));
    }
    if found.walk_errors_without_path > 0 {
        diagnostics.push(skipped(
            None,
            format!("{} directory walk errors did not name a UTF-8 path", found.walk_errors_without_path),
        ));
    }
    diagnostics
}

/// What block retrieval needs besides the file itself.
struct FileContext<'a> {
    registry: &'a AnalyzerRegistry,
    plan: &'a QueryPlan,
    matchers: &'a [RegexMatcher],
    language: Option<Language>,
    allow_tests: bool,
    /// The largest source file that is read.
    max_file_bytes: u64,
}

/// One analyzer can own several languages (TypeScript's owns JavaScript too), so the `--language` filter goes by
/// extension.
fn has_language_extension(language: Language, path: &Utf8Path) -> bool {
    match language {
        Language::TypeScript => matches!(path.extension(), Some("ts" | "tsx" | "mts" | "cts")),
        Language::JavaScript => matches!(path.extension(), Some("js" | "jsx" | "mjs" | "cjs")),
        Language::Rust => path.extension() == Some("rs"),
        Language::Python => false,
    }
}

/// Whether a file can yield blocks: an analyzer owns it and, when `language` is set, its extension belongs to it. Text
/// search asks this before opening a file; [`retrieve_blocks`] keeps the same two checks for a file it is handed.
fn searchable(registry: &AnalyzerRegistry, language: Option<Language>, path: &Utf8Path) -> bool {
    registry.analyzer_for(path).is_some() && language.is_none_or(|language| has_language_extension(language, path))
}

/// Analyzes one surviving file and pushes the annotated blocks that contain a matching line onto `out`.
fn retrieve_blocks(
    ctx: &FileContext<'_>,
    path: &Utf8Path,
    hits: &[LineHit],
    read: &dyn Fn(&Utf8Path) -> io::Result<Source>,
    out: &mut Vec<(CodeBlock, Vec<String>)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(analyzer) = ctx.registry.analyzer_for(path) else { return };
    if ctx.language.is_some_and(|language| !has_language_extension(language, path)) {
        return;
    }
    // Text search checked the size too, but the file can have grown since, and a caller can hand this any path.
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > ctx.max_file_bytes) {
        diagnostics.push(oversize_diagnostic(path, ctx.max_file_bytes));
        return;
    }
    let Source { text, lossy } = match read(path) {
        Ok(source) => source,
        Err(err) => {
            diagnostics.push(skipped(Some(path), format!("could not read source: {err}")));
            return;
        }
    };
    if lossy {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::LossyRead,
            path: Some(path.to_owned()),
            message: "invalid UTF-8 replaced with U+FFFD".to_owned(),
        });
    }
    let analysis = match analyzer.analyze(path, text) {
        Ok(analysis) => analysis,
        Err(err) => {
            diagnostics.push(skipped(Some(path), format!("could not analyze: {err}")));
            return;
        }
    };
    if analysis.had_parse_errors {
        let message = analysis
            .degradation
            .map_or_else(|| "parse errors; blocks may be incomplete".to_owned(), |reason| reason.to_string());
        diagnostics.push(Diagnostic { kind: DiagnosticKind::Degraded, path: Some(path.to_owned()), message });
    }
    let positive: Vec<&RegexMatcher> = ctx
        .plan
        .terms
        .iter()
        .zip(ctx.matchers)
        .filter(|(term, _)| term.modifier != Modifier::Excluded)
        .map(|(_, matcher)| matcher)
        .collect();
    let line_matches = |text: &str| positive.iter().any(|matcher| matches_text(matcher, text));
    // Test blocks always take part in deciding which block owns a hit, so a test method inside a regular `impl` or trait
    // does not leak its matches to the holder; they are returned only when asked for. Only references are collected, so
    // no block is copied.
    let blocks: Vec<&CodeBlock> = analysis.blocks.iter().chain(&analysis.test_blocks).collect();
    let returned = if ctx.allow_tests { blocks.len() } else { analysis.blocks.len() };
    for (block, inside) in blocks.iter().zip(owned_hits(&blocks, hits, &line_matches)).take(returned) {
        if inside.is_empty() {
            continue;
        }
        let annotated =
            CodeBlock { matched_lines: inside.iter().map(|hit| hit.line_number).collect(), ..(*block).clone() };
        let tokens = tokenize_block(&annotated);
        out.push((annotated, tokens));
    }
}

/// Splits `hits` among `blocks`: a hit goes to every block whose own text on that line matches, except one that holds
/// a smaller such block. A class or object therefore keeps only the matches outside its members, and a block with
/// none is dropped by the caller. A block can start or end in the middle of a line (`} b() {`, `}, // note`), so only
/// the part of the line inside the block counts, which sends the match to the member or holder that actually contains
/// the text. Where no block's own text matches (a match across the edge of every block on the line) every block on the
/// line keeps the hit, so a match is never lost. Blocks that only share a line are not nested, so each keeps it: blocks
/// with the same line range (`mod m { #[cfg(test)] fn f() {} }` on one line) are nested only when the larger one's text
/// contains the other's.
/// `hits` must be in line order; each block's hits are too.
fn owned_hits<'h>(
    blocks: &[&CodeBlock],
    hits: &'h [LineHit],
    line_matches: &dyn Fn(&str) -> bool,
) -> Vec<Vec<&'h LineHit>> {
    let mut containing: Vec<Vec<usize>> = vec![Vec::new(); hits.len()];
    for (index, block) in blocks.iter().enumerate() {
        let first = hits.partition_point(|hit| hit.line_number < block.line_start);
        let inside = hits[first..].iter().take_while(|hit| hit.line_number <= block.line_end).count();
        for slot in &mut containing[first..first + inside] {
            slot.push(index);
        }
    }
    let own_text_matches = |index: usize, line: usize| {
        let block = blocks[index];
        block.code.split('\n').nth(line - block.line_start).is_some_and(line_matches)
    };
    let holds = |outer: usize, inner: usize| {
        let (outer, inner) = (blocks[outer], blocks[inner]);
        let within = inner.line_start >= outer.line_start && inner.line_end <= outer.line_end;
        let same_lines = (inner.line_start, inner.line_end) == (outer.line_start, outer.line_end);
        within && (!same_lines || (inner.code.len() < outer.code.len() && outer.code.contains(&inner.code)))
    };
    let mut per_block = vec![Vec::new(); blocks.len()];
    for (hit, containing_all) in hits.iter().zip(&containing) {
        let matching: Vec<usize> =
            containing_all.iter().copied().filter(|&index| own_text_matches(index, hit.line_number)).collect();
        let candidates = if matching.is_empty() { containing_all } else { &matching };
        for &outer in candidates {
            if !candidates.iter().any(|&inner| holds(outer, inner)) {
                per_block[outer].push(hit);
            }
        }
    }
    per_block
}

/// Returns the text the user typed for `term`: a regex as written, a literal with its escaping undone.
fn keyword(term: &ParsedTerm) -> String {
    if term.is_regex {
        return term.pattern.clone();
    }
    // `regex::escape` only ever puts a backslash before the character it escapes.
    let mut text = String::with_capacity(term.pattern.len());
    let mut chars = term.pattern.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            text.extend(chars.next());
        } else {
            text.push(ch);
        }
    }
    text
}

/// Returns the BM25 query terms: the tokens of each non-excluded literal term, each once, in query order.
fn bm25_terms(plan: &QueryPlan) -> Vec<String> {
    let mut seen = FxHashSet::default();
    plan.terms
        .iter()
        .filter(|term| term.modifier != Modifier::Excluded && !term.is_regex)
        .flat_map(|term| tokenize(&keyword(term)))
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

/// Counts, for each query term, the blocks whose tokens contain it at least once.
fn document_frequencies(blocks: &[(CodeBlock, Vec<String>)], terms: &[String]) -> FxHashMap<String, usize> {
    let mut df = FxHashMap::default();
    for (_, tokens) in blocks {
        let present: FxHashSet<&str> = tokens.iter().map(String::as_str).collect();
        for term in terms.iter().filter(|term| present.contains(term.as_str())) {
            *df.entry(term.clone()).or_insert(0) += 1;
        }
    }
    df
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rstest::rstest;

    use super::*;
    use crate::analysis::LanguageId;
    use crate::analysis::file_analysis::{Degradation, FileAnalysis};
    use crate::analysis::lang::LanguageAnalyzer;
    use crate::types::{ParentContext, SymbolKind};

    type MakeBlocks = dyn Fn(&Utf8Path, &str) -> Result<(Vec<CodeBlock>, bool)> + Send + Sync;

    /// A stand-in analyzer for `.ts` and `.js` files whose blocks come from a closure instead of a parser.
    struct Canned {
        make: Box<MakeBlocks>,
        degradation: Option<Degradation>,
    }

    impl LanguageAnalyzer for Canned {
        fn languages(&self) -> &[LanguageId] {
            &[LanguageId::TypeScript]
        }
        fn supports(&self, path: &Utf8Path) -> bool {
            matches!(path.extension(), Some("ts" | "js"))
        }
        fn analyze(&self, path: &Utf8Path, source: String) -> Result<Arc<FileAnalysis>> {
            let (blocks, had_parse_errors) = (self.make)(path, &source)?;
            Ok(Arc::new(FileAnalysis {
                had_parse_errors,
                degradation: self.degradation,
                blocks,
                test_blocks: Vec::new(),
            }))
        }
    }

    fn registry(
        make: impl Fn(&Utf8Path, &str) -> Result<(Vec<CodeBlock>, bool)> + Send + Sync + 'static,
    ) -> AnalyzerRegistry {
        AnalyzerRegistry::new(vec![Arc::new(Canned { make: Box::new(make), degradation: None })])
    }

    /// Lines `start..=end` (1-based) of `source` as a function block.
    fn block(path: &Utf8Path, source: &str, start: usize, end: usize) -> CodeBlock {
        let code: Vec<&str> = source.lines().skip(start - 1).take(end - start + 1).collect();
        CodeBlock {
            file: path.to_owned(),
            line_start: start,
            line_end: end,
            node_kind: SymbolKind::Function,
            code: code.join("\n"),
            symbol_signature: None,
            matched_lines: Vec::new(),
            parent: None,
        }
    }

    /// One block per file covering all of it.
    fn whole_file_registry() -> AnalyzerRegistry {
        registry(|path, source| Ok((vec![block(path, source, 1, source.lines().count().max(1))], false)))
    }

    fn tree(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        (dir, root)
    }

    fn request<'a>(root: &'a Utf8Path, query: &'a str) -> SearchRequest<'a> {
        SearchRequest { root, query, exact: false, allow_tests: false, respect_ignore_files: true, language: None }
    }

    /// The ranked files relative to `root`, in rank order.
    fn files(result: &SearchResult, root: &Utf8Path) -> Vec<String> {
        result.ranked.iter().map(|ranked| ranked.block.file.strip_prefix(root).unwrap().to_string()).collect()
    }

    fn summary(diagnostics: &[Diagnostic]) -> Vec<(DiagnosticKind, Option<&str>, &str)> {
        diagnostics.iter().map(|d| (d.kind, d.path.as_ref().map(|p| p.as_str()), d.message.as_str())).collect()
    }

    #[test]
    fn language_without_an_analyzer_path_is_skipped_with_a_diagnostic() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let request = SearchRequest { language: Some(Language::Python), ..request(&root, "alpha") };
        let result = search(&whole_file_registry(), &request).unwrap();
        assert!(result.ranked.is_empty());
        assert_eq!(result.files_total, 0);
        assert_eq!(
            summary(&result.diagnostics),
            [(
                DiagnosticKind::Skipped,
                None,
                "language Python not supported in v1; only TypeScript, JavaScript and Rust"
            )]
        );
    }

    #[test]
    fn language_with_no_registered_analyzer_is_skipped_with_a_diagnostic() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.rs", "alpha\n")]);
        let request = SearchRequest { language: Some(Language::Rust), ..request(&root, "alpha") };
        let result = search(&whole_file_registry(), &request).unwrap();
        assert!(result.ranked.is_empty());
        assert_eq!(result.files_total, 0);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Skipped, None, "language Rust has no analyzer in this build")]
        );
    }

    #[test]
    fn typescript_and_javascript_languages_select_files_by_extension() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.js", "alpha\n")]);
        for (language, expected) in [(Language::TypeScript, ["a.ts"]), (Language::JavaScript, ["b.js"])] {
            let request = SearchRequest { language: Some(language), ..request(&root, "alpha") };
            let result = search(&whole_file_registry(), &request).unwrap();
            assert_eq!(files(&result, &root), expected);
            assert_eq!(result.diagnostics.len(), 0);
        }
    }

    #[test]
    fn language_filter_drops_files_owned_by_another_language() {
        struct Rusty;
        impl LanguageAnalyzer for Rusty {
            fn languages(&self) -> &[LanguageId] {
                &[LanguageId::Rust]
            }
            fn supports(&self, path: &Utf8Path) -> bool {
                path.extension() == Some("rs")
            }
            fn analyze(&self, path: &Utf8Path, source: String) -> Result<Arc<FileAnalysis>> {
                Ok(Arc::new(FileAnalysis {
                    had_parse_errors: false,
                    degradation: None,
                    blocks: vec![block(path, &source, 1, 1)],
                    test_blocks: Vec::new(),
                }))
            }
        }
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.rs", "alpha\n")]);
        let mixed = AnalyzerRegistry::new(vec![
            Arc::new(Canned {
                make: Box::new(|path, source| Ok((vec![block(path, source, 1, 1)], false))),
                degradation: None,
            }),
            Arc::new(Rusty),
        ]);
        let everything = search(&mixed, &request(&root, "alpha")).unwrap();
        assert_eq!(files(&everything, &root), ["a.ts", "b.rs"]);
        let ts_only = SearchRequest { language: Some(Language::TypeScript), ..request(&root, "alpha") };
        assert_eq!(files(&search(&mixed, &ts_only).unwrap(), &root), ["a.ts"]);
    }

    #[test]
    fn empty_and_operator_only_queries_return_one_skipped_diagnostic() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let expected =
            [(DiagnosticKind::Skipped, None, "query has no positive terms; add a term without a leading `-`")];
        for query in ["", "   ", "+", "-", "regex:"] {
            let result = search(&whole_file_registry(), &request(&root, query)).unwrap();
            assert!(result.ranked.is_empty(), "{query:?}");
            assert_eq!((result.files_total, result.ceiling_hit), (0, false), "{query:?}");
            assert_eq!(summary(&result.diagnostics), expected, "{query:?}");
        }
        let exact = SearchRequest { exact: true, ..request(&root, "  ") };
        let result = search(&whole_file_registry(), &exact).unwrap();
        assert!(result.ranked.is_empty());
        assert_eq!(summary(&result.diagnostics), expected);
    }

    #[test]
    fn query_of_only_excluded_terms_returns_one_skipped_diagnostic() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let result = search(&whole_file_registry(), &request(&root, "-alpha -beta")).unwrap();
        assert!(result.ranked.is_empty());
        assert_eq!(result.files_total, 0);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Skipped, None, "query has no positive terms; add a term without a leading `-`")]
        );
    }

    #[test]
    fn optional_terms_need_only_one_match_and_match_case_insensitively() {
        let (_dir, root) = tree(&[("a.ts", "ALPHA\n"), ("b.ts", "Beta\n"), ("c.ts", "gamma\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha beta")).unwrap();
        assert_eq!(result.files_total, 2);
        let mut found = files(&result, &root);
        found.sort();
        assert_eq!(found, ["a.ts", "b.ts"]);
    }

    #[test]
    fn required_terms_must_each_match_some_hit_line() {
        let (_dir, root) = tree(&[("both.ts", "alpha\nbeta\n"), ("one.ts", "alpha alpha\n"), ("other.ts", "beta\n")]);
        let result = search(&whole_file_registry(), &request(&root, "+alpha +beta")).unwrap();
        assert_eq!(files(&result, &root), ["both.ts"]);
        assert_eq!(result.files_total, 3);
    }

    #[test]
    fn required_term_with_an_optional_term_keeps_files_matching_only_the_required_one() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.ts", "alpha beta\n"), ("c.ts", "beta\n")]);
        let result = search(&whole_file_registry(), &request(&root, "+alpha beta")).unwrap();
        let mut found = files(&result, &root);
        found.sort();
        assert_eq!(found, ["a.ts", "b.ts"]);
    }

    #[test]
    fn excluded_term_drops_files_that_match_it() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.ts", "alpha\nGAMMA\n"), ("c.ts", "gamma\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha -gamma")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.files_total, 2);
    }

    #[test]
    fn excluded_files_past_the_ceiling_still_exclude() {
        // With a ceiling of 2 on the excluded pass, `a.ts` and `b.ts` would use it up and `c.ts` would survive.
        let (_dir, root) =
            tree(&[("a.ts", "gamma\n"), ("b.ts", "gamma\n"), ("c.ts", "alpha gamma\n"), ("d.ts", "alpha\n")]);
        let result = search_with_ceiling(&whole_file_registry(), &request(&root, "alpha -gamma"), 2, None).unwrap();
        assert_eq!(files(&result, &root), ["d.ts"]);
        assert!(!result.ceiling_hit);
    }

    #[test]
    fn file_ceiling_and_total_pass_through_from_text_search() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.ts", "alpha\n"), ("c.ts", "alpha\n")]);
        let result = search_with_ceiling(&whole_file_registry(), &request(&root, "alpha"), 2, None).unwrap();
        assert_eq!(files(&result, &root), ["a.ts", "b.ts"]);
        assert_eq!(result.files_total, 3);
        assert!(result.ceiling_hit);
    }

    #[test]
    fn files_nothing_owns_are_not_counted_and_do_not_use_the_ceiling() {
        let (_dir, root) = tree(&[
            ("a.txt", "alpha\n"),
            ("b.json", "alpha\n"),
            ("c.md", "alpha\n"),
            ("d.ts", "alpha\n"),
            ("e.ts", "alpha\n"),
        ]);
        let result = search_with_ceiling(&whole_file_registry(), &request(&root, "alpha"), 2, None).unwrap();
        assert_eq!(files(&result, &root), ["d.ts", "e.ts"]);
        assert_eq!(result.files_total, 2);
        assert!(!result.ceiling_hit);
        assert_eq!(summary(&result.diagnostics), []);
    }

    #[test]
    fn a_language_filter_leaves_other_owned_files_out_of_the_total() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.js", "alpha\n"), ("c.txt", "alpha\n")]);
        let request = SearchRequest { language: Some(Language::JavaScript), ..request(&root, "alpha") };
        let result = search(&whole_file_registry(), &request).unwrap();
        assert_eq!(files(&result, &root), ["b.js"]);
        assert_eq!(result.files_total, 1);
        // The ceiling counts only the files the filter keeps.
        let result = search_with_ceiling(&whole_file_registry(), &request, 1, None).unwrap();
        assert_eq!((result.files_total, result.ceiling_hit), (1, false));
    }

    #[test]
    fn a_file_that_only_matches_an_excluded_term_and_is_not_owned_changes_nothing() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.ts", "alpha gamma\n"), ("c.txt", "alpha gamma\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha -gamma")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.files_total, 2);
        assert_eq!(summary(&result.diagnostics), []);
    }

    #[cfg(all(unix, feature = "lang-ts"))]
    #[test]
    fn files_nothing_owns_are_never_opened() {
        use std::os::unix::fs::PermissionsExt;

        let ts = AnalyzerRegistry::new(vec![Arc::new(crate::analysis::ts::TsAnalyzer::new())]);
        let function = "function alpha() {\n  return 1;\n}\n";
        let (_dir, root) = tree(&[
            ("a.ts", function),
            ("notes.txt", "alpha\n"),
            ("types/a.d.ts", "alpha\n"),
            ("types/b.d.mts", "alpha\n"),
            ("types/c.d.cts", "alpha\n"),
            ("b.js", function),
        ]);
        let closed = ["notes.txt", "types/a.d.ts", "types/b.d.mts", "types/c.d.cts"];
        for name in closed {
            std::fs::set_permissions(root.join(name), std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let readable = std::fs::read(root.join("notes.txt")).is_ok();
        let outcome = search(&ts, &request(&root, "alpha -beta"));
        let javascript_only = SearchRequest { language: Some(Language::JavaScript), ..request(&root, "alpha") };
        std::fs::set_permissions(root.join("a.ts"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let narrowed = search(&ts, &javascript_only);
        for name in closed.into_iter().chain(["a.ts"]) {
            std::fs::set_permissions(root.join(name), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        if readable {
            // Running as root: permissions cannot make the files unreadable.
            return;
        }
        let outcome = outcome.unwrap();
        assert_eq!(files(&outcome, &root), ["a.ts", "b.js"]);
        assert_eq!(outcome.files_total, 2);
        assert_eq!(summary(&outcome.diagnostics), []);
        let narrowed = narrowed.unwrap();
        assert_eq!(files(&narrowed, &root), ["b.js"]);
        assert_eq!(narrowed.files_total, 1);
        assert_eq!(summary(&narrowed.diagnostics), []);
    }

    #[test]
    fn exact_treats_the_whole_query_as_one_escaped_literal() {
        let (_dir, root) =
            tree(&[("a.ts", "see a.b -x here\n"), ("b.ts", "see axb -x here\n"), ("c.ts", "see a.b here\n")]);
        let exact = SearchRequest { exact: true, ..request(&root, "a.b -x") };
        let result = search(&whole_file_registry(), &exact).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
    }

    #[test]
    fn regex_term_selects_lines_and_is_reported_as_written() {
        let (_dir, root) = tree(&[("a.ts", "boooo\n"), ("b.ts", "bravo\n")]);
        let result = search(&whole_file_registry(), &request(&root, "regex:BO+")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
    }

    #[test]
    fn invalid_regex_fails_the_whole_query_before_any_file_is_analyzed() {
        let (_dir, root) = tree(&[("a.ts", "alpha (\n")]);
        let analyzed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&analyzed);
        let registry = registry(move |_, _| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok((Vec::new(), false))
        });
        let err = search(&registry, &request(&root, "alpha regex:(")).unwrap_err();
        assert!(matches!(err, MonoklError::RegexBuild(_)), "{err:?}");
        assert_eq!(analyzed.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn too_many_terms_is_an_error() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let query = (0..65).map(|i| format!("t{i}")).collect::<Vec<_>>().join(" ");
        let err = search(&whole_file_registry(), &request(&root, &query)).unwrap_err();
        assert!(matches!(err, MonoklError::TooManyTerms { count: 65, limit: 64 }), "{err:?}");
    }

    #[test]
    fn missing_root_is_an_io_error() {
        let (_dir, root) = tree(&[]);
        let missing = root.join("nope");
        let err = search(&whole_file_registry(), &request(&missing, "alpha")).unwrap_err();
        assert!(matches!(err, MonoklError::Io(_)), "{err:?}");
    }

    #[test]
    fn only_blocks_containing_a_hit_are_kept_and_annotated() {
        let source = "function one() {\n  alpha\n}\n\nfunction two() {\n  beta alpha\n  beta\n}\n";
        let (_dir, root) = tree(&[("a.ts", source)]);
        let registry = registry(|path, source| {
            Ok((vec![block(path, source, 1, 3), block(path, source, 4, 4), block(path, source, 5, 8)], false))
        });
        let result = search(&registry, &request(&root, "alpha beta")).unwrap();
        let blocks: Vec<_> =
            result.ranked.iter().map(|r| (r.block.line_start, r.block.matched_lines.clone())).collect();
        assert_eq!(blocks, [(5, vec![6, 7]), (1, vec![2])]);
    }

    #[test]
    fn blocks_are_ranked_by_bm25_over_literal_term_tokens() {
        let (_dir, root) = tree(&[
            ("weak.ts", "getUserName(alpha, bravo, charlie)\n"),
            ("strong.ts", "getUserName()\n"),
            ("none.ts", "GETUSERNAME\n"),
        ]);
        let result = search(&whole_file_registry(), &request(&root, "getUserName")).unwrap();
        assert_eq!(files(&result, &root), ["strong.ts", "weak.ts", "none.ts"]);
        let ranks: Vec<usize> = result.ranked.iter().map(|r| r.rank).collect();
        assert_eq!(ranks, [1, 2, 3]);
        assert!(result.ranked[0].final_score > result.ranked[1].final_score);
        assert!(result.ranked[1].final_score > result.ranked[2].final_score);
        assert!(result.ranked[2].final_score.abs() < f64::EPSILON);
    }

    #[test]
    fn regex_terms_add_no_bm25_terms() {
        let (_dir, root) = tree(&[("a.ts", "getUserName()\n")]);
        let result = search(&whole_file_registry(), &request(&root, "regex:getUser")).unwrap();
        assert_eq!(result.ranked.len(), 1);
        assert!(result.ranked[0].final_score.abs() < f64::EPSILON);
    }

    #[test]
    fn bm25_terms_are_unique_lowercase_tokens_of_non_excluded_literals() {
        let plan = QueryPlan::from_terms(parse("getUserName +userId -secretKey regex:Foo+ user").unwrap());
        assert_eq!(bm25_terms(&plan), ["user", "name", "id"]);
    }

    #[test]
    fn document_frequency_counts_blocks_not_occurrences() {
        let blocks = ["user user name", "user", "other"].map(|code| {
            let block = CodeBlock {
                file: Utf8PathBuf::from("a.ts"),
                line_start: 1,
                line_end: 1,
                node_kind: SymbolKind::Function,
                code: code.to_owned(),
                symbol_signature: None,
                matched_lines: Vec::new(),
                parent: None,
            };
            let tokens = tokenize_block(&block);
            (block, tokens)
        });
        let df = document_frequencies(&blocks, &["user".to_owned(), "name".to_owned(), "absent".to_owned()]);
        let mut df: Vec<_> = df.into_iter().collect();
        df.sort();
        assert_eq!(df, [("name".to_owned(), 1), ("user".to_owned(), 2)]);
    }

    #[test]
    fn overlapping_blocks_in_one_file_are_deduplicated_keeping_the_better_one() {
        let source = "alpha\nbeta\ngamma\n";
        let (_dir, root) = tree(&[("a.ts", source)]);
        let registry = registry(|path, source| Ok((vec![block(path, source, 1, 3), block(path, source, 1, 2)], false)));
        let result = search(&registry, &request(&root, "alpha")).unwrap();
        assert_eq!(result.ranked.len(), 1);
        assert_eq!(
            (result.ranked[0].block.line_start, result.ranked[0].block.line_end, result.ranked[0].rank),
            (1, 2, 1)
        );
    }

    const CLASS_SOURCE: &str = "\
class Foo {
  label = 'return';
  alpha() {
    return 1;
  }
  beta() {
    return 2;
  }
}
";

    /// The class `Foo` (lines 1-9) with the methods `alpha` (3-5) and `beta` (6-8), as the TypeScript analyzer emits them.
    fn class_registry() -> AnalyzerRegistry {
        registry(|path, source| {
            let parent = ParentContext { kind: SymbolKind::Class, name: "Foo".to_owned() };
            let member = |start, end| CodeBlock {
                node_kind: SymbolKind::Method,
                parent: Some(parent.clone()),
                ..block(path, source, start, end)
            };
            let class = CodeBlock { node_kind: SymbolKind::Class, ..block(path, source, 1, 9) };
            Ok((vec![class, member(3, 5), member(6, 8)], false))
        })
    }

    type NestedRow<'a> = (usize, usize, SymbolKind, Vec<usize>, Option<&'a str>);

    /// Each ranked block as `(line_start, line_end, kind, matched_lines, parent name)`.
    fn nested_rows(result: &SearchResult) -> Vec<NestedRow<'_>> {
        result
            .ranked
            .iter()
            .map(|r| {
                let b = &r.block;
                let parent = b.parent.as_ref().map(|p| p.name.as_str());
                (b.line_start, b.line_end, b.node_kind, b.matched_lines.clone(), parent)
            })
            .collect()
    }

    #[test]
    fn a_hit_inside_a_member_goes_to_the_member_and_not_to_its_class() {
        let (_dir, root) = tree(&[("a.ts", CLASS_SOURCE)]);
        let result = search(&class_registry(), &request(&root, "beta")).unwrap();
        assert_eq!(nested_rows(&result), [(6, 8, SymbolKind::Method, vec![6], Some("Foo"))]);
    }

    #[test]
    fn a_hit_on_the_class_header_keeps_the_class_and_drops_members_without_hits() {
        let (_dir, root) = tree(&[("a.ts", CLASS_SOURCE)]);
        let result = search(&class_registry(), &request(&root, "foo")).unwrap();
        assert_eq!(nested_rows(&result), [(1, 9, SymbolKind::Class, vec![1], None)]);
    }

    fn owned_counts(blocks: &[CodeBlock]) -> Vec<usize> {
        let refs: Vec<&CodeBlock> = blocks.iter().collect();
        let hits = [LineHit { line_number: 1, text: String::new() }];
        owned_hits(&refs, &hits, &|text| text.contains("needle")).iter().map(Vec::len).collect()
    }

    #[test]
    fn a_block_on_the_same_lines_as_the_block_that_contains_it_takes_the_hit() {
        let path = Utf8Path::new("a.rs");
        let line = "mod m { #[cfg(test)] fn hidden() { needle(); } }";
        let blocks = [block(path, line, 1, 1), block(path, "#[cfg(test)] fn hidden() { needle(); }", 1, 1)];
        assert_eq!(owned_counts(&blocks), [0, 1]);
        let reversed = [blocks[1].clone(), blocks[0].clone()];
        assert_eq!(owned_counts(&reversed), [1, 0]);
    }

    #[test]
    fn blocks_on_the_same_lines_that_do_not_contain_each_other_each_keep_the_hit() {
        let path = Utf8Path::new("a.rs");
        let blocks = [block(path, "fn a() { needle(); }", 1, 1), block(path, "fn b() { needle(); }", 1, 1)];
        assert_eq!(owned_counts(&blocks), [1, 1]);
    }

    #[test]
    fn hits_split_between_a_class_and_its_members_without_sharing_a_line() {
        let (_dir, root) = tree(&[("a.ts", CLASS_SOURCE)]);
        let result = search(&class_registry(), &request(&root, "return")).unwrap();
        let mut rows = nested_rows(&result);
        rows.sort_by_key(|row| row.0);
        assert_eq!(
            rows,
            [
                (1, 9, SymbolKind::Class, vec![2], None),
                (3, 5, SymbolKind::Method, vec![4], Some("Foo")),
                (6, 8, SymbolKind::Method, vec![7], Some("Foo")),
            ]
        );
        let ranks: Vec<usize> = result.ranked.iter().map(|r| r.rank).collect();
        assert_eq!(ranks, [1, 2, 3]);
    }

    #[test]
    fn a_member_ranked_above_its_class_and_one_ranked_below_both_survive_dedup() {
        let (_dir, root) = tree(&[("a.ts", CLASS_SOURCE)]);
        for query in ["return", "beta return", "alpha return"] {
            let result = search(&class_registry(), &request(&root, query)).unwrap();
            let mut starts: Vec<usize> = result.ranked.iter().map(|r| r.block.line_start).collect();
            starts.sort_unstable();
            assert_eq!(starts, [1, 3, 6], "{query}");
        }
    }

    #[test]
    fn member_rows_carry_their_parent_into_the_ranked_block() {
        let (_dir, root) = tree(&[("a.ts", CLASS_SOURCE)]);
        let result = search(&class_registry(), &request(&root, "alpha")).unwrap();
        let parent = result.ranked[0].block.parent.as_ref().map(|p| (p.kind, p.name.as_str()));
        assert_eq!(parent, Some((SymbolKind::Class, "Foo")));
    }

    #[test]
    fn unsupported_files_yield_nothing_and_no_diagnostic() {
        let (_dir, root) = tree(&[("notes.md", "alpha\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert!(result.ranked.is_empty());
        assert_eq!(result.files_total, 0);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn analyzer_error_skips_the_file_with_a_diagnostic() {
        let (_dir, root) = tree(&[("bad.ts", "alpha\n"), ("good.ts", "alpha\n")]);
        let registry = registry(|path, source| {
            if path.file_name() == Some("bad.ts") {
                return Err(MonoklError::TooManyTerms { count: 9, limit: 8 });
            }
            Ok((vec![block(path, source, 1, 1)], false))
        });
        let result = search(&registry, &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["good.ts"]);
        assert_eq!(
            summary(&result.diagnostics),
            [(
                DiagnosticKind::Skipped,
                Some(root.join("bad.ts").as_str()),
                "could not analyze: query has too many terms: 9 > 8"
            )]
        );
    }

    #[test]
    fn parse_errors_add_a_degraded_diagnostic_and_keep_the_blocks() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let registry = registry(|path, source| Ok((vec![block(path, source, 1, 1)], true)));
        let result = search(&registry, &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Degraded, Some(root.join("a.ts").as_str()), "parse errors; blocks may be incomplete")]
        );
    }

    #[rstest]
    #[case(
        Degradation::TooLarge { bytes: 300_000, cap: 260_096 },
        "file too large to parse (300000 bytes, over the cap of 260096)"
    )]
    #[case(Degradation::Panicked, "parser panicked")]
    #[case(Degradation::ThreadUnavailable, "could not start a parse thread")]
    #[case(Degradation::TooDeep { depth: 9_000, cap: 8_000 }, "file nests too deeply to analyze (depth 9000, over the cap of 8000)")]
    fn a_degraded_file_names_its_reason_in_the_diagnostic(#[case] degradation: Degradation, #[case] message: &str) {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let registry = AnalyzerRegistry::new(vec![Arc::new(Canned {
            make: Box::new(|path, source| Ok((vec![block(path, source, 1, 1)], true))),
            degradation: Some(degradation),
        })]);
        let result = search(&registry, &request(&root, "alpha")).unwrap();
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Degraded, Some(root.join("a.ts").as_str()), message)]
        );
    }

    #[test]
    fn source_read_failure_skips_the_file_with_a_diagnostic() {
        let registry = whole_file_registry();
        let plan = QueryPlan::from_terms(parse("alpha").unwrap());
        let matchers = vec![compile("alpha").unwrap()];
        let ctx = FileContext {
            registry: &registry,
            plan: &plan,
            matchers: &matchers,
            language: None,
            allow_tests: false,
            max_file_bytes: MAX_FILE_BYTES,
        };
        let hits = [LineHit { line_number: 1, text: "alpha".to_owned() }];
        let (mut out, mut diagnostics) = (Vec::new(), Vec::new());
        let failing = |_: &Utf8Path| -> io::Result<Source> { Err(io::Error::from(io::ErrorKind::PermissionDenied)) };
        retrieve_blocks(&ctx, Utf8Path::new("/r/a.ts"), &hits, &failing, &mut out, &mut diagnostics);
        assert!(out.is_empty());
        assert_eq!(
            summary(&diagnostics),
            [(DiagnosticKind::Skipped, Some("/r/a.ts"), "could not read source: permission denied")]
        );
    }

    #[test]
    fn unreadable_paths_and_counts_become_diagnostics_in_order() {
        let found = TextSearchResult {
            unreadable: vec![Utf8PathBuf::from("/r/a.ts"), Utf8PathBuf::from("/r/b.ts")],
            non_utf8_paths: 2,
            walk_errors_without_path: 3,
            ..TextSearchResult::default()
        };
        let unreadable = "file could not be read";
        assert_eq!(
            summary(&text_search_diagnostics(&found)),
            [
                (DiagnosticKind::Skipped, Some("/r/a.ts"), unreadable),
                (DiagnosticKind::Skipped, Some("/r/b.ts"), unreadable),
                (DiagnosticKind::Skipped, None, "2 files skipped: path is not valid UTF-8"),
                (DiagnosticKind::Skipped, None, "3 directory walk errors did not name a UTF-8 path"),
            ]
        );
        assert!(text_search_diagnostics(&TextSearchResult::default()).is_empty());
    }

    const LOSSY: &str = "invalid UTF-8 replaced with U+FFFD";

    #[test]
    fn file_with_a_non_utf8_matching_line_is_searched_and_reported_read_lossily() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), b"alpha \xff\n").unwrap();
        std::fs::write(dir.path().join("b.ts"), b"alpha\n").unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts", "b.ts"]);
        let codes: Vec<&str> = result.ranked.iter().map(|ranked| ranked.block.code.as_str()).collect();
        assert_eq!(codes, ["alpha \u{FFFD}", "alpha"]);
        assert_eq!(result.files_total, 2);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::LossyRead, Some(root.join("a.ts").as_str()), LOSSY)]
        );
    }

    #[test]
    fn file_with_invalid_utf8_outside_the_matching_line_is_searched_and_reported_read_lossily() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), b"// zed\xe9\nalpha\n").unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.ranked[0].block.code, "// zed\u{FFFD}\nalpha");
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::LossyRead, Some(root.join("a.ts").as_str()), LOSSY)]
        );
    }

    #[test]
    fn a_valid_utf8_file_is_not_reported_read_lossily() {
        let (_dir, root) = tree(&[("a.ts", "zed\u{e9} alpha\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.ranked[0].block.code, "zed\u{e9} alpha");
        assert_eq!(summary(&result.diagnostics), []);
    }

    #[test]
    fn an_excluded_pass_is_not_run_when_nothing_matches_a_positive_term() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), b"gamma \xff\n").unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha -gamma")).unwrap();
        assert_eq!(files(&result, &root), Vec::<String>::new());
        assert_eq!(result.files_total, 0);
        assert_eq!(summary(&result.diagnostics), []);
    }

    #[test]
    fn a_file_with_a_non_utf8_line_matching_only_the_excluded_term_is_excluded_without_a_diagnostic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), b"alpha\n").unwrap();
        std::fs::write(dir.path().join("b.ts"), b"gamma \xff\n").unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha -gamma")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(summary(&result.diagnostics), []);
    }

    #[test]
    fn a_file_with_a_non_utf8_line_matching_the_main_and_the_excluded_terms_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), b"alpha\n").unwrap();
        std::fs::write(dir.path().join("b.ts"), b"alpha gamma \xff\n").unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha -gamma")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(summary(&result.diagnostics), []);
    }

    #[test]
    fn an_excluded_term_drops_its_files_when_the_ceiling_is_below_the_file_count() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.ts", "alpha gamma\n"), ("c.ts", "alpha\n")]);
        let result = search_with_ceiling(&whole_file_registry(), &request(&root, "alpha -gamma"), 1, None).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.files_total, 3);
        assert!(result.ceiling_hit);
    }

    #[test]
    fn keyword_undoes_literal_escaping_only() {
        let literal =
            ParsedTerm { modifier: Modifier::Optional, pattern: regex::escape("a.b (c)+\\d"), is_regex: false };
        assert_eq!(keyword(&literal), "a.b (c)+\\d");
        let regex = ParsedTerm { modifier: Modifier::Optional, pattern: r"a\.b".to_owned(), is_regex: true };
        assert_eq!(keyword(&regex), r"a\.b");
    }

    #[test]
    fn identical_runs_give_identical_results() {
        let (_dir, root) = tree(&[
            ("a.ts", "alpha beta\n"),
            ("sub/b.ts", "alpha\nbeta\n"),
            ("sub/deep/c.ts", "alpha alpha\n"),
            ("d.ts", "beta alpha gamma\n"),
        ]);
        let run = || {
            let result = search(&whole_file_registry(), &request(&root, "alpha beta -zzz")).unwrap();
            let rows: Vec<_> = result
                .ranked
                .iter()
                .map(|r| (r.rank, r.block.file.clone(), r.final_score.to_bits(), r.block.matched_lines.clone()))
                .collect();
            (rows, result.files_total, result.ceiling_hit)
        };
        let first = run();
        assert_eq!(first.0.len(), 4);
        assert_eq!(first, run());
    }

    #[test]
    fn results_are_uncapped() {
        let owned: Vec<(String, &str)> = (0..120).map(|i| (format!("f{i:03}.ts"), "alpha\n")).collect();
        let borrowed: Vec<(&str, &str)> = owned.iter().map(|(name, text)| (name.as_str(), *text)).collect();
        let (_dir, root) = tree(&borrowed);
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(result.ranked.len(), 120);
        assert_eq!(result.files_total, 120);
    }

    #[test]
    fn end_to_end_over_a_temp_tree() {
        let (_dir, root) = tree(&[
            (".gitignore", "ignored.ts\n"),
            ("src/auth.ts", "export function login(user) {\n  return checkPassword(user);\n}\n"),
            ("src/session.ts", "export const session = login;\n"),
            ("src/unrelated.ts", "export const x = 1;\n"),
            ("src/legacy.ts", "login(); // deprecated\n"),
            ("src/notes.md", "login docs\n"),
            ("ignored.ts", "login\n"),
            ("tests/auth.test.ts", "login\n"),
        ]);
        let query = "login -deprecated";
        let result = search(&whole_file_registry(), &request(&root, query)).unwrap();
        assert_eq!(files(&result, &root), ["src/auth.ts", "src/session.ts"]);
        // `src/notes.md` has the word but no analyzer owns it, so it is not counted.
        assert_eq!(result.files_total, 3);
        assert!(!result.ceiling_hit);
        assert!(result.diagnostics.is_empty());
        let ranks: Vec<usize> = result.ranked.iter().map(|r| r.rank).collect();
        assert_eq!(ranks, [1, 2]);
        assert_eq!(result.ranked[1].block.matched_lines, [1]);

        let everything = SearchRequest { allow_tests: true, respect_ignore_files: false, ..request(&root, query) };
        let result = search(&whole_file_registry(), &everything).unwrap();
        let mut found = files(&result, &root);
        found.sort();
        assert_eq!(found, ["ignored.ts", "src/auth.ts", "src/session.ts", "tests/auth.test.ts"]);
    }

    /// Runs `f` on a pool of two threads, so a text search batch is 32 files and an analysis batch is 16.
    fn on_two_threads<T: Send>(f: impl FnOnce() -> T + Send) -> T {
        rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap().install(f)
    }

    fn numbered_files(count: usize) -> Vec<(String, &'static str)> {
        (0..count).map(|i| (format!("f{i:03}.ts"), "alpha\n")).collect()
    }

    fn tree_of(files: &[(String, &'static str)]) -> (tempfile::TempDir, Utf8PathBuf) {
        let borrowed: Vec<(&str, &str)> = files.iter().map(|(name, text)| (name.as_str(), *text)).collect();
        tree(&borrowed)
    }

    #[test]
    fn a_file_over_the_size_limit_is_skipped_with_one_grouped_line_and_everything_else_is_found() {
        let (_dir, root) = tree(&[("small.ts", "alpha\n")]);
        let mut big = b"alpha\n".to_vec();
        big.resize(usize::try_from(MAX_FILE_BYTES).unwrap() + 1, b'x');
        std::fs::write(root.join("big.ts"), &big).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["small.ts"]);
        assert_eq!(result.files_total, 1);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Skipped, Some(root.join("big.ts").as_str()), "over 4 MiB")]
        );
    }

    #[test]
    fn a_file_over_the_limit_is_not_read_by_block_retrieval_and_one_at_the_limit_is() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        let path = root.join("a.ts");
        let registry = whole_file_registry();
        let plan = QueryPlan::from_terms(parse("alpha").unwrap());
        let matchers = vec![compile("alpha").unwrap()];
        let ctx = |max_file_bytes| FileContext {
            registry: &registry,
            plan: &plan,
            matchers: &matchers,
            language: None,
            allow_tests: false,
            max_file_bytes,
        };
        let hits = [LineHit { line_number: 1, text: "alpha".to_owned() }];
        let never_read = |_: &Utf8Path| -> io::Result<Source> { panic!("an oversize file must not be read") };

        let (mut out, mut diagnostics) = (Vec::new(), Vec::new());
        retrieve_blocks(&ctx(5), &path, &hits, &never_read, &mut out, &mut diagnostics);
        assert!(out.is_empty());
        assert_eq!(summary(&diagnostics), [(DiagnosticKind::Skipped, Some(path.as_str()), "over 5 bytes")]);

        let (mut out, mut diagnostics) = (Vec::new(), Vec::new());
        retrieve_blocks(&ctx(6), &path, &hits, &read_source, &mut out, &mut diagnostics);
        assert_eq!(out.len(), 1);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn cut_hit_lines_are_counted_in_one_warning_and_normal_lines_report_nothing() {
        let long = format!("alpha {}\n", "x".repeat(5000));
        let (_dir, root) = tree(&[("long.ts", &long), ("normal.ts", "alpha\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root).len(), 2);
        assert_eq!(
            summary(&result.diagnostics),
            [(
                DiagnosticKind::Warning,
                None,
                "1 matching line was cut to 4096 bytes; a term that matches only past the cut is not seen"
            )]
        );
    }

    #[test]
    fn size_labels_use_whole_mebibytes_or_bytes() {
        assert_eq!(
            (size_label(4 * 1024 * 1024), size_label(1024 * 1024 + 1), size_label(5)),
            ("4 MiB".to_owned(), "1048577 bytes".to_owned(), "5 bytes".to_owned())
        );
    }

    #[test]
    fn an_expired_timeout_stops_text_search_after_its_first_batch_and_says_so() {
        let (_dir, root) = tree_of(&numbered_files(100));
        let result = on_two_threads(|| {
            search_with_ceiling(
                &whole_file_registry(),
                &request(&root, "alpha"),
                DEFAULT_MAX_FILES,
                Some(Duration::ZERO),
            )
        })
        .unwrap();
        // 32 files were searched; the first analysis batch of 16 of them was analyzed.
        assert_eq!(result.files_total, 32);
        assert_eq!(result.ranked.len(), 16);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Warning, None, "partial result: 0 s timeout expired after searching 32 of 100 files")]
        );
    }

    #[test]
    fn an_expired_timeout_stops_analysis_between_batches_and_says_so() {
        let (_dir, root) = tree_of(&numbered_files(20));
        let result = on_two_threads(|| {
            search_with_ceiling(
                &whole_file_registry(),
                &request(&root, "alpha"),
                DEFAULT_MAX_FILES,
                Some(Duration::ZERO),
            )
        })
        .unwrap();
        assert_eq!(result.files_total, 20);
        assert_eq!(result.ranked.len(), 16);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Warning, None, "partial result: 0 s timeout expired after analyzing 16 of 20 files")]
        );
    }

    #[test]
    fn a_timeout_that_is_not_reached_changes_nothing() {
        let (_dir, root) = tree_of(&numbered_files(100));
        let (timed, untimed) = on_two_threads(|| {
            let timed =
                search_with_timeout(&whole_file_registry(), &request(&root, "alpha"), Some(Duration::from_secs(3600)));
            (timed.unwrap(), search(&whole_file_registry(), &request(&root, "alpha")).unwrap())
        });
        assert_eq!(files(&timed, &root), files(&untimed, &root));
        assert_eq!(files(&timed, &root).len(), 100);
        assert!(timed.diagnostics.is_empty());
    }

    #[test]
    fn a_fractional_timeout_is_named_as_given() {
        let reached = Incomplete { reached: 1, total: 2 };
        assert_eq!(
            incomplete_diagnostic(Some(Duration::from_millis(1500)), "searching", reached, "").message,
            "partial result: 1.5 s timeout expired after searching 1 of 2 files"
        );
    }
}

#[cfg(test)]
mod proptests;
