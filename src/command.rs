use serde::Serialize;

use crate::cli::{AuthCommand, Cli, Commands, OutputFormat, ProfileCommand};
use crate::config::{
    CredentialRef, LoadedConfig, ProfileConfig, RuntimeContext, load_config, resolve_context,
    resolve_output, save_config,
};
use crate::credential;
use crate::error::CliError;
use crate::output::{OutputWriter, RenderedOutput};

pub fn execute(cli: Cli) -> Result<RenderedOutput, CliError> {
    let output = cli.global.output.unwrap_or(OutputFormat::Table);

    match &cli.command {
        Commands::Auth { command } => execute_auth(command.clone(), &cli, output),
        Commands::Profile { command } => execute_profile(command.clone(), &cli, output),
        Commands::App => Err(CliError::not_implemented("app", None, output)),
        Commands::Env => Err(CliError::not_implemented("env", None, output)),
        Commands::Namespace => Err(CliError::not_implemented("namespace", None, output)),
        Commands::Config => Err(CliError::not_implemented("config", Some(5631), output)),
        Commands::Release => Err(CliError::not_implemented("release", None, output)),
        Commands::Api => Err(CliError::not_implemented("api", None, output)),
    }
}

fn execute_auth(
    command: AuthCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let loaded = load_config(output)?;
    let writer_output = resolve_output(cli, &loaded, output)?;
    let writer = OutputWriter::new(writer_output);

    match command {
        AuthCommand::Status => {
            let context = resolve_context(cli, &loaded, output)?;
            let profile = required_profile(&context, output)?;
            let status = credential::status(&loaded.path, &profile, context.credential.as_ref());
            let response = AuthStatusResponse {
                authenticated: status.authenticated,
                source: status.source.as_str().to_owned(),
                profile,
                backend: status.backend,
                key: status.key,
            };
            Ok(writer.render_success(&response, response.render_table()))
        }
        AuthCommand::Login {
            token_stdin,
            store_token_in_file,
        } => {
            let context = resolve_context(cli, &loaded, output)?;
            let profile = required_profile(&context, output)?;
            let token = credential::token_from_env_or_stdin(token_stdin, writer_output)?;

            let credential_ref = if store_token_in_file {
                credential::store_file(&loaded.path, &profile, &token).map_err(|error| {
                    CliError::credential_store_unavailable(&error, writer_output)
                })?
            } else {
                credential::store_native(&profile, &token).map_err(|error| {
                    CliError::confirmation_required(
                        &format!(
                            "Native credential storage is unavailable: {}. Re-run with --store-token-in-file to use the explicit file fallback.",
                            error
                        ),
                        writer_output,
                    )
                })?
            };

            let mut config = loaded.config.clone();
            let profile_config = config
                .profiles
                .get_mut(&profile)
                .ok_or_else(|| CliError::profile_not_found(&profile, writer_output))?;
            profile_config.credential = Some(credential_ref.clone());
            save_config(&loaded.path, &config, writer_output)?;

            let response = AuthLoginResponse {
                stored: true,
                profile: profile.clone(),
                backend: credential_ref.backend,
                key: credential_ref.key,
            };
            Ok(writer.render_success(&response, response.render_table()))
        }
        AuthCommand::Logout => {
            let context = resolve_context(cli, &loaded, output)?;
            let profile = required_profile(&context, output)?;
            let credential_ref = context.credential.clone().unwrap_or_else(|| CredentialRef {
                backend: "native".to_owned(),
                key: profile.clone(),
            });
            credential::delete(&loaded.path, &credential_ref)
                .map_err(|error| CliError::credential_store_unavailable(&error, writer_output))?;
            let response = AuthLogoutResponse {
                logged_out: true,
                profile,
                backend: credential_ref.backend,
                key: credential_ref.key,
            };
            Ok(writer.render_success(&response, response.render_table()))
        }
    }
}

fn execute_profile(
    command: ProfileCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let loaded = load_config(output)?;
    let writer = OutputWriter::new(resolve_output(cli, &loaded, output)?);

    match command {
        ProfileCommand::List => {
            let response = ProfileListResponse::from_loaded_config(&loaded);
            Ok(writer.render_success(&response, response.render_table()))
        }
        ProfileCommand::Show => {
            let context = resolve_context(cli, &loaded, output)?;
            let response = ProfileShowResponse::from_context(&loaded, context);
            Ok(writer.render_success(&response, response.render_table()))
        }
        ProfileCommand::Use { name } => {
            if !loaded.config.profiles.contains_key(&name) {
                return Err(CliError::profile_not_found(&name, output));
            }

            let mut config = loaded.config.clone();
            config.active_profile = Some(name.clone());
            save_config(&loaded.path, &config, output)?;

            let response = ProfileUseResponse {
                active_profile: name.clone(),
                config_path: loaded.path.display().to_string(),
            };
            Ok(writer.render_success(
                &response,
                format!(
                    "Active profile set to '{}'.\nConfig path: {}",
                    name,
                    loaded.path.display()
                ),
            ))
        }
    }
}

#[derive(Serialize)]
struct ProfileListResponse {
    #[serde(rename = "activeProfile")]
    active_profile: Option<String>,
    #[serde(rename = "configPath")]
    config_path: String,
    profiles: Vec<ProfileSummaryRow>,
}

impl ProfileListResponse {
    fn from_loaded_config(loaded: &LoadedConfig) -> Self {
        let active_profile = loaded.config.active_profile.clone();
        let profiles = loaded
            .config
            .profiles
            .iter()
            .map(|(name, profile)| ProfileSummaryRow::from_profile(name, profile, &active_profile))
            .collect();

        Self {
            active_profile,
            config_path: loaded.path.display().to_string(),
            profiles,
        }
    }

    fn render_table(&self) -> String {
        if self.profiles.is_empty() {
            return format!("No profiles configured.\nConfig path: {}", self.config_path);
        }

        let mut lines = vec![format!("Config path: {}", self.config_path)];
        for profile in &self.profiles {
            let marker = if profile.active { "*" } else { "-" };
            let output = profile.output.as_deref().unwrap_or("table");
            lines.push(format!(
                "{} {}  {}  {}",
                marker, profile.name, profile.server, output
            ));
        }
        lines.join("\n")
    }
}

#[derive(Serialize)]
struct ProfileSummaryRow {
    active: bool,
    name: String,
    server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential: Option<CredentialRef>,
}

impl ProfileSummaryRow {
    fn from_profile(name: &str, profile: &ProfileConfig, active_profile: &Option<String>) -> Self {
        Self {
            active: active_profile.as_deref() == Some(name),
            name: name.to_owned(),
            server: profile.server.clone().unwrap_or_default(),
            output: profile.output.map(|output| output.to_string()),
            operator: profile.operator.clone(),
            credential: profile.credential.clone(),
        }
    }
}

#[derive(Serialize)]
struct ProfileShowResponse {
    #[serde(rename = "activeProfile")]
    active_profile: Option<String>,
    #[serde(rename = "configPath")]
    config_path: String,
    context: RuntimeContextRow,
}

impl ProfileShowResponse {
    fn from_context(loaded: &LoadedConfig, context: RuntimeContext) -> Self {
        Self {
            active_profile: loaded.config.active_profile.clone(),
            config_path: loaded.path.display().to_string(),
            context: RuntimeContextRow::from_runtime_context(context),
        }
    }

    fn render_table(&self) -> String {
        let mut lines = vec![format!("Config path: {}", self.config_path)];
        if let Some(active_profile) = &self.active_profile {
            lines.push(format!("Active profile: {}", active_profile));
        }
        lines.push(format!(
            "Resolved profile: {}",
            self.context.profile.as_deref().unwrap_or("<none>")
        ));
        lines.push(format!(
            "Server: {}",
            self.context.server.as_deref().unwrap_or("<none>")
        ));
        lines.push(format!("Output: {}", self.context.output));
        if let Some(operator) = &self.context.operator {
            lines.push(format!("Operator: {}", operator));
        }
        if let Some(credential) = &self.context.credential {
            lines.push(format!("Credential backend: {}", credential.backend));
            lines.push(format!("Credential key: {}", credential.key));
        }
        lines.join("\n")
    }
}

#[derive(Serialize)]
struct RuntimeContextRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential: Option<CredentialRef>,
}

impl RuntimeContextRow {
    fn from_runtime_context(context: RuntimeContext) -> Self {
        Self {
            profile: context.profile,
            server: context.server,
            output: context.output.to_string(),
            operator: context.operator,
            credential: context.credential,
        }
    }
}

#[derive(Serialize)]
struct ProfileUseResponse {
    #[serde(rename = "activeProfile")]
    active_profile: String,
    #[serde(rename = "configPath")]
    config_path: String,
}

#[derive(Serialize)]
struct AuthStatusResponse {
    authenticated: bool,
    source: String,
    profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

impl AuthStatusResponse {
    fn render_table(&self) -> String {
        let state = if self.authenticated {
            "authenticated"
        } else {
            "not authenticated"
        };
        format!(
            "Profile: {}\nStatus: {}\nSource: {}",
            self.profile, state, self.source
        )
    }
}

#[derive(Serialize)]
struct AuthLoginResponse {
    stored: bool,
    profile: String,
    backend: String,
    key: String,
}

impl AuthLoginResponse {
    fn render_table(&self) -> String {
        format!(
            "Credential stored for profile '{}'.\nBackend: {}\nKey: {}",
            self.profile, self.backend, self.key
        )
    }
}

#[derive(Serialize)]
struct AuthLogoutResponse {
    #[serde(rename = "loggedOut")]
    logged_out: bool,
    profile: String,
    backend: String,
    key: String,
}

impl AuthLogoutResponse {
    fn render_table(&self) -> String {
        format!(
            "Credential removed for profile '{}'.\nBackend: {}\nKey: {}",
            self.profile, self.backend, self.key
        )
    }
}

fn required_profile(context: &RuntimeContext, output: OutputFormat) -> Result<String, CliError> {
    context.profile.clone().ok_or_else(|| {
        CliError::invalid_input("select a profile with --profile or active_profile", output)
    })
}
