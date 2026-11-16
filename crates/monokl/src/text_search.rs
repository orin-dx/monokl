//! The first search stage: walks a root and returns, per file, the lines that match any query pattern.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::WalkBuilder;
use ignore::gitignore::Gitignore;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::analysis::lang::is_test_path;
use crate::error::{MonoklError, Result};
use crate::types::LineHit;

/// How many files each thread searches per batch. A batch ends in a wait for its slowest file, so a small batch costs
/// speed; a large one holds more hits at once.
pub(crate) const BATCH_FILES_PER_THREAD: usize = 64;

/// The largest file whose text a search may keep for analysis: 256 KiB. Source files are far smaller; a larger one is
/// streamed, not kept, and analysis opens it again.
pub(crate) const RETAINED_FILE_BYTES: usize = 256 * 1024;

/// The most bytes of source text a search keeps for analysis, over all files: 16 MiB, whatever the number of matching
/// files. It is the whole extra memory of keeping text. Text beyond it is dropped, not queued, and that file is opened
/// again by analysis, so the budget changes cost and never output.
pub(crate) const RETAINED_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// The default for [`TextSearchOptions::max_files`].
pub const DEFAULT_MAX_FILES: usize = 100_000;

/// The default for [`TextSearchOptions::max_file_bytes`]: 4 MiB.
///
/// It equals the length the TypeScript and Rust analyzers parse, so a file that is read can be analyzed and a larger
/// one, which could only come back as too large to parse, is not read at all. Reading costs about 2.5 times a file's
/// size in memory (40 files of 5 MB reached 479 MB, one 300 MB line 752 MB), and parsing costs 28 to 37 bytes per
/// source byte, so one thread's file in flight is bounded by about 150 MB. Over the limit, those two runs peak at 5 MB.
/// Hand-written source is far below it; a larger file is generated or bundled code.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// The default for [`TextSearchOptions::max_line_bytes`]: 4 KiB. Ordinary source lines are under 200 bytes, so only
/// generated and minified lines reach it.
pub const MAX_LINE_BYTES: usize = 4096;

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
    /// Searches test files too. When false, a file is skipped if [`is_test_path`] says its path below the root is a
    /// test file's. Matching is case-sensitive.
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
    /// The largest file, in bytes, that is read. A larger one is not opened, and is listed in
    /// [`oversize`](TextSearchResult::oversize). The size is checked just before the file is read.
    pub max_file_bytes: u64,
    /// The most bytes of a matching line a [`LineHit`] keeps. A longer line is cut at a UTF-8 character boundary and
    /// counted in [`truncated_lines`](TextSearchResult::truncated_lines); the line still matched as a whole.
    pub max_line_bytes: usize,
    /// When to stop searching. Files not yet started when it passes are not searched, and
    /// [`incomplete`](TextSearchResult::incomplete) says how many were. The first batch of files is always searched, so
    /// a deadline that has already passed still makes progress. `None` runs to completion.
    pub deadline: Option<Instant>,
}

impl Default for TextSearchOptions {
    /// Skips tests, honors ignore files, keeps up to [`DEFAULT_MAX_FILES`] files, reads files up to
    /// [`MAX_FILE_BYTES`], keeps up to [`MAX_LINE_BYTES`] of a line and has no deadline.
    fn default() -> Self {
        Self {
            allow_tests: false,
            respect_ignore_files: true,
            max_files: DEFAULT_MAX_FILES,
            max_file_bytes: MAX_FILE_BYTES,
            max_line_bytes: MAX_LINE_BYTES,
            deadline: None,
        }
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
    /// In walk order, keyed like `hits`: files larger than [`TextSearchOptions::max_file_bytes`], which were not read.
    pub oversize: Vec<Utf8PathBuf>,
    /// How many matching lines were longer than [`TextSearchOptions::max_line_bytes`] and kept cut.
    pub truncated_lines: u64,
    /// Set when [`TextSearchOptions::deadline`] passed before every file was searched.
    pub incomplete: Option<Incomplete>,
}

/// How far a search got when its deadline passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Incomplete {
    /// Files that were searched or skipped for their size.
    pub reached: u64,
    /// Files the walk offered.
    pub total: u64,
}

/// Searches every regular file under `root` (see [`search_files_where`] to skip some) for lines matching any of `patterns`, regexes matched case-insensitively
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
/// The walk lists every file once and holds only the paths. Each file is then stat-ed before it is read, and one larger
/// than [`TextSearchOptions::max_file_bytes`] is not read. Work and memory are bounded by the walk's file count,
/// `max_file_bytes` per file in flight and `max_line_bytes` per kept hit.
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
    search_files_where(root, patterns, opts, &|_| true)
}

/// [`search_files`] restricted to the files `wanted` accepts, which it asks before a file is opened.
///
/// A file `wanted` rejects is never opened, counted in [`files_total`](TextSearchResult::files_total) or the ceiling,
/// or listed in [`unreadable`](TextSearchResult::unreadable). `wanted` sees the path joined to `root`; for a path that
/// is not UTF-8 it sees the path with each invalid byte sequence replaced by U+FFFD, so a rejected file is not counted
/// in [`non_utf8_paths`](TextSearchResult::non_utf8_paths) either. Directories are always walked. Errors are those of
/// [`search_files`].
///
/// # Examples
///
/// ```
/// use camino::Utf8Path;
/// use monokl::text_search::{search_files_where, TextSearchOptions};
///
/// let dir = tempfile::tempdir()?;
/// std::fs::write(dir.path().join("a.ts"), "alpha\n")?;
/// std::fs::write(dir.path().join("a.txt"), "alpha\n")?;
/// let root = Utf8Path::from_path(dir.path()).ok_or("temp dir is not UTF-8")?;
///
/// let result =
///     search_files_where(root, &["alpha"], &TextSearchOptions::default(), &|path| path.extension() == Some("ts"))?;
/// assert_eq!(result.hits.keys().collect::<Vec<_>>(), [&root.join("a.ts")]);
/// assert_eq!(result.files_total, 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn search_files_where(
    root: &Utf8Path,
    patterns: &[&str],
    opts: &TextSearchOptions,
    wanted: &dyn Fn(&Utf8Path) -> bool,
) -> Result<TextSearchResult> {
    search_in_batches(root, patterns, opts, wanted, || rayon::current_num_threads() * BATCH_FILES_PER_THREAD)
}

/// The text of matching files, by path, kept while they were searched.
pub(crate) type Retained = FxHashMap<Utf8PathBuf, KeptText>;

/// A budget of bytes of source text that a search may keep.
pub(crate) struct Retention {
    remaining: AtomicUsize,
}

impl Retention {
    pub(crate) fn new(bytes: usize) -> Self {
        Self { remaining: AtomicUsize::new(bytes) }
    }

    /// Whether a file of `len` bytes is worth reading whole: it is small enough and the budget could still hold it.
    fn wants(&self, len: u64) -> bool {
        usize::try_from(len).is_ok_and(|len| len <= RETAINED_FILE_BYTES && len <= self.remaining())
    }

    /// Keeps `bytes`, charging the budget for the memory the buffer holds, its capacity and not its length, or drops
    /// them when less is left. A [`KeptText`] is the only way to hold text against the budget, so a buffer is never
    /// kept for less than it occupies.
    fn keep(&self, bytes: Vec<u8>) -> Option<KeptText> {
        let charged = bytes.capacity();
        self.remaining.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(charged)).ok()?;
        Some(KeptText { bytes, charged })
    }

    /// Gives back what `text` was charged, for text that is dropped instead of used.
    fn release(&self, text: KeptText) {
        let KeptText { bytes, charged } = text;
        drop(bytes);
        self.remaining.fetch_add(charged, Ordering::Relaxed);
    }

    pub(crate) fn remaining(&self) -> usize {
        self.remaining.load(Ordering::Relaxed)
    }
}

/// A file's bytes held against a [`Retention`] budget. Its fields are private and [`Retention::keep`] is the only way to
/// build one, so the budget is charged for the capacity of every buffer that is kept.
pub(crate) struct KeptText {
    bytes: Vec<u8>,
    /// What the budget was charged, which is the buffer's capacity.
    charged: usize,
}

impl KeptText {
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// How a search gets at its files: how it opens one, and whether it keeps the text of the files that match.
pub(crate) struct FileAccess<'a> {
    pub(crate) open: &'a (dyn Fn(&Utf8Path) -> io::Result<File> + Sync),
    pub(crate) retention: Option<&'a Retention>,
}

pub(crate) fn open_file(path: &Utf8Path) -> io::Result<File> {
    File::open(path)
}

/// [`search_files`] with the number of files searched before their outcomes are folded into the result. The result does
/// not depend on it; it bounds how many files' hits are held beyond the kept ones. `batch_size` is asked for only when
/// there is a file to search, because it comes from the thread count and looking that up starts rayon's pool.
fn search_in_batches(
    root: &Utf8Path,
    patterns: &[&str],
    opts: &TextSearchOptions,
    wanted: &dyn Fn(&Utf8Path) -> bool,
    batch_size: impl FnOnce() -> usize,
) -> Result<TextSearchResult> {
    let access = FileAccess { open: &open_file, retention: None };
    search_in_batches_with(root, patterns, opts, wanted, batch_size, &access).map(|(result, _)| result)
}

/// [`search_in_batches`] with the way files are opened, and the text of the matching files that `access.retention`
/// had room for. A kept file is read whole and searched in memory, so analysis does not open it again; any other file
/// is streamed through a fixed buffer. Both give the same hits.
pub(crate) fn search_in_batches_with(
    root: &Utf8Path,
    patterns: &[&str],
    opts: &TextSearchOptions,
    wanted: &dyn Fn(&Utf8Path) -> bool,
    batch_size: impl FnOnce() -> usize,
    access: &FileAccess<'_>,
) -> Result<(TextSearchResult, Retained)> {
    let matchers = compile(patterns)?;
    let root_error = |source| MonoklError::Walk { path: root.to_owned(), source };
    let metadata = std::fs::metadata(root).map_err(|e| root_error(ignore::Error::Io(e)))?;
    if !metadata.is_dir() {
        return Err(root_error(ignore::Error::Io(io::Error::from(io::ErrorKind::NotADirectory))));
    }
    // Listed here as well as by the walk, so a root that cannot be listed fails even when there is nothing to search.
    std::fs::read_dir(root).map_err(|e| root_error(ignore::Error::Io(e)))?;
    let mut result = TextSearchResult::default();
    let mut retained = Retained::default();
    let Some(matchers) = matchers else {
        return Ok((result, retained));
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
        let path = match Utf8PathBuf::from_path_buf(entry.into_path()) {
            Ok(path) if wanted(&path) => path,
            Ok(_) => continue,
            Err(path) => {
                if wanted(Utf8Path::new(&path.to_string_lossy())) {
                    result.non_utf8_paths += 1;
                }
                continue;
            }
        };
        if !opts.allow_tests && is_test_path(path.strip_prefix(root).unwrap_or(&path)) {
            continue;
        }
        items.push(WalkItem::File(path));
    }
    // Files are searched on separate threads a batch at a time and folded in walk order, so the result and the ceiling
    // never depend on which thread finished first. A file's hits are dropped as soon as it is past the ceiling, so one
    // batch's hits are held beyond the kept files, however many files match.
    let batch_size = if items.is_empty() { 1 } else { batch_size().max(1) };
    let total = items.iter().filter(|item| matches!(item, WalkItem::File(_))).count() as u64;
    let mut reached = 0_u64;
    let mut first = true;
    let mut items = items.into_iter();
    loop {
        let batch: Vec<WalkItem> = items.by_ref().take(batch_size).collect();
        if batch.is_empty() {
            break;
        }
        // The first batch is always searched, so a deadline that is already past still returns something.
        let deadline = if first { None } else { opts.deadline };
        first = false;
        let limits = FileLimits { max_file_bytes: opts.max_file_bytes, max_line_bytes: opts.max_line_bytes, deadline };
        let searched: Vec<Searched> = batch
            .into_par_iter()
            .map_init(
                || searcher_builder.build(),
                |searcher, item| match item {
                    WalkItem::File(path) => {
                        let outcome = search_file(searcher, &matchers, &path, &limits, access);
                        Searched::File(path, outcome)
                    }
                    WalkItem::WalkError(error) => Searched::WalkError(error),
                },
            )
            .collect();
        let mut stopped = false;
        for item in searched {
            match item {
                Searched::WalkError(error) => error.record(&mut result),
                Searched::File(_, FileOutcome::NotReached) => stopped = true,
                Searched::File(path, outcome) => {
                    reached += 1;
                    match outcome {
                        FileOutcome::Binary | FileOutcome::NotReached => {}
                        FileOutcome::Unreadable => result.unreadable.push(path),
                        FileOutcome::Oversize => result.oversize.push(path),
                        FileOutcome::Lines(hits, truncated, text) => {
                            result.truncated_lines += truncated;
                            if hits.is_empty() {
                                continue;
                            }
                            result.files_total += 1;
                            if result.hits.len() < opts.max_files {
                                if let Some(text) = text {
                                    retained.insert(path.clone(), text);
                                }
                                result.hits.insert(path, hits);
                            } else {
                                result.ceiling_hit = true;
                                if let (Some(text), Some(retention)) = (text, access.retention) {
                                    retention.release(text);
                                }
                            }
                        }
                    }
                }
            }
        }
        if stopped {
            result.incomplete = Some(Incomplete { reached, total });
            break;
        }
    }
    Ok((result, retained))
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

/// The patterns of a search compiled for the searcher and for locating matches.
struct Matchers {
    /// All patterns as one alternation, which the searcher runs to find the matching lines.
    all: RegexMatcher,
    /// Each pattern alone. The alternation takes the leftmost match and then skips past it, so one pattern's match can
    /// swallow another's (`class.*` over a line that also holds `m`); running each pattern on its own finds every
    /// pattern's matches.
    each: Vec<RegexMatcher>,
}

/// Compiles each pattern alone, then all together, so an invalid one always fails the search. `None` when there are no
/// patterns.
fn compile(patterns: &[&str]) -> Result<Option<Matchers>> {
    let mut builder = RegexMatcherBuilder::new();
    builder.case_insensitive(true);
    // `build_many` joins the patterns with `|` before parsing, so halves such as `(a` and `b)` would compile together.
    let each = patterns.iter().map(|pattern| builder.build(pattern)).collect::<std::result::Result<Vec<_>, _>>()?;
    if patterns.is_empty() {
        return Ok(None);
    }
    Ok(Some(Matchers { all: builder.build_many(patterns)?, each }))
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
    /// Larger than the size limit, so never read.
    Oversize,
    /// The deadline had passed before the file was started.
    NotReached,
    /// The file's matching lines, possibly none, how many of them were cut to the line limit, and the file's text when
    /// it was kept.
    Lines(Vec<LineHit>, u64, Option<KeptText>),
}

/// What bounds the search of one file.
#[derive(Clone, Copy)]
struct FileLimits {
    max_file_bytes: u64,
    max_line_bytes: usize,
    deadline: Option<Instant>,
}

fn search_file(
    searcher: &mut Searcher,
    matchers: &Matchers,
    path: &Utf8Path,
    limits: &FileLimits,
    access: &FileAccess<'_>,
) -> FileOutcome {
    if limits.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return FileOutcome::NotReached;
    }
    // The size comes from the open handle, so it is the size of the file that is then read and costs no second lookup.
    let Ok(file) = (access.open)(path) else { return FileOutcome::Unreadable };
    let len = match file.metadata() {
        Ok(metadata) if metadata.len() > limits.max_file_bytes => return FileOutcome::Oversize,
        Ok(metadata) => metadata.len(),
        Err(_) => return FileOutcome::Unreadable,
    };
    let mut sink = LineCollector::new(matchers, limits.max_line_bytes);
    if let Some(retention) = access.retention.filter(|retention| retention.wants(len)) {
        return search_in_memory(searcher, matchers, &file, len, retention, sink);
    }
    match searcher.search_file(&matchers.all, &file, &mut sink) {
        Err(_) => FileOutcome::Unreadable,
        Ok(()) if sink.binary => FileOutcome::Binary,
        Ok(()) => FileOutcome::Lines(sink.hits, sink.truncated, None),
    }
}

/// [`search_file`] for a file read whole into memory: the same outcome, and the text itself when the file matched and
/// `retention` has room for it. The `len` bytes the file had when it was opened are read, so a file that grew since is
/// searched as it was.
fn search_in_memory(
    searcher: &mut Searcher,
    matchers: &Matchers,
    file: &File,
    len: u64,
    retention: &Retention,
    mut sink: LineCollector<'_>,
) -> FileOutcome {
    let Ok(text) = read_up_to(file, len) else { return FileOutcome::Unreadable };
    // A binary file is dropped whole, so one scan for the NUL byte settles it before any line is matched.
    if text.contains(&0) {
        return FileOutcome::Binary;
    }
    match searcher.search_slice(&matchers.all, &text, &mut sink) {
        Err(_) => FileOutcome::Unreadable,
        Ok(()) if sink.binary => FileOutcome::Binary,
        Ok(()) => {
            let kept = if sink.hits.is_empty() { None } else { retention.keep(text) };
            FileOutcome::Lines(sink.hits, sink.truncated, kept)
        }
    }
}

/// Reads up to `len` bytes of `reader` in as few reads as the file allows, and none past `len`, so a file that has the
/// size it was found to have costs one read and no read that finds the end, and one that grew since is read as it was.
/// A file that shrank gives a buffer cut to what was read, so it does not hold the memory of the size it had.
fn read_up_to(mut reader: impl Read, len: u64) -> io::Result<Vec<u8>> {
    let len = usize::try_from(len).map_err(io::Error::other)?;
    let mut text = vec![0; len];
    let mut filled = 0;
    while filled < len {
        match reader.read(&mut text[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    text.truncate(filled);
    text.shrink_to_fit();
    Ok(text)
}

/// Collects matching lines with the byte offsets at which their matches start, and stops at the first sign of binary data so the caller
/// can discard the whole file, including lines matched before the NUL byte was reached. A matching line that is not
/// UTF-8 is kept with its invalid bytes replaced by U+FFFD, and does not stop the search, so a NUL later in the file
/// still marks it binary.
///
/// The searcher reports a whole line, so where each match starts comes from a second pass of the matcher over the line. The
/// searcher strips only the `\n` before matching, so a line ending in `\r\n` is matched here against its text without
/// the terminator: a pattern always sees exactly the text a hit reports.
struct LineCollector<'m> {
    matchers: &'m Matchers,
    hits: Vec<LineHit>,
    binary: bool,
    /// The most bytes of a line's text a hit keeps.
    max_line_bytes: usize,
    /// How many kept lines were cut to `max_line_bytes`.
    truncated: u64,
}

/// Where the matches of one line's text start.
struct LineMatches {
    /// The offsets at which a match starts before the cut, as ranges of consecutive offsets.
    starts: Vec<Range<usize>>,
    /// Whether the text holds any match, before or past the cut.
    any: bool,
}

impl<'m> LineCollector<'m> {
    fn new(matchers: &'m Matchers, max_line_bytes: usize) -> Self {
        Self { matchers, hits: Vec::new(), binary: false, max_line_bytes, truncated: 0 }
    }

    /// Records `line` as a hit when it matches. `searcher_matched` is the searcher's verdict, which is final unless the
    /// line ends in `\r\n`.
    fn line(&mut self, line_number: Option<u64>, line: &[u8], searcher_matched: bool) -> io::Result<()> {
        let crlf = line.ends_with(b"\r\n");
        if !searcher_matched && !crlf {
            return Ok(());
        }
        let text = without_line_terminator(line);
        let kept = cut_at_char_boundary(text, self.max_line_bytes);
        let found = self.find_matches(text, kept.len())?;
        if !found.any {
            // The searcher matched this line, so the matcher must find the match again.
            return if crlf { Ok(()) } else { Err(io::Error::other("matched line has no match")) };
        }
        let line_number = line_number
            .and_then(|line_number| usize::try_from(line_number).ok())
            .ok_or_else(|| io::Error::other("matched line has no usable line number"))?;
        self.truncated += u64::from(kept.len() < text.len());
        let lossy = String::from_utf8_lossy(kept);
        let match_starts = match lossy {
            Cow::Borrowed(_) => found.starts,
            Cow::Owned(_) => lossy_runs(kept, &found.starts, &mut || {}),
        };
        self.hits.push(LineHit { line_number, text: lossy.into_owned(), match_starts });
        Ok(())
    }

    /// Finds where the matches of each pattern in `text` start, `cut` being how many of its first bytes a hit keeps, and
    /// merges them. The search of a pattern stops at its first match that starts past the cut, so a line of many matches
    /// costs only what is kept. Starts at consecutive offsets are one range, so a pattern that matches every byte of a
    /// line stores one range, not one per byte, and at most every other byte can begin a range of its own.
    fn find_matches(&self, text: &[u8], cut: usize) -> io::Result<LineMatches> {
        let cut_short = cut < text.len();
        let mut found = LineMatches { starts: Vec::new(), any: false };
        for matcher in &self.matchers.each {
            let first_run = found.starts.len();
            matcher
                .find_iter(text, |found_match| {
                    found.any = true;
                    let start = found_match.start();
                    if cut_short && start >= cut {
                        return false;
                    }
                    match found.starts.get_mut(first_run..).and_then(<[_]>::last_mut) {
                        Some(run) if start < run.end => {}
                        Some(run) if run.end == start => run.end += 1,
                        _ => found.starts.push(start..start + 1),
                    }
                    true
                })
                .map_err(io::Error::other)?;
        }
        if self.matchers.each.len() > 1 {
            found.starts = union_of_runs(found.starts);
        }
        Ok(found)
    }
}

/// `runs` as ascending, non-touching ranges covering the same offsets.
fn union_of_runs(mut runs: Vec<Range<usize>>) -> Vec<Range<usize>> {
    runs.sort_by_key(|run| run.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(runs.len());
    for run in runs {
        match merged.last_mut() {
            Some(last) if run.start <= last.end => last.end = last.end.max(run.end),
            _ => merged.push(run),
        }
    }
    merged
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

/// Maps each run of consecutive offsets in `raw` to the same bytes of `String::from_utf8_lossy(raw)`: from where the
/// first offset lands to just past where the last one does. `runs` must be ascending and not empty ranges. `visit` is
/// called once for each chunk of `raw` that is read, so a test can count the work.
fn lossy_runs(raw: &[u8], runs: &[Range<usize>], visit: &mut dyn FnMut()) -> Vec<Range<usize>> {
    let ends = runs.iter().flat_map(|run| [run.start, run.end - 1]);
    let mapped = lossy_offsets(raw, ends, visit);
    mapped.as_chunks::<2>().0.iter().map(|&[first, last]| first..last + 1).collect()
}

/// Where each byte offset of `raw` falls in `String::from_utf8_lossy(raw)`, which replaces each invalid sequence with
/// one U+FFFD. An offset inside an invalid sequence lands at the start of its replacement, and one past the end of
/// `raw` at the end of the lossy text. `offsets` must not decrease: one forward pass over the chunks of `raw` answers
/// them all, so the work is the length of `raw` plus the number of offsets, not their product.
fn lossy_offsets(raw: &[u8], offsets: impl IntoIterator<Item = usize>, visit: &mut dyn FnMut()) -> Vec<usize> {
    let replacement = char::REPLACEMENT_CHARACTER.len_utf8();
    let mut chunks = raw.utf8_chunks();
    // Where the current chunk starts in `raw` and in the lossy text, and its valid and invalid lengths.
    let (mut raw_at, mut lossy_at) = (0, 0);
    let mut current: Option<(usize, usize)> = None;
    let mut mapped = Vec::new();
    for offset in offsets {
        let lossy = loop {
            let (valid, invalid) = if let Some(lengths) = current {
                lengths
            } else {
                let Some(chunk) = chunks.next() else { break lossy_at };
                visit();
                let lengths = (chunk.valid().len(), chunk.invalid().len());
                current = Some(lengths);
                lengths
            };
            if offset <= raw_at + valid {
                break lossy_at + (offset - raw_at);
            }
            if offset < raw_at + valid + invalid {
                break lossy_at + valid;
            }
            raw_at += valid + invalid;
            lossy_at += valid + if invalid > 0 { replacement } else { 0 };
            current = None;
        };
        mapped.push(lossy);
    }
    mapped
}

/// The first `max` bytes of `text`, or fewer so that a multi-byte character is never split.
fn cut_at_char_boundary(text: &[u8], max: usize) -> &[u8] {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    // A continuation byte is `10xxxxxx`; the cut goes before the lead byte of the character it would split.
    while end > 0 && text.get(end).is_some_and(|byte| byte & 0xC0 == 0x80) {
        end -= 1;
    }
    &text[..end]
}

/// Strips one trailing `\n` or `\r\n`. A `\r` not followed by `\n` is line content and stays.
fn without_line_terminator(line: &[u8]) -> &[u8] {
    match line.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use proptest::prelude::*;
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

    /// A hit on `text` for the pattern `alpha`: a start at each occurrence, ASCII case aside. Occurrences are five
    /// bytes wide, so no two starts are consecutive.
    fn hit(line_number: usize, text: &str) -> LineHit {
        let lower = text.to_ascii_lowercase();
        let match_starts = lower.match_indices("alpha").map(|(start, _)| start..start + 1).collect();
        LineHit { line_number, text: text.to_string(), match_starts }
    }

    fn hit_at(line_number: usize, text: &str, match_starts: &[Range<usize>]) -> LineHit {
        LineHit { line_number, text: text.to_string(), match_starts: match_starts.to_vec() }
    }

    /// A hit whose match starts form one run of consecutive offsets.
    fn hit_in(line_number: usize, text: &str, run: Range<usize>) -> LineHit {
        LineHit { line_number, text: text.to_string(), match_starts: std::iter::once(run).collect() }
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
    fn default_options_skip_tests_respect_ignore_files_and_bound_files_bytes_and_lines() {
        assert_eq!(
            TextSearchOptions::default(),
            TextSearchOptions {
                allow_tests: false,
                respect_ignore_files: true,
                max_files: 100_000,
                max_file_bytes: 4 * 1024 * 1024,
                max_line_bytes: 4096,
                deadline: None,
            }
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
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "alpha"), hit_in(3, "gamma", 0..1)])]));
    }

    #[test]
    fn patterns_are_regexes() {
        let (_dir, root) = tree(&[("a.ts", b"foo1\nfoo\nfooX2\n")]);
        let result = search_files(&root, &[r"foo\w?\d"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, "foo1", 0..1), hit_in(3, "fooX2", 0..1)])]));
    }

    #[test]
    fn a_dot_matches_any_character_rather_than_a_literal_dot() {
        let (_dir, root) = tree(&[("a.ts", b"abc\nac\na.c\n")]);
        let result = search_files(&root, &["a.c"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, "abc", 0..1), hit_in(3, "a.c", 0..1)])]));
    }

    #[test]
    fn case_folding_covers_non_ascii_letters() {
        let (_dir, root) = tree(&[("a.ts", "\u{e9}t\u{e9}\n\u{c9}T\u{c9}\nete\n".as_bytes())]);
        let result = search_files(&root, &["\u{e9}t\u{e9}"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(&root, &[("a.ts", vec![hit_in(1, "\u{e9}t\u{e9}", 0..1), hit_in(2, "\u{c9}T\u{c9}", 0..1)])])
        );
    }

    #[test]
    fn an_empty_pattern_matches_every_line() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n\nbeta\n")]);
        let result = search_files(&root, &[""], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(&root, &[("a.ts", vec![hit_in(1, "alpha", 0..6), hit_in(2, "", 0..1), hit_in(3, "beta", 0..5),])])
        );
    }

    #[test]
    fn a_hit_records_the_offset_where_each_match_starts() {
        let (_dir, root) = tree(&[("a.ts", b"alpha beta ALPHA\nbeta\ngamma alpha\n")]);
        let result = search_files(&root, &["alpha", "gamma"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(
                &root,
                &[(
                    "a.ts",
                    vec![hit_at(1, "alpha beta ALPHA", &[0..1, 11..12]), hit_at(3, "gamma alpha", &[0..1, 6..7])]
                )]
            )
        );
    }

    #[test]
    fn match_starts_count_bytes_not_characters() {
        let (_dir, root) = tree(&[("a.ts", "\u{e9}\u{e9} alpha \u{4e2d} alpha\n".as_bytes())]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(&root, &[("a.ts", vec![hit_at(1, "\u{e9}\u{e9} alpha \u{4e2d} alpha", &[5..6, 15..16])])])
        );
    }

    #[test]
    fn match_starts_count_the_bytes_of_the_text_with_invalid_sequences_replaced() {
        // Each invalid byte becomes a three-byte U+FFFD, so the starts differ from the offsets in the file's own bytes.
        let (_dir, root) = tree(&[("a.ts", b"\xff alpha \xfe\xfe alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(&root, &[("a.ts", vec![hit_at(1, "\u{FFFD} alpha \u{FFFD}\u{FFFD} alpha", &[4..5, 17..18])])])
        );
    }

    #[test]
    fn starts_at_consecutive_offsets_are_one_range() {
        let (_dir, root) = tree(&[("a.ts", b"aaa a aa\nabab ab\n")]);
        let result = search_files(&root, &["a"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(
                &root,
                &[(
                    "a.ts",
                    vec![hit_at(1, "aaa a aa", &[0..3, 4..5, 6..8]), hit_at(2, "abab ab", &[0..1, 2..3, 5..6])]
                )]
            )
        );
        let result = search_files(&root, &["ab"], &TextSearchOptions::default()).unwrap();
        // `ab` is two bytes wide, so its starts at 0, 2 and 5 are not consecutive.
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_at(2, "abab ab", &[0..1, 2..3, 5..6])])]));
    }

    #[test]
    fn a_match_of_one_pattern_does_not_hide_the_matches_of_another() {
        // One alternation would match `a.*` to the end of the line and never see `b`.
        let (_dir, root) = tree(&[("a.ts", b"xa b b\n")]);
        let result = search_files(&root, &["a.*", "b"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_at(1, "xa b b", &[1..2, 3..4, 5..6])])]));
        let reversed = search_files(&root, &["b", "a.*"], &TextSearchOptions::default()).unwrap();
        assert_eq!(reversed.hits, result.hits);
    }

    #[test]
    fn starts_of_different_patterns_that_touch_or_repeat_are_one_run() {
        let (_dir, root) = tree(&[("a.ts", b"abab\n")]);
        let result = search_files(&root, &["ab", "b", "a"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, "abab", 0..4)])]));
    }

    #[test]
    fn a_pattern_matching_every_byte_of_a_line_stores_one_run() {
        // One range per match would be 4096 ranges of 16 bytes each, 16 times the bytes of the line.
        let line = "x".repeat(MAX_LINE_BYTES);
        let (_dir, root) = tree(&[("a.ts", format!("{line}\n").as_bytes())]);
        let result = search_files(&root, &["."], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, &line, 0..MAX_LINE_BYTES)])]));
    }

    #[test]
    fn a_crlf_line_reports_starts_in_its_text_without_the_terminator() {
        let (_dir, root) = tree(&[("a.ts", b"beta alpha\r\nalpha alpha$\r\n")]);
        let result = search_files(&root, &["alpha$"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, "beta alpha", 5..6)])]));
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result.hits,
            hits(&root, &[("a.ts", vec![hit_in(1, "beta alpha", 5..6), hit_at(2, "alpha alpha$", &[0..1, 6..7])])])
        );
    }

    #[test]
    fn a_line_cut_to_the_limit_keeps_the_matches_that_start_before_the_cut() {
        // The limit is 8 bytes: the first match ends at the cut, the second starts past it.
        let (_dir, root) = tree(&[("a.ts", b"ab alpha alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_line_bytes(8)).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, "ab alpha", 3..4)])]));
        assert_eq!(result.truncated_lines, 1);
    }

    #[test]
    fn a_match_that_starts_before_the_cut_and_ends_after_it_is_kept() {
        let (_dir, root) = tree(&[("a.ts", b"ab alpha alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_line_bytes(6)).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_in(1, "ab alp", 3..4)])]));
    }

    #[test]
    fn a_line_whose_matches_all_start_past_the_cut_is_a_hit_with_no_starts() {
        let (_dir, root) = tree(&[("a.ts", b"0123456789 alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_line_bytes(4)).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit_at(1, "0123", &[])])]));
        assert_eq!(result.truncated_lines, 1);
    }

    /// Where byte `offset` of `raw` falls in `String::from_utf8_lossy(raw)`, found by walking the chunks from the start
    /// for this one offset. It costs a pass over the line for each offset, so only tests use it: it is the plain
    /// statement of what [`lossy_offsets`] computes in one pass.
    fn lossy_offset_from_the_start(raw: &[u8], offset: usize) -> usize {
        let (mut raw_at, mut lossy_at) = (0, 0);
        for chunk in raw.utf8_chunks() {
            let valid = chunk.valid().len();
            if offset <= raw_at + valid {
                return lossy_at + (offset - raw_at);
            }
            raw_at += valid;
            lossy_at += valid;
            let invalid = chunk.invalid().len();
            if invalid > 0 {
                if offset < raw_at + invalid {
                    return lossy_at;
                }
                raw_at += invalid;
                lossy_at += char::REPLACEMENT_CHARACTER.len_utf8();
            }
        }
        lossy_at
    }

    /// `ab\xe2\x82cd` reads `ab`, one truncated three-byte sequence of two bytes, and `cd`; lossily it is `ab\u{FFFD}cd`.
    #[rstest]
    #[case::before_the_sequence(2, 2)]
    #[case::inside_it(3, 2)]
    #[case::after_it(4, 5)]
    #[case::in_the_text_after_it(5, 6)]
    #[case::at_the_end(6, 7)]
    fn an_offset_in_raw_bytes_maps_to_the_same_place_in_the_lossy_text(#[case] offset: usize, #[case] expected: usize) {
        assert_eq!(String::from_utf8_lossy(b"ab\xe2\x82cd"), "ab\u{FFFD}cd");
        assert_eq!(lossy_offset_from_the_start(b"ab\xe2\x82cd", offset), expected);
        assert_eq!(lossy_offsets(b"ab\xe2\x82cd", [offset], &mut || {}), [expected]);
    }

    #[test]
    fn offsets_past_the_end_of_the_line_land_at_the_end_of_the_lossy_text() {
        assert_eq!(lossy_offsets(b"ab\xe2\x82cd", [6, 7, 9], &mut || {}), [7, 7, 7]);
        assert_eq!(lossy_offsets(b"", [0, 0, 1], &mut || {}), [0, 0, 0]);
        assert_eq!(lossy_offsets(b"abc", [], &mut || {}), Vec::<usize>::new());
    }

    /// Bytes that make valid text, invalid sequences, and truncated ones of every length in a line.
    fn raw_line() -> impl Strategy<Value = Vec<u8>> {
        let bytes = [b'a', b'b', b' ', 0xff, 0xfe, 0x80, 0xc3, 0xa9, 0xe2, 0x82, 0xac, 0xf0, 0x9f, 0x98, 0x80];
        prop::collection::vec(prop::sample::select(bytes.to_vec()), 0..48)
    }

    proptest! {
        /// One pass over the chunks gives each offset the answer a walk from the start gives it.
        #[test]
        fn one_pass_maps_every_offset_as_a_walk_from_the_start_does(
            raw in raw_line(),
            offsets in prop::collection::vec(0_usize..60, 0..24),
        ) {
            let mut offsets = offsets;
            offsets.sort_unstable();
            let expected: Vec<usize> = offsets.iter().map(|&offset| lossy_offset_from_the_start(&raw, offset)).collect();
            prop_assert_eq!(lossy_offsets(&raw, offsets, &mut || {}), expected);
        }

        /// A run maps to the lossy range from its first start to just past its last.
        #[test]
        fn runs_map_as_their_first_and_last_start_do_by_a_walk_from_the_start(
            raw in raw_line(),
            marks in prop::collection::btree_set(0_usize..48, 0..16),
        ) {
            let starts: Vec<usize> = marks.into_iter().collect();
            let mut runs: Vec<Range<usize>> = Vec::new();
            for start in starts {
                match runs.last_mut() {
                    Some(run) if run.end == start => run.end += 1,
                    _ => runs.push(start..start + 1),
                }
            }
            let expected: Vec<Range<usize>> = runs
                .iter()
                .map(|run| lossy_offset_from_the_start(&raw, run.start)..lossy_offset_from_the_start(&raw, run.end - 1) + 1)
                .collect();
            prop_assert_eq!(lossy_runs(&raw, &runs, &mut || {}), expected);
        }
    }

    /// The chunks of a line of `len` bytes alternating an invalid byte with `a`, and the one-byte ranges a search for
    /// `a` records, which are not consecutive.
    fn alternating(len: usize) -> (Vec<u8>, Vec<Range<usize>>) {
        let line: Vec<u8> = (0..len).map(|index| if index % 2 == 0 { 0xff } else { b'a' }).collect();
        let runs = (1..len).step_by(2).map(|at| at..at + 1).collect();
        (line, runs)
    }

    /// Mapping a line's ranges visits each chunk of the line at most once, so the work is the line plus the ranges, however
    /// many ranges there are. Counting the visits gives the same answer on every machine and load.
    #[rstest]
    #[case::one_times(1024)]
    #[case::two_times(2048)]
    #[case::four_times(4096)]
    fn mapping_the_ranges_of_a_line_of_invalid_bytes_visits_each_chunk_once(#[case] len: usize) {
        let (line, runs) = alternating(len);
        let (ranges, mut visits) = (runs.len(), 0_usize);
        let mapped = lossy_runs(&line, &runs, &mut || visits += 1);
        assert_eq!(mapped.len(), ranges);
        // Each invalid byte ends a chunk, so the line has `len / 2` of them and one more for the final `a`.
        assert_eq!(visits, len / 2 + 1);
        assert!(visits <= len + ranges, "{visits} visits for {len} bytes and {ranges} ranges");
    }

    #[test]
    fn the_ranges_of_an_alternating_line_land_on_the_a_after_each_replacement() {
        // `\xff a \xff a`: each replacement is three bytes, so the `a` at raw offset 1 is at lossy offset 3, then 7.
        let (line, runs) = alternating(4);
        assert_eq!(String::from_utf8_lossy(&line), "\u{FFFD}a\u{FFFD}a");
        assert_eq!(lossy_runs(&line, &runs, &mut || {}), [3..4, 7..8]);
    }

    #[test]
    fn a_search_on_a_line_of_alternating_invalid_bytes_records_each_a_at_its_lossy_offset() {
        let line: Vec<u8> = b"\xffa".repeat(6);
        let (_dir, root) = tree(&[("a.ts", &[line.as_slice(), b"\n"].concat())]);
        let result = search_files(&root, &["a"], &TextSearchOptions::default()).unwrap();
        let expected = hit_at(1, &"\u{FFFD}a".repeat(6), &[3..4, 7..8, 11..12, 15..16, 19..20, 23..24]);
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![expected])]));
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
    #[case::empty_crlf_line(b"alpha\r\n\r\n", "^$", vec![hit_in(2, "", 0..1)])]
    #[case::end_anchor_before_crlf(b"alpha\r\nbeta\r\n", "ta$", vec![hit_in(2, "beta", 2..3)])]
    #[case::lone_carriage_return_is_content(b"a\rc\n", r"a\rc$", vec![hit_in(1, "a\rc", 0..1)])]
    #[case::dot_matches_a_lone_carriage_return(b"a\rc\r\n", "^a.c$", vec![hit_in(1, "a\rc", 0..1)])]
    #[case::lone_carriage_return_is_not_a_line_boundary(b"alpha\rbeta\r\n", "^beta", vec![])]
    #[case::carriage_return_before_crlf_is_content(b"alpha\r\r\n", r"^alpha\r$", vec![hit_in(1, "alpha\r", 0..1)])]
    #[case::carriage_return_at_end_of_file_is_content(b"alpha\r", r"^alpha\r$", vec![hit_in(1, "alpha\r", 0..1)])]
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

    /// Searches for `alpha` the way the pipeline does: keeping the matching files' text, up to `budget` bytes. Returns
    /// the result, the kept text by relative path, and how many files were opened.
    fn search_retaining(
        root: &Utf8Path,
        opts: &TextSearchOptions,
        budget: usize,
    ) -> (TextSearchResult, BTreeMap<String, Vec<u8>>, u64) {
        use std::sync::atomic::{AtomicU64, Ordering};

        let opens = AtomicU64::new(0);
        let open = |path: &Utf8Path| {
            opens.fetch_add(1, Ordering::Relaxed);
            fs::File::open(path)
        };
        let retention = Retention::new(budget);
        let access = FileAccess { open: &open, retention: Some(&retention) };
        let (result, retained) = search_in_batches_with(root, &["alpha"], opts, &|_| true, || 4, &access).unwrap();
        let retained = retained
            .into_iter()
            .map(|(path, bytes)| (path.strip_prefix(root).unwrap().to_string(), bytes.into_bytes()))
            .collect();
        (result, retained, opens.load(Ordering::Relaxed))
    }

    #[test]
    fn a_matching_file_is_opened_once_and_its_text_is_handed_back_whole() {
        let (_dir, root) = tree(&[
            ("a.ts", b"alpha\n"),
            ("b.ts", b"beta\n"),
            ("c/d.ts", b"x\nALPHA\r\n"),
            ("e.bin", b"alpha\0"),
            ("f.ts", b""),
        ]);
        let (result, retained, opens) = search_retaining(&root, &TextSearchOptions::default(), 1 << 20);
        assert_eq!(opens, 5);
        assert_eq!(hit_files(&root, &result), ["a.ts", "c/d.ts"]);
        let expected: BTreeMap<String, Vec<u8>> =
            [("a.ts".to_owned(), b"alpha\n".to_vec()), ("c/d.ts".to_owned(), b"x\nALPHA\r\n".to_vec())].into();
        assert_eq!(retained, expected);
    }

    #[rstest]
    #[case::plain(b"alpha\nbeta\nalpha\n")]
    #[case::no_final_newline(b"x\nalpha")]
    #[case::crlf(b"alpha\r\nx\r\nALPHA\r\n")]
    #[case::bom(b"\xEF\xBB\xBFalpha\n")]
    #[case::invalid_utf8(b"alpha \xFF\xFE\nalpha\n")]
    #[case::nul_late(b"alpha\nbeta\nalpha\0\n")]
    #[case::utf16(b"\xFF\xFEa\0l\0p\0h\0a\0\n\0")]
    #[case::empty(b"")]
    #[case::only_newlines(b"\n\n\n")]
    fn a_file_searched_from_memory_gives_what_a_streamed_search_gives(#[case] contents: &[u8]) {
        let (_dir, root) = tree(&[("a.ts", contents)]);
        let opts = TextSearchOptions::default();
        let (retained_result, _, _) = search_retaining(&root, &opts, 1 << 20);
        assert_eq!(retained_result, search_files(&root, &["alpha"], &opts).unwrap());
    }

    #[test]
    fn a_nul_far_past_the_first_buffer_makes_a_file_binary_in_memory_as_it_does_streamed() {
        let mut contents = b"alpha\n".to_vec();
        contents.extend(std::iter::repeat_n(b'x', 200_000));
        contents.extend_from_slice(b"\n\0\n");
        let (_dir, root) = tree(&[("a.ts", &contents)]);
        let opts = TextSearchOptions::default();
        let (result, retained, _) = search_retaining(&root, &opts, 1 << 20);
        assert_eq!(result, TextSearchResult::default());
        assert!(retained.is_empty());
        assert_eq!(result, search_files(&root, &["alpha"], &opts).unwrap());
    }

    #[test]
    fn text_is_kept_only_up_to_the_budget_and_every_file_is_still_found() {
        let line = format!("alpha {}\n", "x".repeat(93));
        assert_eq!(line.len(), 100);
        let files: Vec<(String, Vec<u8>)> = (0..6).map(|i| (format!("f{i}.ts"), line.clone().into_bytes())).collect();
        let borrowed: Vec<(&str, &[u8])> = files.iter().map(|(name, text)| (name.as_str(), text.as_slice())).collect();
        let (_dir, root) = tree(&borrowed);
        let opts = TextSearchOptions::default();
        let (result, retained, _) = search_retaining(&root, &opts, 250);
        assert_eq!(retained.len(), 2);
        assert!(retained.values().all(|bytes| bytes.as_slice() == line.as_bytes()));
        assert_eq!(result, search_files(&root, &["alpha"], &opts).unwrap());
        assert_eq!(result.files_total, 6);
    }

    #[test]
    fn a_file_over_the_per_file_limit_is_found_but_its_text_is_not_kept() {
        let mut big = b"alpha\n".to_vec();
        big.resize(RETAINED_FILE_BYTES + 1, b'x');
        let (_dir, root) = tree(&[("big.ts", &big), ("small.ts", b"alpha\n")]);
        let (result, retained, opens) = search_retaining(&root, &TextSearchOptions::default(), usize::MAX / 2);
        assert_eq!(hit_files(&root, &result), ["big.ts", "small.ts"]);
        assert_eq!(retained.keys().collect::<Vec<_>>(), ["small.ts"]);
        assert_eq!(opens, 2);
    }

    #[test]
    fn a_file_past_the_ceiling_keeps_no_text_and_gives_its_budget_back() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n"), ("c.ts", b"alpha\n")]);
        let retention = Retention::new(1000);
        let open = |path: &Utf8Path| fs::File::open(path);
        let access = FileAccess { open: &open, retention: Some(&retention) };
        let (result, retained) =
            search_in_batches_with(&root, &["alpha"], &with_max_files(1), &|_| true, || 1, &access).unwrap();
        assert_eq!((result.files_total, result.ceiling_hit), (3, true));
        let kept: Vec<_> = retained.keys().map(|path| path.file_name().unwrap()).collect();
        assert_eq!(kept, ["a.ts"]);
        assert_eq!(retention.remaining(), 1000 - "alpha\n".len());
    }

    #[test]
    fn a_file_that_shrank_is_read_to_what_it_has_and_its_buffer_is_cut_to_match() {
        let bytes = read_up_to(&b"alpha"[..], 100_000).unwrap();
        assert_eq!(bytes, b"alpha");
        assert!(bytes.capacity() < 100_000, "capacity {} still holds the size the file had", bytes.capacity());
    }

    #[test]
    fn a_file_that_grew_is_read_no_further_than_the_size_it_was_found_to_have() {
        let data = [b'x'; 2000];
        let mut source: &[u8] = &data;
        let bytes = read_up_to(&mut source, 1000).unwrap();
        assert_eq!(bytes.len(), 1000);
        assert_eq!(source.len(), 1000);
    }

    #[test]
    fn a_buffer_is_charged_for_its_capacity_and_a_full_budget_refuses_it() {
        let retention = Retention::new(1000);
        let mut bytes = Vec::with_capacity(400);
        bytes.extend_from_slice(b"alpha");
        let capacity = bytes.capacity();
        let kept = retention.keep(bytes).unwrap();
        assert_eq!(kept.charged, capacity);
        assert_eq!(retention.remaining(), 1000 - capacity);

        let refused = retention.keep(Vec::with_capacity(700));
        assert!(refused.is_none());
        assert_eq!(retention.remaining(), 1000 - capacity);

        retention.release(kept);
        assert_eq!(retention.remaining(), 1000);
    }

    /// Buffers that were allocated for a larger file than they ended up holding cost their capacity, so however many
    /// there are, what is kept never passes the budget.
    #[test]
    fn many_buffers_with_spare_capacity_never_keep_more_than_the_budget() {
        let retention = Retention::new(RETAINED_SOURCE_BYTES);
        let buffers = (0..300).map(|_| {
            let mut bytes = Vec::with_capacity(100_000);
            bytes.push(b'a');
            bytes
        });
        let kept: Vec<KeptText> = buffers.filter_map(|bytes| retention.keep(bytes)).collect();
        let held: usize = kept.iter().map(|text| text.bytes.capacity()).sum();
        assert!(kept.iter().all(|text| text.charged == text.bytes.capacity()));
        assert_eq!(held, RETAINED_SOURCE_BYTES - retention.remaining());
        assert_eq!(kept.len(), RETAINED_SOURCE_BYTES / kept[0].bytes.capacity());
        assert!(held <= RETAINED_SOURCE_BYTES);
    }

    #[test]
    fn a_search_that_keeps_nothing_reads_like_the_public_one() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"beta\n")]);
        let (result, retained, opens) = search_retaining(&root, &TextSearchOptions::default(), 0);
        assert!(retained.is_empty());
        assert_eq!(opens, 2);
        assert_eq!(result, search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap());
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
        let result = search_in_batches(&root, &["alpha"], &opts, &|_| true, || batch_size).unwrap();
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
    #[case("tests.rs")]
    #[case("src/analysis/rust/tests.rs")]
    #[case("src/test.rs")]
    #[case("src/cart_tests.rs")]
    #[case("src/cart_test.rs")]
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
    #[case("src/contests.rs")]
    #[case("src/latest.rs")]
    #[case("src/tests_helper.rs")]
    #[case("src/tests.rs.bak")]
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

    fn ts_only(path: &Utf8Path) -> bool {
        path.extension() == Some("ts")
    }

    #[test]
    fn a_rejected_file_is_not_counted_and_does_not_use_the_ceiling() {
        let (_dir, root) = tree(&[
            ("a.txt", b"alpha\n"),
            ("b.json", b"alpha\n"),
            ("c.ts", b"alpha\n"),
            ("d.ts", b"alpha\n"),
            ("e.ts", b"alpha\n"),
        ]);
        let result = search_files_where(&root, &["alpha"], &with_max_files(2), &ts_only).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("c.ts", vec![hit(1, "alpha")]), ("d.ts", vec![hit(1, "alpha")])]),
                files_total: 3,
                ceiling_hit: true,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn the_filter_sees_the_path_below_the_root_and_applies_with_a_batch_of_one() {
        let (_dir, root) = tree(&[("src/a.ts", b"alpha\n"), ("src/a.txt", b"alpha\n"), ("b.ts", b"alpha\n")]);
        let seen = Mutex::new(Vec::new());
        let wanted = |path: &Utf8Path| {
            seen.lock().unwrap().push(path.to_owned());
            ts_only(path)
        };
        let result = search_in_batches(&root, &["alpha"], &TextSearchOptions::default(), &wanted, || 1).unwrap();
        assert_eq!(hit_files(&root, &result), ["b.ts", "src/a.ts"]);
        assert_eq!(result.files_total, 2);
        assert_eq!(*seen.lock().unwrap(), [root.join("b.ts"), root.join("src/a.ts"), root.join("src/a.txt")]);
    }

    #[test]
    fn accepting_every_file_is_search_files() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.txt", b"alpha\n"), ("test/c.ts", b"alpha\n")]);
        let opts = TextSearchOptions::default();
        assert_eq!(
            search_files_where(&root, &["alpha"], &opts, &|_| true).unwrap(),
            search_files(&root, &["alpha"], &opts).unwrap()
        );
    }

    #[test]
    fn a_rejected_test_file_and_an_accepted_one_follow_the_test_rules() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("test/b.ts", b"alpha\n"), ("test/c.txt", b"alpha\n")]);
        let skipped = search_files_where(&root, &["alpha"], &TextSearchOptions::default(), &ts_only).unwrap();
        assert_eq!(hit_files(&root, &skipped), ["a.ts"]);
        let opts = TextSearchOptions { allow_tests: true, ..TextSearchOptions::default() };
        let allowed = search_files_where(&root, &["alpha"], &opts, &ts_only).unwrap();
        assert_eq!(hit_files(&root, &allowed), ["a.ts", "test/b.ts"]);
        assert_eq!(allowed.files_total, 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_rejected_file_is_never_opened() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.txt", b"alpha\n"), ("c.bin", b"alpha\n\0")]);
        for name in ["b.txt", "c.bin"] {
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o000)).unwrap();
        }
        let readable = fs::read(root.join("b.txt")).is_ok();
        let outcome = search_files_where(&root, &["alpha"], &TextSearchOptions::default(), &ts_only);
        let unfiltered = search_files(&root, &["alpha"], &TextSearchOptions::default());
        for name in ["b.txt", "c.bin"] {
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o644)).unwrap();
        }
        if readable {
            // Running as root: permissions cannot make the files unreadable.
            return;
        }
        assert_eq!(
            outcome.unwrap(),
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
        // Without the filter the same files are opened and listed.
        assert_eq!(unfiltered.unwrap().unreadable, [root.join("b.txt"), root.join("c.bin")]);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_non_utf8_path_is_counted_only_when_the_filter_accepts_it() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let (_dir, root) = tree(&[("a.ts", b"alpha\n")]);
        fs::write(root.as_std_path().join(OsStr::from_bytes(b"bad\xff.ts")), b"alpha\n").unwrap();
        fs::write(root.as_std_path().join(OsStr::from_bytes(b"bad\xff.txt")), b"alpha\n").unwrap();
        let result = search_files_where(&root, &["alpha"], &TextSearchOptions::default(), &ts_only).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                non_utf8_paths: 1,
                ..TextSearchResult::default()
            }
        );
    }

    fn with_max_file_bytes(max_file_bytes: u64) -> TextSearchOptions {
        TextSearchOptions { max_file_bytes, ..TextSearchOptions::default() }
    }

    #[test]
    fn a_file_over_the_size_limit_is_not_read_and_is_listed() {
        let (_dir, root) = tree(&[("big.ts", b"alpha 12345"), ("small.ts", b"alpha")]);
        let result = search_files(&root, &["alpha"], &with_max_file_bytes(10)).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("small.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                oversize: vec![root.join("big.ts")],
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn a_file_exactly_at_the_size_limit_is_read() {
        let (_dir, root) = tree(&[("a.ts", b"alpha 1234")]);
        let result = search_files(&root, &["alpha"], &with_max_file_bytes(10)).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha 1234")])]),
                files_total: 1,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn an_oversize_file_without_a_match_is_still_listed() {
        let (_dir, root) = tree(&[("big.ts", b"nothing to see")]);
        let result = search_files(&root, &["alpha"], &with_max_file_bytes(4)).unwrap();
        assert_eq!(result.oversize, [root.join("big.ts")]);
    }

    #[test]
    fn the_default_size_limit_is_four_mebibytes() {
        let (_dir, root) = tree(&[]);
        let limit = usize::try_from(MAX_FILE_BYTES).unwrap();
        let mut at_limit = b"alpha\n".to_vec();
        at_limit.resize(limit, b'x');
        let mut over_limit = at_limit.clone();
        over_limit.push(b'x');
        write_files(&root, &[("at.ts", &at_limit), ("over.ts", &over_limit)]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["at.ts"]);
        assert_eq!(result.oversize, [root.join("over.ts")]);
    }

    fn with_max_line_bytes(max_line_bytes: usize) -> TextSearchOptions {
        TextSearchOptions { max_line_bytes, ..TextSearchOptions::default() }
    }

    #[test]
    fn a_long_hit_line_is_cut_on_a_char_boundary_and_counted() {
        // Byte 10 falls inside the second two-byte character.
        let (_dir, root) = tree(&[("a.ts", "alpha \u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\nalpha short\n".as_bytes())]);
        let result = search_files(&root, &["alpha"], &with_max_line_bytes(10)).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha \u{e9}\u{e9}"), hit(2, "alpha shor")])]),
                files_total: 1,
                truncated_lines: 2,
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn a_hit_line_at_the_line_limit_is_kept_whole() {
        let (_dir, root) = tree(&[("a.ts", b"alpha 1234\nalpha 12345\n")]);
        let result = search_files(&root, &["alpha"], &with_max_line_bytes(10)).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, "alpha 1234"), hit(2, "alpha 1234")])]));
        assert_eq!(result.truncated_lines, 1);
    }

    #[test]
    fn the_default_line_limit_is_four_kibibytes_and_leaves_normal_lines_alone() {
        let long = format!("alpha {}", "x".repeat(5000));
        let normal = format!("alpha {}", "y".repeat(100));
        let (_dir, root) = tree(&[("a.ts", format!("{long}\n{normal}\n").as_bytes())]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.hits, hits(&root, &[("a.ts", vec![hit(1, &long[..4096]), hit(2, &normal)])]));
        assert_eq!(result.truncated_lines, 1);
    }

    /// Text search asks `wanted` once for each file and no more, however many files there are, so the walk and the
    /// filter cost linear work. Counting the calls gives the same answer on every machine and load.
    #[rstest]
    #[case::few(250)]
    #[case::four_times_as_many(1000)]
    fn the_filter_is_asked_once_for_each_file(#[case] count: usize) {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let names: Vec<String> = (0..count).map(|index| format!("d{:02}/f{index:04}.ts", index % 20)).collect();
        let files: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), b"alpha\n".as_slice())).collect();
        let (_dir, root) = tree(&files);
        let asked = AtomicUsize::new(0);
        let wanted = |_: &Utf8Path| {
            asked.fetch_add(1, Ordering::Relaxed);
            true
        };
        let result = search_in_batches(&root, &["alpha"], &TextSearchOptions::default(), &wanted, || 16).unwrap();
        assert_eq!(result.files_total, count as u64);
        assert_eq!(asked.load(Ordering::Relaxed), count);
    }

    #[test]
    fn a_deadline_that_has_passed_still_searches_the_first_batch_and_reports_how_far_it_got() {
        let names = ["a.ts", "b.ts", "c.ts", "d.ts", "e.ts"];
        let files: Vec<(&str, &[u8])> = names.iter().map(|name| (*name, b"alpha\n".as_slice())).collect();
        let (_dir, root) = tree(&files);
        let opts = TextSearchOptions { deadline: Some(Instant::now()), ..TextSearchOptions::default() };
        let result = search_in_batches(&root, &["alpha"], &opts, &|_| true, || 2).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")]), ("b.ts", vec![hit(1, "alpha")])]),
                files_total: 2,
                incomplete: Some(Incomplete { reached: 2, total: 5 }),
                ..TextSearchResult::default()
            }
        );
    }

    #[test]
    fn a_deadline_far_in_the_future_changes_nothing() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n"), ("c.ts", b"alpha\n")]);
        let far = Instant::now() + std::time::Duration::from_secs(3600);
        let with_deadline = TextSearchOptions { deadline: Some(far), ..TextSearchOptions::default() };
        let timed = search_in_batches(&root, &["alpha"], &with_deadline, &|_| true, || 1).unwrap();
        let untimed = search_in_batches(&root, &["alpha"], &TextSearchOptions::default(), &|_| true, || 1).unwrap();
        assert_eq!(timed, untimed);
        assert_eq!(timed.incomplete, None);
    }

    #[test]
    fn the_batch_size_is_not_asked_for_when_no_file_is_wanted() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.txt", b"alpha\n")]);
        let ask =
            || -> usize { panic!("the batch size depends on the thread count, whose lookup starts rayon's pool") };
        let result = search_in_batches(&root, &["alpha"], &TextSearchOptions::default(), &|_| false, ask).unwrap();
        assert_eq!(result, TextSearchResult::default());
    }
}
