//! The `monokl` command line: parses arguments, runs a command and prints agent-facing output.
//!
//! This is the agent layer. It budgets a core result and renders it with michi; core never prints.

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

use crate::analysis::registry::AnalyzerRegistry;
use crate::budget::{Budget, apply_budget};
use crate::error::MonoklError;
use crate::output::agent::{ErrorContext, ToDomainError, render_projection};
use crate::pipeline::{SearchRequest, search};
use crate::text_search::DEFAULT_MAX_FILES;
use crate::types::{Diagnostic, DiagnosticKind, Language};

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
    /// Searches hidden files and files that .gitignore or .ignore would exclude.
    #[arg(long)]
    no_ignore: bool,
    /// Searches only this language.
    #[arg(long, value_enum)]
    language: Option<Language>,
    /// The most results to print.
    #[arg(long, default_value_t = Budget::default().max_results)]
    max_results: usize,
    /// The most tokens of code to print.
    #[arg(long, default_value_t = Budget::default().max_tokens)]
    max_tokens: usize,
    /// The most bytes of code to print.
    #[arg(long, default_value_t = Budget::default().max_bytes)]
    max_bytes: usize,
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
    let Command::Search(args) = cli.command;
    match run_search(&args) {
        Ok(text) => {
            let _written = writeln!(out, "{text}");
            ExitCode::SUCCESS
        }
        Err(text) => {
            let _written = writeln!(err, "{text}");
            ExitCode::FAILURE
        }
    }
}

fn registry() -> AnalyzerRegistry {
    #[cfg(feature = "lang-ts")]
    let analyzers: Vec<Arc<dyn crate::analysis::LanguageAnalyzer>> =
        vec![Arc::new(crate::analysis::ts::TsAnalyzer::new())];
    #[cfg(not(feature = "lang-ts"))]
    let analyzers: Vec<Arc<dyn crate::analysis::LanguageAnalyzer>> = Vec::new();
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
    let result = search(&registry(), &request).map_err(|e| error_text(&e))?;

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
    let budget = Budget { max_results: args.max_results, max_tokens: args.max_tokens, max_bytes: args.max_bytes };
    let total_before = ranked.len();
    let response = apply_budget(ranked, total_before, &budget, diagnostics);
    render_projection(&response).map_err(|e| e.render())
}

fn relativize(path: &mut Utf8PathBuf, base: &Utf8PathBuf) {
    if let Ok(relative) = path.strip_prefix(base) {
        *path = relative.to_owned();
    }
}

fn error_text(error: &MonoklError) -> String {
    error.to_domain_error(ErrorContext { command: "search" }).render()
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
