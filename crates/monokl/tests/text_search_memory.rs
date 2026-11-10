//! The text search keeps memory bounded by a batch of files plus the kept files, however many files match.

use std::error::Error;
use std::fs;

use camino::Utf8Path;
use monokl::text_search::{TextSearchOptions, search_files};
use peak_alloc::PeakAlloc;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

const FILES: usize = 400;
const LINES_PER_FILE: usize = 4;
const LINE_BYTES: usize = 16 * 1024;

#[test]
fn matching_files_past_the_ceiling_are_not_held_together() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let root = Utf8Path::from_path(dir.path()).ok_or("temp dir is not UTF-8")?;
    let line = format!("alpha {}\n", "x".repeat(LINE_BYTES));
    for index in 0..FILES {
        fs::write(root.join(format!("f{index:04}.ts")), line.repeat(LINES_PER_FILE))?;
    }
    // Every file's hits are about 64 KiB, so holding all of them takes about 25 MiB.
    let all_hits = FILES * LINES_PER_FILE * LINE_BYTES;
    let opts = TextSearchOptions { max_files: 1, ..TextSearchOptions::default() };
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;

    ALLOCATOR.reset_peak_usage();
    let result = pool.install(|| search_files(root, &["alpha"], &opts))?;
    let peak = ALLOCATOR.peak_usage();

    assert_eq!(result.files_total, FILES as u64);
    assert!(result.ceiling_hit);
    assert_eq!(result.hits.keys().map(|path| path.file_name()).collect::<Vec<_>>(), [Some("f0000.ts")]);
    assert!(peak < all_hits / 3, "peak {peak} bytes is not well below all hits at {all_hits} bytes");
    Ok(())
}
