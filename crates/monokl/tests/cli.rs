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
        Some("search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:")
    );
    assert_eq!(
        lines.next(),
        Some(
            "  src/utils/format.ts,1,3,function,null,\"function formatCurrency(amount: number, currency: string = 'USD'): string\",\"2: return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);\",100"
        )
    );
    assert!(text.lines().skip(1).all(|line| !line.trim_start().starts_with('/')), "paths should be relative: {text}");
}

#[test]
fn a_row_shows_the_matching_lines_inside_its_block_and_nothing_else() {
    let output = monokl(&["search", "format price", "src/", "--max-results", "3", "--snippet-lines", "3"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let rows: Vec<&str> = text.lines().skip(1).take(3).collect();
    assert_eq!(
        rows[1],
        "  src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\
         \"9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\\n\
         10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",70"
    );
    assert_eq!(
        rows[2],
        "  src/index.ts,3,3,other,null,null,\"3: export { formatCurrency, truncateString } from './utils/format';\",23"
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
fn a_query_that_matches_no_file_of_a_populated_tree_exits_zero_with_the_empty_result() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.ts"), "export const alpha = 1;\n").unwrap();
    let output = monokl_in(dir.path(), &["search", "zzzqqqxx", "."]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(stdout(&output).starts_with("search: []\n"), "got: {}", stdout(&output));
    assert!(!stdout(&output).contains("totalCount"), "got: {}", stdout(&output));
    assert_eq!(stderr(&output), "");
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
    assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:");
    assert_eq!(lines[1], "  normal.ts,1,1,function,null,function needle(),null,100");
    assert_eq!(lines[2], "  nested.ts,1,1,variable,null,null,\"1: export const needle = /'/;\",71");
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
    assert!(text.contains("\n  normal.ts,1,1,function,null,function needle(),null,100\n"), "got: {text}");
    assert!(
        text.contains("1 file degraded: file nests too deeply to analyze (depth 16385, over the cap of 16384): "),
        "got: {text}"
    );
    assert!(text.contains("deep.ts"), "got: {text}");
}

/// Searches `needle` over one generated file and returns the output, which must name the file and report no degradation.
fn search_generated(name: &str, text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(name), text).unwrap();
    let output = monokl_in(dir.path(), &["search", "needle", "."]);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("degraded"), "got: {text}");
    assert!(text.starts_with("search[1]{"), "got: {text}");
    text
}

#[test]
fn a_component_with_three_thousand_sibling_elements_is_searched() {
    let items =
        (0..3_000).map(|i| format!("      <li key={{{i}}}>item {i} needle</li>\n")).collect::<Vec<String>>().concat();
    let text = search_generated(
        "List.tsx",
        &format!("export function List() {{\n  return (\n    <ul>\n{items}    </ul>\n  );\n}}\n"),
    );
    assert!(text.contains("  List.tsx,1,3006,function,null,function List(),"), "got: {text}");
}

#[test]
fn a_twelve_thousand_term_string_concatenation_is_searched() {
    let terms = vec!["\"needle\""; 12_000].join(" + ");
    let text = search_generated("concat.ts", &format!("export const joined = {terms};\n"));
    assert!(text.contains("  concat.ts,1,1,variable,null,null,"), "got: {text}");
}

#[test]
fn a_twenty_thousand_row_array_literal_is_searched() {
    let rows = (0..20_000).map(|i| format!("  {{ id: {i}, name: 'needle {i}' }},\n")).collect::<Vec<String>>().concat();
    let text = search_generated("rows.ts", &format!("export const rows = [\n{rows}];\n"));
    assert!(text.contains("  rows.ts,1,20002,variable,null,null,"), "got: {text}");
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
    assert_eq!(lines[0], "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:");
    assert_eq!(lines[1], "  legacy.ts,2,2,function,null,function needle(),null,100");
    assert_eq!(lines[2], "  modern.ts,1,1,function,null,function needle(),null,100");
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
        Some("search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:")
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
        Some("search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:")
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
fn an_unsupported_language_is_a_usage_error_listing_the_supported_ones() {
    let output = monokl(&["search", "format", ".", "--language", "python"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("invalid value 'python' for '--language <LANGUAGE>'"), "got: {}", stderr(&output));
    assert!(stderr(&output).contains("[possible values: typescript, javascript, rust]"), "got: {}", stderr(&output));
}

#[test]
fn a_file_as_the_search_root_is_invalid_input_naming_the_path() {
    let output = monokl(&["search", "alpha", "src/index.ts"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).starts_with("error: invalid_input"), "got: {}", stderr(&output));
    assert!(stderr(&output).contains("src/index.ts"), "got: {}", stderr(&output));
}

const PRICE_HEADER: &str = "search[N]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:";
const PRICE_ROW_1: &str = "  src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\"9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\\n10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",100";
const PRICE_ROW_2: &str = "  src/utils/validation.ts,14,14,typeAlias,null,null,\"14: export type PriceValidator = (price: number) => ValidationResult;\",70";

/// The flags that give the output the shape it had before the relevance cut and the one-line snippet: every row up to
/// the cap, and a snippet of up to three lines. The row and token cap tests below use them so that each tests one cap.
const WHOLE_ROWS: [&str; 4] = ["--min-score", "0", "--snippet-lines", "3"];

/// `args` followed by `WHOLE_ROWS`.
fn with_whole_rows<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().chain(WHOLE_ROWS.iter()).copied().collect()
}

/// The expected output for the first `rows` of the `price` query, cut from 8, whose help line names `flag`.
fn price_cut(rows: &[&str], guidance: &str) -> String {
    let n = rows.len();
    let header = PRICE_HEADER.replace('N', &n.to_string());
    let body: String = rows.iter().flat_map(|row| [*row, "\n"]).collect();
    format!("{header}\n{body}totalCount: 8\nhelp[1]: \"showing {n} of 8; {guidance}\"\n")
}

#[test]
fn a_cut_by_rows_keeps_the_first_rows_and_says_so() {
    let output = monokl(&with_whole_rows(&["search", "price", ".", "--max-results", "2"]));
    assert_eq!(stdout(&output), price_cut(&[PRICE_ROW_1, PRICE_ROW_2], "next: --offset 2, or refine with +term/-term"));
}

#[test]
fn a_cut_by_tokens_counts_the_printed_rows_and_not_the_blocks() {
    let printed = monokl::budget::count_tokens(PRICE_ROW_1) + monokl::budget::count_tokens(PRICE_ROW_2);
    let at = monokl(&with_whole_rows(&["search", "price", ".", "--max-tokens", &printed.to_string()]));
    assert_eq!(stdout(&at), price_cut(&[PRICE_ROW_1, PRICE_ROW_2], "next: --offset 2, or refine with +term/-term"));
    let under = monokl(&with_whole_rows(&["search", "price", ".", "--max-tokens", &(printed - 1).to_string()]));
    assert_eq!(stdout(&under), price_cut(&[PRICE_ROW_1], "next: --offset 1, or refine with +term/-term"));
}

#[test]
fn a_first_row_larger_than_the_budget_prints_no_rows_and_says_so() {
    let tokens = (monokl::budget::count_tokens(PRICE_ROW_1) - 1).to_string();
    let output = monokl(&with_whole_rows(&["search", "price", ".", "--max-tokens", &tokens]));
    assert_eq!(
        stdout(&output),
        "search: []\ntotalCount: 8\nhelp[2]: \"showing 0 of 8; --max-tokens N for more, or refine with +term/-term\",\
         \"only declarations and the members of their classes, objects, impls and traits are searched; for text in tests, calls or config try rg\"\n"
    );
}

/// The rows of the `price` query in `small-ts` before any cut, with the snippet at three lines.
const PRICE_ROWS_WHOLE: [&str; 8] = [
    PRICE_ROW_1,
    PRICE_ROW_2,
    "  src/components/Button.tsx,11,18,function,null,\"function Button({ label, price, onClick, disabled }: ButtonProps)\",\"12: const priceValid = price === undefined || validatePrice(price).valid;\\n14: <button onClick={onClick} disabled={disabled || !priceValid}>\",38",
    "  src/hooks/useCart.ts,11,25,function,null,function useCart(),\"15: const result = validatePrice(item.price);\",22",
    "  src/hooks/useCart.ts,5,9,interface,null,null,\"8: price: number;\",21",
    "  src/components/Button.tsx,4,9,interface,null,null,\"6: price?: number;\",20",
    "  src/index.ts,4,4,other,null,null,\"4: export { validatePrice } from './utils/validation';\",12",
    "  src/index.ts,5,5,other,null,null,\"5: export type { ButtonProps, CartItem, ValidationResult, PriceValidator } from './utils/validation';\",11",
];

#[test]
fn the_flags_that_restore_the_whole_output_print_every_row_with_a_three_line_snippet() {
    let output = monokl(&["search", "price", ".", "--min-score", "0", "--snippet-lines", "3", "--max-results", "50"]);
    let rows: String = PRICE_ROWS_WHOLE.iter().flat_map(|row| [*row, "\n"]).collect();
    assert_eq!(stdout(&output), format!("{}\n{rows}", PRICE_HEADER.replace('N', "8")));
}

#[test]
fn by_default_a_result_of_up_to_eight_rows_is_printed_whole_with_one_line_snippets() {
    // The top row is relevance 100, so the relevance cut is 30 and rows 4 to 8 (22 down to 11) lie below it, but the
    // floor keeps the first eight rows, which are all there are.
    let output = monokl(&["search", "price", "."]);
    assert_eq!(
        stdout(&output),
        "search[8]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\"9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\\n+1 more\",100\n\
         \x20 src/utils/validation.ts,14,14,typeAlias,null,null,\"14: export type PriceValidator = (price: number) => ValidationResult;\",70\n\
         \x20 src/components/Button.tsx,11,18,function,null,\"function Button({ label, price, onClick, disabled }: ButtonProps)\",\"12: const priceValid = price === undefined || validatePrice(price).valid;\\n+1 more\",38\n\
         \x20 src/hooks/useCart.ts,11,25,function,null,function useCart(),\"15: const result = validatePrice(item.price);\",22\n\
         \x20 src/hooks/useCart.ts,5,9,interface,null,null,\"8: price: number;\",21\n\
         \x20 src/components/Button.tsx,4,9,interface,null,null,\"6: price?: number;\",20\n\
         \x20 src/index.ts,4,4,other,null,null,\"4: export { validatePrice } from './utils/validation';\",12\n\
         \x20 src/index.ts,5,5,other,null,null,\"5: export type { ButtonProps, CartItem, ValidationResult, PriceValidator } from './utils/validation';\",11\n"
    );
}

/// The `price` function, two functions that name it in a parameter or a template, and ten that only call it: thirteen
/// matching blocks in all, the last ten far below the top score.
fn pricing_ts() -> String {
    let callers: String = ('A'..='J')
        .map(|letter| {
            "\nexport function listL(items: Item[]): number {\n  return items.map(price).length;\n}\n"
                .replace('L', &letter.to_string())
        })
        .collect();
    format!(
        "export function price(item: Item): number {{\n  return item.cents;\n}}\n\n\
         export function applyPrice(item: Item, price: number): Item {{\n  return {{ ...item, cents: price }};\n}}\n\n\
         export function priceLabel(item: Item): string {{\n  return `${{price(item)}} cents`;\n}}\n\
         {callers}\nexport interface Item {{\n  cents: number;\n}}\n"
    )
}

/// The same shape in Rust.
fn pricing_rs() -> String {
    let callers: String = ('a'..='j')
        .map(|letter| {
            "\npub fn list_L(items: &[Item]) -> usize {\n    items.iter().map(price).count()\n}\n"
                .replace('L', &letter.to_string())
        })
        .collect();
    format!(
        "pub fn price(item: &Item) -> u64 {{\n    item.cents\n}}\n\n\
         pub fn apply_price(item: &Item, price: u64) -> Item {{\n    Item {{ cents: price, ..*item }}\n}}\n\n\
         pub fn price_label(item: &Item) -> String {{\n    format!(\"{{}} cents\", price(item))\n}}\n\
         {callers}\npub struct Item {{\n    pub cents: u64,\n}}\n"
    )
}

fn pricing_project(file: &str, text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join(file), text).unwrap();
    dir
}

/// The `lineStart` of each row a search printed, in print order: the second cell, after a file name with no comma. The
/// `price` projects hold one block per line number, so it names the row.
fn printed_line_starts(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter(|row| row.starts_with("  "))
        .filter_map(|row| row.split(',').nth(1))
        .map(str::to_owned)
        .collect()
}

/// The first lines of the `price` blocks of `pricing_ts`, best first: the three that name `price`, then the ten callers.
fn priced_line_starts(count: usize) -> Vec<String> {
    [1, 5, 9].into_iter().chain((13..).step_by(4)).take(count).map(|line| line.to_string()).collect()
}

#[test]
fn by_default_a_typescript_search_keeps_the_rows_above_the_cut_and_up_to_the_floor_then_says_how_to_see_more() {
    // The cut is relevance 30: rows 1 to 3 (100, 48, 43) reach it. The floor keeps rows 4 to 8 (10) and drops rows 9
    // to 13.
    let dir = pricing_project("pricing.ts", &pricing_ts());
    let output = monokl_in(dir.path(), &["search", "price", "."]);
    assert_eq!(
        stdout(&output),
        "search[8]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/pricing.ts,1,3,function,null,\"function price(item: Item): number\",null,100\n\
         \x20 src/pricing.ts,5,7,function,null,\"function applyPrice(item: Item, price: number): Item\",\"6: return { ...item, cents: price };\",48\n\
         \x20 src/pricing.ts,9,11,function,null,\"function priceLabel(item: Item): string\",\"10: return `${price(item)} cents`;\",43\n\
         \x20 src/pricing.ts,13,15,function,null,\"function listA(items: Item[]): number\",\"14: return items.map(price).length;\",10\n\
         \x20 src/pricing.ts,17,19,function,null,\"function listB(items: Item[]): number\",\"18: return items.map(price).length;\",10\n\
         \x20 src/pricing.ts,21,23,function,null,\"function listC(items: Item[]): number\",\"22: return items.map(price).length;\",10\n\
         \x20 src/pricing.ts,25,27,function,null,\"function listD(items: Item[]): number\",\"26: return items.map(price).length;\",10\n\
         \x20 src/pricing.ts,29,31,function,null,\"function listE(items: Item[]): number\",\"30: return items.map(price).length;\",10\n\
         totalCount: 13\n\
         help[1]: \"showing 8 of 13; --min-score 0 for the 5 rows below relevance 30, or refine with +term/-term\"\n"
    );
}

#[cfg(feature = "lang-rust")]
#[test]
fn by_default_a_rust_search_keeps_the_rows_above_the_cut_and_up_to_the_floor_then_says_how_to_see_more() {
    // The cut is relevance 30: rows 1 to 3 (100, 47, 42) reach it. The floor keeps rows 4 to 8 (10) and drops rows 9
    // to 13.
    let dir = pricing_project("pricing.rs", &pricing_rs());
    let output = monokl_in(dir.path(), &["search", "price", "."]);
    assert_eq!(
        stdout(&output),
        "search[8]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/pricing.rs,1,3,function,null,\"pub fn price(item: &Item) -> u64\",null,100\n\
         \x20 src/pricing.rs,5,7,function,null,\"pub fn apply_price(item: &Item, price: u64) -> Item\",\"6: Item { cents: price, ..*item }\",47\n\
         \x20 src/pricing.rs,9,11,function,null,\"pub fn price_label(item: &Item) -> String\",\"10: format!(\\\"{} cents\\\", price(item))\",42\n\
         \x20 src/pricing.rs,13,15,function,null,\"pub fn list_a(items: &[Item]) -> usize\",\"14: items.iter().map(price).count()\",10\n\
         \x20 src/pricing.rs,17,19,function,null,\"pub fn list_b(items: &[Item]) -> usize\",\"18: items.iter().map(price).count()\",10\n\
         \x20 src/pricing.rs,21,23,function,null,\"pub fn list_c(items: &[Item]) -> usize\",\"22: items.iter().map(price).count()\",10\n\
         \x20 src/pricing.rs,25,27,function,null,\"pub fn list_d(items: &[Item]) -> usize\",\"26: items.iter().map(price).count()\",10\n\
         \x20 src/pricing.rs,29,31,function,null,\"pub fn list_e(items: &[Item]) -> usize\",\"30: items.iter().map(price).count()\",10\n\
         totalCount: 13\n\
         help[1]: \"showing 8 of 13; --min-score 0 for the 5 rows below relevance 30, or refine with +term/-term\"\n"
    );
}

#[test]
fn a_min_score_of_zero_prints_every_row_up_to_the_cap_and_no_help_line() {
    let dir = pricing_project("pricing.ts", &pricing_ts());
    let text = stdout(&monokl_in(dir.path(), &["search", "price", ".", "--min-score", "0"]));
    assert_eq!(
        text.lines().next(),
        Some("search[13]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:")
    );
    assert_eq!(printed_line_starts(&text), priced_line_starts(13), "got: {text}");
    assert!(!text.contains("help["), "got: {text}");
    assert!(!text.contains("totalCount"), "got: {text}");
}

#[test]
fn a_min_score_of_one_keeps_the_floor_and_the_rows_that_tie_with_the_top() {
    let dir = pricing_project("pricing.ts", &pricing_ts());
    let text = stdout(&monokl_in(dir.path(), &["search", "price", ".", "--min-score", "1"]));
    assert_eq!(printed_line_starts(&text), priced_line_starts(8), "got: {text}");
}

#[test]
fn the_row_cap_wins_over_the_floor_and_the_help_line_names_the_offset_and_the_min_score_flag_when_both_cut() {
    let dir = pricing_project("pricing.ts", &pricing_ts());
    let text = stdout(&monokl_in(dir.path(), &["search", "price", ".", "--max-results", "5"]));
    assert_eq!(printed_line_starts(&text), priced_line_starts(5), "got: {text}");
    assert_eq!(
        text.lines().last(),
        Some(
            "help[1]: \"showing 5 of 13; next: --offset 5, --min-score 0 for the 5 rows below relevance 30, or refine with +term/-term\""
        )
    );
}

#[test]
fn the_help_line_names_only_the_offset_when_the_relevance_cut_dropped_nothing() {
    let output = monokl(&["search", "price", ".", "--max-results", "2"]);
    assert_eq!(
        stdout(&output).lines().last(),
        Some("help[1]: \"showing 2 of 8; next: --offset 2, or refine with +term/-term\"")
    );
}

#[test]
fn a_snippet_of_no_lines_prints_null_and_a_snippet_of_two_prints_two() {
    let none = stdout(&monokl(&["search", "price", ".", "--snippet-lines", "0"]));
    let snippets: Vec<&str> = none.lines().skip(1).take(3).map(|row| row.rsplit(',').nth(1).unwrap_or("")).collect();
    assert_eq!(snippets, ["null", "null", "null"], "got: {none}");

    let two = stdout(&monokl(&["search", "price", ".", "--snippet-lines", "2"]));
    assert_eq!(
        two.lines().nth(1),
        Some(
            "  src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\"9: if (price < 0) return { valid: false, message: 'Price cannot be negative' };\\n10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",100"
        )
    );
}

#[test]
fn a_flag_out_of_range_is_a_usage_error_with_no_output() {
    for args in [["--min-score", "1.5"], ["--snippet-lines", "4"]] {
        let output = monokl(&["search", "price", ".", args[0], args[1]]);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&output), "", "{args:?}");
        assert!(stderr(&output).contains(&format!("invalid value '{}' for '{} <", args[1], args[0])), "{args:?}");
    }
}

#[test]
fn a_result_within_every_cap_is_printed_whole_without_a_cut_marker() {
    let output = monokl(&["search", "format", "."]);
    assert_eq!(
        stdout(&output),
        "search[3]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/utils/format.ts,1,3,function,null,\"function formatCurrency(amount: number, currency: string = 'USD'): string\",\"2: return new Intl.NumberFormat('en-US', { style: 'currency', currency }).format(amount);\",100\n\
         \x20 src/utils/validation.ts,8,12,function,null,\"function validatePrice(price: number): ValidationResult\",\"10: if (price > 1_000_000) return { valid: false, message: formatCurrency(1_000_000) + ' is the maximum' };\",18\n\
         \x20 src/index.ts,3,3,other,null,null,\"3: export { formatCurrency, truncateString } from './utils/format';\",17\n"
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

const SEARCH_HEADER: &str = "search[N]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:";

/// A project with one Rust file and one TypeScript file that both mention `total_price`.
fn mixed_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/cart.rs"), CART_RS).unwrap();
    std::fs::write(dir.path().join("src/cart.ts"), "export function total_price(): number {\n  return 1;\n}\n")
        .unwrap();
    dir
}

const PRICING_RS: &str = "\
pub struct Money {
    cents: u64,
}

pub fn price(item: &Item) -> Money {
    Money { cents: item.cents }
}

pub fn total_price(items: &[Item]) -> Money {
    let sum = items.iter().map(|i| price(i).cents).sum();
    Money { cents: sum }
}

pub fn format_money(money: &Money) -> String {
    format!(\"{}.{:02}\", money.cents / 100, money.cents % 100)
}

pub fn describe(item: &Item) -> String {
    let shown = format_money(&price(item));
    format!(\"{} costs {}\", item.name, shown)
}

pub struct Item {
    pub name: String,
    pub cents: u64,
}
";

#[cfg(feature = "lang-rust")]
#[test]
fn a_rust_snippet_is_one_line_by_default_and_the_old_three_lines_on_request() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/pricing.rs"), PRICING_RS).unwrap();

    // Five rows are fewer than the floor, so every row prints; only the second row's second matching line is counted
    // instead of shown.
    let output = monokl_in(dir.path(), &["search", "price money", "."]);
    assert_eq!(
        stdout(&output),
        "search[5]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/pricing.rs,5,7,function,null,\"pub fn price(item: &Item) -> Money\",\"6: Money { cents: item.cents }\",100\n\
         \x20 src/pricing.rs,9,12,function,null,\"pub fn total_price(items: &[Item]) -> Money\",\"10: let sum = items.iter().map(|i| price(i).cents).sum();\\n+1 more\",89\n\
         \x20 src/pricing.rs,18,21,function,null,\"pub fn describe(item: &Item) -> String\",\"19: let shown = format_money(&price(item));\",26\n\
         \x20 src/pricing.rs,14,16,function,null,\"pub fn format_money(money: &Money) -> String\",\"15: format!(\\\"{}.{:02}\\\", money.cents / 100, money.cents % 100)\",16\n\
         \x20 src/pricing.rs,1,3,struct,null,pub struct Money,null,13\n"
    );

    let output = monokl_in(dir.path(), &["search", "price money", ".", "--min-score", "0", "--snippet-lines", "3"]);
    assert_eq!(
        stdout(&output),
        "search[5]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/pricing.rs,5,7,function,null,\"pub fn price(item: &Item) -> Money\",\"6: Money { cents: item.cents }\",100\n\
         \x20 src/pricing.rs,9,12,function,null,\"pub fn total_price(items: &[Item]) -> Money\",\"10: let sum = items.iter().map(|i| price(i).cents).sum();\\n11: Money { cents: sum }\",89\n\
         \x20 src/pricing.rs,18,21,function,null,\"pub fn describe(item: &Item) -> String\",\"19: let shown = format_money(&price(item));\",26\n\
         \x20 src/pricing.rs,14,16,function,null,\"pub fn format_money(money: &Money) -> String\",\"15: format!(\\\"{}.{:02}\\\", money.cents / 100, money.cents % 100)\",16\n\
         \x20 src/pricing.rs,1,3,struct,null,pub struct Money,null,13\n"
    );
}

const LEDGER_RS: &str = "\
/// Quotes the ledger for a customer.
#[inline]
pub fn quote<T>(ledger: T) -> u64
where
    T: Ledger,
{
    ledger.total()
}

/// Totals the ledger.
pub fn total(ledger: &Ledger) -> u64 {
    ledger.sum()
}
";

#[cfg(feature = "lang-rust")]
#[test]
fn a_rust_row_shows_a_code_line_not_a_doc_comment_and_leaves_the_where_clause_out_of_the_signature() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/ledger.rs"), LEDGER_RS).unwrap();

    let output = monokl_in(dir.path(), &["search", "quote ledger", "."]);
    assert_eq!(
        stdout(&output),
        "search[2]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/ledger.rs,1,8,function,null,\"pub fn quote<T>(ledger: T) -> u64 where …\",\"7: ledger.total()\\n+2 more\",100\n\
         \x20 src/ledger.rs,10,13,function,null,\"pub fn total(ledger: &Ledger) -> u64\",\"12: ledger.sum()\\n+1 more\",12\n"
    );

    // The doc comment is the only matching line left once the body does not match, so it is shown.
    let output = monokl_in(dir.path(), &["search", "customer", "."]);
    assert_eq!(
        stdout(&output),
        "search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/ledger.rs,1,8,function,null,\"pub fn quote<T>(ledger: T) -> u64 where …\",\"1: /// Quotes the ledger for a customer.\",100\n"
    );
}

#[cfg(feature = "lang-rust")]
#[test]
fn a_page_of_rust_rows_keeps_the_where_cut_and_the_more_marker_and_names_the_next_offset() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/ledger.rs"), LEDGER_RS).unwrap();

    let output = monokl_in(dir.path(), &["search", "quote ledger", ".", "--max-results", "1"]);
    assert_eq!(
        stdout(&output),
        "search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/ledger.rs,1,8,function,null,\"pub fn quote<T>(ledger: T) -> u64 where …\",\"7: ledger.total()\\n+2 more\",100\n\
         totalCount: 2\n\
         help[1]: \"showing 1 of 2; next: --offset 1, or refine with +term/-term\"\n"
    );

    let output = monokl_in(dir.path(), &["search", "quote ledger", ".", "--max-results", "1", "--offset", "1"]);
    assert_eq!(
        stdout(&output),
        "search[1]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,relevance}:\n\
         \x20 src/ledger.rs,10,13,function,null,\"pub fn total(ledger: &Ledger) -> u64\",\"12: ledger.sum()\\n+1 more\",12\n\
         totalCount: 2\n\
         help[1]: showing 1 of 2; refine with +term/-term\n"
    );
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
            "{}\n  src/cart.rs,6,9,method,impl Cart,pub fn total_price(&self) -> u32,null,100\n",
            SEARCH_HEADER.replace("[N]", "[1]")
        )
    );

    let output = monokl_in(dir.path(), &["search", "total_price", "src", "--language", "rust", "--tests"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "{}\n  src/cart.rs,6,9,method,impl Cart,pub fn total_price(&self) -> u32,null,100\n  src/cart.rs,14,17,function,module tests,fn total_price_adds_up(),null,41\n",
            SEARCH_HEADER.replace("[N]", "[2]")
        )
    );
}

/// The files of `files` and `monokl search <query> .` over them: the rows without their header.
fn rows_for(files: &[(&str, &str)], query: &str) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in files {
        std::fs::write(dir.path().join(name), body).unwrap();
    }
    let output = monokl_in(dir.path(), &["search", query, "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    stdout(&output).lines().skip(1).map(str::to_owned).collect()
}

#[test]
fn two_typescript_declarations_on_one_line_each_get_a_row_when_each_holds_a_match() {
    let rows = rows_for(&[("a.ts", "const first = 1; const second = 2;\n")], "first second");
    assert_eq!(
        rows,
        [
            "  a.ts,1,1,variable,null,null,\"1: const first = 1;\",100",
            "  a.ts,1,1,variable,null,null,\"1: const second = 2;\",100",
        ]
    );
}

#[cfg(feature = "lang-rust")]
#[test]
fn two_rust_items_on_one_line_each_get_a_row_when_each_holds_a_match() {
    let rows = rows_for(&[("a.rs", "fn first() {} fn second() {}\n")], "first second");
    assert_eq!(
        rows,
        ["  a.rs,1,1,function,null,fn first(),null,100", "  a.rs,1,1,function,null,fn second(),null,100",]
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

/// `count` TypeScript files that each hold one function matching `needle`.
fn project_of_many_matching_files(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..count {
        let folder = dir.path().join(format!("d{:03}", index % 200));
        std::fs::create_dir_all(&folder).unwrap();
        let text = format!("export function needle{index}(a: number) {{\n  return a + {index};\n}}\n");
        std::fs::write(folder.join(format!("f{index:05}.ts")), text).unwrap();
    }
    dir
}

#[test]
fn a_tiny_timeout_returns_a_partial_result_that_says_so_and_still_succeeds() {
    let dir = project_of_many_matching_files(20_000);
    let output = monokl_in(dir.path(), &["search", "needle", ".", "--timeout", "0.001", "--max-results", "1"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    // Text search always finishes its first batch, which is 64 files per thread.
    let reached = rayon::current_num_threads() * 64;
    let expected = format!("partial result: 0.001 s timeout expired after searching {reached} of 20000 files");
    assert!(stdout(&output).contains(&expected), "wanted `{expected}` in: {}", stdout(&output));
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_timeout_of_zero_runs_to_completion_without_a_partial_marker() {
    let dir = project_of_many_matching_files(2_000);
    let output = monokl_in(dir.path(), &["search", "needle", ".", "--timeout", "0", "--max-results", "1"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("totalCount: 2000"), "got: {}", stdout(&output));
    assert!(!stdout(&output).contains("partial result"), "got: {}", stdout(&output));
}

#[test]
fn help_names_the_timeout_its_default_and_how_to_disable_it() {
    let output = monokl(&["search", "--help"]);
    let help = stdout(&output);
    assert!(help.contains("--timeout <SECONDS>"), "got: {help}");
    assert!(help.contains("0 means no limit"), "got: {help}");
    assert!(help.contains("[default: 30]"), "got: {help}");
}

#[test]
fn help_ties_min_score_to_the_relevance_column() {
    let help = stdout(&monokl(&["search", "--help"]));
    let min_score = &help[help.find("--min-score").unwrap()..];
    let flag = &min_score[..min_score.find("--snippet-lines").unwrap()];
    assert!(flag.contains("relevance"), "got: {flag}");
}

#[test]
fn a_timeout_that_is_not_a_number_of_seconds_is_a_usage_error() {
    for bad in ["soon", "-1", "NaN", "inf"] {
        let output = monokl(&["search", "format", ".", "--timeout", bad]);
        assert_eq!(output.status.code(), Some(2), "{bad}: {}", stderr(&output));
    }
}

#[test]
fn files_over_the_size_limit_are_skipped_and_reported_in_one_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("small.ts"), "export const needle = 1;\n").unwrap();
    for name in ["big_a.ts", "big_b.ts"] {
        let mut text = b"export const needle = 1;\n".to_vec();
        text.resize(4 * 1024 * 1024 + 1, b' ');
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let output = monokl_in(dir.path(), &["search", "needle", "."]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("2 files skipped: over 4 MiB: big_a.ts, big_b.ts"), "got: {text}");
    assert!(text.contains("small.ts,1,1,"), "got: {text}");
    assert!(!text.contains("big_a.ts,1"), "got: {text}");
}

/// A project of `files` source files with the extension `ext`, each declaring `per_file` functions whose names hold the
/// word `item`, so one query matches every block.
fn project_of_items(ext: &str, files: usize, per_file: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for file in 0..files {
        let body: String = (0..per_file)
            .map(|n| match ext {
                "rs" => format!("pub fn item_{file}_{n}() -> u32 {{\n    {n}\n}}\n\n"),
                _ => format!("export function item_{file}_{n}(): number {{\n  return {n};\n}}\n\n"),
            })
            .collect();
        std::fs::write(dir.path().join(format!("m{file:02}.{ext}")), body).unwrap();
    }
    dir
}

/// The rows of a printed result, without the header, the count and the hints.
fn rows(text: &str) -> Vec<String> {
    text.lines().skip(1).take_while(|line| line.starts_with("  ")).map(str::to_owned).collect()
}

/// The `help[1]` line of a printed result, without its quotes.
fn first_help(text: &str) -> Option<String> {
    text.lines().find_map(|line| line.strip_prefix("help[1]: ").map(|h| h.trim_matches('"').to_owned()))
}

#[test]
fn pages_laid_end_to_end_are_the_one_large_run_in_typescript_and_rust() {
    for ext in ["ts", "rs"] {
        let dir = project_of_items(ext, 6, 6);
        let big = stdout(&monokl_in(dir.path(), &["search", "item", ".", "--min-score", "0", "--max-results", "100"]));
        let whole = rows(&big);
        assert_eq!(whole.len(), 36, "{ext}");
        let mut paged = Vec::new();
        for offset in ["0", "10", "20", "30"] {
            let args = ["search", "item", ".", "--min-score", "0", "--max-results", "10", "--offset", offset];
            let page = monokl_in(dir.path(), &args);
            assert!(page.status.success(), "{ext} {offset}: {}", stderr(&page));
            paged.extend(rows(&stdout(&page)));
        }
        assert_eq!(paged, whole, "{ext}");
    }
}

#[test]
fn the_help_line_names_the_exact_next_offset_and_the_last_page_names_none() {
    for ext in ["ts", "rs"] {
        let dir = project_of_items(ext, 6, 6);
        let first = stdout(&monokl_in(dir.path(), &["search", "item", ".", "--min-score", "0", "--max-results", "8"]));
        assert_eq!(rows(&first).len(), 8, "{ext}");
        assert_eq!(
            first_help(&first).as_deref(),
            Some("showing 8 of 36; next: --offset 8, or refine with +term/-term"),
            "{ext}"
        );
        let args = ["search", "item", ".", "--min-score", "0", "--max-results", "8", "--offset", "32"];
        let last = stdout(&monokl_in(dir.path(), &args));
        assert_eq!(rows(&last).len(), 4, "{ext}");
        assert_eq!(first_help(&last).as_deref(), Some("showing 4 of 36; refine with +term/-term"), "{ext}");
    }
}

#[test]
fn an_offset_past_the_end_prints_no_rows_and_the_count_and_exits_zero() {
    for ext in ["ts", "rs"] {
        let dir = project_of_items(ext, 2, 5);
        let output = monokl_in(dir.path(), &["search", "item", ".", "--min-score", "0", "--offset", "10"]);
        assert!(output.status.success(), "{ext}: {}", stderr(&output));
        assert_eq!(
            stdout(&output),
            "search: []\ntotalCount: 10\nhelp[1]: showing 0 of 10; offset 10 is past the last of 10 rows\n",
            "{ext}"
        );
        assert_eq!(stderr(&output), "");
    }
}

#[test]
fn the_default_relevance_cut_gives_the_same_rows_whatever_the_offset() {
    let dir = project_of_items("rs", 3, 4);
    let default = stdout(&monokl_in(dir.path(), &["search", "item", ".", "--max-results", "100"]));
    let second = stdout(&monokl_in(dir.path(), &["search", "item", ".", "--max-results", "100", "--offset", "5"]));
    assert_eq!(rows(&second), rows(&default)[5..], "the same rows follow the same cut");
}

#[test]
fn the_token_budget_bounds_the_page_and_not_the_rows_before_it() {
    let dir = project_of_items("ts", 6, 6);
    let all = ["search", "item", ".", "--min-score", "0", "--max-results", "100"];
    let whole = rows(&stdout(&monokl_in(dir.path(), &all)));
    let tight = [all.as_slice(), &["--max-tokens", "60", "--offset", "30"]].concat();
    let page = rows(&stdout(&monokl_in(dir.path(), &tight)));
    assert!(!page.is_empty() && page.len() < 6, "the budget cuts the last page too: {}", page.len());
    assert_eq!(page, whole[30..30 + page.len()]);
}

#[test]
fn a_non_numeric_offset_is_a_usage_error() {
    let output = monokl(&["search", "format", ".", "--offset", "next"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("invalid value 'next' for '--offset <N>'"), "got: {}", stderr(&output));
}

/// Twelve files whose function matches both query words and twenty whose function matches one, so the default relevance
/// cut keeps the first and hides the second.
fn project_with_strong_and_weak_matches() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..12 {
        std::fs::write(dir.path().join(format!("s{n:02}.rs")), format!("pub fn alpha_beta_{n}() -> u32 {{ {n} }}\n"))
            .unwrap();
    }
    for n in 0..20 {
        std::fs::write(dir.path().join(format!("w{n:02}.rs")), format!("pub fn alpha_{n}() -> u32 {{ {n} }}\n"))
            .unwrap();
    }
    dir
}

#[test]
fn following_the_hint_walks_the_rows_that_pass_the_cut_and_never_reaches_an_empty_page() {
    let dir = project_with_strong_and_weak_matches();
    let walk = |extra: &[&str], first_hint: &str, last_hint: &str| {
        let mut seen = Vec::new();
        let mut offset = 0;
        let mut hints = Vec::new();
        loop {
            let off = offset.to_string();
            let args = [&["search", "alpha beta", ".", "--max-results", "5", "--offset", off.as_str()], extra].concat();
            let text = stdout(&monokl_in(dir.path(), &args));
            let page = rows(&text);
            assert!(!page.is_empty(), "the hint led to an empty page at offset {offset}: {text}");
            offset += page.len();
            seen.extend(page);
            let help = first_help(&text).unwrap();
            let next =
                help.split("next: --offset ").nth(1).map(|rest| rest.split([',', ';']).next().unwrap().to_owned());
            hints.push(help);
            match next {
                Some(n) => assert_eq!(n, offset.to_string()),
                None => break,
            }
        }
        assert!(hints[0].starts_with(first_hint), "{}", hints[0]);
        assert!(hints.last().unwrap().starts_with(last_hint), "{}", hints.last().unwrap());
        (seen, hints.len())
    };
    let big = |extra: &[&str]| {
        let args = [&["search", "alpha beta", ".", "--max-results", "100"], extra].concat();
        let text = stdout(&monokl_in(dir.path(), &args));
        (rows(&text), first_help(&text))
    };

    let (kept, help) = big(&[]);
    assert_eq!(kept.len(), 12);
    assert_eq!(
        help.as_deref(),
        Some("showing 12 of 32; --min-score 0 for the 20 rows below relevance 30, or refine with +term/-term")
    );
    let (seen, pages) = walk(
        &[],
        "showing 5 of 32; next: --offset 5, --min-score 0 for the 20 rows below relevance 30",
        "showing 2 of 32; --min-score 0 for the 20 rows below relevance 30, or refine",
    );
    assert_eq!((seen, pages), (kept.clone(), 3));

    let (all, help) = big(&["--min-score", "0"]);
    assert_eq!((all.len(), help), (32, None));
    assert_eq!(all[..12], kept[..]);
    let (seen, pages) =
        walk(&["--min-score", "0"], "showing 5 of 32; next: --offset 5, or refine", "showing 2 of 32; refine");
    assert_eq!((seen, pages), (all, 7));
}
