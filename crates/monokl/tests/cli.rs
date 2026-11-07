//! Black-box tests: run the built `monokl` binary against the `small-ts` fixture project.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a failing assertion should panic in a test")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small-ts")
}

/// Runs `monokl` with the fixture as the working directory, so printed paths are relative.
fn monokl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_monokl")).args(args).current_dir(fixture()).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn search_prints_ranked_blocks_with_relative_paths() {
    let output = monokl(&["search", "format", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("search[3]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:"));
    let first = lines.next().unwrap();
    assert!(
        first.starts_with(
            "  src/utils/format.ts,1,3,function,\"function formatCurrency(amount: number, currency: string = 'USD'): string\",1,"
        ),
        "got: {first}"
    );
    assert!(text.lines().skip(1).all(|line| !line.trim_start().starts_with('/')), "paths should be relative: {text}");
}

#[test]
fn search_is_deterministic() {
    assert_eq!(stdout(&monokl(&["search", "price", "."])), stdout(&monokl(&["search", "price", "."])));
}

#[test]
fn search_honours_the_result_cap_and_says_so() {
    let output = monokl(&["search", "price", ".", "--max-results", "2"]);
    let text = stdout(&output);
    assert!(text.starts_with("search[2]"), "got: {text}");
    assert!(text.contains("totalCount: 8"), "got: {text}");
    assert!(text.contains("showing 2 of 8"), "got: {text}");
}

#[test]
fn excluded_terms_remove_files() {
    let with = stdout(&monokl(&["search", "price", "."]));
    let without = stdout(&monokl(&["search", "price -validation", "."]));
    assert!(with.contains("src/utils/validation.ts"));
    assert!(!without.contains("src/utils/validation.ts"));
}

#[test]
fn a_query_with_no_match_prints_an_empty_result() {
    let output = monokl(&["search", "zzzqqqxx", "."]);
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("search: []"), "got: {}", stdout(&output));
}

#[test]
fn an_invalid_regex_fails_with_a_structured_error() {
    let output = monokl(&["search", "regex:(", "."]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).starts_with("error: invalid_input"), "got: {}", stderr(&output));
    assert!(stderr(&output).contains("regex_build"), "got: {}", stderr(&output));
}

#[test]
fn a_deeply_nested_file_is_searched_like_any_other() {
    let dir = tempfile::tempdir().unwrap();
    let nested = format!("export const needle = /'/;\nexport const x = {}1{};\n", "(".repeat(5000), ")".repeat(5000));
    std::fs::write(dir.path().join("nested.ts"), nested).unwrap();
    std::fs::write(dir.path().join("normal.ts"), "export function needle() {}\n").unwrap();

    let output =
        Command::new(env!("CARGO_BIN_EXE_monokl")).args(["search", "needle", "."]).current_dir(&dir).output().unwrap();

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().take(3).collect();
    assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:");
    assert!(lines[1].starts_with("  normal.ts,1,1,function,function needle(),1,"), "got: {text}");
    assert!(lines[2].starts_with("  nested.ts,1,1,variable,null,2,"), "got: {text}");
}

#[test]
fn files_that_are_not_valid_utf8_are_searched_and_reported_as_read_lossily() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("legacy.ts"), b"// zed\xe9\nexport function needle() {}\n").unwrap();
    std::fs::write(dir.path().join("modern.ts"), "export function needle() {}\n").unwrap();

    let output = monokl_in(dir.path(), &["search", "needle", "."]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "got: {text}");
    assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:");
    assert!(lines[1].starts_with("  legacy.ts,2,2,function,function needle(),1,"), "got: {text}");
    assert!(lines[2].starts_with("  modern.ts,1,1,function,function needle(),2,"), "got: {text}");
    assert_eq!(lines[3], "help[1]: \"1 file read lossily: invalid UTF-8 replaced with U+FFFD: legacy.ts\"");
}

#[test]
fn a_missing_query_is_a_usage_error() {
    let output = monokl(&["search"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("<QUERY>"));
}

/// A temp project with a gitignored `build/` directory holding the only match for `generated`.
fn project_with_a_gitignored_match() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir_all(dir.path().join("build")).unwrap();
    std::fs::write(dir.path().join("build/out.ts"), "export function generated() {}\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.ts"), "export function main() {}\n").unwrap();
    dir
}

fn monokl_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_monokl")).args(args).current_dir(dir).output().unwrap()
}

#[test]
fn ignored_directories_are_not_searched_by_default() {
    let dir = project_with_a_gitignored_match();
    let output = monokl_in(dir.path(), &["search", "generated", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).starts_with("search: []"), "got: {}", stdout(&output));
}

#[test]
fn no_ignore_searches_ignored_directories() {
    let dir = project_with_a_gitignored_match();
    let output = monokl_in(dir.path(), &["search", "generated", ".", "--no-ignore"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("search[1]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:"));
    assert!(lines.next().unwrap().starts_with("  build/out.ts,1,1,function,"), "got: {text}");
}

/// A repository whose root `.gitignore` excludes `build/`, which holds the only match for `generated` in `pkg/build/`.
fn repository_with_a_gitignored_match_below_a_subdirectory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir_all(dir.path().join("pkg/build")).unwrap();
    std::fs::write(dir.path().join("pkg/build/out.ts"), "export function generated() {}\n").unwrap();
    dir
}

#[test]
fn a_parent_gitignore_in_the_repository_excludes_files_below_the_search_path() {
    let dir = repository_with_a_gitignored_match_below_a_subdirectory();
    let output = monokl_in(&dir.path().join("pkg"), &["search", "generated", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).starts_with("search: []"), "got: {}", stdout(&output));
}

#[test]
fn no_ignore_also_skips_parent_ignore_files() {
    let dir = repository_with_a_gitignored_match_below_a_subdirectory();
    let output = monokl_in(&dir.path().join("pkg"), &["search", "generated", ".", "--no-ignore"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(
        text.lines().next(),
        Some("search[1]{file,lineStart,lineEnd,nodeKind,symbolSignature,rank,finalScore}:")
    );
    assert!(text.lines().nth(1).unwrap().starts_with("  build/out.ts,1,1,function,"), "got: {text}");
}

#[test]
fn a_missing_search_root_exits_with_a_structured_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = monokl_in(dir.path(), &["search", "alpha", "no-such-directory"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).starts_with("error: not_found"), "got: {}", stderr(&output));
}

#[test]
fn a_query_with_no_positive_terms_says_so() {
    let output = monokl(&["search", "--", "-foo", "."]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "search: []\nhelp[2]: query has no positive terms; add a term without a leading `-`,\
         \"only top-level declarations are searched; for text in tests, calls or config try rg\"\n"
    );
}

#[test]
fn an_unsupported_language_says_so() {
    let output = monokl(&["search", "format", ".", "--language", "rust"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "search: []\nhelp[2]: language Rust not supported in v1; only TypeScript/JavaScript,\
         \"only top-level declarations are searched; for text in tests, calls or config try rg\"\n"
    );
}

#[test]
fn a_file_as_the_search_root_is_invalid_input_naming_the_path() {
    let output = monokl(&["search", "alpha", "src/index.ts"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).starts_with("error: invalid_input"), "got: {}", stderr(&output));
    assert!(stderr(&output).contains("src/index.ts"), "got: {}", stderr(&output));
}
