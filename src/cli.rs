use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(
    name = "apollo",
    bin_name = "apollo",
    version,
    about = "Standalone Apollo OpenAPI CLI",
    long_about = "Standalone Apollo CLI for profile/auth management and Apollo Portal OpenAPI v0 workflows."
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
    Init(InitArgs),
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    App {
        #[command(subcommand)]
        command: AppCommand,
    },
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    Namespace {
        #[command(subcommand)]
        command: NamespaceCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    Api(ApiArgs),
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ProfileCommand {
    Add(ProfileAddArgs),
    List,
    Show,
    Use { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct InitArgs {
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub operator: Option<String>,
    #[arg(long)]
    pub token_stdin: bool,
    #[arg(long)]
    pub store_token_in_file: bool,
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct ProfileAddArgs {
    pub name: Option<String>,
    #[arg(long)]
    pub operator: Option<String>,
    #[arg(long)]
    pub token_stdin: bool,
    #[arg(long)]
    pub store_token_in_file: bool,
    #[arg(long)]
    pub overwrite: bool,
    #[arg(long = "use")]
    pub use_profile: bool,
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

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum AppCommand {
    List {
        #[arg(long)]
        app_ids: Option<String>,
    },
    Get {
        app_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum EnvCommand {
    List {
        #[arg(long)]
        app: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum NamespaceCommand {
    List {
        #[command(flatten)]
        scope: ClusterScopeArgs,
    },
    Get {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
    },
    Create {
        #[command(flatten)]
        scope: ClusterScopeArgs,
        name: String,
        #[arg(long)]
        operator: Option<String>,
        #[arg(long = "public")]
        public_namespace: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ConfigCommand {
    List {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long)]
        page: Option<u32>,
        #[arg(long)]
        size: Option<u32>,
    },
    Get {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        key: String,
    },
    Set {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        key: String,
        value: String,
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        operator: Option<String>,
    },
    Delete {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        key: String,
        #[arg(long)]
        operator: Option<String>,
    },
    Diff {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long)]
        target_env: String,
        #[arg(long, default_value = "default")]
        target_cluster: String,
        #[arg(long)]
        target_namespace: Option<String>,
    },
    Apply {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long)]
        target_env: String,
        #[arg(long, default_value = "default")]
        target_cluster: String,
        #[arg(long)]
        target_namespace: Option<String>,
        #[arg(long)]
        operator: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ReleaseCommand {
    List {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long)]
        page: Option<u32>,
        #[arg(long)]
        size: Option<u32>,
    },
    Create {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long)]
        title: String,
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        emergency: bool,
        #[arg(long)]
        operator: Option<String>,
    },
    Rollback {
        #[arg(long)]
        env: String,
        release_id: i64,
        #[arg(long)]
        to_release_id: Option<i64>,
        #[arg(long)]
        operator: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct ApiArgs {
    pub method: HttpMethod,
    pub path: String,
    #[arg(long)]
    pub body: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct ClusterScopeArgs {
    #[arg(long)]
    pub env: String,
    #[arg(long)]
    pub app: String,
    #[arg(long, default_value = "default")]
    pub cluster: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct NamespaceScopeArgs {
    #[command(flatten)]
    pub cluster_scope: ClusterScopeArgs,
    #[arg(long, default_value = "application")]
    pub namespace: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}
