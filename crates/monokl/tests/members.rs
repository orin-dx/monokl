//! Class members are searched as blocks of their own, next to the class that holds them.
#![cfg(all(feature = "cli", feature = "lang-ts"))]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a failing assertion should panic in a test")]

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::sync::Arc;

use camino::Utf8Path;
use monokl::analysis::registry::AnalyzerRegistry;
use monokl::analysis::ts::TsAnalyzer;
use monokl::pipeline::{SearchRequest, SearchResult, search};

const HEADER: &str = "search[N]{file,lineStart,lineEnd,nodeKind,parent,symbolSignature,snippet,rank,finalScore}:";

/// A class with a constructor, two methods, an arrow-function property and two members too short to get a block.
const FOO: &str = "\
export class Foo {
  constructor(private readonly store: Store) {
    this.store = store;
  }
  async loadUser(id: string): Promise<User> {
    return this.store.get(id);
  }
  saveUser(user: User): void {
    this.store.put(user);
  }
  onChange = (event: Event) => {
    this.dispatch(event);
  };
  tiny() { return 0; }
  pair() {
  }
}
";

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

/// The text `monokl search <query> .` prints in `dir`.
fn run(dir: &tempfile::TempDir, query: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_monokl"))
        .args(["search", query, "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

/// Expected output for `rows`, each given as the whole printed line.
fn table(rows: &[&str]) -> String {
    let header = HEADER.replace('N', &rows.len().to_string());
    let body: String = rows.iter().flat_map(|row| [*row, "\n"]).collect();
    format!("{header}\n{body}")
}

#[test]
fn a_query_naming_one_method_returns_that_method_alone_with_its_class() {
    let dir = project(&[("src/foo.ts", FOO)]);
    assert_eq!(
        run(&dir, "loadUser"),
        table(&["  src/foo.ts,5,7,method,class Foo,\"async loadUser(id: string): Promise<User>\",null,1,2.307646"])
    );
}

#[test]
fn a_query_naming_the_class_returns_the_class_alone() {
    let dir = project(&[("src/foo.ts", FOO)]);
    assert_eq!(run(&dir, "Foo"), table(&["  src/foo.ts,1,17,class,null,null,\"1: export class Foo {\",1,1.402755"]));
}

#[test]
fn a_constructor_is_a_row_of_its_own() {
    let dir = project(&[("src/foo.ts", FOO)]);
    assert_eq!(
        run(&dir, "constructor"),
        table(&[
            "  src/foo.ts,2,4,constructor,class Foo,\"constructor(private readonly store: Store)\",null,1,1.575364"
        ])
    );
}

#[test]
fn an_arrow_function_property_is_a_row_of_its_own() {
    let dir = project(&[("src/foo.ts", FOO)]);
    assert_eq!(
        run(&dir, "onChange"),
        table(&["  src/foo.ts,11,13,property,class Foo,\"onChange = (event: Event) =>\",null,1,1.821949"])
    );
}

#[test]
fn a_member_shorter_than_three_lines_is_found_through_its_class() {
    let dir = project(&[("src/foo.ts", FOO)]);
    assert_eq!(
        run(&dir, "tiny"),
        table(&["  src/foo.ts,1,17,class,null,null,\"14: tiny() { return 0; }\",1,1.402755"])
    );
    assert_eq!(run(&dir, "pair"), table(&["  src/foo.ts,1,17,class,null,null,\"15: pair() {\",1,1.402755"]));
}

#[test]
fn every_member_matching_a_query_gets_its_own_row_in_rank_order() {
    let dir = project(&[("src/foo.ts", FOO)]);
    assert_eq!(
        run(&dir, "store"),
        table(&[
            "  src/foo.ts,2,4,constructor,class Foo,\"constructor(private readonly store: Store)\",\"3: this.store = store;\",1,1.375721",
            "  src/foo.ts,8,10,method,class Foo,\"saveUser(user: User): void\",\"9: this.store.put(user);\",2,1.189721",
            "  src/foo.ts,5,7,method,class Foo,\"async loadUser(id: string): Promise<User>\",\"6: return this.store.get(id);\",3,1.181628",
        ])
    );
}

#[test]
fn the_same_search_prints_the_same_rows_every_time() {
    let dir = project(&[("src/foo.ts", FOO), ("src/bar.ts", FOO.replace("Foo", "Bar").as_str())]);
    let first = run(&dir, "store user");
    for _ in 0..5 {
        assert_eq!(run(&dir, "store user"), first);
    }
}

fn library_search(dir: &tempfile::TempDir, query: &str) -> SearchResult {
    let registry = AnalyzerRegistry::new(vec![Arc::new(TsAnalyzer::new())]);
    let root = Utf8Path::from_path(dir.path()).unwrap();
    let request =
        SearchRequest { root, query, exact: false, allow_tests: false, respect_ignore_files: true, language: None };
    search(&registry, &request).unwrap()
}

#[test]
fn no_line_is_credited_to_two_blocks_whatever_the_query() {
    let dir = project(&[("src/foo.ts", FOO)]);
    for query in ["store", "this", "Foo", "user", "return", "event dispatch", "regex:\\w+", "constructor tiny"] {
        let result = library_search(&dir, query);
        let mut owners: BTreeMap<usize, usize> = BTreeMap::new();
        for ranked in &result.ranked {
            for &line in &ranked.block.matched_lines {
                assert!(
                    (ranked.block.line_start..=ranked.block.line_end).contains(&line),
                    "{query}: line {line} outside {}..={}",
                    ranked.block.line_start,
                    ranked.block.line_end
                );
                *owners.entry(line).or_default() += 1;
            }
        }
        let shared: Vec<_> = owners.iter().filter(|&(_, &count)| count > 1).collect();
        assert_eq!(shared, [], "{query}: lines in more than one block");
    }
}

#[test]
fn a_class_keeps_only_the_matches_outside_its_members() {
    let source = "\
export class Foo {
  label = 'needle';
  run() {
    return 'needle';
  }
  other() {
    return 1;
  }
}
";
    let dir = project(&[("src/foo.ts", source)]);
    let result = library_search(&dir, "needle");
    let mut rows: Vec<_> = result
        .ranked
        .iter()
        .map(|r| (r.block.line_start, r.block.line_end, r.block.matched_lines.clone(), r.parent_context.is_some()))
        .collect();
    rows.sort();
    assert_eq!(rows, [(1, 9, vec![2], false), (3, 5, vec![4], true)]);
}

#[test]
fn a_member_row_is_ranked_with_the_kind_boost_of_its_own_kind() {
    let dir = project(&[("src/foo.ts", FOO)]);
    let result = library_search(&dir, "loadUser");
    let ranked = &result.ranked[0];
    assert_eq!(
        (
            ranked.block.node_kind,
            ranked.node_type_boost,
            ranked.rank,
            ranked.parent_context.as_ref().map(|p| p.name.as_str())
        ),
        (monokl::types::SymbolKind::Method, 1.4, 1, Some("Foo"))
    );
}
