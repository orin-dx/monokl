//! The thread every parse runs on.
//!
//! A stack overflow aborts the process, so what a parse may consume is bounded before it starts, not found out by
//! running. The bound has two halves: [`nesting::check`](super::nesting::check) rejects a file whose recursion could
//! pass [`PARSE_STACK_BYTES`], and every parse runs on a stack of exactly that size. Neither depends on the input's
//! length.

use std::thread;

use super::file_analysis::Degradation;

/// The stack of the thread that parses and extracts one file, whatever the file.
///
/// Measured in a build without optimizations, which uses the most: the deepest recursion per nesting unit (see
/// `nesting::FRAME_COST`) is 1.7 KB, from a `new` or a generic argument list in TypeScript, and 1.9 KB from a unary
/// minus in Rust. The languages' caps are 16,384 and 2,048 units. Every one of the nestings the tests build at the cap,
/// 38 in TypeScript and 32 in Rust, parses in 24 MiB and the worst overflows 16 MiB, so 64 MiB is at least 2.6 times
/// what the worst accepted file needs. Only the pages a parse touches are committed.
pub(crate) const PARSE_STACK_BYTES: usize = 64 << 20;

/// Runs `run` on a thread with [`PARSE_STACK_BYTES`] of stack. A panic in it, or a thread that cannot be started,
/// yields the [`Degradation`] that says so.
///
/// A scoped thread per call costs about 30 microseconds (1,000 in 0.03 s, 64 MiB stack each) against the milliseconds a
/// parse takes, and shares nothing, so it keeps the analyzers `Send` and `Sync` without a pool to size.
pub(crate) fn on_parse_stack<T: Send>(run: impl FnOnce() -> T + Send) -> Result<T, Degradation> {
    thread::scope(|scope| {
        let builder = thread::Builder::new().name("monokl-parse".to_owned()).stack_size(PARSE_STACK_BYTES);
        let Ok(handle) = builder.spawn_scoped(scope, run) else {
            return Err(Degradation::ThreadUnavailable);
        };
        // The closure's output is discarded on a panic, so no torn state is observable afterwards.
        handle.join().map_err(|_panic| Degradation::Panicked)
    })
}

#[cfg(test)]
mod tests {
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
}
