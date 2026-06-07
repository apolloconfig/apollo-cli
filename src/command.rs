use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::{
    ApiArgs, AppCommand, AuthCommand, Cli, Commands, ConfigCommand, EnvCommand, NamespaceCommand,
    NamespaceScopeArgs, OutputFormat, ProfileCommand, ReleaseCommand,
};
use crate::config::{
    CredentialRef, LoadedConfig, ProfileConfig, RuntimeContext, load_config, resolve_context,
    resolve_output, save_config,
};
use crate::credential;
use crate::error::CliError;
use crate::http::{OpenApiClient, OpenApiResponse, append_query, encode_path_segment};
use crate::output::{OutputWriter, RenderedOutput};

pub fn execute(cli: Cli) -> Result<RenderedOutput, CliError> {
    let output = cli.global.output.unwrap_or(OutputFormat::Table);

    match &cli.command {
        Commands::Auth { command } => execute_auth(command.clone(), &cli, output),
        Commands::Profile { command } => execute_profile(command.clone(), &cli, output),
        Commands::App { command } => execute_app(command.clone(), &cli, output),
        Commands::Env { command } => execute_env(command.clone(), &cli, output),
        Commands::Namespace { command } => execute_namespace(command.clone(), &cli, output),
        Commands::Config { command } => execute_config(command.clone(), &cli, output),
        Commands::Release { command } => execute_release(command.clone(), &cli, output),
        Commands::Api(args) => execute_api(args.clone(), &cli, output),
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

fn execute_app(
    command: AppCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    let path = match command {
        AppCommand::List { app_ids } => {
            let path = "/openapi/v1/apps".to_owned();
            if let Some(app_ids) = app_ids {
                append_query(path, "appIds", &app_ids)
            } else {
                path
            }
        }
        AppCommand::Get { app_id } => format!("/openapi/v1/apps/{}", encode_path_segment(&app_id)),
    };
    openapi.request("GET", &path, None)
}

fn execute_env(
    command: EnvCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    match command {
        EnvCommand::List => openapi.request("GET", "/openapi/v1/envs", None),
    }
}

fn execute_namespace(
    command: NamespaceCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    match command {
        NamespaceCommand::List { scope } => {
            let path = cluster_namespaces_path(&scope.env, &scope.app, &scope.cluster);
            openapi.request("GET", &path, None)
        }
        NamespaceCommand::Get { scope } => {
            let path = namespace_path(&scope);
            openapi.request("GET", &path, None)
        }
        NamespaceCommand::Create {
            scope,
            name,
            operator,
        } => {
            require_yes(cli, output)?;
            let operator = required_operator(operator.as_deref(), &openapi.context, output)?;
            let path = append_query("/openapi/v1/namespaces".to_owned(), "operator", &operator);
            let body = json!([{
                "appId": scope.app,
                "env": scope.env,
                "clusterName": scope.cluster,
                "appNamespaceName": name,
            }]);
            openapi.request("POST", &path, Some(body))
        }
    }
}

fn execute_config(
    command: ConfigCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    match command {
        ConfigCommand::List { scope, page, size } => {
            let mut path = format!("{}/items", namespace_path(&scope));
            if let Some(page) = page {
                path = append_query(path, "page", &page.to_string());
            }
            if let Some(size) = size {
                path = append_query(path, "size", &size.to_string());
            }
            openapi.request("GET", &path, None)
        }
        ConfigCommand::Get { scope, key } => {
            let path = format!(
                "{}/items/{}",
                namespace_path(&scope),
                encode_path_segment(&key)
            );
            openapi.request("GET", &path, None)
        }
        ConfigCommand::Set {
            scope,
            key,
            value,
            comment,
            operator,
        } => {
            require_yes(cli, output)?;
            let operator = required_operator(operator.as_deref(), &openapi.context, output)?;
            let path = append_query(
                format!(
                    "{}/items/{}",
                    namespace_path(&scope),
                    encode_path_segment(&key)
                ),
                "createIfNotExists",
                "true",
            );
            let body = json!({
                "key": key,
                "value": value,
                "comment": comment.unwrap_or_default(),
                "dataChangeCreatedBy": operator,
                "dataChangeLastModifiedBy": operator,
            });
            openapi.request("PUT", &path, Some(body))
        }
        ConfigCommand::Delete {
            scope,
            key,
            operator,
        } => {
            require_yes(cli, output)?;
            let operator = required_operator(operator.as_deref(), &openapi.context, output)?;
            let path = append_query(
                format!(
                    "{}/items/{}",
                    namespace_path(&scope),
                    encode_path_segment(&key)
                ),
                "operator",
                &operator,
            );
            openapi.request("DELETE", &path, None)
        }
        ConfigCommand::Diff {
            scope,
            target_env,
            target_cluster,
            target_namespace,
        } => {
            let body = sync_body(&scope, target_env, target_cluster, target_namespace);
            let path = format!("{}/items/diff", namespace_path(&scope));
            openapi.request("POST", &path, Some(body))
        }
        ConfigCommand::Apply {
            scope,
            target_env,
            target_cluster,
            target_namespace,
            operator,
        } => {
            require_yes(cli, output)?;
            let operator = required_operator(operator.as_deref(), &openapi.context, output)?;
            let body = sync_body(&scope, target_env, target_cluster, target_namespace);
            let path = append_query(
                format!("{}/items/synchronize", namespace_path(&scope)),
                "operator",
                &operator,
            );
            openapi.request("POST", &path, Some(body))
        }
    }
}

fn execute_release(
    command: ReleaseCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    match command {
        ReleaseCommand::List { scope, page, size } => {
            let mut path = format!("{}/releases/active", namespace_path(&scope));
            if let Some(page) = page {
                path = append_query(path, "page", &page.to_string());
            }
            if let Some(size) = size {
                path = append_query(path, "size", &size.to_string());
            }
            openapi.request("GET", &path, None)
        }
        ReleaseCommand::Create {
            scope,
            title,
            comment,
            emergency,
            operator,
        } => {
            require_yes(cli, output)?;
            let operator = required_operator(operator.as_deref(), &openapi.context, output)?;
            let path = format!("{}/releases", namespace_path(&scope));
            let body = json!({
                "releaseTitle": title,
                "releaseComment": comment.unwrap_or_default(),
                "releasedBy": operator,
                "isEmergencyPublish": emergency,
            });
            openapi.request("POST", &path, Some(body))
        }
        ReleaseCommand::Rollback {
            env,
            release_id,
            to_release_id,
            operator,
        } => {
            require_yes(cli, output)?;
            let operator = required_operator(operator.as_deref(), &openapi.context, output)?;
            let mut path = append_query(
                format!(
                    "/openapi/v1/envs/{}/releases/{}/rollback",
                    encode_path_segment(&env),
                    release_id
                ),
                "operator",
                &operator,
            );
            if let Some(to_release_id) = to_release_id {
                path = append_query(path, "toReleaseId", &to_release_id.to_string());
            }
            openapi.request("PUT", &path, None)
        }
    }
}

fn execute_api(args: ApiArgs, cli: &Cli, output: OutputFormat) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    let body = match args.body {
        Some(body) => Some(
            serde_json::from_str::<Value>(&body)
                .map_err(|error| CliError::invalid_input(&error.to_string(), output))?,
        ),
        None => None,
    };
    if matches!(
        args.method,
        crate::cli::HttpMethod::Post
            | crate::cli::HttpMethod::Put
            | crate::cli::HttpMethod::Patch
            | crate::cli::HttpMethod::Delete
    ) {
        require_yes(cli, output)?;
    }
    openapi.request(args.method.as_str(), &args.path, body)
}

struct OpenApiCommandContext {
    context: RuntimeContext,
    writer: OutputWriter,
    client: OpenApiClient,
}

impl OpenApiCommandContext {
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<RenderedOutput, CliError> {
        let response = self.client.request(method, path, body)?;
        Ok(render_openapi_response(&self.writer, &response))
    }
}

fn openapi_context(cli: &Cli, output: OutputFormat) -> Result<OpenApiCommandContext, CliError> {
    let loaded = load_config(output)?;
    let writer_output = resolve_output(cli, &loaded, output)?;
    let context = resolve_context(cli, &loaded, writer_output)?;
    let server = required_server(&context, writer_output)?;
    let token = credential::resolve_token(
        &loaded.path,
        context.profile.as_deref(),
        context.credential.as_ref(),
    )
    .map_err(|error| CliError::credential_store_unavailable(&error, writer_output))?
    .ok_or_else(|| {
        CliError::authentication_required(
            "Authenticate with APOLLO_TOKEN or `apollo auth login` before calling OpenAPI.",
            writer_output,
        )
    })?;
    Ok(OpenApiCommandContext {
        context,
        writer: OutputWriter::new(writer_output),
        client: OpenApiClient::new(server, token, writer_output),
    })
}

fn render_openapi_response(writer: &OutputWriter, response: &OpenApiResponse) -> RenderedOutput {
    writer.render_success(response, response.render_table())
}

fn required_server(context: &RuntimeContext, output: OutputFormat) -> Result<String, CliError> {
    context.server.clone().ok_or_else(|| {
        CliError::invalid_input(
            "provide a server with --server, APOLLO_SERVER, or profile config",
            output,
        )
    })
}

fn required_operator(
    command_operator: Option<&str>,
    context: &RuntimeContext,
    output: OutputFormat,
) -> Result<String, CliError> {
    command_operator
        .map(ToOwned::to_owned)
        .or_else(|| context.operator.clone())
        .ok_or_else(|| {
            CliError::invalid_input(
                "provide an operator with --operator or profile config",
                output,
            )
        })
}

fn require_yes(cli: &Cli, output: OutputFormat) -> Result<(), CliError> {
    if cli.global.yes {
        Ok(())
    } else {
        Err(CliError::confirmation_required(
            "This command mutates Apollo state. Re-run with --yes to confirm.",
            output,
        ))
    }
}

fn cluster_namespaces_path(env: &str, app: &str, cluster: &str) -> String {
    format!(
        "/openapi/v1/envs/{}/apps/{}/clusters/{}/namespaces",
        encode_path_segment(env),
        encode_path_segment(app),
        encode_path_segment(cluster),
    )
}

fn namespace_path(scope: &NamespaceScopeArgs) -> String {
    format!(
        "{}/{}",
        cluster_namespaces_path(
            &scope.cluster_scope.env,
            &scope.cluster_scope.app,
            &scope.cluster_scope.cluster
        ),
        encode_path_segment(&scope.namespace),
    )
}

fn sync_body(
    scope: &NamespaceScopeArgs,
    target_env: String,
    target_cluster: String,
    target_namespace: Option<String>,
) -> Value {
    json!({
        "syncToNamespaces": [{
            "appId": scope.cluster_scope.app.clone(),
            "env": target_env,
            "clusterName": target_cluster,
            "namespaceName": target_namespace.unwrap_or_else(|| scope.namespace.clone()),
        }],
    })
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
