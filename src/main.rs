use std::process::ExitCode;

fn main() -> ExitCode {
    apollo_cli::main_entry(std::env::args_os())
}
