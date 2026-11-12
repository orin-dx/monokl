//! Work grows near-linearly with the number of matching files and candidate blocks.
//!
//! Each test runs one stage on an input and on one four times larger, takes the fastest of several runs of each so a
//! busy machine does not fail it, and asserts the time ratio. A step that is quadratic in its input would take 16 times
//! as long; the bound of 10 leaves room for cache effects and allocator noise. Only the ratio is asserted, never an
//! absolute time, so a slow runner does not fail it either.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a failing assertion should panic in a test")]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use monokl::dedup::dedup_blocks;
use monokl::rank::{rank_blocks, tokenize_block};
use monokl::text_search::{TextSearchOptions, search_files};
use monokl::types::{CodeBlock, SymbolKind};

const SMALL: usize = 2_500;
const LARGE: usize = 4 * SMALL;
const RUNS: usize = 5;
/// A stage that scales linearly takes 4 times as long on the large input; a quadratic one takes 16 times.
const MAX_RATIO: f64 = 10.0;
/// Timings under this are noise, so the small input is never taken to be faster than it.
const TIMER_FLOOR: Duration = Duration::from_millis(1);

/// The fastest of `RUNS` runs of `work`, each on a fresh input from `setup` that is not part of the time.
fn fastest<T>(mut setup: impl FnMut() -> T, mut work: impl FnMut(T)) -> Duration {
    (0..RUNS)
        .map(|_| {
            let input = setup();
            let started = Instant::now();
            work(input);
            started.elapsed()
        })
        .min()
        .unwrap()
}

fn assert_near_linear(stage: &str, small: Duration, large: Duration) {
    let ratio = large.as_secs_f64() / small.max(TIMER_FLOOR).as_secs_f64();
    assert!(
        ratio < MAX_RATIO,
        "{stage}: {LARGE} inputs took {large:?} against {small:?} for {SMALL}, a ratio of {ratio:.1} (bound {MAX_RATIO})"
    );
}

/// How candidate blocks are laid out over files.
#[derive(Clone, Copy)]
enum Layout {
    /// Five blocks per file, none overlapping.
    ManyFiles,
    /// Every block in one file, none overlapping.
    OneFile,
    /// Every block in one file, each overlapping the ones around it.
    OneFileOverlapping,
}

fn candidates(count: usize, layout: Layout) -> Vec<(CodeBlock, Vec<String>)> {
    (0..count)
        .map(|index| {
            let (file, line_start, line_end) = match layout {
                Layout::ManyFiles => (format!("src/f{}.ts", index / 5), 1 + (index % 5) * 10, 8 + (index % 5) * 10),
                Layout::OneFile => ("src/one.ts".to_owned(), 1 + index * 4, 3 + index * 4),
                Layout::OneFileOverlapping => ("src/one.ts".to_owned(), 1 + index, 20 + index),
            };
            let block = CodeBlock {
                file: Utf8PathBuf::from(file),
                line_start,
                line_end,
                node_kind: SymbolKind::Function,
                code: format!("function handle{index}(input) {{\n  return needle + {index};\n}}"),
                symbol_signature: Some(format!("function handle{index}(input)")),
                matched_lines: vec![line_start + 1],
                parent: None,
            };
            let tokens = tokenize_block(&block);
            (block, tokens)
        })
        .collect()
}

fn rank_and_dedup(blocks: Vec<(CodeBlock, Vec<String>)>) {
    let df = HashMap::from([("needle".to_owned(), blocks.len())]);
    let ranked = dedup_blocks(rank_blocks(blocks, &["needle"], &df));
    assert!(!ranked.is_empty());
}

fn rank_and_dedup_scales_near_linearly(layout: Layout) {
    let small = fastest(|| candidates(SMALL, layout), rank_and_dedup);
    let large = fastest(|| candidates(LARGE, layout), rank_and_dedup);
    assert_near_linear("rank and dedup", small, large);
}

#[test]
fn ranking_and_dedup_scale_near_linearly_with_blocks_spread_over_files() {
    rank_and_dedup_scales_near_linearly(Layout::ManyFiles);
}

#[test]
fn ranking_and_dedup_scale_near_linearly_with_every_block_in_one_file() {
    rank_and_dedup_scales_near_linearly(Layout::OneFile);
}

#[test]
fn ranking_and_dedup_scale_near_linearly_with_overlapping_blocks_in_one_file() {
    rank_and_dedup_scales_near_linearly(Layout::OneFileOverlapping);
}

/// A temp directory with `count` small files that each match `needle`.
fn matching_files(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..count {
        let folder = dir.path().join(format!("d{:03}", index % 200));
        std::fs::create_dir_all(&folder).unwrap();
        let text = format!("export function needle{index}(a: number) {{\n  return a + {index};\n}}\n");
        std::fs::write(folder.join(format!("f{index:05}.ts")), text).unwrap();
    }
    dir
}

fn utf8(dir: &tempfile::TempDir) -> &Utf8Path {
    Utf8Path::from_path(dir.path()).expect("temp dir is UTF-8")
}

#[test]
fn text_search_scales_near_linearly_with_matching_files() {
    let (small_dir, large_dir) = (matching_files(SMALL), matching_files(LARGE));
    let search = |dir: &tempfile::TempDir| {
        let result = search_files(utf8(dir), &["needle"], &TextSearchOptions::default()).unwrap();
        assert!(result.files_total > 0);
    };
    let small = fastest(|| (), |()| search(&small_dir));
    let large = fastest(|| (), |()| search(&large_dir));
    assert_near_linear("text search", small, large);
}

#[cfg(feature = "lang-ts")]
#[test]
fn a_whole_search_scales_near_linearly_with_matching_files() {
    use std::sync::Arc;

    use monokl::analysis::registry::AnalyzerRegistry;
    use monokl::analysis::ts::TsAnalyzer;
    use monokl::pipeline::{SearchRequest, search};

    let (small_dir, large_dir) = (matching_files(SMALL), matching_files(LARGE));
    let registry = AnalyzerRegistry::new(vec![Arc::new(TsAnalyzer::new())]);
    let run = |dir: &tempfile::TempDir| {
        let request = SearchRequest {
            root: utf8(dir),
            query: "needle",
            exact: false,
            allow_tests: false,
            respect_ignore_files: true,
            language: None,
        };
        assert!(!search(&registry, &request).unwrap().ranked.is_empty());
    };
    let small = fastest(|| (), |()| run(&small_dir));
    let large = fastest(|| (), |()| run(&large_dir));
    assert_near_linear("whole search", small, large);
}
