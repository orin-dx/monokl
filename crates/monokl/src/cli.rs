//! The `monokl` command line: parses arguments, runs a command and prints agent-facing output.
//!
//! This is the agent layer. It budgets a core result and renders it with michi; core never prints.

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::analysis::lang::{Language, TEST_DIRS};
use crate::analysis::registry::AnalyzerRegistry;
use crate::budget::{Budget, apply_budget};
use crate::error::MonoklError;
use crate::output::agent::{ToDomainError, render_search, search_row_texts};
use crate::pipeline::{SearchRequest, search_with_timeout};
use crate::text_search::DEFAULT_MAX_FILES;
use crate::types::{Diagnostic, DiagnosticKind};

/// AST-aware code search for coding agents.
#[derive(Debug, Parser)]
#[command(name = "monokl", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Searches a directory and prints the matching code blocks, best first.
    Search(SearchArgs),
    /// Prints the file rules of every language as JSON, for the scripts that compare monokl with ripgrep.
    #[command(hide = true)]
    LanguageRules,
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// The query: words, "quoted phrases", `+required`, `-excluded` and `regex:pattern` terms.
    query: String,
    /// The directory to search.
    #[arg(default_value = ".")]
    path: Utf8PathBuf,
    /// Treats the whole query as one literal term.
    #[arg(long)]
    exact: bool,
    /// Searches test files too.
    #[arg(long)]
    tests: bool,
    /// Searches hidden files and files that .gitignore or .ignore (in the search path or its parents) would exclude.
    #[arg(long)]
    no_ignore: bool,
    /// Searches only this language: typescript, javascript or rust.
    #[arg(long, value_enum)]
    language: Option<Language>,
    /// The most results to print.
    #[arg(long, default_value_t = Budget::default().max_results)]
    max_results: usize,
    /// The most estimated tokens (one per three bytes) across the printed rows.
    #[arg(long, default_value_t = Budget::default().max_tokens)]
    max_tokens: usize,
    /// Stops after this many seconds and prints what was found so far, marked as a partial result. 0 means no limit.
    #[arg(long, value_name = "SECONDS", default_value = "30", value_parser = parse_timeout)]
    timeout: f64,
}

/// Reads `--timeout`: a non-negative number of seconds, fractions allowed.
fn parse_timeout(text: &str) -> Result<f64, String> {
    let seconds: f64 = text.parse().map_err(|e| format!("`{text}` is not a number of seconds: {e}"))?;
    Duration::try_from_secs_f64(seconds)
        .map(|_| seconds)
        .map_err(|e| format!("`{text}` is not a usable number of seconds: {e}"))
}

/// Runs the command line with `args`, writing results to `out` and errors to `err`.
///
/// Returns success when the command ran, even if it found nothing; failure for any error.
#[must_use]
pub fn run<I, T>(args: I, out: &mut impl Write, err: &mut impl Write) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            // clap renders help and version on stdout and usage errors on stderr, with its own exit codes.
            let code = e.exit_code();
            let target: &mut dyn Write = if e.use_stderr() { err } else { out };
            let _written = write!(target, "{e}");
            return ExitCode::from(u8::try_from(code).unwrap_or(2));
        }
    };
    match cli.command {
        Command::Search(args) => match run_search(&args) {
            Ok(text) => exit_code(writeln!(out, "{text}"), ExitCode::SUCCESS),
            Err(text) => exit_code(writeln!(err, "{text}"), ExitCode::FAILURE),
        },
        Command::LanguageRules => exit_code(writeln!(out, "{}", language_rules_json()), ExitCode::SUCCESS),
    }
}

/// `["a","b"]`. Every string in the rules is plain ASCII with nothing to escape.
fn json_strings(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("\"{item}\"")).collect();
    format!("[{}]", quoted.join(","))
}

/// The rules in [`crate::analysis::lang`] as one line of JSON, so a script reads them instead of copying them.
fn language_rules_json() -> String {
    let languages: Vec<String> = Language::ALL
        .into_iter()
        .filter_map(|language| {
            let name = language.to_possible_value()?.get_name().to_owned();
            let rules = language.rules();
            Some(format!(
                "{{\"name\":\"{name}\",\"extensions\":{},\"excluded_suffixes\":{},\"test_stem_suffixes\":{},\"test_stems\":{}}}",
                json_strings(rules.extensions),
                json_strings(rules.excluded_suffixes),
                json_strings(rules.test_stem_suffixes),
                json_strings(rules.test_stems),
            ))
        })
        .collect();
    format!("{{\"test_dirs\":{},\"languages\":[{}]}}", json_strings(TEST_DIRS), languages.join(","))
}

/// `ok` unless the write failed. A closed reader (`| head`) is not a failure.
fn exit_code(written: std::io::Result<()>, ok: ExitCode) -> ExitCode {
    match written {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => ExitCode::FAILURE,
        Ok(()) | Err(_) => ok,
    }
}

fn registry() -> AnalyzerRegistry {
    let analyzers: Vec<Arc<dyn crate::analysis::LanguageAnalyzer>> = vec![
        #[cfg(feature = "lang-ts")]
        Arc::new(crate::analysis::ts::TsAnalyzer::new()),
        #[cfg(feature = "lang-rust")]
        Arc::new(crate::analysis::rust::RustAnalyzer::new()),
    ];
    AnalyzerRegistry::new(analyzers)
}

fn run_search(args: &SearchArgs) -> Result<String, String> {
    let request = SearchRequest {
        root: &args.path,
        query: &args.query,
        exact: args.exact,
        allow_tests: args.tests,
        respect_ignore_files: !args.no_ignore,
        language: args.language,
    };
    let result = search_with_timeout(&registry(), &request, timeout(args)).map_err(|e| error_text(&e))?;

    let mut ranked = result.ranked;
    let mut diagnostics = result.diagnostics;
    // Paths print relative to the working directory when they are under it, so every row spends fewer tokens.
    if let Some(cwd) = std::env::current_dir().ok().and_then(|dir| Utf8PathBuf::from_path_buf(dir).ok()) {
        for block in &mut ranked {
            relativize(&mut block.block.file, &cwd);
        }
        for diagnostic in &mut diagnostics {
            if let Some(path) = &mut diagnostic.path {
                relativize(path, &cwd);
            }
        }
    }
    if result.ceiling_hit {
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::Warning,
            path: None,
            message: format!(
                "{} files matched but hits were kept for only the first {DEFAULT_MAX_FILES}; narrow the query or path",
                result.files_total
            ),
        });
    }
    let budget = Budget { max_results: args.max_results, max_tokens: args.max_tokens };
    let total_before = ranked.len();
    let row_texts = search_row_texts(&ranked);
    let response = apply_budget(ranked, total_before, &budget, &row_texts, diagnostics);
    Ok(render_search(&response))
}

/// The deadline `--timeout` asks for; `0` asks for none.
fn timeout(args: &SearchArgs) -> Option<Duration> {
    (args.timeout > 0.0).then(|| Duration::from_secs_f64(args.timeout))
}

fn relativize(path: &mut Utf8PathBuf, base: &Utf8PathBuf) {
    if let Ok(relative) = path.strip_prefix(base) {
        *path = relative.to_owned();
    }
}

fn error_text(error: &MonoklError) -> String {
    error.to_domain_error().render()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> (ExitCode, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(std::iter::once("monokl").chain(args.iter().copied()), &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn help_goes_to_stdout_and_succeeds() {
        let (code, out, err) = run_args(&["--help"]);
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(out.contains("Searches a directory"), "got: {out}");
        assert_eq!(err, "");
    }

    #[test]
    fn language_rules_are_one_line_of_json_for_every_language() {
        let (code, out, err) = run_args(&["language-rules"]);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(err, "");
        assert_eq!(
            out,
            concat!(
                r#"{"test_dirs":["__tests__","test","tests","spec"],"languages":["#,
                r#"{"name":"typescript","extensions":["ts","tsx","mts","cts"],"#,
                r#""excluded_suffixes":[".d.ts",".d.mts",".d.cts"],"test_stem_suffixes":[".test",".spec"],"test_stems":[]},"#,
                r#"{"name":"javascript","extensions":["js","jsx","mjs","cjs"],"#,
                r#""excluded_suffixes":[],"test_stem_suffixes":[".test",".spec"],"test_stems":[]},"#,
                r#"{"name":"rust","extensions":["rs"],"#,
                r#""excluded_suffixes":[],"test_stem_suffixes":["_test","_tests"],"test_stems":["test","tests"]}"#,
                "]}\n"
            )
        );
    }

    #[test]
    fn language_rules_are_not_listed_in_help() {
        let (_, out, _) = run_args(&["--help"]);
        assert!(!out.contains("language-rules"), "got: {out}");
    }

    struct FailingWriter(std::io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn a_failed_stdout_write_fails_the_command_but_a_closed_pipe_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let args = ["monokl", "search", "zzzqqqxx", dir.path().to_str().unwrap()];
        let failed = run(args, &mut FailingWriter(std::io::ErrorKind::StorageFull), &mut Vec::new());
        assert_eq!(failed, ExitCode::FAILURE);
        let closed = run(args, &mut FailingWriter(std::io::ErrorKind::BrokenPipe), &mut Vec::new());
        assert_eq!(closed, ExitCode::SUCCESS);
    }

    #[test]
    fn a_missing_query_is_a_usage_error_on_stderr() {
        let (code, out, err) = run_args(&["search"]);
        assert_eq!(code, ExitCode::from(2));
        assert_eq!(out, "");
        assert!(err.contains("<QUERY>"), "got: {err}");
    }

    #[test]
    fn a_missing_directory_is_a_structured_error_on_stderr() {
        let (code, out, err) = run_args(&["search", "alpha", "/definitely/not/a/directory"]);
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(out, "");
        assert!(err.starts_with("error: not_found"), "got: {err}");
    }
}
