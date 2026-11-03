//! Black-box tests for the query language and file filters, against throwaway project trees.
#![cfg(feature = "cli")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a failing assertion should panic in a test")]

use std::fs;
use std::process::Command;

/// Writes `files` to a tempdir, runs `monokl search <args>` there and returns the sorted `src/` file names printed.
fn matched(files: &[(&str, &str)], args: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let output =
        Command::new(env!("CARGO_BIN_EXE_monokl")).arg("search").args(args).current_dir(dir.path()).output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    let mut names: Vec<String> = text
        .lines()
        .filter_map(|line| line.trim_start().split(',').next())
        .filter(|name| name.starts_with("src/"))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    names
}

const TWO_FILES: [(&str, &str); 2] =
    [("src/a.ts", "export function bar() { return foo; }\n"), ("src/b.ts", "export function bar() { return 1; }\n")];

#[test]
fn minus_regex_excludes_files() {
    assert_eq!(matched(&TWO_FILES, &["bar -regex:foo", "."]), ["src/b.ts"]);
}

#[test]
fn plus_regex_requires_files() {
    assert_eq!(matched(&TWO_FILES, &["bar +regex:foo", "."]), ["src/a.ts"]);
}

#[test]
fn quoted_regex_pattern_may_contain_spaces() {
    let files = [("src/a.ts", "export function useThing() {}\n"), ("src/b.ts", "export const useThing = 1;\n")];
    assert_eq!(matched(&files, &["regex:\"function use[A-Z]\"", "."]), ["src/a.ts"]);
}

#[test]
fn language_flag_selects_by_extension() {
    let files = [("src/a.ts", "const alpha = 1;\n"), ("src/b.js", "const alpha = 1;\n")];
    assert_eq!(matched(&files, &["alpha", ".", "--language", "typescript"]), ["src/a.ts"]);
    assert_eq!(matched(&files, &["alpha", ".", "--language", "javascript"]), ["src/b.js"]);
}

#[test]
fn declaration_and_test_files_in_every_extension_are_skipped() {
    let files = [
        ("src/a.ts", "const alpha = 1;\n"),
        ("src/t.d.mts", "declare const alpha: number;\n"),
        ("src/t.d.cts", "declare const alpha: number;\n"),
        ("src/u.test.mjs", "const alpha = 1;\n"),
        ("src/v.spec.cts", "const alpha = 1;\n"),
    ];
    assert_eq!(matched(&files, &["alpha", "."]), ["src/a.ts"]);
}
