//! The first search stage: walks a root and returns, per file, the lines that match any query pattern.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::WalkBuilder;
use ignore::gitignore::Gitignore;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::error::{MonoklError, Result};
use crate::types::LineHit;

/// How many files each thread searches per batch. A batch ends in a wait for its slowest file, so a small batch costs
/// speed; a large one holds more hits at once.
const BATCH_FILES_PER_THREAD: usize = 16;

/// The default for [`TextSearchOptions::max_files`].
pub const DEFAULT_MAX_FILES: usize = 100_000;

/// Options for [`search_files`].
///
/// # Examples
///
/// ```
/// use monokl::text_search::TextSearchOptions;
///
/// let opts = TextSearchOptions { allow_tests: true, ..TextSearchOptions::default() };
/// assert!(opts.respect_ignore_files);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSearchOptions {
    /// Searches test files too. When false, a file is skipped if its path below the root has a `__tests__`, `test`,
    /// `tests` or `spec` directory, or ends in `.test.ts`, `.test.tsx`, `.test.js`, `.spec.ts`, `.spec.tsx` or
    /// `.spec.js`. Matching is case-sensitive.
    pub allow_tests: bool,
    /// Skips hidden files and what `.gitignore` and `.ignore` files inside the root exclude, with or without a `.git`
    /// directory. When the root lies inside a git repository (the nearest ancestor with a `.git` entry), the
    /// `.gitignore` and `.ignore` files of the directories from the repository root down to the root's parent apply
    /// too, as they do for ripgrep, and rules inside the root take precedence over them. Nothing above the repository
    /// root is read, and without a `.git` ancestor no parent is read. `.git/info/exclude` and the user's global
    /// excludes are never read, so the same checkout gives the same result on every machine.
    pub respect_ignore_files: bool,
    /// The most files whose hits are kept. Matching files past it are still searched and counted.
    pub max_files: usize,
}

impl Default for TextSearchOptions {
    /// Skips tests, honors ignore files and keeps up to [`DEFAULT_MAX_FILES`] files.
    fn default() -> Self {
        Self { allow_tests: false, respect_ignore_files: true, max_files: DEFAULT_MAX_FILES }
    }
}

/// What [`search_files`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextSearchResult {
    /// The kept files, keyed by `root` joined with the path below it, each with its matching lines in line order.
    pub hits: BTreeMap<Utf8PathBuf, Vec<LineHit>>,
    /// Every file with at least one matching line, kept or not.
    pub files_total: u64,
    /// Whether `files_total` exceeded [`TextSearchOptions::max_files`], so some matching files have no hits stored.
    pub ceiling_hit: bool,
    /// In walk order and each listed once, keyed like `hits`: files that could not be read, directories that could not
    /// be listed, and ignore files that could not be read or parsed. A parent ignore file above the root is named by its
    /// canonical path.
    pub unreadable: Vec<Utf8PathBuf>,
    /// How many regular files were skipped because their path is not UTF-8.
    pub non_utf8_paths: u64,
    /// How many walk errors below the root carried no UTF-8 path to put in `unreadable`.
    pub walk_errors_without_path: u64,
}

/// Searches every regular file under `root` for lines matching any of `patterns`, regexes matched case-insensitively
/// with Unicode case folding.
///
/// A pattern sees each line without its `\n` or `\r\n` terminator, exactly the text a hit reports, so `^` and `$`
/// anchor at the line's start and end in LF and CRLF files alike, and a pattern for `\n` matches no line.
///
/// Each directory's entries are visited in byte order of their file names, so the result is the same on every run.
/// Symbolic links are neither followed nor searched. Binary files (any NUL byte) are skipped silently, even when a
/// matching line is also not UTF-8. A matching line that is not UTF-8 is a hit with each invalid byte sequence
/// replaced by U+FFFD. A file that cannot be read, and any walk failure below `root`, go in
/// [`unreadable`](TextSearchResult::unreadable) and the search continues. No patterns
/// means nothing to search: once `root` is checked, the result is empty and nothing is walked.
///
/// # Errors
///
/// - [`MonoklError::RegexBuild`] if any pattern does not compile, before `root` is checked.
/// - [`MonoklError::Walk`] with `path` set to `root` if `root` is missing, is not a directory or cannot be listed,
///   even when there are no patterns.
///
/// # Examples
///
/// ```
/// use camino::Utf8Path;
/// use monokl::text_search::{search_files, TextSearchOptions};
///
/// let dir = tempfile::tempdir()?;
/// std::fs::write(dir.path().join("a.ts"), "const Alpha = 1;\nconst beta = 2;\n")?;
/// let root = Utf8Path::from_path(dir.path()).ok_or("temp dir is not UTF-8")?;
///
/// let result = search_files(root, &["alpha", "gamma"], &TextSearchOptions::default())?;
/// let lines = &result.hits[&root.join("a.ts")];
/// assert_eq!((lines[0].line_number, lines[0].text.as_str()), (1, "const Alpha = 1;"));
/// assert_eq!(result.files_total, 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn search_files(root: &Utf8Path, patterns: &[&str], opts: &TextSearchOptions) -> Result<TextSearchResult> {
    search_in_batches(root, patterns, opts, rayon::current_num_threads() * BATCH_FILES_PER_THREAD)
}

/// [`search_files`] with the number of files searched before their outcomes are folded into the result. The result does
/// not depend on it; it bounds how many files' hits are held beyond the kept ones.
fn search_in_batches(
    root: &Utf8Path,
    patterns: &[&str],
    opts: &TextSearchOptions,
    batch_size: usize,
) -> Result<TextSearchResult> {
    let matcher = compile(patterns)?;
    let root_error = |source| MonoklError::Walk { path: root.to_owned(), source };
    let metadata = std::fs::metadata(root).map_err(|e| root_error(ignore::Error::Io(e)))?;
    if !metadata.is_dir() {
        return Err(root_error(ignore::Error::Io(io::Error::from(io::ErrorKind::NotADirectory))));
    }
    // Listed here as well as by the walk, so a root that cannot be listed fails even when there is nothing to search.
    std::fs::read_dir(root).map_err(|e| root_error(ignore::Error::Io(e)))?;
    let mut result = TextSearchResult::default();
    let Some(matcher) = matcher else {
        return Ok(result);
    };
    // Passthru hands every line to the sink, so a CRLF line the matcher rejected because it saw the `\r` can still be
    // checked against its text without the terminator. BOM sniffing is off so a UTF-16 file's NUL bytes mark it binary
    // and a UTF-8 BOM stays in the first line's text instead of being decoded away.
    let mut searcher_builder = SearcherBuilder::new();
    searcher_builder.binary_detection(BinaryDetection::quit(b'\x00')).bom_sniffing(false).passthru(true);
    // The `ignore` crate's own parent support reads every ancestor up to `/`, so the walk covers the root and the
    // repository's parent files are applied by `ParentIgnores`. Global and `.git/info/exclude` rules stay off so the
    // same checkout gives the same result on every machine.
    let parents = if opts.respect_ignore_files { ParentIgnores::load(root, &mut result) } else { None };
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(opts.respect_ignore_files)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .sort_by_file_name(std::cmp::Ord::cmp);
    if let Some(parents) = parents {
        let root = root.to_owned();
        builder.filter_entry(move |entry| {
            let Some(path) = Utf8Path::from_path(entry.path()) else { return true };
            let Ok(relative) = path.strip_prefix(&root) else { return true };
            !parents.excludes(relative, entry.file_type().is_some_and(|file_type| file_type.is_dir()))
        });
    }
    let walk = builder.build();
    // Walk errors and files keep the position they had in the walk, so `unreadable` lists them in walk order.
    let mut items = Vec::new();
    for entry in walk {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) if err.depth() == Some(0) => return Err(root_error(err)),
            Err(err) => {
                collect_walk_error(&err, &mut items);
                continue;
            }
        };
        if let Some(err) = entry.error() {
            collect_walk_error(err, &mut items);
        }
        if !entry.file_type().is_some_and(|file_type| file_type.is_file()) {
            continue;
        }
        let Ok(path) = Utf8PathBuf::from_path_buf(entry.into_path()) else {
            result.non_utf8_paths += 1;
            continue;
        };
        if !opts.allow_tests && is_test_path(path.strip_prefix(root).unwrap_or(&path)) {
            continue;
        }
        items.push(WalkItem::File(path));
    }
    // Files are searched on separate threads a batch at a time and folded in walk order, so the result and the ceiling
    // never depend on which thread finished first. A file's hits are dropped as soon as it is past the ceiling, so one
    // batch's hits are held beyond the kept files, however many files match.
    let batch_size = batch_size.max(1);
    let mut items = items.into_iter();
    loop {
        let batch: Vec<WalkItem> = items.by_ref().take(batch_size).collect();
        if batch.is_empty() {
            break;
        }
        let searched: Vec<Searched> = batch
            .into_par_iter()
            .map_init(
                || searcher_builder.build(),
                |searcher, item| match item {
                    WalkItem::File(path) => {
                        let outcome = search_file(searcher, &matcher, &path);
                        Searched::File(path, outcome)
                    }
                    WalkItem::WalkError(error) => Searched::WalkError(error),
                },
            )
            .collect();
        for item in searched {
            match item {
                Searched::WalkError(error) => error.record(&mut result),
                Searched::File(_, FileOutcome::Binary) => {}
                Searched::File(path, FileOutcome::Unreadable) => result.unreadable.push(path),
                Searched::File(path, FileOutcome::Lines(hits)) => {
                    if hits.is_empty() {
                        continue;
                    }
                    result.files_total += 1;
                    if result.hits.len() < opts.max_files {
                        result.hits.insert(path, hits);
                    } else {
                        result.ceiling_hit = true;
                    }
                }
            }
        }
    }
    Ok(result)
}

/// The `.ignore` and `.gitignore` rules of the directories from a git repository's root down to the search root's
/// parent, applied as the `ignore` crate applies parent files: a rule nearer the entry wins, `.ignore` wins over
/// `.gitignore` in one directory, and rules inside the search root win over all of them.
struct ParentIgnores {
    /// The canonical search root, which the matchers' paths are built from.
    base: Utf8PathBuf,
    /// Nearest directory first.
    parents: Vec<DirRules>,
    /// Rules of the directories inside the root, read when first needed.
    inside: Mutex<FxHashMap<Utf8PathBuf, Arc<DirRules>>>,
}

/// One directory's `.ignore` and `.gitignore` files.
struct DirRules {
    ignore: Gitignore,
    gitignore: Gitignore,
}

impl DirRules {
    /// Reads the directory's rules, with the errors from files that could not be read or parsed.
    fn read(dir: &Utf8Path) -> (Self, Vec<ignore::Error>) {
        let (ignore, ignore_err) = Gitignore::new(dir.join(".ignore"));
        let (gitignore, gitignore_err) = Gitignore::new(dir.join(".gitignore"));
        (Self { ignore, gitignore }, ignore_err.into_iter().chain(gitignore_err).collect())
    }

    /// Whether either file has a rule, ignoring or whitelisting, that matches. `Some(true)` is an ignore.
    fn verdict(&self, path: &Utf8Path, is_dir: bool) -> Option<bool> {
        let matched =
            self.ignore.matched(path, is_dir).map(|_| ()).or(self.gitignore.matched(path, is_dir).map(|_| ()));
        match matched {
            ignore::Match::None => None,
            ignore::Match::Ignore(()) => Some(true),
            ignore::Match::Whitelist(()) => Some(false),
        }
    }
}

impl ParentIgnores {
    /// Reads the parent rules for `root`, or `None` when `root` has no `.git` ancestor, is itself the repository
    /// root, or no parent directory has a rule. Read errors go in `result` like walk errors do.
    fn load(root: &Utf8Path, result: &mut TextSearchResult) -> Option<Self> {
        let base = root.canonicalize_utf8().ok()?;
        let repo_root = base.ancestors().find(|dir| dir.join(".git").symlink_metadata().is_ok())?;
        let mut parents = Vec::new();
        for dir in base.ancestors().skip(1).take_while(|dir| dir.starts_with(repo_root)) {
            let (rules, errors) = DirRules::read(dir);
            for err in &errors {
                record_walk_error(err, result);
            }
            parents.push(rules);
        }
        parents.iter().any(|rules| !rules.ignore.is_empty() || !rules.gitignore.is_empty()).then(|| Self {
            base,
            parents,
            inside: Mutex::new(FxHashMap::default()),
        })
    }

    /// Whether a parent rule excludes the entry at `relative` below the root, and no rule inside the root matches it.
    fn excludes(&self, relative: &Utf8Path, is_dir: bool) -> bool {
        if relative.as_str().is_empty() {
            return false;
        }
        let path = self.base.join(relative);
        let decided_inside = path
            .ancestors()
            .skip(1)
            .take_while(|dir| dir.starts_with(&self.base))
            .any(|dir| self.inside_rules(dir).verdict(&path, is_dir).is_some());
        if decided_inside {
            return false;
        }
        self.parents.iter().find_map(|rules| rules.verdict(&path, is_dir)).unwrap_or(false)
    }

    fn inside_rules(&self, dir: &Utf8Path) -> Arc<DirRules> {
        let mut cache = self.inside.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Read errors are dropped: the walk reads the same files and reports them.
        Arc::clone(cache.entry(dir.to_owned()).or_insert_with(|| Arc::new(DirRules::read(dir).0)))
    }
}

/// Compiles each pattern alone, then all together, so an invalid one always fails the search. `None` when there are no
/// patterns.
fn compile(patterns: &[&str]) -> Result<Option<RegexMatcher>> {
    let mut builder = RegexMatcherBuilder::new();
    builder.case_insensitive(true);
    // `build_many` joins the patterns with `|` before parsing, so halves such as `(a` and `b)` would compile together.
    for pattern in patterns {
        builder.build(pattern)?;
    }
    if patterns.is_empty() {
        return Ok(None);
    }
    Ok(Some(builder.build_many(patterns)?))
}

/// Lists the path a walk error names in `unreadable`, or counts it when it names none that is UTF-8.
fn record_walk_error(err: &ignore::Error, result: &mut TextSearchResult) {
    let mut items = Vec::new();
    collect_walk_error(err, &mut items);
    for item in items {
        match item {
            WalkItem::WalkError(error) => error.record(result),
            WalkItem::File(_) => {}
        }
    }
}

/// Appends one item per path a walk error names, or per failure that names no UTF-8 path.
fn collect_walk_error(err: &ignore::Error, items: &mut Vec<WalkItem>) {
    match err {
        ignore::Error::Partial(errs) => {
            for err in errs {
                collect_walk_error(err, items);
            }
        }
        ignore::Error::WithPath { path, .. } | ignore::Error::Loop { child: path, .. } => {
            items.push(WalkItem::WalkError(match Utf8Path::from_path(path) {
                Some(path) => WalkError::Path(path.to_owned()),
                None => WalkError::WithoutPath,
            }));
        }
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            collect_walk_error(err, items);
        }
        ignore::Error::Io(_)
        | ignore::Error::Glob { .. }
        | ignore::Error::UnrecognizedFileType(_)
        | ignore::Error::InvalidDefinition => items.push(WalkItem::WalkError(WalkError::WithoutPath)),
    }
}

/// What the walk produced, in the order it produced it.
enum WalkItem {
    File(Utf8PathBuf),
    WalkError(WalkError),
}

/// One failure the walk reported.
enum WalkError {
    Path(Utf8PathBuf),
    WithoutPath,
}

impl WalkError {
    /// Lists the path once, or counts a failure that names no UTF-8 path.
    fn record(self, result: &mut TextSearchResult) {
        match self {
            Self::Path(path) if result.unreadable.contains(&path) => {}
            Self::Path(path) => result.unreadable.push(path),
            Self::WithoutPath => result.walk_errors_without_path += 1,
        }
    }
}

/// A [`WalkItem`] after its file, if any, has been searched.
enum Searched {
    File(Utf8PathBuf, FileOutcome),
    WalkError(WalkError),
}

/// How searching one file ended.
enum FileOutcome {
    Binary,
    Unreadable,
    /// The file's matching lines, possibly none.
    Lines(Vec<LineHit>),
}

fn search_file(searcher: &mut Searcher, matcher: &RegexMatcher, path: &Utf8Path) -> FileOutcome {
    let mut sink = LineCollector::new(matcher);
    match searcher.search_path(matcher, path.as_std_path(), &mut sink) {
        Err(_) => FileOutcome::Unreadable,
        Ok(()) if sink.binary => FileOutcome::Binary,
        Ok(()) => FileOutcome::Lines(sink.hits),
    }
}

/// Collects matching lines, and stops at the first sign of binary data so the caller can discard the whole file,
/// including lines matched before the NUL byte was reached. A matching line that is not UTF-8 is kept with its invalid
/// bytes replaced by U+FFFD, and does not stop the search, so a NUL later in the file still marks it binary.
///
/// The searcher strips only the `\n` before matching, so a line ending in `\r\n` is matched again here against its
/// text without the terminator: a pattern always sees exactly the text a hit reports.
struct LineCollector<'m> {
    matcher: &'m RegexMatcher,
    /// Matches one CRLF line's text, which holds no `\n`, so it searches it as a single line.
    line_searcher: Searcher,
    /// Reused to hold the text [`Self::line_searcher`] searches.
    line_buf: Vec<u8>,
    hits: Vec<LineHit>,
    binary: bool,
}

impl<'m> LineCollector<'m> {
    fn new(matcher: &'m RegexMatcher) -> Self {
        Self {
            matcher,
            line_searcher: SearcherBuilder::new().bom_sniffing(false).build(),
            line_buf: Vec::new(),
            hits: Vec::new(),
            binary: false,
        }
    }

    /// Records `line` as a hit when it matches. `searcher_matched` is the searcher's verdict, which is final unless the
    /// line ends in `\r\n`.
    fn line(&mut self, line_number: Option<u64>, line: &[u8], searcher_matched: bool) -> io::Result<()> {
        let text = without_line_terminator(line);
        let matched = if line.ends_with(b"\r\n") { self.text_matches(text)? } else { searcher_matched };
        if !matched {
            return Ok(());
        }
        let line_number = line_number
            .and_then(|line_number| usize::try_from(line_number).ok())
            .ok_or_else(|| io::Error::other("matched line has no usable line number"))?;
        self.hits.push(LineHit { line_number, text: String::from_utf8_lossy(text).into_owned() });
        Ok(())
    }

    fn text_matches(&mut self, text: &[u8]) -> io::Result<bool> {
        // An empty slice has no lines to match, so the text is searched as one `\n`-terminated line, which the
        // searcher strips again before matching.
        self.line_buf.clear();
        self.line_buf.extend_from_slice(text);
        self.line_buf.push(b'\n');
        let mut found = AnyMatch(false);
        self.line_searcher.search_slice(self.matcher, &self.line_buf, &mut found)?;
        Ok(found.0)
    }
}

impl Sink for LineCollector<'_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> io::Result<bool> {
        self.line(mat.line_number(), mat.bytes(), true)?;
        Ok(true)
    }

    fn context(&mut self, _searcher: &Searcher, context: &SinkContext<'_>) -> io::Result<bool> {
        self.line(context.line_number(), context.bytes(), false)?;
        Ok(true)
    }

    fn binary_data(&mut self, _searcher: &Searcher, _binary_byte_offset: u64) -> io::Result<bool> {
        self.binary = true;
        Ok(false)
    }
}

/// Records whether a search found any match, stopping at the first.
struct AnyMatch(bool);

impl Sink for AnyMatch {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, _mat: &SinkMatch<'_>) -> io::Result<bool> {
        self.0 = true;
        Ok(false)
    }
}

/// Strips one trailing `\n` or `\r\n`. A `\r` not followed by `\n` is line content and stays.
fn without_line_terminator(line: &[u8]) -> &[u8] {
    match line.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => line,
    }
}

/// Classifies a path relative to the search root, so directories above the root never make a file a test.
fn is_test_path(relative: &Utf8Path) -> bool {
    let path: String = relative.components().flat_map(|component| ["/", component.as_str()]).collect();
    path.contains("/__tests__/")
        || path.contains("/test/")
        || path.contains("/tests/")
        || path.contains("/spec/")
        || path.ends_with(".test.ts")
        || path.ends_with(".test.tsx")
        || path.ends_with(".test.js")
        || path.ends_with(".test.jsx")
        || path.ends_with(".test.mjs")
        || path.ends_with(".test.cjs")
        || path.ends_with(".test.mts")
        || path.ends_with(".test.cts")
        || path.ends_with(".spec.ts")
        || path.ends_with(".spec.tsx")
        || path.ends_with(".spec.js")
        || path.ends_with(".spec.jsx")
        || path.ends_with(".spec.mjs")
        || path.ends_with(".spec.cjs")
        || path.ends_with(".spec.mts")
        || path.ends_with(".spec.cts")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

    /// Builds a temp tree from `(relative path, contents)` pairs and returns it with its root.
    fn tree(files: &[(&str, &[u8])]) -> (TempDir, Utf8PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        write_files(&root, files);
        (dir, root)
    }

    fn write_files(root: &Utf8Path, files: &[(&str, &[u8])]) {
        for (relative, contents) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
        }
    }

    fn hit(line_number: usize, text: &str) -> LineHit {
        LineHit { line_number, text: text.to_string() }
    }

    fn hits(root: &Utf8Path, files: &[(&str, Vec<LineHit>)]) -> BTreeMap<Utf8PathBuf, Vec<LineHit>> {
        files.iter().map(|(relative, lines)| (root.join(relative), lines.clone())).collect()
    }

    /// The relative paths of the files that have hits, in path order.
    fn hit_files(root: &Utf8Path, result: &TextSearchResult) -> Vec<String> {
        result.hits.keys().map(|path| path.strip_prefix(root).unwrap().to_string()).collect()
    }

    fn with_max_files(max_files: usize) -> TextSearchOptions {
        TextSearchOptions { max_files, ..TextSearchOptions::default() }
    }

    #[test]
    fn default_options_skip_tests_respect_ignore_files_and_keep_a_hundred_thousand_files() {
        assert_eq!(
            TextSearchOptions::default(),
            TextSearchOptions { allow_tests: false, respect_ignore_files: true, max_files: 100_000 }
        );
    }

    #[test]
    fn matches_case_insensitively_with_one_based_line_numbers() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\nbeta\nALPHA beta\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha"), hit(3, "ALPHA beta")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn patterns_are_alternatives() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\nbeta\ngamma\n")]);
        let result = search_files(&root, &["alpha", "gamma"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "alpha"), hit(3, "gamma")])]));
    }

    #[test]
    fn patterns_are_regexes() {
        let (_dir, root) = tree(&[("a.ts", b"foo1\nfoo\nfooX2\n")]);
        let result = search_files(&root, &[r"foo\w?\d"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "foo1"), hit(3, "fooX2")])]));
    }

    #[test]
    fn a_dot_matches_any_character_rather_than_a_literal_dot() {
        let (_dir, root) = tree(&[("a.ts", b"abc\nac\na.c\n")]);
        let result = search_files(&root, &["a.c"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "abc"), hit(3, "a.c")])]));
    }

    #[test]
    fn case_folding_covers_non_ascii_letters() {
        let (_dir, root) = tree(&[("a.ts", "\u{e9}t\u{e9}\n\u{c9}T\u{c9}\nete\n".as_bytes())]);
        let result = search_files(&root, &["\u{e9}t\u{e9}"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "\u{e9}t\u{e9}"), hit(2, "\u{c9}T\u{c9}")])]));
    }

    #[test]
    fn an_empty_pattern_matches_every_line() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n\nbeta\n")]);
        let result = search_files(&root, &[""], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "alpha"), hit(2, ""), hit(3, "beta")])]));
    }

    #[test]
    fn line_text_drops_its_lf_or_crlf_terminator() {
        let (_dir, root) = tree(&[("a.ts", b"alpha one\r\nalpha two\nbeta\r\nalpha three")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(&root, &[("a.ts", vec![hit(1, "alpha one"), hit(2, "alpha two"), hit(4, "alpha three")])])
        );
    }

    #[test]
    fn line_text_keeps_a_carriage_return_that_is_not_a_line_terminator() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\rbeta\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "alpha\rbeta")])]));
    }

    #[test]
    fn no_patterns_return_an_empty_result_without_walking() {
        // Walking would list the unparsable ignore file in `unreadable`.
        let (_dir, root) = tree(&[(".gitignore", b"[z-a]\n"), ("a.ts", b"alpha\n")]);
        assert_eq!(search_files(&root, &[], &TextSearchOptions::default()).unwrap(), TextSearchResult::default());
    }

    #[test]
    fn no_patterns_with_a_missing_root_is_a_walk_error_naming_the_root() {
        let (_dir, root) = tree(&[]);
        let missing = root.join("missing");
        assert_walk_error_naming(&missing, search_files(&missing, &[], &TextSearchOptions::default()).unwrap_err());
    }

    #[test]
    fn no_patterns_with_a_root_that_is_a_file_is_a_walk_error_naming_the_root() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n")]);
        let file = root.join("a.ts");
        assert_walk_error_naming(&file, search_files(&file, &[], &TextSearchOptions::default()).unwrap_err());
    }

    #[test]
    fn anchors_match_at_line_start_and_end_in_lf_and_crlf_files() {
        let (_dir, root) = tree(&[("crlf.ts", b"alpha\r\nalpha beta\r\n"), ("lf.ts", b"alpha\nalpha beta\n")]);
        let result = search_files(&root, &["^alpha$"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("crlf.ts", vec![hit(1, "alpha")]), ("lf.ts", vec![hit(1, "alpha")])]));
    }

    #[test]
    fn a_newline_pattern_compiles_and_matches_no_line() {
        let (_dir, root) = tree(&[("crlf.ts", b"alpha\r\nbeta\r\n"), ("lf.ts", b"alpha\nbeta\n")]);
        let result = search_files(&root, &[r"\n", r"alpha\nbeta"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result, TextSearchResult::default());
    }

    /// A pattern sees exactly the line text a hit reports, so the `\r` of a CRLF terminator is invisible to it while a
    /// `\r` that is line content is not.
    #[rstest]
    #[case::crlf_terminator_is_not_seen(b"alpha\r\n", r"\r", vec![])]
    #[case::empty_crlf_line(b"alpha\r\n\r\n", "^$", vec![hit(2, "")])]
    #[case::end_anchor_before_crlf(b"alpha\r\nbeta\r\n", "ta$", vec![hit(2, "beta")])]
    #[case::lone_carriage_return_is_content(b"a\rc\n", r"a\rc$", vec![hit(1, "a\rc")])]
    #[case::dot_matches_a_lone_carriage_return(b"a\rc\r\n", "^a.c$", vec![hit(1, "a\rc")])]
    #[case::lone_carriage_return_is_not_a_line_boundary(b"alpha\rbeta\r\n", "^beta", vec![])]
    #[case::carriage_return_before_crlf_is_content(b"alpha\r\r\n", r"^alpha\r$", vec![hit(1, "alpha\r")])]
    #[case::carriage_return_at_end_of_file_is_content(b"alpha\r", r"^alpha\r$", vec![hit(1, "alpha\r")])]
    fn a_pattern_sees_the_line_text_without_its_terminator(
        #[case] contents: &[u8],
        #[case] pattern: &str,
        #[case] expected: Vec<LineHit>,
    ) {
        let (_dir, root) = tree(&[("a.ts", contents)]);
        let result = search_files(&root, &[pattern], &TextSearchOptions::default()).unwrap();
        let expected = if expected.is_empty() { BTreeMap::new() } else { hits(&root, &[("a.ts", expected)]) };
        assert_eq!(result.hits, expected);
    }

    #[test]
    fn no_patterns_do_not_match_every_line() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n")]);
        assert_eq!(search_files(&root, &[], &TextSearchOptions::default()).unwrap(), TextSearchResult::default());
    }

    #[rstest]
    #[case::alone(&["("])]
    #[case::beside_a_valid_pattern(&["alpha", "("])]
    #[case::halves_that_would_compile_once_joined(&["(alpha", "beta)"])]
    fn an_invalid_pattern_fails_the_whole_search_before_walking(#[case] patterns: &[&str]) {
        // The root does not exist, so a walk would fail with a different error.
        let (_dir, root) = tree(&[]);
        let missing = root.join("missing");
        let err = search_files(&missing, patterns, &TextSearchOptions::default()).unwrap_err();
        assert!(matches!(err, MonoklError::RegexBuild(_)), "got {err:?}");
    }

    fn assert_walk_error_naming(root: &Utf8Path, err: MonoklError) {
        let MonoklError::Walk { path, .. } = err else { panic!("expected a walk error, got {err:?}") };
        assert_eq!(path, root);
    }

    #[test]
    fn a_missing_root_is_a_walk_error_naming_the_root() {
        let (_dir, root) = tree(&[]);
        let missing = root.join("missing");
        assert_walk_error_naming(
            &missing,
            search_files(&missing, &["alpha"], &TextSearchOptions::default()).unwrap_err(),
        );
    }

    #[test]
    fn a_root_that_is_a_file_is_a_walk_error_naming_the_root() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n")]);
        let file = root.join("a.ts");
        assert_walk_error_naming(&file, search_files(&file, &["alpha"], &TextSearchOptions::default()).unwrap_err());
    }

    #[cfg(unix)]
    #[rstest]
    #[case::with_patterns(&["alpha"])]
    #[case::without_patterns(&[])]
    fn a_root_that_cannot_be_listed_is_a_walk_error_naming_the_root(#[case] patterns: &[&str]) {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[]);
        let locked = root.join("locked");
        write_files(&locked, &[("a.ts", b"alpha\n")]);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let listable = fs::read_dir(&locked).is_ok();
        let outcome = search_files(&locked, patterns, &TextSearchOptions::default());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if listable {
            // Running as root: permissions cannot make the directory unreadable.
            return;
        }
        assert_walk_error_naming(&locked, outcome.unwrap_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_opened_is_unreadable_even_when_it_holds_a_nul() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[("a.ts", b"alpha\n\x00\n")]);
        let file = root.join("a.ts");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read(&file).is_ok();
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        if readable {
            // Running as root: permissions cannot make the file unreadable.
            return;
        }
        assert_eq!(result, TextSearchResult { unreadable: vec![file], ..TextSearchResult::default() });
    }

    #[test]
    fn a_utf16_file_with_a_bom_is_binary_not_transcoded() {
        let (_dir, root) = tree(&[("a.ts", b"\xff\xfea\x00l\x00p\x00h\x00a\x00\n\x00")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result, TextSearchResult::default());
    }

    #[test]
    fn a_utf8_bom_stays_in_the_first_line_text() {
        let (_dir, root) = tree(&[("a.ts", "\u{feff}alpha\nalpha\n".as_bytes())]);
        let result = search_files(&root, &["^alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(2, "alpha")])]));
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "\u{feff}alpha"), hit(2, "alpha")])]));
    }

    #[cfg(unix)]
    #[test]
    fn a_subdirectory_that_cannot_be_listed_is_unreadable_and_the_search_continues() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("locked/b.ts", b"alpha\n"), ("z.ts", b"alpha\n")]);
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let listable = fs::read_dir(root.join("locked")).is_ok();
        let outcome = search_files(&root, &["alpha"], &TextSearchOptions::default());
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        if listable {
            // Running as root: permissions cannot make the directory unreadable.
            return;
        }
        assert_eq!(
            outcome.unwrap(),
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")]), ("z.ts", vec![hit(1, "alpha")])]),
                files_total: 2,
                unreadable: vec![root.join("locked")],
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn an_ignore_file_that_does_not_parse_is_unreadable_and_its_valid_lines_still_apply() {
        let (_dir, root) =
            tree(&[(".gitignore", b"[z-a]\nignored.ts\n[y-b]\n"), ("a.ts", b"alpha\n"), ("ignored.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                unreadable: vec![root.join(".gitignore")],
                ..TextSearchResult::default()
            }
        );
    }

    fn io_error() -> ignore::Error {
        ignore::Error::Io(io::Error::other("boom"))
    }

    fn with_path(path: &str, err: ignore::Error) -> ignore::Error {
        ignore::Error::WithPath { path: path.into(), err: Box::new(err) }
    }

    #[rstest]
    #[case::with_path(with_path("/r/a", io_error()), &["/r/a"], 0)]
    #[case::path_under_depth(
        ignore::Error::WithDepth { depth: 2, err: Box::new(with_path("/r/a", io_error())) },
        &["/r/a"],
        0
    )]
    #[case::outer_path_over_line_number(
        with_path("/r/.gitignore", ignore::Error::WithLineNumber { line: 3, err: Box::new(io_error()) }),
        &["/r/.gitignore"],
        0
    )]
    #[case::line_number_around_path(
        ignore::Error::WithLineNumber { line: 3, err: Box::new(with_path("/r/.ignore", io_error())) },
        &["/r/.ignore"],
        0
    )]
    #[case::loop_names_its_child(ignore::Error::Loop { ancestor: "/r".into(), child: "/r/l/x".into() }, &["/r/l/x"], 0)]
    #[case::bare_io(io_error(), &[], 1)]
    #[case::bare_depth(ignore::Error::WithDepth { depth: 1, err: Box::new(io_error()) }, &[], 1)]
    #[case::glob(ignore::Error::Glob { glob: Some("a[".into()), err: "unclosed".into() }, &[], 1)]
    #[case::unrecognized_file_type(ignore::Error::UnrecognizedFileType("x".into()), &[], 1)]
    #[case::invalid_definition(ignore::Error::InvalidDefinition, &[], 1)]
    #[case::partial_each_part_once(
        ignore::Error::Partial(vec![
            with_path("/r/b", io_error()),
            io_error(),
            with_path("/r/a", io_error()),
            with_path("/r/b", io_error()),
            io_error(),
        ]),
        &["/r/b", "/r/a"],
        2
    )]
    fn a_walk_error_records_its_path_or_counts_as_pathless(
        #[case] err: ignore::Error,
        #[case] unreadable: &[&str],
        #[case] walk_errors_without_path: u64,
    ) {
        let mut result = TextSearchResult::default();
        record_walk_error(&err, &mut result);
        assert_eq!(
            result,
            TextSearchResult {
                unreadable: unreadable.iter().map(Utf8PathBuf::from).collect(),
                walk_errors_without_path,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn a_walk_error_path_already_listed_is_not_listed_again() {
        let mut result =
            TextSearchResult { unreadable: vec![Utf8PathBuf::from("/r/a")], ..TextSearchResult::default() };
        record_walk_error(&with_path("/r/a", io_error()), &mut result);
        assert_eq!(result.unreadable, [Utf8PathBuf::from("/r/a")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_walk_error_with_a_non_utf8_path_counts_as_pathless() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let err = ignore::Error::WithPath { path: OsStr::from_bytes(b"/r/\xff").into(), err: Box::new(io_error()) };
        let mut result = TextSearchResult::default();
        record_walk_error(&err, &mut result);
        assert_eq!(result, TextSearchResult { walk_errors_without_path: 1, ..TextSearchResult::default() });
    }

    #[test]
    fn the_files_kept_under_the_ceiling_are_the_first_in_walk_order() {
        let (_dir, root) = tree(&[("b.ts", b"alpha\n"), ("a/x.ts", b"alpha\n"), ("a.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_files(2)).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a/x.ts", vec![hit(1, "alpha")]), ("a.ts", vec![hit(1, "alpha")])]),
                files_total: 3,
                ceiling_hit: true,
                ..TextSearchResult::default()
            }
        );
        assert_eq!(search_files(&root, &["alpha"], &with_max_files(2)).unwrap(), result);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_are_listed_in_walk_order() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) =
            tree(&[("z.ts", b"alpha\n"), ("m/n.ts", b"alpha\n"), ("b.ts", b"alpha\n"), ("a.ts", b"alpha\n")]);
        for name in ["z.ts", "m/n.ts", "b.ts"] {
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o000)).unwrap();
        }
        if fs::read(root.join("b.ts")).is_ok() {
            // Running as root: permissions cannot make the file unreadable.
            return;
        }
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                unreadable: vec![root.join("b.ts"), root.join("m/n.ts"), root.join("z.ts")],
                ..TextSearchResult::default()
            }
        );
    }

    #[rstest]
    fn the_batch_size_changes_neither_the_kept_files_nor_the_counts(
        #[values(1, 2, 3, 7, 1000)] batch_size: usize,
        #[values(0, 1, 3, 5, usize::MAX)] max_files: usize,
    ) {
        let (_dir, root) = tree(&[
            ("a.ts", b"alpha\n"),
            ("b.ts", b"beta\n"),
            ("c/d.ts", b"alpha\nalpha\n"),
            ("c/e.bin", b"alpha\0"),
            ("f.ts", b"alpha\n"),
            ("g.ts", b"gamma\n"),
            ("h.ts", b"ALPHA\n"),
            ("i.ts", b"alpha\n"),
        ]);
        let opts = with_max_files(max_files);
        let result = search_in_batches(&root, &["alpha"], &opts, batch_size).unwrap();
        let kept = ["a.ts", "c/d.ts", "f.ts", "h.ts", "i.ts"].into_iter().take(max_files).collect::<Vec<_>>();
        assert_eq!(hit_files(&root, &result), kept);
        assert_eq!((result.files_total, result.ceiling_hit), (5, max_files < 5));
        assert_eq!(result, search_files(&root, &["alpha"], &opts).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn an_unlistable_directory_and_an_unreadable_file_are_listed_in_walk_order() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) =
            tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n"), ("z/c.ts", b"alpha\n"), ("zz.ts", b"alpha\n")]);
        for name in ["a.ts", "z"] {
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o000)).unwrap();
        }
        let readable = fs::read(root.join("a.ts")).is_ok();
        let listable = fs::read_dir(root.join("z")).is_ok();
        let result = (!readable && !listable).then(|| search_files(&root, &["alpha"], &TextSearchOptions::default()));
        fs::set_permissions(root.join("z"), fs::Permissions::from_mode(0o755)).unwrap();
        let Some(result) = result else {
            // Running as root: permissions cannot make the file unreadable.
            return;
        };
        assert_eq!(
            result.unwrap(),
            TextSearchResult {
                hits: hits(&root, &[("b.ts", vec![hit(1, "alpha")]), ("zz.ts", vec![hit(1, "alpha")])]),
                files_total: 2,
                unreadable: vec![root.join("a.ts"), root.join("z")],
                ..TextSearchResult::default()
            }
        );
    }

    #[rstest]
    #[case::below_the_matching_count(2, &["a.ts", "b.ts"], true)]
    #[case::at_the_matching_count(3, &["a.ts", "b.ts", "c.ts"], false)]
    #[case::above_the_matching_count(4, &["a.ts", "b.ts", "c.ts"], false)]
    #[case::zero(0, &[], true)]
    fn max_files_caps_the_kept_files_but_every_matching_file_is_counted(
        #[case] max_files: usize,
        #[case] kept: &[&str],
        #[case] ceiling_hit: bool,
    ) {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n"), ("c.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_files(max_files)).unwrap();
        let kept: Vec<_> = kept.iter().map(|name| (*name, vec![hit(1, "alpha")])).collect();
        assert_eq!(
            result,
            TextSearchResult { hits: hits(&root, &kept), files_total: 3, ceiling_hit, ..TextSearchResult::default() }
        );
    }

    #[rstest]
    #[case::one_match(b"alpha\n", 1, true)]
    #[case::no_match(b"beta\n", 0, false)]
    fn a_zero_ceiling_keeps_nothing_and_is_hit_only_by_a_match(
        #[case] contents: &[u8],
        #[case] files_total: u64,
        #[case] ceiling_hit: bool,
    ) {
        let (_dir, root) = tree(&[("a.ts", contents)]);
        let result = search_files(&root, &["alpha"], &with_max_files(0)).unwrap();
        assert_eq!(result, TextSearchResult { files_total, ceiling_hit, ..TextSearchResult::default() });
    }

    #[test]
    fn files_past_the_ceiling_count_when_a_matching_line_is_not_utf8() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha \xff\n"), ("c.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_files(0)).unwrap();
        assert_eq!(result, TextSearchResult { files_total: 3, ceiling_hit: true, ..TextSearchResult::default() });
    }

    #[test]
    fn files_without_a_matching_line_are_neither_kept_nor_counted() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"beta\n"), ("c.ts", b"alpha\n"), ("d.ts", b"")]);
        let result = search_files(&root, &["alpha"], &with_max_files(2)).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")]), ("c.ts", vec![hit(1, "alpha")])]),
                files_total: 2,
                ..TextSearchResult::default()
            }
        );
    }

    const TEST_TREE: &[(&str, &[u8])] = &[
        ("src/a.ts", b"alpha\n"),
        ("src/__tests__/b.ts", b"alpha\n"),
        ("test/c.ts", b"alpha\n"),
        ("tests/d.ts", b"alpha\n"),
        ("pkg/spec/e.ts", b"alpha\n"),
        ("src/f.test.ts", b"alpha\n"),
        ("src/g.test.tsx", b"alpha\n"),
        ("src/h.test.js", b"alpha\n"),
        ("src/i.spec.ts", b"alpha\n"),
        ("src/j.spec.tsx", b"alpha\n"),
        ("src/k.spec.js", b"alpha\n"),
        ("src/testing/l.ts", b"alpha\n"),
        ("src/test.ts", b"alpha\n"),
    ];

    #[test]
    fn test_files_are_skipped_unless_allowed() {
        let (_dir, root) = tree(TEST_TREE);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["src/a.ts", "src/test.ts", "src/testing/l.ts"]);
        assert_eq!(result.files_total, 3);
    }

    #[test]
    fn allowing_tests_searches_every_file() {
        let (_dir, root) = tree(TEST_TREE);
        let opts = TextSearchOptions { allow_tests: true, ..TextSearchOptions::default() };
        let result = search_files(&root, &["alpha"], &opts).unwrap();
        let mut expected: Vec<_> = TEST_TREE.iter().map(|(relative, _)| (*relative).to_string()).collect();
        expected.sort();
        assert_eq!(hit_files(&root, &result), expected);
    }

    #[rstest]
    #[case::test("test")]
    #[case::tests("tests")]
    #[case::spec("spec")]
    #[case::dunder_tests("__tests__")]
    fn a_root_inside_a_test_directory_is_still_searched(#[case] parent: &str) {
        let (_dir, root) = tree(&[]);
        let root = root.join(parent).join("project");
        write_files(&root, &[("src/a.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("src/a.ts", vec![hit(1, "alpha")])]));
    }

    #[rstest]
    #[case("__tests__/a.ts")]
    #[case("src/__tests__/a.ts")]
    #[case("test/a.ts")]
    #[case("src/test/a.ts")]
    #[case("tests/a.ts")]
    #[case("spec/a.ts")]
    #[case("src/spec/deep/a.ts")]
    #[case("a.test.ts")]
    #[case("src/a.test.tsx")]
    #[case("src/a.test.js")]
    #[case("src/a.spec.ts")]
    #[case("src/a.spec.tsx")]
    #[case("src/a.spec.js")]
    #[case("src/a.test.jsx")]
    #[case("src/a.test.mjs")]
    #[case("src/a.test.cjs")]
    #[case("src/a.test.mts")]
    #[case("src/a.test.cts")]
    #[case("src/a.spec.jsx")]
    #[case("src/a.spec.mjs")]
    #[case("src/a.spec.cjs")]
    #[case("src/a.spec.mts")]
    #[case("src/a.spec.cts")]
    fn test_paths_are_classified_as_tests(#[case] relative: &str) {
        assert!(is_test_path(Utf8Path::new(relative)));
    }

    #[rstest]
    #[case("a.ts")]
    #[case("src/a.ts")]
    #[case("test")]
    #[case("test.ts")]
    #[case("src/testing/a.ts")]
    #[case("src/contest/a.ts")]
    #[case("src/latest/a.ts")]
    #[case("src/specs/a.ts")]
    #[case("src/a.test.ts.map")]
    #[case("Test/a.ts")]
    #[case("src/Spec/a.ts")]
    #[case("src/a.Test.ts")]
    fn other_paths_are_not_classified_as_tests(#[case] relative: &str) {
        assert!(!is_test_path(Utf8Path::new(relative)));
    }

    fn without_ignore_files() -> TextSearchOptions {
        TextSearchOptions { respect_ignore_files: false, ..TextSearchOptions::default() }
    }

    #[test]
    fn ignore_files_and_hidden_files_are_skipped_when_respected() {
        let (_dir, root) = tree(&[
            (".ignore", b"b.ts\n"),
            ("a.ts", b"alpha\n"),
            ("b.ts", b"alpha\n"),
            ("sub/.gitignore", b"c.ts\n"),
            ("sub/c.ts", b"alpha\n"),
            ("sub/d.ts", b"alpha\n"),
            (".hidden.ts", b"alpha\n"),
            (".hidden/e.ts", b"alpha\n"),
        ]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts", "sub/d.ts"]);
        let result = search_files(&root, &["alpha"], &without_ignore_files()).unwrap();
        assert_eq!(hit_files(&root, &result), [".hidden/e.ts", ".hidden.ts", "a.ts", "b.ts", "sub/c.ts", "sub/d.ts"]);
    }

    #[rstest]
    #[case::without_a_git_directory(false)]
    #[case::in_a_git_repository(true)]
    fn gitignored_files_are_skipped(#[case] git_dir: bool) {
        let (_dir, root) = tree(&[(".gitignore", b"ignored.ts\n"), ("a.ts", b"alpha\n"), ("ignored.ts", b"alpha\n")]);
        if git_dir {
            fs::create_dir(root.join(".git")).unwrap();
        }
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts"]);
    }

    /// A repository at `repo` below a temp dir, holding `files` (paths relative to `repo`), with a `.git` directory.
    fn repo_tree(files: &[(&str, &[u8])]) -> (TempDir, Utf8PathBuf, Utf8PathBuf) {
        let (dir, top) = tree(&[]);
        let repo = top.join("repo");
        write_files(&repo, files);
        write_files(&repo, &[(".git/HEAD", b"ref: refs/heads/main\n")]);
        (dir, top, repo)
    }

    fn found(root: &Utf8Path, opts: &TextSearchOptions) -> Vec<String> {
        hit_files(root, &search_files(root, &["alpha"], opts).unwrap())
    }

    #[rstest]
    #[case::gitignore(".gitignore")]
    #[case::ignore(".ignore")]
    fn a_parent_ignore_file_inside_the_repository_applies(#[case] name: &str) {
        let (_dir, _top, repo) = repo_tree(&[
            (name, b"b.ts\n/sub/anchored.ts\n"),
            ("sub/a.ts", b"alpha\n"),
            ("sub/b.ts", b"alpha\n"),
            ("sub/anchored.ts", b"alpha\n"),
        ]);
        let root = repo.join("sub");
        assert_eq!(found(&root, &TextSearchOptions::default()), ["a.ts"]);
        assert_eq!(found(&root, &without_ignore_files()), ["a.ts", "anchored.ts", "b.ts"]);
    }

    #[test]
    fn parent_ignore_files_apply_at_every_level_and_the_nearer_one_wins() {
        let (_dir, _top, repo) = repo_tree(&[
            (".gitignore", b"*.gen.ts\nkeep.ts\n"),
            ("pkg/.gitignore", b"!keep.ts\nlocal.ts\n"),
            ("pkg/src/a.ts", b"alpha\n"),
            ("pkg/src/keep.ts", b"alpha\n"),
            ("pkg/src/local.ts", b"alpha\n"),
            ("pkg/src/x.gen.ts", b"alpha\n"),
        ]);
        assert_eq!(found(&repo.join("pkg/src"), &TextSearchOptions::default()), ["a.ts", "keep.ts"]);
    }

    #[test]
    fn rules_inside_the_root_win_over_parent_rules() {
        let (_dir, _top, repo) = repo_tree(&[
            (".gitignore", b"b.ts\nc.ts\n"),
            ("sub/.gitignore", b"!b.ts\n"),
            ("sub/a.ts", b"alpha\n"),
            ("sub/b.ts", b"alpha\n"),
            ("sub/c.ts", b"alpha\n"),
            ("sub/deep/b.ts", b"alpha\n"),
        ]);
        assert_eq!(found(&repo.join("sub"), &TextSearchOptions::default()), ["a.ts", "b.ts", "deep/b.ts"]);
    }

    /// The layout and result of `rg --files` run in `repo/sub` (ripgrep 15), with and without `--no-ignore`.
    #[test]
    fn parent_ignore_files_give_the_files_ripgrep_lists() {
        let (_dir, top, repo) = repo_tree(&[
            (".gitignore", b"b.ts\nc.ts\nanchored.ts\n"),
            ("sub/.gitignore", b"!c.ts\n"),
            ("sub/a.ts", b"alpha\n"),
            ("sub/anchored.ts", b"alpha\n"),
            ("sub/b.ts", b"alpha\n"),
            ("sub/c.ts", b"alpha\n"),
            ("sub/deep/b.ts", b"alpha\n"),
        ]);
        write_files(&top, &[(".gitignore", b"b.ts\n")]);
        let root = repo.join("sub");
        assert_eq!(found(&root, &TextSearchOptions::default()), ["a.ts", "c.ts"]);
        assert_eq!(found(&root, &without_ignore_files()), ["a.ts", "anchored.ts", "b.ts", "c.ts", "deep/b.ts"]);
    }

    #[test]
    fn a_parent_rule_naming_the_root_does_not_exclude_the_root() {
        let (_dir, _top, repo) = repo_tree(&[
            (".gitignore", b"sub/\nignored/\n"),
            ("sub/a.ts", b"alpha\n"),
            ("sub/ignored/b.ts", b"alpha\n"),
        ]);
        assert_eq!(found(&repo.join("sub"), &TextSearchOptions::default()), ["a.ts"]);
    }

    #[test]
    fn parent_ignore_files_apply_when_the_root_is_not_in_canonical_form() {
        let (_dir, _top, repo) =
            repo_tree(&[(".gitignore", b"b.ts\n"), ("sub/a.ts", b"alpha\n"), ("sub/b.ts", b"alpha\n")]);
        let root = repo.join("sub");
        let walked = root.join("../sub");
        assert_eq!(found(&walked, &TextSearchOptions::default()), ["a.ts"]);
    }

    #[rstest]
    #[case::ignore(".ignore")]
    #[case::gitignore(".gitignore")]
    fn ignore_files_above_the_repository_root_do_not_apply(#[case] name: &str) {
        let (_dir, top, repo) = repo_tree(&[("sub/a.ts", b"alpha\n"), ("sub/b.ts", b"alpha\n")]);
        write_files(&top, &[(name, b"b.ts\n")]);
        assert_eq!(found(&repo.join("sub"), &TextSearchOptions::default()), ["a.ts", "b.ts"]);
        assert_eq!(found(&repo, &TextSearchOptions::default()), ["sub/a.ts", "sub/b.ts"]);
    }

    #[test]
    fn an_unreadable_ignore_file_above_the_repository_root_is_never_opened() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, top, repo) = repo_tree(&[("sub/a.ts", b"alpha\n")]);
        write_files(&top, &[(".gitignore", b"a.ts\n")]);
        fs::set_permissions(top.join(".gitignore"), fs::Permissions::from_mode(0o000)).unwrap();
        let result = search_files(&repo.join("sub"), &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            (hit_files(&repo.join("sub"), &result), result.unreadable),
            (["a.ts"].map(String::from).to_vec(), vec![])
        );
    }

    #[rstest]
    #[case::ignore(".ignore")]
    #[case::gitignore(".gitignore")]
    fn parent_ignore_files_do_not_apply_without_a_git_ancestor(#[case] name: &str) {
        let (_dir, top) = tree(&[(name, b"b.ts\n"), ("sub/a.ts", b"alpha\n"), ("sub/b.ts", b"alpha\n")]);
        assert_eq!(found(&top.join("sub"), &TextSearchOptions::default()), ["a.ts", "b.ts"]);
    }

    #[test]
    fn a_git_file_marks_the_repository_root_too() {
        let (_dir, top) = tree(&[
            (".git", b"gitdir: /elsewhere\n"),
            (".gitignore", b"b.ts\n"),
            ("sub/a.ts", b"alpha\n"),
            ("sub/b.ts", b"alpha\n"),
        ]);
        assert_eq!(found(&top.join("sub"), &TextSearchOptions::default()), ["a.ts"]);
    }

    #[test]
    fn a_root_that_is_the_repository_root_reads_no_parent_files() {
        let (_dir, top) = tree(&[(".gitignore", b"a.ts\n"), ("repo/.git/HEAD", b"x\n"), ("repo/a.ts", b"alpha\n")]);
        assert_eq!(found(&top.join("repo"), &TextSearchOptions::default()), ["a.ts"]);
    }

    #[test]
    fn an_unparsable_parent_ignore_file_is_unreadable_and_its_valid_lines_still_apply() {
        let (_dir, _top, repo) =
            repo_tree(&[(".gitignore", b"[z-a]\nb.ts\n"), ("sub/a.ts", b"alpha\n"), ("sub/b.ts", b"alpha\n")]);
        let result = search_files(&repo.join("sub"), &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            (hit_files(&repo.join("sub"), &result), result.unreadable),
            (vec!["a.ts".to_string()], vec![repo.canonicalize_utf8().unwrap().join(".gitignore")])
        );
    }

    #[test]
    fn git_info_exclude_does_not_apply() {
        let (_dir, root) = tree(&[(".git/info/exclude", b"b.ts\n"), ("a.ts", b"alpha\n"), ("b.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts", "b.ts"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_root_that_is_a_symlink_to_a_directory_is_searched() {
        let (_dir, root) = tree(&[("real/a.ts", b"alpha\n")]);
        let link = root.join("link");
        std::os::unix::fs::symlink(root.join("real"), &link).unwrap();
        let result = search_files(&link, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&link, &[("a.ts", vec![hit(1, "alpha")])]));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_neither_followed_nor_counted() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("dir/b.ts", b"alpha\n")]);
        std::os::unix::fs::symlink(root.join("a.ts"), root.join("file-link.ts")).unwrap();
        std::os::unix::fs::symlink(root.join("dir"), root.join("dir-link")).unwrap();
        std::os::unix::fs::symlink(root.join("missing.ts"), root.join("dangling.ts")).unwrap();
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")]), ("dir/b.ts", vec![hit(1, "alpha")])]),
                files_total: 2,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn binary_files_are_skipped_without_being_reported() {
        // The second binary file's NUL lies well past the searcher's first buffer, after a match has been seen.
        let mut late_nul = b"alpha\n".to_vec();
        late_nul.extend(b"filler line\n".repeat(50_000));
        late_nul.extend(b"\0alpha\n");
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n\0alpha\n"), ("c.ts", &late_nul)]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[rstest]
    #[case::nul_after_the_invalid_line(b"alpha\nalpha \xff\n\0\n".to_vec())]
    #[case::nul_before_the_invalid_line(b"alpha\n\0\nalpha \xff\n".to_vec())]
    #[case::nul_past_the_first_buffer([b"alpha \xff\n".as_slice(), &b"filler line\n".repeat(50_000), b"\0\n"].concat())]
    fn a_binary_file_is_skipped_even_with_an_invalid_utf8_matching_line(#[case] contents: Vec<u8>) {
        let (_dir, root) = tree(&[("a.ts", &contents), ("b.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("b.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn invalid_utf8_in_a_matching_line_is_a_hit_with_the_bytes_replaced() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\nalpha \xff\n"), ("b.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(
                    &root,
                    &[("a.ts", vec![hit(1, "alpha"), hit(2, "alpha \u{FFFD}")]), ("b.ts", vec![hit(1, "alpha")])]
                ),
                files_total: 2,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn invalid_utf8_outside_matching_lines_is_ignored() {
        let (_dir, root) = tree(&[("a.ts", b"\xff beta\nalpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(2, "alpha")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_opened_is_unreadable_and_the_search_continues() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n")]);
        fs::set_permissions(root.join("a.ts"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(root.join("a.ts")).is_ok() {
            // Running as root: permissions cannot make the file unreadable.
            return;
        }
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("b.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                unreadable: vec![root.join("a.ts")],
                ..TextSearchResult::default()
            }
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn non_utf8_paths_are_skipped_and_counted() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // Counted whether or not the file would match or is a test, and directories are not counted.
        let (_dir, root) = tree(&[("a.ts", b"alpha\n")]);
        fs::write(root.as_std_path().join(OsStr::from_bytes(b"bad\xff.ts")), b"alpha\n").unwrap();
        fs::write(root.as_std_path().join(OsStr::from_bytes(b"other\xff.ts")), b"beta\n").unwrap();
        let dir = root.as_std_path().join(OsStr::from_bytes(b"dir\xfe"));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("c.ts"), b"alpha\n").unwrap();
        fs::create_dir_all(root.join("test")).unwrap();
        fs::write(root.as_std_path().join("test").join(OsStr::from_bytes(b"d\xff.ts")), b"alpha\n").unwrap();
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                non_utf8_paths: 4,
                ..TextSearchResult::default()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_excluded_test_file_is_never_opened() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[("src/a.ts", b"alpha\n"), ("src/test/a.ts", b"alpha\n")]);
        fs::set_permissions(root.join("src/test/a.ts"), fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read(root.join("src/test/a.ts")).is_ok();
        let outcome = search_files(&root, &["alpha"], &TextSearchOptions::default());
        fs::set_permissions(root.join("src/test/a.ts"), fs::Permissions::from_mode(0o644)).unwrap();
        if readable {
            // Running as root: permissions cannot make the file unreadable.
            return;
        }
        assert_eq!(
            outcome.unwrap(),
            TextSearchResult {
                hits: hits(&root, &[("src/a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn excluded_binary_and_invalid_utf8_test_files_appear_nowhere() {
        let (_dir, root) =
            tree(&[("a.ts", b"alpha\n"), ("test/b.ts", b"alpha\n\0\n"), ("src/c.test.ts", b"alpha \xff\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_test_directory_that_cannot_be_listed_is_still_unreadable() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("test/b.ts", b"alpha\n")]);
        fs::set_permissions(root.join("test"), fs::Permissions::from_mode(0o000)).unwrap();
        let listable = fs::read_dir(root.join("test")).is_ok();
        let outcome = search_files(&root, &["alpha"], &TextSearchOptions::default());
        fs::set_permissions(root.join("test"), fs::Permissions::from_mode(0o755)).unwrap();
        if listable {
            // Running as root: permissions cannot make the directory unreadable.
            return;
        }
        assert_eq!(
            outcome.unwrap(),
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                unreadable: vec![root.join("test")],
                ..TextSearchResult::default()
            }
        );
    }
}
