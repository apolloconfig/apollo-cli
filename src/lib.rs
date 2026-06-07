mod cli;
mod command;
mod config;
mod credential;
mod error;
mod output;
pub mod redaction;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;
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
    match Cli::try_parse_from(args) {
        Ok(cli) => execute(cli),
        Err(error) => {
            if error.use_stderr() {
                Err(CliError::parse(error.to_string()))
            } else {
                Ok(output::RenderedOutput::stdout(error.to_string()))
            }
        }
    }
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
