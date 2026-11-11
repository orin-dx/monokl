//! Black-box tests: run the built `monokl` binary against the `small-ts` fixture project.
#![cfg(feature = "cli")]
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
    assert_eq!(
        lines.next(),
        Some("search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:")
    );
    let first = lines.next().unwrap();
    assert!(
        first.starts_with(
            "  src/utils/format.ts,1,3,function,null,\"function formatCurrency(amount: number, currency: string = 'USD'): string\",\"2: return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);\",1,"
        ),
        "got: {first}"
    );
    assert!(text.lines().skip(1).all(|line| !line.trim_start().starts_with('/')), "paths should be relative: {text}");
}

#[test]
fn a_row_shows_the_matching_lines_inside_its_block_and_nothing_else() {
    let output = monokl(&["search", "format price", "src/", "--max-results", "3"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let rows: Vec<&str> = text.lines().skip(1).take(3).collect();
    assert_eq!(
        rows[1],
        "  src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\
         \"9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\\n\
         10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",2,5.151139"
    );
    assert_eq!(
        rows[2],
        "  src/index.ts,3,3,other,null,null,\"3: export { formatCurrency, truncateString } from './utils/format';\",3,2.345138"
    );
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
    let nested = format!("export const needle = /'/;\nexport const x = {}1{};\n", "(".repeat(1000), ")".repeat(1000));
    std::fs::write(dir.path().join("nested.ts"), nested).unwrap();
    std::fs::write(dir.path().join("normal.ts"), "export function needle() {}\n").unwrap();

    let output =
        Command::new(env!("CARGO_BIN_EXE_monokl")).args(["search", "needle", "."]).current_dir(&dir).output().unwrap();

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().take(3).collect();
    assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:");
    assert!(lines[1].starts_with("  normal.ts,1,1,function,null,function needle(),null,1,"), "got: {text}");
    assert!(
        lines[2].starts_with("  nested.ts,1,1,variable,null,null,\"1: export const needle = /'/;\",2,"),
        "got: {text}"
    );
}

#[test]
fn a_file_nested_past_the_cap_is_skipped_and_reported_while_the_others_are_searched() {
    let dir = tempfile::tempdir().unwrap();
    let nested = format!("export const needle = {}1{};\n", "[".repeat(100_000), "]".repeat(100_000));
    std::fs::write(dir.path().join("deep.ts"), nested).unwrap();
    std::fs::write(dir.path().join("normal.ts"), "export function needle() {}\n").unwrap();

    let output = monokl_in(dir.path(), &["search", "needle", "."]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("search[1]{"), "got: {text}");
    assert!(text.contains("  normal.ts,1,1,function,null,function needle(),null,1,"), "got: {text}");
    assert!(
        text.contains("1 file degraded: file nests too deeply to analyze (depth 4097, over the cap of 4096): "),
        "got: {text}"
    );
    assert!(text.contains("deep.ts"), "got: {text}");
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
    assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:");
    assert!(lines[1].starts_with("  legacy.ts,2,2,function,null,function needle(),null,1,"), "got: {text}");
    assert!(lines[2].starts_with("  modern.ts,1,1,function,null,function needle(),null,2,"), "got: {text}");
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
    assert_eq!(
        lines.next(),
        Some("search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:")
    );
    assert!(lines.next().unwrap().starts_with("  build/out.ts,1,1,function,null,"), "got: {text}");
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
        Some("search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:")
    );
    assert!(text.lines().nth(1).unwrap().starts_with("  build/out.ts,1,1,function,null,"), "got: {text}");
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
         \"only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg\"\n"
    );
}

#[test]
fn an_unsupported_language_says_so() {
    let output = monokl(&["search", "format", ".", "--language", "python"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "search: []\nhelp[2]: \"language Python not supported in v1; only TypeScript, JavaScript and Rust\",\
         \"only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg\"\n"
    );
}

#[test]
fn a_file_as_the_search_root_is_invalid_input_naming_the_path() {
    let output = monokl(&["search", "alpha", "src/index.ts"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).starts_with("error: invalid_input"), "got: {}", stderr(&output));
    assert!(stderr(&output).contains("src/index.ts"), "got: {}", stderr(&output));
}

const PRICE_HEADER: &str = "search[N]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:";
const PRICE_ROW_1: &str = "  src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\"9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\\n10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",1,1.49154";
const PRICE_ROW_2: &str = "  src/utils/validation.ts,14,14,typeAlias,null,null,\"14: export type PriceValidator = (price: number) => ValidationResult;\",2,1.346154";

/// The expected output for the first `rows` of the `price` query, cut from 8.
fn price_cut(rows: &[&str]) -> String {
    let n = rows.len();
    let header = PRICE_HEADER.replace('N', &n.to_string());
    let body: String = rows.iter().flat_map(|row| [*row, "\n"]).collect();
    format!(
        "{header}\n{body}totalCount: 8\nhelp[1]: showing {n} of 8; raise the cap or refine the query with +term or -term\n"
    )
}

#[test]
fn a_cut_by_rows_keeps_the_first_rows_and_says_so() {
    let output = monokl(&["search", "price", ".", "--max-results", "2"]);
    assert_eq!(stdout(&output), price_cut(&[PRICE_ROW_1, PRICE_ROW_2]));
}

#[test]
fn a_cut_by_bytes_counts_the_printed_rows_and_not_the_blocks() {
    let printed = PRICE_ROW_1.len() + PRICE_ROW_2.len();
    let at = monokl(&["search", "price", ".", "--max-bytes", &printed.to_string()]);
    assert_eq!(stdout(&at), price_cut(&[PRICE_ROW_1, PRICE_ROW_2]));
    let under = monokl(&["search", "price", ".", "--max-bytes", &(printed - 1).to_string()]);
    assert_eq!(stdout(&under), price_cut(&[PRICE_ROW_1]));
}

#[test]
fn a_cut_by_tokens_counts_the_printed_rows_and_not_the_blocks() {
    let printed = monokl::budget::count_tokens(PRICE_ROW_1) + monokl::budget::count_tokens(PRICE_ROW_2);
    let at = monokl(&["search", "price", ".", "--max-tokens", &printed.to_string()]);
    assert_eq!(stdout(&at), price_cut(&[PRICE_ROW_1, PRICE_ROW_2]));
    let under = monokl(&["search", "price", ".", "--max-tokens", &(printed - 1).to_string()]);
    assert_eq!(stdout(&under), price_cut(&[PRICE_ROW_1]));
}

#[test]
fn a_first_row_larger_than_the_budget_prints_no_rows_and_says_so() {
    let output = monokl(&["search", "price", ".", "--max-bytes", &(PRICE_ROW_1.len() - 1).to_string()]);
    assert_eq!(
        stdout(&output),
        "search: []\ntotalCount: 8\nhelp[2]: showing 0 of 8; raise the cap or refine the query with +term or -term,\
         \"only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg\"\n"
    );
}

#[test]
fn a_result_within_every_cap_is_printed_whole_without_a_cut_marker() {
    let output = monokl(&["search", "format", "."]);
    assert_eq!(
        stdout(&output),
        "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:\n\
         \x20 src/utils/format.ts,1,3,function,null,\"function formatCurrency(amount: number, currency: string = 'USD'): string\",\"2: return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);\",1,2.068251\n\
         \x20 src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\"10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",2,1.170466\n\
         \x20 src/index.ts,3,3,other,null,null,\"3: export { formatCurrency, truncateString } from './utils/format';\",3,1.159101\n"
    );
}

const CART_RS: &str = "\
pub struct Cart {
    items: Vec<u32>,
}

impl Cart {
    pub fn total_price(&self) -> u32 {
        let sum = self.items.iter().sum();
        sum
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn total_price_adds_up() {
        assert_eq!(1, 1);
    }
}
";

const SEARCH_HEADER: &str =
    "search[N]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:";

/// A project with one Rust file and one TypeScript file that both mention `total_price`.
fn mixed_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/cart.rs"), CART_RS).unwrap();
    std::fs::write(dir.path().join("src/cart.ts"), "export function total_price(): number {\n  return 1;\n}\n")
        .unwrap();
    dir
}

#[cfg(feature = "lang-rust")]
#[test]
fn rust_code_is_searched_as_blocks_and_test_code_only_on_request() {
    let dir = mixed_project();
    let output = monokl_in(dir.path(), &["search", "total_price", "src", "--language", "rust"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "{}\n  src/cart.rs,6,9,method,impl Cart,pub fn total_price(&self) -> u32,null,1,4.452186\n",
            SEARCH_HEADER.replace("[N]", "[1]")
        )
    );

    let output = monokl_in(dir.path(), &["search", "total_price", "src", "--language", "rust", "--tests"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "{}\n  src/cart.rs,14,17,function,module tests,fn total_price_adds_up(),\"15: fn total_price_adds_up() {{\",1,3.344136\n  src/cart.rs,6,9,method,impl Cart,pub fn total_price(&self) -> u32,null,2,3.18786\n",
            SEARCH_HEADER.replace("[N]", "[2]")
        )
    );
}

#[cfg(feature = "lang-rust")]
#[test]
fn the_language_flag_selects_rust_or_typescript_files() {
    let dir = mixed_project();
    let files = |args: &[&str]| -> Vec<String> {
        let output = monokl_in(dir.path(), args);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let mut files: Vec<String> = stdout(&output)
            .lines()
            .skip(1)
            .map(|line| line.trim_start().split(',').next().unwrap().to_owned())
            .collect();
        files.sort();
        files
    };
    assert_eq!(files(&["search", "total_price", ".", "--tests"]), ["src/cart.rs", "src/cart.rs", "src/cart.ts"]);
    assert_eq!(files(&["search", "total_price", ".", "--language", "rust"]), ["src/cart.rs"]);
    assert_eq!(files(&["search", "total_price", ".", "--language", "typescript"]), ["src/cart.ts"]);
}
