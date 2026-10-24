//! The first search stage: walks a root and returns, per file, the lines that match any query pattern.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::WalkBuilder;

use crate::error::{MonoklError, Result};
use crate::types::LineHit;

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
    /// directory. Ignore files above the root, `.git/info/exclude` and the user's global excludes are never read, so
    /// the same tree gives the same result on every machine.
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
pub struct TextSearchResult {
    /// The kept files, keyed by `root` joined with the path below it, each with its matching lines in line order.
    pub hits: BTreeMap<Utf8PathBuf, Vec<LineHit>>,
    /// Every file with at least one matching line, kept or not.
    pub files_total: u64,
    /// Whether `files_total` exceeded [`TextSearchOptions::max_files`], so some matching files have no hits stored.
    pub ceiling_hit: bool,
    /// In walk order and each listed once, keyed like `hits`: files that could not be read or whose matching lines are
    /// not UTF-8, directories that could not be listed, and ignore files that could not be read or parsed.
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
/// matching line is also not UTF-8. A file that cannot be read, or whose matching line is not UTF-8, and any walk
/// failure below `root`, go in [`unreadable`](TextSearchResult::unreadable) and the search continues. No patterns
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
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .bom_sniffing(false)
        .passthru(true)
        .build();
    // Only ignore files inside the root apply, so the same tree gives the same result on every machine.
    let walk = WalkBuilder::new(root)
        .standard_filters(opts.respect_ignore_files)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .sort_by_file_name(std::cmp::Ord::cmp)
        .build();
    for entry in walk {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) if err.depth() == Some(0) => return Err(root_error(err)),
            Err(err) => {
                record_walk_error(&err, &mut result);
                continue;
            }
        };
        if let Some(err) = entry.error() {
            record_walk_error(err, &mut result);
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
        match search_file(&mut searcher, &matcher, &path) {
            FileOutcome::Binary => {}
            FileOutcome::Unreadable => result.unreadable.push(path),
            FileOutcome::Lines(hits) => {
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
    Ok(result)
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
    match err {
        ignore::Error::Partial(errs) => {
            for err in errs {
                record_walk_error(err, result);
            }
        }
        ignore::Error::WithPath { path, .. } | ignore::Error::Loop { child: path, .. } => {
            match Utf8Path::from_path(path) {
                Some(path) if result.unreadable.iter().any(|listed| listed == path) => {}
                Some(path) => result.unreadable.push(path.to_owned()),
                None => result.walk_errors_without_path += 1,
            }
        }
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            record_walk_error(err, result);
        }
        ignore::Error::Io(_)
        | ignore::Error::Glob { .. }
        | ignore::Error::UnrecognizedFileType(_)
        | ignore::Error::InvalidDefinition => result.walk_errors_without_path += 1,
    }
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
        Ok(()) if sink.not_utf8 => FileOutcome::Unreadable,
        Ok(()) => FileOutcome::Lines(sink.hits),
    }
}

/// Collects matching lines, and stops at the first sign of binary data so the caller can discard the whole file,
/// including lines matched before the NUL byte was reached. A matching line that is not UTF-8 does not stop the
/// search, so a NUL later in the file still marks it binary.
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
    not_utf8: bool,
}

impl<'m> LineCollector<'m> {
    fn new(matcher: &'m RegexMatcher) -> Self {
        Self {
            matcher,
            line_searcher: SearcherBuilder::new().bom_sniffing(false).build(),
            line_buf: Vec::new(),
            hits: Vec::new(),
            binary: false,
            not_utf8: false,
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
        match std::str::from_utf8(text) {
            Ok(text) => self.hits.push(LineHit { line_number, text: text.to_owned() }),
            Err(_) => self.not_utf8 = true,
        }
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
        || path.ends_with(".spec.ts")
        || path.ends_with(".spec.tsx")
        || path.ends_with(".spec.js")
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
    fn walk_order_is_by_file_name_within_each_directory() {
        // "a" sorts before "a.ts", so the directory's contents come first, unlike a sort of the full path strings.
        let files: &[(&str, &[u8])] =
            &[("b.ts", b"alpha \xff\n"), ("a/x.ts", b"alpha \xff\n"), ("a.ts", b"alpha \xff\n")];
        let (_dir, root) = tree(files);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(result.unreadable, [root.join("a/x.ts"), root.join("a.ts"), root.join("b.ts")]);
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

    #[test]
    fn unreadable_files_are_listed_in_walk_order() {
        let (_dir, root) = tree(&[
            ("z.ts", b"alpha \xff\n"),
            ("m/n.ts", b"alpha \xfe\n"),
            ("b.ts", b"alpha \xff\n"),
            ("a.ts", b"alpha\n"),
        ]);
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
    fn files_past_the_ceiling_are_still_searched_for_unreadable_lines() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\n"), ("b.ts", b"alpha \xff\n"), ("c.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_files(0)).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                files_total: 2,
                ceiling_hit: true,
                unreadable: vec![root.join("b.ts")],
                ..TextSearchResult::default()
            }
        );
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
    #[case("src/a.test.mjs")]
    #[case("src/a.spec.jsx")]
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

    #[rstest]
    #[case::ignore(".ignore")]
    #[case::gitignore(".gitignore")]
    fn ignore_files_above_the_root_do_not_apply(#[case] name: &str) {
        let (_dir, parent) = tree(&[(name, b"b.ts\n"), (".git/HEAD", b"ref: refs/heads/main\n")]);
        let root = parent.join("project");
        write_files(&root, &[("a.ts", b"alpha\n"), ("b.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts", "b.ts"]);
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
    fn invalid_utf8_in_a_matching_line_makes_the_file_unreadable_and_drops_its_hits() {
        let (_dir, root) = tree(&[("a.ts", b"alpha\nalpha \xff\n"), ("b.ts", b"alpha\n")]);
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
