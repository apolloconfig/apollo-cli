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
    #[arg(long, global = true, help = "Use a named Apollo CLI profile")]
    pub profile: Option<String>,
    #[arg(long, global = true, help = "Override the Apollo Portal base URL")]
    pub server: Option<String>,
    #[arg(
        long,
        value_enum,
        global = true,
        help = "Render output as json or table"
    )]
    pub output: Option<OutputFormat>,
    #[arg(
        long,
        global = true,
        help = "Skip confirmation prompts for mutating OpenAPI requests"
    )]
    pub yes: bool,
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    UserToken,
    #[default]
    ConsumerToken,
}

impl AuthMode {
    pub fn from_token_value(token: &str) -> Self {
        if token.trim().starts_with(USER_TOKEN_PREFIX) {
            Self::UserToken
        } else {
            Self::ConsumerToken
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserToken => "user-token",
            Self::ConsumerToken => "consumer-token",
        }
    }

    pub fn is_user_token(self) -> bool {
        matches!(self, Self::UserToken)
    }
}

impl std::fmt::Display for AuthMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub const USER_TOKEN_PREFIX: &str = "apollo_pat_";

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum Commands {
    /// Configure the first local profile and token.
    Init(InitArgs),
    /// Log in, log out, inspect local auth, and check user tokens.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Manage named server/token/operator profiles.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Read Apollo application metadata.
    App {
        #[command(subcommand)]
        command: AppCommand,
    },
    /// List environments and clusters for an app.
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// List, inspect, and create namespaces.
    Namespace {
        #[command(subcommand)]
        command: NamespaceCommand,
    },
    /// Read, change, diff, and sync namespace items.
    #[command(
        after_help = "Scope options on config subcommands: --env <ENV>, --app <APP>, --cluster <CLUSTER> (default: default), --namespace <NAMESPACE> (default: application)."
    )]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Create, list, and roll back releases.
    #[command(
        after_help = "Scope options on release list/create: --env <ENV>, --app <APP>, --cluster <CLUSTER> (default: default), --namespace <NAMESPACE> (default: application)."
    )]
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    /// Send a raw Apollo Portal OpenAPI request.
    Api(ApiArgs),
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ProfileCommand {
    /// Add or update a named profile.
    Add(ProfileAddArgs),
    /// List configured profiles.
    List,
    /// Show the active profile.
    Show,
    /// Set the active profile.
    Use {
        #[arg(help = "Profile name to activate")]
        name: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct InitArgs {
    #[arg(long, help = "Profile name to create")]
    pub name: Option<String>,
    #[arg(long, value_enum, help = "Authentication mode for the profile token")]
    pub auth_mode: Option<AuthMode>,
    #[arg(long, help = "Default operator for consumer-token write commands")]
    pub operator: Option<String>,
    #[arg(long, help = "Read the token from standard input")]
    pub token_stdin: bool,
    #[arg(long, help = "Store the token in the config file instead of keychain")]
    pub store_token_in_file: bool,
    #[arg(long, help = "Overwrite an existing profile with the same name")]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct ProfileAddArgs {
    #[arg(help = "Profile name to create or update")]
    pub name: Option<String>,
    #[arg(long, value_enum, help = "Authentication mode for the profile token")]
    pub auth_mode: Option<AuthMode>,
    #[arg(long, help = "Default operator for consumer-token write commands")]
    pub operator: Option<String>,
    #[arg(long, help = "Read the token from standard input")]
    pub token_stdin: bool,
    #[arg(long, help = "Store the token in the config file instead of keychain")]
    pub store_token_in_file: bool,
    #[arg(long, help = "Overwrite an existing profile with the same name")]
    pub overwrite: bool,
    #[arg(long = "use", help = "Make this profile active after saving")]
    pub use_profile: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum AuthCommand {
    /// Store a token for the active profile.
    Login {
        #[arg(long, value_enum, help = "Authentication mode for the profile token")]
        auth_mode: Option<AuthMode>,
        #[arg(long, help = "Read the token from standard input")]
        token_stdin: bool,
        #[arg(long, help = "Store the token in the config file instead of keychain")]
        store_token_in_file: bool,
    },
    /// Show local auth state without contacting the server.
    Status,
    /// Verify the current user token and show its owner.
    Whoami,
    /// List server capabilities for the current user token.
    Capabilities,
    /// Remove the stored token for the active profile.
    Logout,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum AppCommand {
    /// List applications visible to the token.
    List {
        #[arg(long, help = "Comma-separated app IDs to filter")]
        app_ids: Option<String>,
    },
    /// Get one application's metadata.
    Get {
        #[arg(help = "Apollo application ID")]
        app_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum EnvCommand {
    /// List environments and clusters for an app.
    List {
        #[arg(long, help = "Apollo application ID")]
        app: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum NamespaceCommand {
    /// List namespaces in an app environment and cluster.
    #[command(
        override_usage = "apollo namespace list [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>]"
    )]
    List {
        #[command(flatten)]
        scope: ClusterScopeArgs,
    },
    /// Get one namespace and its metadata.
    #[command(
        override_usage = "apollo namespace get [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] <NAMESPACE>"
    )]
    Get {
        #[command(flatten)]
        scope: ClusterScopeArgs,
        #[arg(value_name = "NAMESPACE", help = "Namespace name to inspect")]
        namespace: String,
    },
    /// Create a namespace in an app environment and cluster.
    #[command(
        override_usage = "apollo namespace create [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] <NAME>"
    )]
    Create {
        #[command(flatten)]
        scope: ClusterScopeArgs,
        #[arg(help = "Namespace name to create")]
        name: String,
        #[arg(long, help = "Operator for consumer-token mode")]
        operator: Option<String>,
        #[arg(long = "public", help = "Create a public namespace")]
        public_namespace: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ConfigCommand {
    /// List namespace configuration items.
    #[command(
        override_usage = "apollo config list [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>]"
    )]
    List {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long, help = "Page number for paginated output")]
        page: Option<u32>,
        #[arg(long, help = "Page size for paginated output")]
        size: Option<u32>,
    },
    /// Get one namespace configuration item.
    #[command(
        override_usage = "apollo config get [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>] <KEY>"
    )]
    Get {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(help = "Configuration item key")]
        key: String,
    },
    /// Create or update a namespace configuration item.
    #[command(
        override_usage = "apollo config set [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>] <KEY> <VALUE>"
    )]
    Set {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(help = "Configuration item key")]
        key: String,
        #[arg(help = "Configuration item value")]
        value: String,
        #[arg(long, help = "Change comment")]
        comment: Option<String>,
        #[arg(long, help = "Operator for consumer-token mode")]
        operator: Option<String>,
    },
    /// Delete one namespace configuration item.
    #[command(
        override_usage = "apollo config delete [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>] <KEY>"
    )]
    Delete {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(help = "Configuration item key")]
        key: String,
        #[arg(long, help = "Operator for consumer-token mode")]
        operator: Option<String>,
    },
    /// Compare a namespace against another target.
    #[command(
        override_usage = "apollo config diff [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>] --target-env <TARGET_ENV> [--target-cluster <TARGET_CLUSTER>] [--target-namespace <TARGET_NAMESPACE>]"
    )]
    Diff {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long, help = "Target environment")]
        target_env: String,
        #[arg(long, default_value = "default", help = "Target cluster")]
        target_cluster: String,
        #[arg(long, help = "Target namespace, defaults to source namespace")]
        target_namespace: Option<String>,
    },
    /// Sync namespace configuration items to another target.
    #[command(
        override_usage = "apollo config apply [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>] --target-env <TARGET_ENV> [--target-cluster <TARGET_CLUSTER>] [--target-namespace <TARGET_NAMESPACE>]"
    )]
    Apply {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long, help = "Target environment")]
        target_env: String,
        #[arg(long, default_value = "default", help = "Target cluster")]
        target_cluster: String,
        #[arg(long, help = "Target namespace, defaults to source namespace")]
        target_namespace: Option<String>,
        #[arg(long, help = "Operator for consumer-token mode")]
        operator: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ReleaseCommand {
    /// List releases for a namespace.
    #[command(
        override_usage = "apollo release list [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>]"
    )]
    List {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long, help = "Page number for paginated output")]
        page: Option<u32>,
        #[arg(long, help = "Page size for paginated output")]
        size: Option<u32>,
    },
    /// Create a release for a namespace.
    #[command(
        override_usage = "apollo release create [OPTIONS] --env <ENV> --app <APP> [--cluster <CLUSTER>] [--namespace <NAMESPACE>] --title <TITLE>"
    )]
    Create {
        #[command(flatten)]
        scope: NamespaceScopeArgs,
        #[arg(long, help = "Release title")]
        title: String,
        #[arg(long, help = "Release comment")]
        comment: Option<String>,
        #[arg(long, help = "Mark the release as emergency publish")]
        emergency: bool,
        #[arg(long, help = "Operator for consumer-token mode")]
        operator: Option<String>,
    },
    /// Roll back a release.
    Rollback {
        #[arg(long, help = "Apollo environment")]
        env: String,
        #[arg(help = "Release ID to roll back")]
        release_id: i64,
        #[arg(long, help = "Target release ID to roll back to")]
        to_release_id: Option<i64>,
        #[arg(long, help = "Operator for consumer-token mode")]
        operator: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct ApiArgs {
    #[arg(help = "HTTP method")]
    pub method: HttpMethod,
    #[arg(help = "OpenAPI path, for example /openapi/v1/apps")]
    pub path: String,
    #[arg(long, help = "Raw JSON request body")]
    pub body: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct ClusterScopeArgs {
    #[arg(long, help = "Apollo environment")]
    pub env: String,
    #[arg(long, help = "Apollo application ID")]
    pub app: String,
    #[arg(long, default_value = "default", help = "Apollo cluster name")]
    pub cluster: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Args)]
pub struct NamespaceScopeArgs {
    #[command(flatten)]
    pub cluster_scope: ClusterScopeArgs,
    #[arg(long, default_value = "application", help = "Apollo namespace name")]
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
