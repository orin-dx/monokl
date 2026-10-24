//! Black-box tests: run the built `monokl` binary against the `small-ts` fixture project.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a failing assertion should panic in a test")]

use std::path::PathBuf;
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
    assert!(text.lines().all(|line| !line.contains("/Users/")), "paths should be relative: {text}");
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
fn a_missing_query_is_a_usage_error() {
    let output = monokl(&["search"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("<QUERY>"));
}
