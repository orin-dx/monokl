//! The thread every parse runs on.
//!
//! A stack overflow aborts the process, so what a parse may consume is bounded before it starts, not found out by
//! running. The bound has two halves: [`nesting::check`](super::nesting::check) rejects a file whose recursion could
//! pass [`PARSE_STACK_BYTES`], and every parse runs on a stack of exactly that size. Neither depends on the input's
//! length.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::thread;

use super::file_analysis::Degradation;

/// The stack of the thread that parses and extracts one file, whatever the file.
///
/// Measured in a build without optimizations, which uses the most: the deepest recursion per nesting unit (see
/// `nesting::FRAME_COST`) is 1.7 KB, from a `new` or a generic argument list in TypeScript, and 1.9 KB from a unary
/// minus in Rust. The languages' caps are 65,536 and 1,024 units. Every one of the nestings the tests build at the cap,
/// 38 in TypeScript and 32 in Rust, parses in 96 MiB and the worst overflows 64 MiB, so 256 MiB is at least 2.6 times
/// what the worst accepted file needs. Only the pages a parse touches are committed.
pub(crate) const PARSE_STACK_BYTES: usize = 256 << 20;

thread_local! {
    /// Whether [`with_parse_threads`] started this thread, with [`PARSE_STACK_BYTES`] of stack.
    static HAS_PARSE_STACK: Cell<bool> = const { Cell::new(false) };
}

/// Runs `run` on a stack of [`PARSE_STACK_BYTES`]: the calling thread when [`with_parse_threads`] started it, a new
/// scoped thread otherwise. A panic in it, or a thread that cannot be started, yields the [`Degradation`] that says so.
///
/// A scoped thread per call costs about 30 microseconds on an idle machine, but 20,000 of them across every core of a
/// busy one took 3.5 s of CPU against 35 ms of parsing, mostly mapping and unmapping 256 MiB stacks. A caller that
/// analyzes many files starts its threads once with [`with_parse_threads`] instead.
pub(crate) fn on_parse_stack<T: Send>(run: impl FnOnce() -> T + Send) -> Result<T, Degradation> {
    // The closure's output is discarded on a panic, so no torn state is observable afterwards.
    if HAS_PARSE_STACK.get() {
        return panic::catch_unwind(AssertUnwindSafe(run)).map_err(|_panic| Degradation::Panicked);
    }
    thread::scope(|scope| {
        let builder = thread::Builder::new().name("monokl-parse".to_owned()).stack_size(PARSE_STACK_BYTES);
        let Ok(handle) = builder.spawn_scoped(scope, run) else {
            return Err(Degradation::ThreadUnavailable);
        };
        handle.join().map_err(|_panic| Degradation::Panicked)
    })
}

/// Starts the threads for analyzing `files` files, as many as the files can use: none for no files, otherwise
/// `min(cpus(), files)`. `cpus` is not asked for when there are no files, because asking rayon starts its own pool.
/// `None` when no thread is needed or the pool could not be started.
fn parse_pool(files: usize, cpus: impl FnOnce() -> usize) -> Option<rayon::ThreadPool> {
    if files == 0 {
        return None;
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(cpus().min(files))
        .stack_size(PARSE_STACK_BYTES)
        .thread_name(|index| format!("monokl-parse-{index}"))
        .start_handler(|_| HAS_PARSE_STACK.set(true))
        .build()
        .ok()
}

/// Runs `run`, which analyzes `files` files, on threads that each have [`PARSE_STACK_BYTES`] of stack, so an
/// [`on_parse_stack`] made by a rayon task inside it parses where it stands instead of starting a thread.
///
/// The threads are started when there is a file to analyze, and no more than the files or the CPUs of the pool `run`
/// is called from: a search that analyzes nothing starts no thread, and one that analyzes three files starts three.
/// When they cannot be started, `run` goes ahead on the calling thread and every parse starts its own thread, as it
/// does outside this function.
pub(crate) fn with_parse_threads<T: Send>(files: usize, run: impl FnOnce() -> T + Send) -> T {
    match parse_pool(files, rayon::current_num_threads) {
        Some(pool) => pool.install(run),
        None => run(),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn a_panic_becomes_the_panicked_degradation() {
        let result: Result<(), Degradation> = on_parse_stack(|| panic!("boom"));
        assert_eq!(result, Err(Degradation::Panicked));
    }

    #[test]
    fn a_run_that_does_not_panic_returns_its_value() {
        assert_eq!(on_parse_stack(|| vec!["x"]), Ok(vec!["x"]));
    }

    /// Recurses `depth` times with a 1 KiB frame and returns how many frames it went through.
    fn recurse(depth: usize) -> usize {
        let frame = std::hint::black_box([1u8; 1024]);
        if depth == 0 {
            return 0;
        }
        recurse(depth - 1) + usize::from(std::hint::black_box(frame)[0])
    }

    #[test]
    fn the_run_gets_the_fixed_stack() {
        // 8 MiB or more of frames would overflow the 2 MiB a test thread gets.
        assert_eq!(on_parse_stack(|| recurse(8 << 10)), Ok(8 << 10));
    }

    #[test]
    fn a_thousand_parse_threads_in_a_row_start_without_a_pool() {
        let started = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(on_parse_stack(|| 1), Ok(1));
        }
        // Not an assertion about speed: it only fails if starting a thread is so slow that a search of many files is
        // not viable, which a second per thousand would be.
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn a_pool_thread_parses_where_it_stands_and_any_other_thread_starts_one() {
        let here = thread::current().id();
        assert_ne!(on_parse_stack(|| thread::current().id()), Ok(here));
        let (pool_thread, parse_thread) =
            with_parse_threads(1, || (thread::current().id(), on_parse_stack(|| thread::current().id())));
        assert_ne!(pool_thread, here);
        assert_eq!(parse_thread, Ok(pool_thread));
    }

    #[test]
    fn a_pool_thread_has_the_fixed_stack_and_survives_a_panic() {
        let results = with_parse_threads(1, || {
            (
                on_parse_stack(|| recurse(8 << 10)),
                on_parse_stack(|| -> () { panic!("boom") }),
                on_parse_stack(|| recurse(8 << 10)),
            )
        });
        assert_eq!(results, (Ok(8 << 10), Err(Degradation::Panicked), Ok(8 << 10)));
    }

    #[test]
    fn no_files_start_no_pool_and_do_not_even_ask_for_the_cpu_count() {
        let pool = parse_pool(0, || panic!("the cpu count is not needed for no files"));
        assert!(pool.is_none());
        assert_eq!(with_parse_threads(0, || 7), 7);
    }

    #[rstest]
    #[case::one_file(1, 8, 1)]
    #[case::two_files(2, 8, 2)]
    #[case::as_many_files_as_cpus(8, 8, 8)]
    #[case::many_files(1_000, 8, 8)]
    #[case::one_cpu(1_000, 1, 1)]
    fn the_pool_is_as_wide_as_the_cpus_or_the_files_whichever_is_fewer(
        #[case] files: usize,
        #[case] cpus: usize,
        #[case] width: usize,
    ) {
        let pool = parse_pool(files, || cpus).unwrap();
        assert_eq!(pool.current_num_threads(), width);
    }

    #[test]
    fn the_pool_is_sized_from_the_pool_it_is_started_from() {
        let two = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        assert_eq!(two.install(|| with_parse_threads(100, rayon::current_num_threads)), 2);
        assert_eq!(two.install(|| with_parse_threads(1, rayon::current_num_threads)), 1);
    }
}
