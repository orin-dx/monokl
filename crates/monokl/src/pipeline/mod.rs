//! The core of `monokl search`: a query in, fully ranked and deduplicated blocks out.
//!
//! Nothing here caps, budgets, serializes or prints. Callers get every surviving block with the counts needed to
//! describe what was left out, and apply their own budget.

use std::collections::BTreeSet;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::analysis::LanguageId;
use crate::analysis::registry::AnalyzerRegistry;
use crate::dedup::dedup_blocks;
use crate::error::{MonoklError, Result};
use crate::file_io::FileIoError;
use crate::query::{Modifier, ParsedTerm, QueryPlan, parse};
use crate::rank::{rank_blocks, tokenize_block};
use crate::text_search::{DEFAULT_MAX_FILES, TextSearchOptions, TextSearchResult, search_files};
use crate::tokens::tokenize;
use crate::types::{CodeBlock, Diagnostic, DiagnosticKind, Language, LineHit, RankedBlock};

/// What to search for and where.
#[derive(Clone, Copy, Debug)]
pub struct SearchRequest<'a> {
    /// The directory to search; it is canonicalized first.
    pub root: &'a Utf8Path,
    /// The query text, in the syntax [`parse`] reads, or a literal when `exact` is set.
    pub query: &'a str,
    /// Treats the whole query as one literal term, operators and quotes included.
    pub exact: bool,
    /// Searches test files too.
    pub allow_tests: bool,
    /// Skips hidden files and what `.gitignore` and `.ignore` files inside the root exclude.
    pub respect_ignore_files: bool,
    /// Restricts the search to one language; only TypeScript and JavaScript are supported.
    pub language: Option<Language>,
}

/// Everything a search found, best first and uncapped.
#[derive(Clone, Debug)]
pub struct SearchResult {
    /// Deduplicated blocks with 1-based, contiguous ranks.
    pub ranked: Vec<RankedBlock>,
    /// Files with at least one line matching a positive term, kept or not.
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
/// blocks containing a matching line are kept, ranked with BM25 and deduplicated. Files no analyzer supports are
/// ignored without a diagnostic.
///
/// BM25 query terms are the lowercase [`tokenize`] output of each non-excluded literal term. Regex terms select
/// lines and appear in `matched_keywords` but add no BM25 terms. A query with only excluded terms has nothing to
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
/// // With no analyzer registered, the file matches but yields no blocks.
/// let result = search(&AnalyzerRegistry::new(Vec::new()), &request)?;
/// assert_eq!(result.files_total, 1);
/// assert!(result.ranked.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn search(registry: &AnalyzerRegistry, request: &SearchRequest<'_>) -> Result<SearchResult> {
    search_with_ceiling(registry, request, DEFAULT_MAX_FILES)
}

/// [`search`] with the file ceiling as a parameter, so tests can reach it without 100,000 files.
fn search_with_ceiling(
    registry: &AnalyzerRegistry,
    request: &SearchRequest<'_>,
    max_files: usize,
) -> Result<SearchResult> {
    let SearchRequest { root, query, exact, allow_tests, respect_ignore_files, language } = *request;
    if let Some(language) = language
        && !is_supported(language)
    {
        return Ok(SearchResult::skipped(format!(
            "language {language:?} not supported in v1; only TypeScript/JavaScript"
        )));
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
    if plan.is_empty() {
        return Ok(SearchResult::empty());
    }
    let patterns = plan.search_patterns();
    if patterns.is_empty() {
        return Ok(SearchResult::skipped("query has no positive terms; add a term without a leading `-`"));
    }

    // Every term, excluded ones too, compiles before any file is read, so an invalid one fails the whole query.
    let matchers = plan.terms.iter().map(|term| compile(&term.pattern)).collect::<Result<Vec<_>>>()?;
    let root = canonicalize(root)?;

    let found = search_files(&root, &patterns, &TextSearchOptions { allow_tests, respect_ignore_files, max_files })?;
    let mut diagnostics = text_search_diagnostics(&found);

    // The ceiling must not apply here: an excluded file past it would wrongly survive.
    let excluded_files = if found.hits.is_empty() || plan.excluded.is_empty() {
        BTreeSet::new()
    } else {
        let excluded_patterns: Vec<&str> = plan.excluded.iter().map(|&i| plan.terms[i].pattern.as_str()).collect();
        let excluded = search_files(
            &root,
            &excluded_patterns,
            &TextSearchOptions { allow_tests, respect_ignore_files, max_files: usize::MAX },
        )?;
        // The walk is the same, so only a file unreadable for the excluded patterns alone is new.
        for path in &excluded.unreadable {
            if !found.unreadable.contains(path) {
                diagnostics.push(unreadable_diagnostic(path));
            }
        }
        excluded.hits.into_keys().collect()
    };

    let mut candidates: Vec<(CodeBlock, Vec<String>)> = Vec::new();
    for (path, hits) in &found.hits {
        if excluded_files.contains(path) {
            continue;
        }
        let has_every_required =
            plan.required.iter().all(|&i| hits.iter().any(|hit| matches_text(&matchers[i], &hit.text)));
        if !has_every_required {
            continue;
        }
        let ctx = FileContext { registry, plan: &plan, matchers: &matchers, language };
        retrieve_blocks(&ctx, path, hits, &read_source, &mut candidates, &mut diagnostics);
    }

    let bm25_terms = bm25_terms(&plan);
    let df = document_frequencies(&candidates, &bm25_terms);
    let bm25_terms: Vec<&str> = bm25_terms.iter().map(String::as_str).collect();
    let ranked = dedup_blocks(rank_blocks(candidates, &bm25_terms, &df));

    Ok(SearchResult { ranked, files_total: found.files_total, ceiling_hit: found.ceiling_hit, diagnostics })
}

/// Returns whether `language` has an analyzer path in v1.
fn is_supported(language: Language) -> bool {
    match language {
        Language::TypeScript | Language::JavaScript => true,
        Language::Rust | Language::Python => false,
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

fn read_source(path: &Utf8Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// Whether `matcher` matches `text`. Matching in memory cannot fail, so an error counts as no match.
fn matches_text(matcher: &RegexMatcher, text: &str) -> bool {
    matches!(matcher.is_match(text.as_bytes()), Ok(true))
}

fn skipped(path: Option<&Utf8Path>, message: impl Into<String>) -> Diagnostic {
    Diagnostic { kind: DiagnosticKind::Skipped, path: path.map(Utf8Path::to_owned), message: message.into() }
}

fn unreadable_diagnostic(path: &Utf8Path) -> Diagnostic {
    skipped(Some(path), "file could not be read, or a matching line is not UTF-8")
}

/// Turns what text search could not read into diagnostics: one per unreadable path, then one per non-zero count.
fn text_search_diagnostics(found: &TextSearchResult) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = found.unreadable.iter().map(|path| unreadable_diagnostic(path)).collect();
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
}

/// Analyzes one surviving file and pushes the annotated blocks that contain a matching line onto `out`.
fn retrieve_blocks(
    ctx: &FileContext<'_>,
    path: &Utf8Path,
    hits: &[LineHit],
    read: &dyn Fn(&Utf8Path) -> io::Result<String>,
    out: &mut Vec<(CodeBlock, Vec<String>)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(analyzer) = ctx.registry.analyzer_for(path) else { return };
    // JavaScript files belong to the TypeScript analyzer.
    if ctx.language.is_some() && !ctx.registry.supports_language(path, &[LanguageId::TypeScript]) {
        return;
    }
    let source = match read(path) {
        Ok(source) => source,
        Err(err) => {
            diagnostics.push(skipped(Some(path), format!("could not read source: {err}")));
            return;
        }
    };
    let analysis = match analyzer.analyze(path, Box::new(move || Ok(source))) {
        Ok(analysis) => analysis,
        Err(err) => {
            diagnostics.push(skipped(Some(path), format!("could not analyze: {err}")));
            return;
        }
    };
    if analysis.had_parse_errors {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Degraded,
            path: Some(path.to_owned()),
            message: "parse errors; blocks may be incomplete".to_owned(),
        });
    }
    for block in &analysis.blocks {
        // Hits arrive in line order, so each block's slice of them is too.
        let inside: Vec<&LineHit> =
            hits.iter().filter(|hit| (block.line_start..=block.line_end).contains(&hit.line_number)).collect();
        if inside.is_empty() {
            continue;
        }
        let matched_keywords = ctx
            .plan
            .terms
            .iter()
            .zip(ctx.matchers)
            .filter(|(term, _)| term.modifier != Modifier::Excluded)
            .filter(|(_, matcher)| inside.iter().any(|hit| matches_text(matcher, &hit.text)))
            .map(|(term, _)| keyword(term))
            .collect();
        let annotated = CodeBlock {
            matched_lines: inside.iter().map(|hit| hit.line_number).collect(),
            matched_keywords,
            ..block.clone()
        };
        let tokens = tokenize_block(&annotated);
        out.push((annotated, tokens));
    }
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

    use super::*;
    use crate::analysis::file_analysis::FileAnalysis;
    use crate::analysis::lang::{AnalysisProfile, AnalyzerCapabilities, CapabilityPrecision, LanguageAnalyzer};
    use crate::digest::Digest;
    use crate::types::{LangData, SymbolKind, TsData};

    type MakeBlocks = dyn Fn(&Utf8Path, &str) -> Result<(Vec<CodeBlock>, bool)> + Send + Sync;

    /// A stand-in analyzer for `.ts` and `.js` files whose blocks come from a closure instead of a parser.
    struct Canned {
        make: Box<MakeBlocks>,
    }

    impl LanguageAnalyzer for Canned {
        fn languages(&self) -> &[LanguageId] {
            &[LanguageId::TypeScript]
        }
        fn supports(&self, path: &Utf8Path) -> bool {
            matches!(path.extension(), Some("ts" | "js"))
        }
        fn language_for(&self, path: &Utf8Path) -> Option<LanguageId> {
            self.supports(path).then_some(LanguageId::TypeScript)
        }
        fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
            AnalyzerCapabilities {
                blocks: true,
                classified_imports: false,
                resolved_import_graph: CapabilityPrecision::Unsupported,
                refs: CapabilityPrecision::Unsupported,
                definition: CapabilityPrecision::Unsupported,
                inspect_detail: CapabilityPrecision::Unsupported,
            }
        }
        fn analyze_with_profile(
            &self,
            path: &Utf8Path,
            source: Box<dyn FnOnce() -> Result<String>>,
            profile: AnalysisProfile,
        ) -> Result<Arc<FileAnalysis>> {
            let source = source()?;
            let (blocks, had_parse_errors) = (self.make)(path, &source)?;
            Ok(Arc::new(FileAnalysis {
                source_path: path.to_owned(),
                digest: Digest::blake3(source.as_bytes()),
                profile,
                had_parse_errors,
                symbols: Vec::new(),
                dependencies: Vec::new(),
                exports: Vec::new(),
                blocks,
                lang: LangData::Ts(TsData::default()),
            }))
        }
    }

    fn registry(
        make: impl Fn(&Utf8Path, &str) -> Result<(Vec<CodeBlock>, bool)> + Send + Sync + 'static,
    ) -> AnalyzerRegistry {
        AnalyzerRegistry::new(vec![Arc::new(Canned { make: Box::new(make) })])
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
            matched_keywords: Vec::new(),
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
    fn language_other_than_typescript_or_javascript_is_skipped_with_a_diagnostic() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        for (language, name) in [(Language::Rust, "Rust"), (Language::Python, "Python")] {
            let request = SearchRequest { language: Some(language), ..request(&root, "alpha") };
            let result = search(&whole_file_registry(), &request).unwrap();
            assert!(result.ranked.is_empty());
            assert_eq!(result.files_total, 0);
            assert_eq!(
                summary(&result.diagnostics),
                [(
                    DiagnosticKind::Skipped,
                    None,
                    format!("language {name} not supported in v1; only TypeScript/JavaScript").as_str()
                )]
            );
        }
    }

    #[test]
    fn typescript_and_javascript_languages_search_typescript_analyzer_files() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.js", "alpha\n")]);
        for language in [Language::TypeScript, Language::JavaScript] {
            let request = SearchRequest { language: Some(language), ..request(&root, "alpha") };
            let result = search(&whole_file_registry(), &request).unwrap();
            assert_eq!(files(&result, &root), ["a.ts", "b.js"]);
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
            fn language_for(&self, path: &Utf8Path) -> Option<LanguageId> {
                self.supports(path).then_some(LanguageId::Rust)
            }
            fn capabilities(&self, _language: LanguageId) -> AnalyzerCapabilities {
                Canned { make: Box::new(|_, _| Ok((Vec::new(), false))) }.capabilities(LanguageId::Rust)
            }
            fn analyze_with_profile(
                &self,
                path: &Utf8Path,
                source: Box<dyn FnOnce() -> Result<String>>,
                profile: AnalysisProfile,
            ) -> Result<Arc<FileAnalysis>> {
                let source = source()?;
                Ok(Arc::new(FileAnalysis {
                    source_path: path.to_owned(),
                    digest: Digest::blake3(source.as_bytes()),
                    profile,
                    had_parse_errors: false,
                    symbols: Vec::new(),
                    dependencies: Vec::new(),
                    exports: Vec::new(),
                    blocks: vec![block(path, &source, 1, 1)],
                    lang: LangData::Ts(TsData::default()),
                }))
            }
        }
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.rs", "alpha\n")]);
        let mixed = AnalyzerRegistry::new(vec![
            Arc::new(Canned { make: Box::new(|path, source| Ok((vec![block(path, source, 1, 1)], false))) }),
            Arc::new(Rusty),
        ]);
        let everything = search(&mixed, &request(&root, "alpha")).unwrap();
        assert_eq!(files(&everything, &root), ["a.ts", "b.rs"]);
        let ts_only = SearchRequest { language: Some(Language::TypeScript), ..request(&root, "alpha") };
        assert_eq!(files(&search(&mixed, &ts_only).unwrap(), &root), ["a.ts"]);
    }

    #[test]
    fn empty_and_operator_only_queries_return_an_empty_result_without_diagnostics() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n")]);
        for query in ["", "   ", "+", "-", "regex:"] {
            let result = search(&whole_file_registry(), &request(&root, query)).unwrap();
            assert!(result.ranked.is_empty(), "{query:?}");
            assert_eq!((result.files_total, result.ceiling_hit), (0, false), "{query:?}");
            assert!(result.diagnostics.is_empty(), "{query:?}");
        }
        let exact = SearchRequest { exact: true, ..request(&root, "  ") };
        let result = search(&whole_file_registry(), &exact).unwrap();
        assert!(result.ranked.is_empty());
        assert!(result.diagnostics.is_empty());
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
        let result = search_with_ceiling(&whole_file_registry(), &request(&root, "alpha -gamma"), 2).unwrap();
        assert_eq!(files(&result, &root), ["d.ts"]);
        assert!(!result.ceiling_hit);
    }

    #[test]
    fn file_ceiling_and_total_pass_through_from_text_search() {
        let (_dir, root) = tree(&[("a.ts", "alpha\n"), ("b.ts", "alpha\n"), ("c.ts", "alpha\n")]);
        let result = search_with_ceiling(&whole_file_registry(), &request(&root, "alpha"), 2).unwrap();
        assert_eq!(files(&result, &root), ["a.ts", "b.ts"]);
        assert_eq!(result.files_total, 3);
        assert!(result.ceiling_hit);
    }

    #[test]
    fn exact_treats_the_whole_query_as_one_escaped_literal() {
        let (_dir, root) =
            tree(&[("a.ts", "see a.b -x here\n"), ("b.ts", "see axb -x here\n"), ("c.ts", "see a.b here\n")]);
        let exact = SearchRequest { exact: true, ..request(&root, "a.b -x") };
        let result = search(&whole_file_registry(), &exact).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.ranked[0].block.matched_keywords, ["a.b -x"]);
    }

    #[test]
    fn regex_term_selects_lines_and_is_reported_as_written() {
        let (_dir, root) = tree(&[("a.ts", "boooo\n"), ("b.ts", "bravo\n")]);
        let result = search(&whole_file_registry(), &request(&root, "regex:BO+")).unwrap();
        assert_eq!(files(&result, &root), ["a.ts"]);
        assert_eq!(result.ranked[0].block.matched_keywords, ["BO+"]);
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
        let blocks: Vec<_> = result
            .ranked
            .iter()
            .map(|r| (r.block.line_start, r.block.matched_lines.clone(), r.block.matched_keywords.clone()))
            .collect();
        assert_eq!(
            blocks,
            [(5, vec![6, 7], vec!["alpha".to_owned(), "beta".to_owned()]), (1, vec![2], vec!["alpha".to_owned()]),]
        );
    }

    #[test]
    fn matched_keywords_follow_query_order_not_text_order() {
        let (_dir, root) = tree(&[("a.ts", "beta alpha\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha beta")).unwrap();
        assert_eq!(result.ranked[0].block.matched_keywords, ["alpha", "beta"]);
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
        assert!(result.ranked[0].bm25_score > result.ranked[1].bm25_score);
        assert!(result.ranked[1].bm25_score > result.ranked[2].bm25_score);
        assert!(result.ranked[2].bm25_score.abs() < f64::EPSILON);
    }

    #[test]
    fn regex_terms_add_no_bm25_terms() {
        let (_dir, root) = tree(&[("a.ts", "getUserName()\n")]);
        let result = search(&whole_file_registry(), &request(&root, "regex:getUser")).unwrap();
        assert_eq!(result.ranked.len(), 1);
        assert!(result.ranked[0].bm25_score.abs() < f64::EPSILON);
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
                matched_keywords: Vec::new(),
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

    #[test]
    fn unsupported_files_yield_nothing_and_no_diagnostic() {
        let (_dir, root) = tree(&[("notes.md", "alpha\n")]);
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert!(result.ranked.is_empty());
        assert_eq!(result.files_total, 1);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn analyzer_error_skips_the_file_with_a_diagnostic() {
        let (_dir, root) = tree(&[("bad.ts", "alpha\n"), ("good.ts", "alpha\n")]);
        let registry = registry(|path, source| {
            if path.file_name() == Some("bad.ts") {
                return Err(MonoklError::TokenizerInit);
            }
            Ok((vec![block(path, source, 1, 1)], false))
        });
        let result = search(&registry, &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["good.ts"]);
        assert_eq!(
            summary(&result.diagnostics),
            [(DiagnosticKind::Skipped, Some(root.join("bad.ts").as_str()), "could not analyze: tokenizer init failed")]
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

    #[test]
    fn source_read_failure_skips_the_file_with_a_diagnostic() {
        let registry = whole_file_registry();
        let plan = QueryPlan::from_terms(parse("alpha").unwrap());
        let matchers = vec![compile("alpha").unwrap()];
        let ctx = FileContext { registry: &registry, plan: &plan, matchers: &matchers, language: None };
        let hits = [LineHit { line_number: 1, text: "alpha".to_owned() }];
        let (mut out, mut diagnostics) = (Vec::new(), Vec::new());
        let failing = |_: &Utf8Path| Err(io::Error::from(io::ErrorKind::PermissionDenied));
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
        let unreadable = "file could not be read, or a matching line is not UTF-8";
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

    #[test]
    fn file_with_a_non_utf8_matching_line_is_reported_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), b"alpha \xff\n").unwrap();
        std::fs::write(dir.path().join("b.ts"), b"alpha\n").unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().canonicalize().unwrap()).unwrap();
        let result = search(&whole_file_registry(), &request(&root, "alpha")).unwrap();
        assert_eq!(files(&result, &root), ["b.ts"]);
        assert_eq!(
            summary(&result.diagnostics),
            [(
                DiagnosticKind::Skipped,
                Some(root.join("a.ts").as_str()),
                "file could not be read, or a matching line is not UTF-8"
            )]
        );
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
        assert_eq!(files(&result, &root), ["src/session.ts", "src/auth.ts"]);
        assert_eq!(result.files_total, 4);
        assert!(!result.ceiling_hit);
        assert!(result.diagnostics.is_empty());
        let ranks: Vec<usize> = result.ranked.iter().map(|r| r.rank).collect();
        assert_eq!(ranks, [1, 2]);
        assert_eq!(result.ranked[1].block.matched_lines, [1]);
        assert_eq!(result.ranked[1].block.matched_keywords, ["login"]);

        let everything = SearchRequest { allow_tests: true, respect_ignore_files: false, ..request(&root, query) };
        let result = search(&whole_file_registry(), &everything).unwrap();
        let mut found = files(&result, &root);
        found.sort();
        assert_eq!(found, ["ignored.ts", "src/auth.ts", "src/session.ts", "tests/auth.test.ts"]);
    }
}
