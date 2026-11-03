//! The README's options table documents the default caps; `Budget::default()` owns them.
#![cfg(feature = "cli")]
#![allow(clippy::expect_used, clippy::panic, reason = "a failing assertion should panic in a test")]

use monokl::budget::Budget;

fn readme() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    std::fs::read_to_string(path).expect("README.md is readable from the workspace root")
}

/// The default cell of the options-table row for `flag`, e.g. `20,000` for `--max-tokens`.
fn default_cell(readme: &str, flag: &str) -> String {
    let row_start = format!("| `{flag} ");
    let row = readme
        .lines()
        .find(|line| line.starts_with(&row_start))
        .unwrap_or_else(|| panic!("no options-table row for {flag}"));
    row.split('|').nth(2).unwrap_or_else(|| panic!("row for {flag} has no default cell")).trim().to_owned()
}

fn parse_cap(cell: &str) -> usize {
    match cell.strip_suffix(" MiB") {
        Some(mebibytes) => mebibytes.parse::<usize>().expect("MiB count is an integer") * 1024 * 1024,
        None => cell.replace(',', "").parse().expect("cap is an integer"),
    }
}

#[test]
fn readme_options_table_matches_budget_default() {
    let readme = readme();
    let budget = Budget::default();
    assert_eq!(parse_cap(&default_cell(&readme, "--max-results")), budget.max_results);
    assert_eq!(parse_cap(&default_cell(&readme, "--max-tokens")), budget.max_tokens);
    assert_eq!(parse_cap(&default_cell(&readme, "--max-bytes")), budget.max_bytes);
}
