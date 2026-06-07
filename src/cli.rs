use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(
    name = "apollo",
    bin_name = "apollo",
    version,
    about = "Apollo CLI v0 scaffold",
    long_about = "Standalone Apollo CLI scaffold for v0 command routing, global flags, and structured output/error handling."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOptions,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args, Debug)]
pub struct GlobalOptions {
    #[arg(long, global = true)]
    pub profile: Option<String>,
    #[arg(long, global = true)]
    pub server: Option<String>,
    #[arg(long, value_enum, global = true)]
    pub output: Option<OutputFormat>,
    #[arg(long, global = true)]
    pub yes: bool,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum Commands {
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    App,
    Env,
    Namespace,
    Config,
    Release,
    Api,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ProfileCommand {
    List,
    Show,
    Use { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum AuthCommand {
    Login {
        #[arg(long)]
        token_stdin: bool,
        #[arg(long)]
        store_token_in_file: bool,
    },
    Status,
    Logout,
}
