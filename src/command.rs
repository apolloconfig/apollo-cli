use std::io::{self, BufRead, IsTerminal, Write};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::{
    ApiArgs, AppCommand, AuthCommand, Cli, Commands, ConfigCommand, EnvCommand, InitArgs,
    NamespaceCommand, NamespaceScopeArgs, OutputFormat, ProfileCommand, ReleaseCommand,
};
use crate::config::{
    CredentialRef, LoadedConfig, ProfileConfig, RuntimeContext, load_config, read_env_output,
    resolve_context, resolve_output, save_config,
};
use crate::credential;
use crate::error::CliError;
use crate::http::{OpenApiClient, OpenApiResponse, append_query, encode_path_segment};
use crate::output::{OutputWriter, RenderedOutput};
use crate::redaction::Sensitive;

const DEFAULT_PAGE: u32 = 0;
const DEFAULT_PAGE_SIZE: u32 = 20;
const SYNC_ITEMS_PAGE_SIZE: u32 = 500;
const DEFAULT_INIT_PROFILE: &str = "local";
const DEFAULT_INIT_SERVER: &str = "http://127.0.0.1:8070";
const DEFAULT_INIT_OPERATOR: &str = "apollo";

pub fn execute(cli: Cli) -> Result<RenderedOutput, CliError> {
    let output = output_from_flags_or_env(&cli).unwrap_or(OutputFormat::Table);

    match &cli.command {
        Commands::Init(args) => execute_init(args.clone(), &cli, output),
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

fn output_from_flags_or_env(cli: &Cli) -> Option<OutputFormat> {
    cli.global.output.or_else(read_env_output)
}

fn non_blank(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn execute_init(
    args: InitArgs,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let options = ProfileSetupOptions {
        mode: ProfileSetupMode::Init,
        name: args.name,
        operator: args.operator,
        token_stdin: args.token_stdin,
        store_token_in_file: args.store_token_in_file,
        overwrite: args.overwrite,
        use_profile: true,
    };
    execute_profile_setup(options, cli, output)
}

fn execute_auth(
    command: AuthCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    match command {
        AuthCommand::Status => {
            if env_token_is_set() {
                let loaded = load_config(output).ok();
                let writer_output = loaded
                    .as_ref()
                    .and_then(|loaded| resolve_output(cli, loaded, output).ok())
                    .unwrap_or_else(|| output_from_flags_or_env(cli).unwrap_or(output));
                let writer = OutputWriter::new(writer_output);
                let profile = cli
                    .global
                    .profile
                    .clone()
                    .and_then(non_blank)
                    .or_else(|| std::env::var("APOLLO_PROFILE").ok().and_then(non_blank))
                    .or_else(|| {
                        loaded
                            .as_ref()
                            .and_then(|loaded| loaded.config.active_profile.clone())
                            .and_then(non_blank)
                    });
                let response = AuthStatusResponse {
                    authenticated: true,
                    source: "env".to_owned(),
                    profile,
                    backend: Some("env".to_owned()),
                    key: Some("APOLLO_TOKEN".to_owned()),
                };
                return Ok(writer.render_success(&response, response.render_table()));
            }

            let loaded = load_config(output)?;
            let writer_output = resolve_output(cli, &loaded, output)?;
            let writer = OutputWriter::new(writer_output);
            let context = resolve_context(cli, &loaded, output)?;
            let profile = required_profile(&context, writer_output)?;
            let status = credential::status(&loaded.path, &profile, context.credential.as_ref());
            let response = AuthStatusResponse {
                authenticated: status.authenticated,
                source: status.source.as_str().to_owned(),
                profile: Some(profile),
                backend: status.backend,
                key: status.key,
            };
            Ok(writer.render_success(&response, response.render_table()))
        }
        AuthCommand::Login {
            token_stdin,
            store_token_in_file,
        } => {
            let loaded = load_config(output)?;
            let writer_output = resolve_output(cli, &loaded, output)?;
            let writer = OutputWriter::new(writer_output);
            let context = resolve_context(cli, &loaded, output)?;
            let profile = required_profile(&context, writer_output)?;
            if !loaded.config.profiles.contains_key(&profile) {
                return Err(CliError::profile_not_found(&profile, writer_output));
            }
            let token = credential::token_from_login_input(token_stdin, writer_output)?;

            let credential_ref = store_setup_token(
                &loaded.path,
                &profile,
                &token,
                store_token_in_file,
                is_interactive_terminal(),
                writer_output,
            )?;

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
            let loaded = load_config(output)?;
            let writer_output = resolve_output(cli, &loaded, output)?;
            let writer = OutputWriter::new(writer_output);
            let context = resolve_context(cli, &loaded, output)?;
            let profile = required_profile(&context, output)?;
            let environment_token_still_active = std::env::var("APOLLO_TOKEN")
                .ok()
                .is_some_and(|token| !token.trim().is_empty());
            let credential_ref = context.credential.clone().unwrap_or_else(|| CredentialRef {
                backend: "native".to_owned(),
                key: profile.clone(),
            });
            credential::delete(&loaded.path, &credential_ref)
                .map_err(|error| CliError::credential_store_unavailable(&error, writer_output))?;
            let message = if environment_token_still_active {
                Some(
                    "Local credential was removed, but APOLLO_TOKEN is still set and will continue to authenticate commands in this shell. Run `unset APOLLO_TOKEN` to disable it."
                        .to_owned(),
                )
            } else {
                None
            };
            let response = AuthLogoutResponse {
                logged_out: true,
                profile,
                backend: credential_ref.backend,
                key: credential_ref.key,
                environment_token_still_active,
                message,
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

    match command {
        ProfileCommand::Add(args) => {
            let options = ProfileSetupOptions {
                mode: ProfileSetupMode::Add,
                name: args.name,
                operator: args.operator,
                token_stdin: args.token_stdin,
                store_token_in_file: args.store_token_in_file,
                overwrite: args.overwrite,
                use_profile: args.use_profile,
            };
            execute_profile_setup(options, cli, output)
        }
        ProfileCommand::List => {
            let writer_output = resolve_output(cli, &loaded, output)
                .unwrap_or_else(|_| output_from_flags_or_env(cli).unwrap_or(output));
            let writer = OutputWriter::new(writer_output);
            let response = ProfileListResponse::from_loaded_config(&loaded);
            Ok(writer.render_success(&response, response.render_table()))
        }
        ProfileCommand::Show => {
            let writer = OutputWriter::new(resolve_output(cli, &loaded, output)?);
            let context = resolve_context(cli, &loaded, output)?;
            let response = ProfileShowResponse::from_context(&loaded, context);
            Ok(writer.render_success(&response, response.render_table()))
        }
        ProfileCommand::Use { name } => {
            let writer_output = resolve_output(cli, &loaded, output)
                .unwrap_or_else(|_| output_from_flags_or_env(cli).unwrap_or(output));
            let writer = OutputWriter::new(writer_output);
            if !loaded.config.profiles.contains_key(&name) {
                return Err(CliError::profile_not_found(&name, writer_output));
            }

            let mut config = loaded.config.clone();
            config.active_profile = Some(name.clone());
            save_config(&loaded.path, &config, writer_output)?;

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

#[derive(Copy, Clone)]
enum ProfileSetupMode {
    Init,
    Add,
}

impl ProfileSetupMode {
    fn command_name(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Add => "profile add",
        }
    }
}

struct ProfileSetupOptions {
    mode: ProfileSetupMode,
    name: Option<String>,
    operator: Option<String>,
    token_stdin: bool,
    store_token_in_file: bool,
    overwrite: bool,
    use_profile: bool,
}

fn execute_profile_setup(
    options: ProfileSetupOptions,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let loaded = load_config(output)?;
    let writer_output = resolve_output(cli, &loaded, output)
        .unwrap_or_else(|_| output_from_flags_or_env(cli).unwrap_or(output));
    let writer = OutputWriter::new(writer_output);
    let interactive = is_interactive_terminal();

    let profile_name = resolve_setup_profile_name(&options, cli, interactive, writer_output)?;
    if loaded.config.profiles.contains_key(&profile_name) && !options.overwrite {
        return Err(CliError::profile_already_exists(
            &profile_name,
            options.mode.command_name(),
            writer_output,
        ));
    }

    let server = resolve_setup_server(&options, cli, interactive, writer_output)?;
    let profile_output = cli.global.output;
    let response_output = profile_output.unwrap_or(OutputFormat::Table);
    let operator = resolve_setup_operator(&options, interactive, writer_output)?;
    let existing_profile = loaded.config.profiles.get(&profile_name);

    let mut profile_config = ProfileConfig {
        server: Some(server.clone()),
        output: profile_output,
        operator: operator.clone(),
        credential: existing_profile.and_then(|profile| profile.credential.clone()),
    };

    let credential = resolve_setup_token(&options, interactive, writer_output)?
        .map(|token| {
            store_setup_token(
                &loaded.path,
                &profile_name,
                &token,
                options.store_token_in_file,
                interactive,
                writer_output,
            )
        })
        .transpose()?;
    if let Some(credential) = credential.clone() {
        profile_config.credential = Some(credential);
    }

    let mut config = loaded.config.clone();
    config.profiles.insert(profile_name.clone(), profile_config);
    let should_set_active = options.use_profile || config.active_profile.is_none();
    if should_set_active {
        config.active_profile = Some(profile_name.clone());
    }
    save_config(&loaded.path, &config, writer_output)?;
    let response_credential = config
        .profiles
        .get(&profile_name)
        .and_then(|profile| profile.credential.clone());

    let response = ProfileSetupResponse {
        profile: profile_name,
        active_profile: config.active_profile.clone(),
        server,
        output: response_output.to_string(),
        operator,
        credential: response_credential,
        config_path: loaded.path.display().to_string(),
        next_steps: vec![
            "apollo profile show".to_owned(),
            "apollo app list".to_owned(),
            "apollo env list --app <appId>".to_owned(),
        ],
    };
    Ok(writer.render_success(&response, response.render_table()))
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
        AppCommand::Get { app_id } => {
            format!("/openapi/v1/apps/{}", encode_path_segment(&app_id))
        }
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
        EnvCommand::List { app } => {
            let path = format!("/openapi/v1/apps/{}/envclusters", encode_path_segment(&app));
            openapi.request("GET", &path, None)
        }
    }
}

fn execute_namespace(
    command: NamespaceCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    if matches!(command, NamespaceCommand::Create { .. }) {
        require_yes_for_openapi(cli, output)?;
    }
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
            public_namespace,
        } => {
            let operator = required_operator(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let app_namespace_name =
                register_app_namespace(&openapi, &scope.app, &name, public_namespace, &operator)?;
            let path = append_query("/openapi/v1/namespaces".to_owned(), "operator", &operator);
            let body = json!([{
                "appId": scope.app,
                "env": scope.env,
                "clusterName": scope.cluster,
                "appNamespaceName": app_namespace_name,
            }]);
            openapi.request("POST", &path, Some(body))
        }
    }
}

struct AppNamespaceRegistration {
    name: String,
    format: &'static str,
}

fn register_app_namespace(
    openapi: &OpenApiCommandContext,
    app_id: &str,
    namespace_name: &str,
    public_namespace: bool,
    operator: &str,
) -> Result<String, CliError> {
    if let Some(existing_name) = find_app_namespace(openapi, app_id, namespace_name)? {
        return Ok(existing_name);
    }

    let registration = app_namespace_registration(namespace_name);
    let path = format!(
        "/openapi/v1/apps/{}/appnamespaces",
        encode_path_segment(app_id)
    );
    let body = json!({
        "appId": app_id,
        "name": registration.name,
        "format": registration.format,
        "isPublic": public_namespace,
        "dataChangeCreatedBy": operator,
    });
    let response = openapi.client.request("POST", &path, Some(body))?;
    Ok(response
        .data
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(namespace_name)
        .to_owned())
}

fn find_app_namespace(
    openapi: &OpenApiCommandContext,
    app_id: &str,
    namespace_name: &str,
) -> Result<Option<String>, CliError> {
    let path = format!(
        "/openapi/v1/apps/{}/appnamespaces/{}",
        encode_path_segment(app_id),
        encode_path_segment(namespace_name)
    );
    match openapi.client.request("GET", &path, None) {
        Ok(response) => Ok(Some(
            response
                .data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(namespace_name)
                .to_owned(),
        )),
        Err(error) if error.http_status_code() == Some(404) => Ok(None),
        Err(error) => Err(error),
    }
}

fn app_namespace_registration(namespace_name: &str) -> AppNamespaceRegistration {
    let lowercase_name = namespace_name.to_ascii_lowercase();
    for format in ["yaml", "yml", "json", "xml"] {
        let suffix = format!(".{format}");
        if lowercase_name.ends_with(&suffix) && namespace_name.len() > suffix.len() {
            return AppNamespaceRegistration {
                name: namespace_name[..namespace_name.len() - suffix.len()].to_owned(),
                format,
            };
        }
    }

    AppNamespaceRegistration {
        name: namespace_name.to_owned(),
        format: "properties",
    }
}

fn execute_config(
    command: ConfigCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    if config_command_requires_confirmation(&command) {
        require_yes_for_openapi(cli, output)?;
    }
    let openapi = openapi_context(cli, output)?;
    match command {
        ConfigCommand::List { scope, page, size } => {
            let mut path = format!("{}/items", namespace_path(&scope));
            path = append_query(path, "page", &page.unwrap_or(DEFAULT_PAGE).to_string());
            path = append_query(path, "size", &size.unwrap_or(DEFAULT_PAGE_SIZE).to_string());
            openapi.request("GET", &path, None)
        }
        ConfigCommand::Get { scope, key } => {
            let path = item_path(&scope, &key);
            openapi.request("GET", &path, None)
        }
        ConfigCommand::Set {
            scope,
            key,
            value,
            comment,
            operator,
        } => {
            let operator = required_operator(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let update_path = append_query(item_path(&scope, &key), "createIfNotExists", "true");
            let create_path = append_query(
                format!("{}/items", namespace_path(&scope)),
                "operator",
                &operator,
            );
            let body = json!({
                "key": key,
                "value": value,
                "dataChangeCreatedBy": operator,
                "dataChangeLastModifiedBy": operator,
            });
            let mut body = body;
            if let Some(comment) = comment {
                body["comment"] = json!(comment);
            }
            match openapi
                .client
                .request("PUT", &update_path, Some(body.clone()))
            {
                Ok(response) => Ok(render_openapi_response(&openapi.writer, &response)),
                Err(error) if error.http_status_code() == Some(404) => {
                    let response = openapi.client.request("POST", &create_path, Some(body))?;
                    Ok(render_openapi_response(&openapi.writer, &response))
                }
                Err(error) => Err(error),
            }
        }
        ConfigCommand::Delete {
            scope,
            key,
            operator,
        } => {
            let operator = required_operator(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let path = append_query(item_path(&scope, &key), "operator", &operator);
            openapi.request("DELETE", &path, None)
        }
        ConfigCommand::Diff {
            scope,
            target_env,
            target_cluster,
            target_namespace,
        } => {
            let sync_items = source_sync_items(&openapi, &scope)?;
            let body = sync_body(
                &scope,
                target_env,
                target_cluster,
                target_namespace,
                sync_items,
            );
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
            let operator = required_operator(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            if let Some(namespace) = &target_namespace
                && namespace != &scope.namespace
            {
                return Err(CliError::invalid_input(
                    "config apply does not support --target-namespace different from the source namespace because Apollo synchronize requires matching namespace names",
                    openapi.context.output,
                ));
            }
            let sync_items = source_sync_items(&openapi, &scope)?;
            let body = sync_body(
                &scope,
                target_env,
                target_cluster,
                target_namespace,
                sync_items,
            );
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
    if release_command_requires_confirmation(&command) {
        require_yes_for_openapi(cli, output)?;
    }
    let openapi = openapi_context(cli, output)?;
    match command {
        ReleaseCommand::List { scope, page, size } => {
            let mut path = format!("{}/releases/active", namespace_path(&scope));
            path = append_query(path, "page", &page.unwrap_or(DEFAULT_PAGE).to_string());
            path = append_query(path, "size", &size.unwrap_or(DEFAULT_PAGE_SIZE).to_string());
            openapi.request("GET", &path, None)
        }
        ReleaseCommand::Create {
            scope,
            title,
            comment,
            emergency,
            operator,
        } => {
            let operator = required_operator(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
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
            let operator = required_operator(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
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
    if http_method_requires_confirmation(args.method) {
        require_yes_for_openapi(cli, output)?;
    }
    let openapi = openapi_context(cli, output)?;
    let body = match args.body {
        Some(body) => Some(serde_json::from_str::<Value>(&body).map_err(|error| {
            CliError::invalid_input(&error.to_string(), openapi.context.output)
        })?),
        None => None,
    };
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
    if env_token_is_set() && explicit_server(cli).is_some() {
        if let Ok(loaded) = load_config(output)
            && let Some(context) = env_openapi_context(cli, &loaded, output)?
        {
            return Ok(context);
        }
        return env_only_openapi_context(cli, output);
    }

    let loaded = load_config(output)?;
    if let Some(context) = env_openapi_context(cli, &loaded, output)? {
        return Ok(context);
    }

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

fn env_openapi_context(
    cli: &Cli,
    loaded: &LoadedConfig,
    output: OutputFormat,
) -> Result<Option<OpenApiCommandContext>, CliError> {
    if !env_token_is_set() {
        return Ok(None);
    }
    let Some(server) = explicit_server(cli) else {
        return Ok(None);
    };

    let writer_output = resolve_output(cli, loaded, output)
        .unwrap_or_else(|_| output_from_flags_or_env(cli).unwrap_or(output));
    let token = credential::resolve_token(&loaded.path, None, None)
        .map_err(|error| CliError::credential_store_unavailable(&error, writer_output))?
        .ok_or_else(|| {
            CliError::authentication_required(
                "Authenticate with APOLLO_TOKEN or `apollo auth login` before calling OpenAPI.",
                writer_output,
            )
        })?;
    let selected_profile = cli
        .global
        .profile
        .clone()
        .and_then(non_blank)
        .or_else(|| std::env::var("APOLLO_PROFILE").ok().and_then(non_blank))
        .or_else(|| loaded.config.active_profile.clone().and_then(non_blank));
    let operator = selected_profile
        .as_ref()
        .and_then(|profile| loaded.config.profiles.get(profile))
        .and_then(|profile| profile.operator.clone().and_then(non_blank));

    let context = RuntimeContext {
        profile: selected_profile,
        server: Some(server.clone()),
        output: writer_output,
        operator,
        credential: None,
    };

    Ok(Some(OpenApiCommandContext {
        context,
        writer: OutputWriter::new(writer_output),
        client: OpenApiClient::new(server, token, writer_output),
    }))
}

fn env_only_openapi_context(
    cli: &Cli,
    output: OutputFormat,
) -> Result<OpenApiCommandContext, CliError> {
    let server = explicit_server(cli).ok_or_else(|| {
        CliError::authentication_required(
            "Provide --server or APOLLO_SERVER when using APOLLO_TOKEN without a profile.",
            output,
        )
    })?;
    let writer_output = output_from_flags_or_env(cli).unwrap_or(output);
    let token = credential::resolve_token(std::path::Path::new(""), None, None)
        .map_err(|error| CliError::credential_store_unavailable(&error, writer_output))?
        .ok_or_else(|| {
            CliError::authentication_required(
                "Authenticate with APOLLO_TOKEN or `apollo auth login` before calling OpenAPI.",
                writer_output,
            )
        })?;
    let selected_profile = cli
        .global
        .profile
        .clone()
        .and_then(non_blank)
        .or_else(|| std::env::var("APOLLO_PROFILE").ok().and_then(non_blank));
    let context = RuntimeContext {
        profile: selected_profile,
        server: Some(server.clone()),
        output: writer_output,
        operator: None,
        credential: None,
    };
    Ok(OpenApiCommandContext {
        context,
        writer: OutputWriter::new(writer_output),
        client: OpenApiClient::new(server, token, writer_output),
    })
}

fn render_openapi_response(writer: &OutputWriter, response: &OpenApiResponse) -> RenderedOutput {
    writer.render_success(response, response.render_table())
}

fn config_command_requires_confirmation(command: &ConfigCommand) -> bool {
    matches!(
        command,
        ConfigCommand::Set { .. } | ConfigCommand::Delete { .. } | ConfigCommand::Apply { .. }
    )
}

fn release_command_requires_confirmation(command: &ReleaseCommand) -> bool {
    matches!(
        command,
        ReleaseCommand::Create { .. } | ReleaseCommand::Rollback { .. }
    )
}

fn http_method_requires_confirmation(method: crate::cli::HttpMethod) -> bool {
    matches!(
        method,
        crate::cli::HttpMethod::Post
            | crate::cli::HttpMethod::Put
            | crate::cli::HttpMethod::Patch
            | crate::cli::HttpMethod::Delete
    )
}

fn require_yes_for_openapi(cli: &Cli, output: OutputFormat) -> Result<(), CliError> {
    require_yes(cli, output_for_confirmation(cli, output))
}

fn output_for_confirmation(cli: &Cli, output: OutputFormat) -> OutputFormat {
    if let Some(output) = output_from_flags_or_env(cli) {
        return output;
    }

    load_config(output)
        .ok()
        .and_then(|loaded| resolve_output(cli, &loaded, output).ok())
        .unwrap_or(output)
}

fn explicit_server(cli: &Cli) -> Option<String> {
    cli.global
        .server
        .clone()
        .and_then(non_blank)
        .or_else(|| std::env::var("APOLLO_SERVER").ok().and_then(non_blank))
}

fn env_token_is_set() -> bool {
    std::env::var("APOLLO_TOKEN")
        .ok()
        .is_some_and(|token| !token.trim().is_empty())
}

fn required_server(context: &RuntimeContext, output: OutputFormat) -> Result<String, CliError> {
    context
        .server
        .clone()
        .filter(|server| !server.trim().is_empty())
        .ok_or_else(|| {
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
        .filter(|operator| !operator.trim().is_empty())
        .or_else(|| {
            context
                .operator
                .clone()
                .filter(|operator| !operator.trim().is_empty())
        })
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

fn item_path(scope: &NamespaceScopeArgs, key: &str) -> String {
    if key.contains('/') || key.contains('\\') {
        format!(
            "{}/encodedItems/{}",
            namespace_path(scope),
            encode_path_segment(&URL_SAFE_NO_PAD.encode(key.as_bytes()))
        )
    } else {
        format!(
            "{}/items/{}",
            namespace_path(scope),
            encode_path_segment(key)
        )
    }
}

fn source_sync_items(
    openapi: &OpenApiCommandContext,
    scope: &NamespaceScopeArgs,
) -> Result<Vec<Value>, CliError> {
    let mut items = Vec::new();
    let mut page = DEFAULT_PAGE;

    loop {
        let mut path = format!("{}/items", namespace_path(scope));
        path = append_query(path, "page", &page.to_string());
        path = append_query(path, "size", &SYNC_ITEMS_PAGE_SIZE.to_string());
        let response = openapi.client.request("GET", &path, None)?;
        let content = item_page_content(&response.data, openapi.context.output)?;
        let content_len = content.len();
        items.extend(content);

        let total = response.data.get("total").and_then(Value::as_u64);
        if total.is_some_and(|total| items.len() as u64 >= total)
            || content_len < SYNC_ITEMS_PAGE_SIZE as usize
        {
            break;
        }
        page += 1;
    }

    Ok(items)
}

fn item_page_content(data: &Value, output: OutputFormat) -> Result<Vec<Value>, CliError> {
    data.get("content")
        .and_then(Value::as_array)
        .or_else(|| data.as_array())
        .cloned()
        .ok_or_else(|| {
            CliError::invalid_input(
                "OpenAPI item list response did not contain a content array",
                output,
            )
        })
}

fn sync_body(
    scope: &NamespaceScopeArgs,
    target_env: String,
    target_cluster: String,
    target_namespace: Option<String>,
    sync_items: Vec<Value>,
) -> Value {
    json!({
        "syncToNamespaces": [{
            "appId": scope.cluster_scope.app.clone(),
            "env": target_env,
            "clusterName": target_cluster,
            "namespaceName": target_namespace.unwrap_or_else(|| scope.namespace.clone()),
        }],
        "syncItems": sync_items,
    })
}

fn resolve_setup_profile_name(
    options: &ProfileSetupOptions,
    cli: &Cli,
    interactive: bool,
    output: OutputFormat,
) -> Result<String, CliError> {
    options
        .name
        .clone()
        .and_then(non_blank)
        .or_else(|| cli.global.profile.clone().and_then(non_blank))
        .or_else(|| match options.mode {
            ProfileSetupMode::Init => Some(DEFAULT_INIT_PROFILE.to_owned()),
            ProfileSetupMode::Add => None,
        })
        .map(Ok)
        .unwrap_or_else(|| {
            if interactive {
                prompt_required("Profile name", None, output)
            } else {
                Err(CliError::invalid_input(
                    "provide a profile name, for example `apollo profile add dev --server ...`",
                    output,
                ))
            }
        })
}

fn resolve_setup_server(
    options: &ProfileSetupOptions,
    cli: &Cli,
    interactive: bool,
    output: OutputFormat,
) -> Result<String, CliError> {
    cli.global
        .server
        .clone()
        .and_then(non_blank)
        .or_else(|| match options.mode {
            ProfileSetupMode::Init => Some(DEFAULT_INIT_SERVER.to_owned()),
            ProfileSetupMode::Add => None,
        })
        .map(Ok)
        .unwrap_or_else(|| {
            if interactive {
                prompt_required("Apollo Portal server URL", None, output)
            } else {
                Err(CliError::invalid_input(
                    "provide a server with --server when adding a profile non-interactively",
                    output,
                ))
            }
        })
}

fn resolve_setup_operator(
    options: &ProfileSetupOptions,
    interactive: bool,
    output: OutputFormat,
) -> Result<Option<String>, CliError> {
    if let Some(operator) = options.operator.clone().and_then(non_blank) {
        return Ok(Some(operator));
    }
    if matches!(options.mode, ProfileSetupMode::Init) {
        return Ok(Some(DEFAULT_INIT_OPERATOR.to_owned()));
    }
    if interactive {
        prompt_optional("Default operator", output)
    } else {
        Ok(None)
    }
}

fn resolve_setup_token(
    options: &ProfileSetupOptions,
    interactive: bool,
    output: OutputFormat,
) -> Result<Option<Sensitive>, CliError> {
    if options.token_stdin {
        return credential::token_from_env_or_stdin(true, output).map(Some);
    }
    if interactive && prompt_yes_no("Store a Consumer token now?", false, output)? {
        credential::prompt_token(output).map(Some)
    } else {
        Ok(None)
    }
}

fn store_setup_token(
    config_path: &std::path::Path,
    profile: &str,
    token: &Sensitive,
    store_token_in_file: bool,
    interactive: bool,
    output: OutputFormat,
) -> Result<CredentialRef, CliError> {
    if store_token_in_file {
        return credential::store_file(config_path, profile, token)
            .map_err(|error| CliError::credential_store_unavailable(&error, output));
    }

    match credential::store_native(profile, token) {
        Ok(credential) => Ok(credential),
        Err(error) => {
            if interactive
                && prompt_yes_no(
                    "Native credential storage is unavailable. Store token in a local file instead?",
                    false,
                    output,
                )?
            {
                credential::store_file(config_path, profile, token)
                    .map_err(|error| CliError::credential_store_unavailable(&error, output))
            } else {
                Err(CliError::confirmation_required(
                    &format!(
                        "Native credential storage is unavailable: {}. Re-run with --store-token-in-file to use the explicit file fallback.",
                        error
                    ),
                    output,
                ))
            }
        }
    }
}

fn is_interactive_terminal() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

fn prompt_required(
    label: &str,
    default: Option<&str>,
    output: OutputFormat,
) -> Result<String, CliError> {
    loop {
        let value = prompt_line(label, default, output)?;
        if !value.trim().is_empty() {
            return Ok(value.trim().to_owned());
        }
        eprintln!("{} is required.", label);
    }
}

fn prompt_optional(label: &str, output: OutputFormat) -> Result<Option<String>, CliError> {
    let value = prompt_line(label, None, output)?;
    let value = value.trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

fn prompt_yes_no(label: &str, default: bool, output: OutputFormat) -> Result<bool, CliError> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        eprint!("{} {} ", label, suffix);
        io::stderr()
            .flush()
            .map_err(|error| CliError::invalid_input(&error.to_string(), output))?;
        let line = read_prompt_line(&mut io::stdin().lock(), output)?;
        let value = line.trim().to_ascii_lowercase();
        if value.is_empty() {
            return Ok(default);
        }
        match value.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => eprintln!("Please answer y or n."),
        }
    }
}

fn prompt_line(
    label: &str,
    default: Option<&str>,
    output: OutputFormat,
) -> Result<String, CliError> {
    if let Some(default) = default {
        eprint!("{} [{}]: ", label, default);
    } else {
        eprint!("{}: ", label);
    }
    io::stderr()
        .flush()
        .map_err(|error| CliError::invalid_input(&error.to_string(), output))?;

    let line = read_prompt_line(&mut io::stdin().lock(), output)?;
    let value = line.trim();
    if value.is_empty() {
        Ok(default.unwrap_or_default().to_owned())
    } else {
        Ok(value.to_owned())
    }
}

fn read_prompt_line<R: BufRead>(reader: &mut R, output: OutputFormat) -> Result<String, CliError> {
    let mut line = String::new();
    let bytes_read = reader
        .read_line(&mut line)
        .map_err(|error| CliError::invalid_input(&error.to_string(), output))?;
    if bytes_read == 0 {
        return Err(CliError::invalid_input("input aborted", output));
    }
    Ok(line)
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
            server: profile
                .server
                .clone()
                .unwrap_or_else(|| "<none>".to_owned()),
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
#[serde(rename_all = "camelCase")]
struct ProfileSetupResponse {
    profile: String,
    active_profile: Option<String>,
    server: String,
    output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential: Option<CredentialRef>,
    config_path: String,
    next_steps: Vec<String>,
}

impl ProfileSetupResponse {
    fn render_table(&self) -> String {
        let mut lines = vec![
            format!("Profile '{}' configured.", self.profile),
            format!("Config path: {}", self.config_path),
            format!("Server: {}", self.server),
            format!("Output: {}", self.output),
        ];
        if self.active_profile.as_deref() == Some(self.profile.as_str()) {
            lines.push("Active profile: yes".to_owned());
        }
        if let Some(operator) = &self.operator {
            lines.push(format!("Operator: {}", operator));
        }
        if let Some(credential) = &self.credential {
            lines.push(format!("Credential backend: {}", credential.backend));
            lines.push(format!("Credential key: {}", credential.key));
        } else {
            lines.push("Credential: not configured".to_owned());
            lines.push(
                "Run `apollo auth login --token-stdin` when you have a Consumer token.".to_owned(),
            );
        }
        if !self.next_steps.is_empty() {
            lines.push("Next steps:".to_owned());
            lines.extend(self.next_steps.iter().map(|step| format!("  {}", step)));
        }
        lines.join("\n")
    }
}

#[derive(Serialize)]
struct AuthStatusResponse {
    authenticated: bool,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
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
            self.profile.as_deref().unwrap_or("<none>"),
            state,
            self.source
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::cli::OutputFormat;

    #[test]
    fn read_prompt_line_reports_eof_as_aborted_input() {
        let mut reader = Cursor::new(Vec::<u8>::new());
        let error =
            super::read_prompt_line(&mut reader, OutputFormat::Json).expect_err("EOF should fail");

        assert_eq!(error.exit_code(), 1);
    }
}

#[derive(Serialize)]
struct AuthLogoutResponse {
    #[serde(rename = "loggedOut")]
    logged_out: bool,
    profile: String,
    backend: String,
    key: String,
    #[serde(rename = "environmentCredentialStillActive")]
    environment_token_still_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl AuthLogoutResponse {
    fn render_table(&self) -> String {
        let mut body = format!(
            "Credential removed for profile '{}'.\nBackend: {}\nKey: {}",
            self.profile, self.backend, self.key
        );
        if let Some(message) = &self.message {
            body.push('\n');
            body.push_str(message);
        }
        body
    }
}

fn required_profile(context: &RuntimeContext, output: OutputFormat) -> Result<String, CliError> {
    context.profile.clone().ok_or_else(|| {
        CliError::invalid_input("select a profile with --profile or active_profile", output)
    })
}
