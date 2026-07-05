mod cli;
mod command;
mod config;
mod credential;
mod error;
mod http;
mod output;
pub mod redaction;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, OutputFormat};
use crate::command::execute;
use crate::error::CliError;
use crate::output::OutputStream;

pub fn main_entry<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run(args) {
        Ok(rendered) => {
            match rendered.stream {
                OutputStream::Stdout => print!("{}", rendered.body),
                OutputStream::Stderr => eprint!("{}", rendered.body),
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            let rendered = error.render();
            match rendered.stream {
                OutputStream::Stdout => print!("{}", rendered.body),
                OutputStream::Stderr => eprint!("{}", rendered.body),
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn run<I, T>(args: I) -> Result<output::RenderedOutput, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let parse_error_output = output_format_from_raw_args(&args)
        .or_else(output_format_from_env)
        .unwrap_or(OutputFormat::Table);
    match Cli::try_parse_from(args) {
        Ok(cli) => execute(cli),
        Err(error) => {
            if error.use_stderr() {
                Err(CliError::parse(error.to_string(), parse_error_output))
            } else {
                Ok(output::RenderedOutput::stdout(error.to_string()))
            }
        }
    }
}

fn output_format_from_raw_args(args: &[OsString]) -> Option<OutputFormat> {
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else {
            continue;
        };
        if arg == "--output" {
            return args
                .next()
                .and_then(|value| value.to_str())
                .and_then(OutputFormat::parse);
        }
        if let Some(value) = arg.strip_prefix("--output=") {
            return OutputFormat::parse(value);
        }
    }
    None
}

fn output_format_from_env() -> Option<OutputFormat> {
    std::env::var("APOLLO_OUTPUT")
        .ok()
        .and_then(|value| OutputFormat::parse(&value))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::{Cli, Commands, OutputFormat, ProfileCommand};

    #[test]
    fn parse_global_flags_before_subcommand() {
        let cli = Cli::parse_from([
            "apollo",
            "--profile",
            "dev",
            "--server",
            "https://apollo.example.com",
            "--output",
            "json",
            "--yes",
            "profile",
            "show",
        ]);

        assert_eq!(cli.global.profile.as_deref(), Some("dev"));
        assert_eq!(
            cli.global.server.as_deref(),
            Some("https://apollo.example.com")
        );
        assert_eq!(cli.global.output, Some(OutputFormat::Json));
        assert!(cli.global.yes);
        assert_eq!(
            cli.command,
            Commands::Profile {
                command: ProfileCommand::Show,
            }
        );
    }
}
