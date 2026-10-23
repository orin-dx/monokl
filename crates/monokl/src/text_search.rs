//! The first search stage: walks a root and returns, per file, the lines that match any query pattern.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
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
    /// `.spec.js`.
    pub allow_tests: bool,
    /// Honors `.gitignore`, `.ignore` and hidden-file rules.
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
    /// The kept files, keyed by their walked path (`root` joined with the path below it), each with its matching
    /// lines in line order.
    pub hits: BTreeMap<Utf8PathBuf, Vec<LineHit>>,
    /// Every file with at least one matching line, kept or not.
    pub files_total: u64,
    /// Whether `files_total` exceeded [`TextSearchOptions::max_files`], so some matching files have no hits stored.
    pub ceiling_hit: bool,
    /// Files that could not be read, or whose matching lines are not UTF-8, in walk order.
    pub unreadable: Vec<Utf8PathBuf>,
    /// How many file paths were skipped because they are not UTF-8.
    pub non_utf8_paths: u64,
}

/// Searches every file under `root` for lines matching any of `patterns`, case-insensitively.
///
/// The walk is sorted by file name, so the result is the same on every run. Binary files (any NUL byte) are skipped
/// silently. A file that cannot be read, or whose matching line is not UTF-8, goes in
/// [`unreadable`](TextSearchResult::unreadable) and the search continues. No patterns means no matches; nothing is
/// walked.
///
/// # Errors
///
/// - [`MonoklError::RegexBuild`] if any pattern does not compile, before anything is walked.
/// - [`MonoklError::Walk`] with `path` set to `root` if the walk fails, for example because `root` does not exist.
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
    let mut result = TextSearchResult::default();
    if patterns.is_empty() {
        return Ok(result);
    }
    let mut builder = RegexMatcherBuilder::new();
    builder.case_insensitive(true);
    // `build_many` joins the patterns with `|` before parsing, so halves such as `(a` and `b)` would compile together.
    // Each pattern is compiled alone first so an invalid one always fails the search.
    for pattern in patterns {
        builder.build(pattern)?;
    }
    let matcher = builder.build_many(patterns)?;
    let mut searcher = SearcherBuilder::new().binary_detection(BinaryDetection::quit(b'\x00')).build();
    let walk = WalkBuilder::new(root)
        .standard_filters(opts.respect_ignore_files)
        .sort_by_file_name(std::cmp::Ord::cmp)
        .build();
    for entry in walk {
        let entry = entry.map_err(|source| MonoklError::Walk { path: root.to_owned(), source })?;
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

/// How searching one file ended.
enum FileOutcome {
    Binary,
    Unreadable,
    /// The file's matching lines, possibly none.
    Lines(Vec<LineHit>),
}

fn search_file(searcher: &mut Searcher, matcher: &RegexMatcher, path: &Utf8Path) -> FileOutcome {
    let mut sink = LineCollector::default();
    match searcher.search_path(matcher, path.as_std_path(), &mut sink) {
        Err(_) => FileOutcome::Unreadable,
        Ok(()) if sink.binary => FileOutcome::Binary,
        Ok(()) => FileOutcome::Lines(sink.hits),
    }
}

/// Collects matching lines, and stops at the first sign of binary data so the caller can discard the whole file,
/// including lines matched before the NUL byte was reached.
#[derive(Default)]
struct LineCollector {
    hits: Vec<LineHit>,
    binary: bool,
}

impl Sink for LineCollector {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> io::Result<bool> {
        let line_number = mat
            .line_number()
            .and_then(|line_number| usize::try_from(line_number).ok())
            .ok_or_else(|| io::Error::other("matched line has no usable line number"))?;
        let text = std::str::from_utf8(without_line_terminator(mat.bytes()))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.hits.push(LineHit { line_number, text: text.to_owned() });
        Ok(true)
    }

    fn binary_data(&mut self, _searcher: &Searcher, _binary_byte_offset: u64) -> io::Result<bool> {
        self.binary = true;
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
        // The root does not exist, so any walk would fail.
        let (_dir, root) = tree(&[]);
        let missing = root.join("missing");
        assert_eq!(search_files(&missing, &[], &TextSearchOptions::default()).unwrap(), TextSearchResult::default());
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

    #[test]
    fn a_missing_root_is_a_walk_error_naming_the_root() {
        let (_dir, root) = tree(&[]);
        let missing = root.join("missing");
        let err = search_files(&missing, &["alpha"], &TextSearchOptions::default()).unwrap_err();
        let MonoklError::Walk { path, .. } = err else { panic!("expected a walk error, got {err:?}") };
        assert_eq!(path, missing);
    }

    #[test]
    fn walk_order_is_by_file_name_so_the_kept_files_are_reproducible() {
        // Written out of order, with a directory sorting between two files.
        let (_dir, root) = tree(&[("c.ts", b"alpha\n"), ("b/z.ts", b"alpha\n"), ("a.ts", b"alpha\n")]);
        let result = search_files(&root, &["alpha"], &with_max_files(2)).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts", "b/z.ts"]);
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
    fn other_paths_are_not_classified_as_tests(#[case] relative: &str) {
        assert!(!is_test_path(Utf8Path::new(relative)));
    }

    #[test]
    fn ignore_files_and_hidden_files_are_skipped_when_respected() {
        let (_dir, root) = tree(&[
            (".ignore", b"ignored.ts\n"),
            ("a.ts", b"alpha\n"),
            ("ignored.ts", b"alpha\n"),
            (".hidden.ts", b"alpha\n"),
            (".hidden/b.ts", b"alpha\n"),
        ]);
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts"]);
        let opts = TextSearchOptions { respect_ignore_files: false, ..TextSearchOptions::default() };
        let result = search_files(&root, &["alpha"], &opts).unwrap();
        assert_eq!(hit_files(&root, &result), [".hidden/b.ts", ".hidden.ts", "a.ts", "ignored.ts"]);
    }

    #[test]
    fn gitignored_files_are_skipped_in_a_git_repository() {
        let (_dir, root) = tree(&[(".gitignore", b"ignored.ts\n"), ("a.ts", b"alpha\n"), ("ignored.ts", b"alpha\n")]);
        fs::create_dir(root.join(".git")).unwrap();
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(hit_files(&root, &result), ["a.ts"]);
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

        let (_dir, root) = tree(&[("a.ts", b"alpha\n")]);
        fs::write(root.as_std_path().join(OsStr::from_bytes(b"bad\xff.ts")), b"alpha\n").unwrap();
        let dir = root.as_std_path().join(OsStr::from_bytes(b"dir\xfe"));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("c.ts"), b"alpha\n").unwrap();
        let result = search_files(&root, &["alpha"], &TextSearchOptions::default()).unwrap();
        assert_eq!(
            result,
            TextSearchResult {
                hits: hits(&root, &[("a.ts", vec![hit(1, "alpha")])]),
                files_total: 1,
                non_utf8_paths: 2,
                ..TextSearchResult::default()
            }
        );
    }
}
