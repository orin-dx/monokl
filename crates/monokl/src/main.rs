use std::process::ExitCode;

fn main() -> ExitCode {
    monokl::cli::run(std::env::args_os(), &mut std::io::stdout(), &mut std::io::stderr())
}
