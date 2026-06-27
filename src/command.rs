use std::collections::HashSet;
use std::io::{self, BufRead, IsTerminal, Write};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::{
    ApiArgs, AppCommand, AuthCommand, AuthMode, Cli, Commands, ConfigCommand, EnvCommand, InitArgs,
    NamespaceCommand, NamespaceScopeArgs, OutputFormat, ProfileCommand, ReleaseCommand,
    USER_TOKEN_PREFIX,
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
        auth_mode: args.auth_mode,
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
                    auth_mode: Some(
                        AuthMode::from_token_value(
                            &std::env::var("APOLLO_TOKEN").unwrap_or_default(),
                        )
                        .to_string(),
                    ),
                    backend: Some("env".to_owned()),
                    key: Some("APOLLO_TOKEN".to_owned()),
                };
                return Ok(writer.render_success(&response, response.render_table()));
            }

            let loaded = load_config(output)?;
            let writer_output = resolve_output(cli, &loaded, output)
                .unwrap_or_else(|_| output_from_flags_or_env(cli).unwrap_or(output));
            let writer = OutputWriter::new(writer_output);
            let explicit_profile = cli
                .global
                .profile
                .clone()
                .and_then(non_blank)
                .or_else(|| std::env::var("APOLLO_PROFILE").ok().and_then(non_blank));
            let profile = explicit_profile.clone().or_else(|| {
                loaded
                    .config
                    .active_profile
                    .clone()
                    .and_then(non_blank)
                    .filter(|profile| loaded.config.profiles.contains_key(profile))
            });
            let Some(profile) = profile else {
                let response = AuthStatusResponse {
                    authenticated: false,
                    source: "none".to_owned(),
                    profile: None,
                    auth_mode: None,
                    backend: None,
                    key: None,
                };
                return Ok(writer.render_success(&response, response.render_table()));
            };
            let profile_config = loaded
                .config
                .profiles
                .get(&profile)
                .ok_or_else(|| CliError::profile_not_found(&profile, writer_output))?;
            let status =
                credential::status(&loaded.path, &profile, profile_config.credential.as_ref());
            let response = AuthStatusResponse {
                authenticated: status.authenticated,
                source: status.source.as_str().to_owned(),
                profile: Some(profile),
                auth_mode: Some(profile_config.resolved_auth_mode().to_string()),
                backend: status.backend,
                key: status.key,
            };
            Ok(writer.render_success(&response, response.render_table()))
        }
        AuthCommand::Login {
            auth_mode,
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
            let existing_auth_mode = loaded
                .config
                .profiles
                .get(&profile)
                .and_then(|profile| profile.auth_mode);
            let auth_mode = resolve_auth_mode(
                auth_mode,
                Some(&token),
                existing_auth_mode,
                AuthMode::UserToken,
                writer_output,
            )?;
            let credential_ref = store_setup_token(
                &loaded.path,
                &profile,
                &token,
                store_token_in_file,
                is_interactive_terminal(),
                writer_output,
            )?;
            delete_replaced_credential(
                &loaded.path,
                replaced_credential_to_delete(
                    &profile,
                    loaded.config.profiles.get(&profile),
                    &credential_ref,
                ),
                writer_output,
            )?;

            let mut config = loaded.config.clone();
            let profile_config = config
                .profiles
                .get_mut(&profile)
                .ok_or_else(|| CliError::profile_not_found(&profile, writer_output))?;
            profile_config.auth_mode = Some(auth_mode);
            if auth_mode.is_user_token() {
                profile_config.operator = None;
            }
            profile_config.credential = Some(credential_ref.clone());
            save_config(&loaded.path, &config, writer_output)?;

            let response = AuthLoginResponse {
                stored: true,
                profile: profile.clone(),
                auth_mode: auth_mode.to_string(),
                backend: credential_ref.backend,
                key: credential_ref.key,
            };
            Ok(writer.render_success(&response, response.render_table()))
        }
        AuthCommand::Whoami => {
            execute_auth_self_check(cli, output, "/openapi/v1/user-tokens/current")
        }
        AuthCommand::Capabilities => {
            execute_auth_self_check(cli, output, "/openapi/v1/user-tokens/current/capabilities")
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
                auth_mode: args.auth_mode,
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
    auth_mode: Option<AuthMode>,
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

    let existing_profile = loaded.config.profiles.get(&profile_name);
    let server = resolve_setup_server(&options, cli, existing_profile, interactive, writer_output)?;
    let profile_output = cli
        .global
        .output
        .or_else(|| existing_profile.and_then(|profile| profile.output));
    let response_output = profile_output.unwrap_or(OutputFormat::Table);
    let setup_token = resolve_setup_token(&options, interactive, writer_output)?;
    reject_auth_mode_change_without_token(
        options.auth_mode,
        setup_token.as_ref(),
        existing_profile,
        writer_output,
    )?;
    let auth_mode = resolve_auth_mode(
        options.auth_mode,
        setup_token.as_ref(),
        existing_profile.map(ProfileConfig::resolved_auth_mode),
        AuthMode::UserToken,
        writer_output,
    )?;
    let operator = resolve_setup_operator(
        &options,
        auth_mode,
        existing_profile,
        interactive,
        writer_output,
    )?
    .or_else(|| {
        (!auth_mode.is_user_token())
            .then(|| {
                existing_profile.and_then(|profile| profile.operator.clone().and_then(non_blank))
            })
            .flatten()
    });

    let mut profile_config = ProfileConfig {
        server: Some(server.clone()),
        output: profile_output,
        auth_mode: Some(auth_mode),
        operator: operator.clone(),
        credential: preserved_setup_credential(existing_profile, setup_token.as_ref()),
    };

    let credential = setup_token
        .as_ref()
        .map(|token| {
            store_setup_token(
                &loaded.path,
                &profile_name,
                token,
                options.store_token_in_file,
                interactive,
                writer_output,
            )
        })
        .transpose()?;
    if let Some(credential) = credential.clone() {
        delete_replaced_credential(
            &loaded.path,
            replaced_credential_to_delete(&profile_name, existing_profile, &credential),
            writer_output,
        )?;
        profile_config.credential = Some(credential);
    }

    let mut config = loaded.config.clone();
    config.profiles.insert(profile_name.clone(), profile_config);
    let should_set_active = options.use_profile
        || match config.active_profile.as_deref() {
            Some(active_profile) => !config.profiles.contains_key(active_profile),
            None => true,
        };
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
        auth_mode: auth_mode.to_string(),
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
    match command {
        AppCommand::List { app_ids } => {
            if let Some(app_ids) = app_ids {
                if openapi.context.auth_mode.is_user_token() {
                    let path = append_query("/openapi/v1/apps".to_owned(), "appIds", &app_ids);
                    openapi.request("GET", &path, None)
                } else {
                    let response =
                        openapi
                            .client
                            .request("GET", "/openapi/v1/apps/authorized", None)?;
                    let data = filter_apps_by_ids(response.data.clone(), &app_ids);
                    Ok(render_openapi_response_with_data(
                        &openapi.writer,
                        &response,
                        data,
                    ))
                }
            } else {
                let path = if openapi.context.auth_mode.is_user_token() {
                    "/openapi/v1/apps"
                } else {
                    "/openapi/v1/apps/authorized"
                };
                openapi.request("GET", path, None)
            }
        }
        AppCommand::Get { app_id } => {
            ensure_consumer_token_app_authorized(&openapi, &app_id)?;
            let path = format!("/openapi/v1/apps/{}", encode_path_segment(&app_id));
            openapi.request("GET", &path, None)
        }
    }
}

fn execute_env(
    command: EnvCommand,
    cli: &Cli,
    output: OutputFormat,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    match command {
        EnvCommand::List { app } => {
            ensure_consumer_token_app_authorized(&openapi, &app)?;
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
            let response = openapi.client.request("GET", &path, None)?;
            let data = redact_nested_item_values(response.data.clone());
            Ok(render_openapi_response_with_data(
                &openapi.writer,
                &response,
                data,
            ))
        }
        NamespaceCommand::Get { scope, namespace } => {
            let path = namespace_path(&NamespaceScopeArgs {
                cluster_scope: scope,
                namespace,
            });
            let response = openapi.client.request("GET", &path, None)?;
            let data = redact_nested_item_values(response.data.clone());
            Ok(render_openapi_response_with_data(
                &openapi.writer,
                &response,
                data,
            ))
        }
        NamespaceCommand::Create {
            scope,
            name,
            operator,
            public_namespace,
            comment,
            append_namespace_prefix,
        } => {
            let operator = operator_for_mutation(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let app_namespace = register_app_namespace(
                &openapi,
                &scope.app,
                &name,
                public_namespace,
                comment.as_deref(),
                append_namespace_prefix,
                operator.as_deref(),
            )?;
            let namespace_scope = NamespaceScopeArgs {
                cluster_scope: scope.clone(),
                namespace: app_namespace.name.clone(),
            };
            if !app_namespace.created {
                ensure_namespace_absent(&openapi, &namespace_scope)?;
            }
            let path = append_optional_query(
                "/openapi/v1/namespaces".to_owned(),
                "operator",
                operator.as_deref(),
            );
            let body = json!([{
                "appId": &scope.app,
                "env": &scope.env,
                "clusterName": &scope.cluster,
                "appNamespaceName": &app_namespace.name,
            }]);
            match openapi.request("POST", &path, Some(body)) {
                Ok(output) => Ok(output),
                Err(error) if is_namespace_create_reported_failed(&error) => {
                    match openapi
                        .client
                        .request("GET", &namespace_path(&namespace_scope), None)
                    {
                        Ok(response) => Ok(render_openapi_response(&openapi.writer, &response)),
                        Err(_) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        }
    }
}

fn ensure_namespace_absent(
    openapi: &OpenApiCommandContext,
    namespace_scope: &NamespaceScopeArgs,
) -> Result<(), CliError> {
    match openapi
        .client
        .request("GET", &namespace_path(namespace_scope), None)
    {
        Ok(_) => Err(CliError::invalid_input(
            &format!(
                "namespace already exists: app={} env={} cluster={} namespace={}",
                namespace_scope.cluster_scope.app,
                namespace_scope.cluster_scope.env,
                namespace_scope.cluster_scope.cluster,
                namespace_scope.namespace
            ),
            openapi.context.output,
        )),
        Err(error) if is_missing_namespace(&error) => Ok(()),
        Err(error)
            if openapi.context.auth_mode.is_user_token()
                && matches!(error.http_status_code(), Some(403)) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

struct AppNamespaceRegistration {
    name: String,
    format: &'static str,
}

struct RegisteredAppNamespace {
    name: String,
    created: bool,
}

fn register_app_namespace(
    openapi: &OpenApiCommandContext,
    app_id: &str,
    namespace_name: &str,
    public_namespace: bool,
    comment: Option<&str>,
    append_namespace_prefix: bool,
    operator: Option<&str>,
) -> Result<RegisteredAppNamespace, CliError> {
    let registration = app_namespace_registration(namespace_name);
    if let Some(existing) = find_app_namespace(openapi, app_id, namespace_name)? {
        if public_namespace && matches!(existing.is_public, Some(false)) {
            if let Some(existing) =
                find_prefixed_public_app_namespace(openapi, app_id, &registration)?
            {
                return Ok(RegisteredAppNamespace {
                    name: existing.name,
                    created: false,
                });
            }
            return Err(CliError::invalid_input(
                "existing app namespace is private; remove --public or create a public app namespace before reusing it",
                openapi.context.output,
            ));
        }
        if !public_namespace && matches!(existing.is_public, Some(true)) {
            return Err(CliError::invalid_input(
                "existing app namespace is public; add --public to reuse it or choose a private app namespace name",
                openapi.context.output,
            ));
        }
        return Ok(RegisteredAppNamespace {
            name: existing.name,
            created: false,
        });
    }
    if public_namespace
        && let Some(existing) = find_prefixed_public_app_namespace(openapi, app_id, &registration)?
    {
        return Ok(RegisteredAppNamespace {
            name: existing.name,
            created: false,
        });
    }

    let path = format!(
        "/openapi/v1/apps/{}/appnamespaces",
        encode_path_segment(app_id)
    );
    let mut body = json!({
        "appId": app_id,
        "name": registration.name,
        "format": registration.format,
        "isPublic": public_namespace,
        "appendNamespacePrefix": append_namespace_prefix,
    });
    if let Some(comment) = comment {
        body["comment"] = json!(comment);
    }
    if let Some(operator) = operator {
        body["dataChangeCreatedBy"] = json!(operator);
    }
    let response = openapi.client.request("POST", &path, Some(body))?;
    Ok(RegisteredAppNamespace {
        name: response
            .data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(namespace_name)
            .to_owned(),
        created: true,
    })
}

struct ExistingAppNamespace {
    name: String,
    is_public: Option<bool>,
}

fn find_app_namespace(
    openapi: &OpenApiCommandContext,
    app_id: &str,
    namespace_name: &str,
) -> Result<Option<ExistingAppNamespace>, CliError> {
    let path = format!(
        "/openapi/v1/apps/{}/appnamespaces/{}",
        encode_path_segment(app_id),
        encode_path_segment(namespace_name)
    );
    match openapi.client.request("GET", &path, None) {
        Ok(response) => {
            let Some(name) = response.data.get("name").and_then(Value::as_str) else {
                return Ok(None);
            };
            if name.trim().is_empty() {
                return Ok(None);
            }
            Ok(Some(ExistingAppNamespace {
                name: name.to_owned(),
                is_public: response.data.get("isPublic").and_then(Value::as_bool),
            }))
        }
        Err(error) if is_missing_app_namespace(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn find_prefixed_public_app_namespace(
    openapi: &OpenApiCommandContext,
    app_id: &str,
    registration: &AppNamespaceRegistration,
) -> Result<Option<ExistingAppNamespace>, CliError> {
    let path = format!(
        "/openapi/v1/apps/{}/appnamespaces",
        encode_path_segment(app_id)
    );
    match openapi.client.request("GET", &path, None) {
        Ok(response) => Ok(app_namespace_items(&response.data)
            .into_iter()
            .filter_map(existing_app_namespace_from_value)
            .find(|namespace| {
                matches!(namespace.is_public, Some(true))
                    && stored_public_app_namespace_matches(&namespace.name, registration)
            })),
        Err(error) if is_missing_app_namespace(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn app_namespace_items(data: &Value) -> Vec<&Value> {
    if let Some(items) = data.as_array() {
        return items.iter().collect();
    }
    data.get("content")
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn existing_app_namespace_from_value(value: &Value) -> Option<ExistingAppNamespace> {
    let name = value.get("name").and_then(Value::as_str)?;
    if name.trim().is_empty() {
        return None;
    }
    Some(ExistingAppNamespace {
        name: name.to_owned(),
        is_public: value.get("isPublic").and_then(Value::as_bool),
    })
}

fn stored_public_app_namespace_matches(
    stored_name: &str,
    registration: &AppNamespaceRegistration,
) -> bool {
    let requested_name = if registration.format == "properties" {
        registration.name.clone()
    } else {
        format!("{}.{}", registration.name, registration.format)
    };
    stored_name == requested_name
        || stored_name
            .split_once('.')
            .is_some_and(|(_, suffix)| suffix == requested_name)
}

fn is_missing_app_namespace(error: &CliError) -> bool {
    matches!(error.http_status_code(), Some(404))
        || (matches!(error.http_status_code(), Some(400))
            && error
                .http_status_message()
                .is_some_and(|message| message.contains("appNamespace not exist")))
}

fn is_missing_namespace(error: &CliError) -> bool {
    matches!(error.http_status_code(), Some(404))
        || (matches!(error.http_status_code(), Some(400))
            && error.http_status_message().is_some_and(|message| {
                message.contains("namespace not exist")
                    && !message.contains("appNamespace not exist")
            }))
}

fn is_namespace_create_reported_failed(error: &CliError) -> bool {
    matches!(error.http_status_code(), Some(400))
        && error
            .http_status_message()
            .is_some_and(|message| message.contains("create namespace failed for"))
}

fn app_namespace_registration(namespace_name: &str) -> AppNamespaceRegistration {
    let lowercase_name = namespace_name.to_ascii_lowercase();
    for format in ["yaml", "yml", "json", "xml", "txt"] {
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
            let response = openapi.client.request("GET", &path, None)?;
            let data = redact_config_item_values(response.data.clone());
            Ok(render_openapi_response_with_data(
                &openapi.writer,
                &response,
                data,
            ))
        }
        ConfigCommand::Get { scope, key } => {
            ensure_consumer_token_app_authorized(&openapi, &scope.cluster_scope.app)?;
            let path = item_read_path(&scope, &key);
            openapi.request("GET", &path, None)
        }
        ConfigCommand::Set {
            scope,
            key,
            value,
            item_type,
            comment,
            operator,
        } => {
            let operator = operator_for_mutation(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let update_path =
                append_query(item_write_path(&scope, &key), "createIfNotExists", "true");
            let create_path = append_optional_query(
                format!("{}/items", namespace_path(&scope)),
                "operator",
                operator.as_deref(),
            );
            let mut body = json!({
                "key": key,
                "value": value,
                "type": item_type,
            });
            if let Some(operator) = &operator {
                body["dataChangeCreatedBy"] = json!(operator);
                body["dataChangeLastModifiedBy"] = json!(operator);
            }
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
            let operator = operator_for_mutation(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let path = append_optional_query(
                item_write_path(&scope, &key),
                "operator",
                operator.as_deref(),
            );
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
            let response = openapi.client.request("POST", &path, Some(body))?;
            let data = redact_config_item_values(response.data.clone());
            Ok(render_openapi_response_with_data(
                &openapi.writer,
                &response,
                data,
            ))
        }
        ConfigCommand::Apply {
            scope,
            target_env,
            target_cluster,
            target_namespace,
            operator,
        } => {
            let operator = operator_for_mutation(
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
            let path = append_optional_query(
                format!("{}/items/synchronize", namespace_path(&scope)),
                "operator",
                operator.as_deref(),
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
            let response = openapi.client.request("GET", &path, None)?;
            let data = redact_release_configurations(response.data.clone());
            Ok(render_openapi_response_with_data(
                &openapi.writer,
                &response,
                data,
            ))
        }
        ReleaseCommand::Create {
            scope,
            title,
            comment,
            emergency,
            operator,
        } => {
            let operator = operator_for_mutation(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let path = format!("{}/releases", namespace_path(&scope));
            let mut body = json!({
                "releaseTitle": title,
                "releaseComment": comment.unwrap_or_default(),
                "isEmergencyPublish": emergency,
            });
            if let Some(operator) = operator {
                body["releasedBy"] = json!(operator);
            }
            let response = openapi.client.request("POST", &path, Some(body))?;
            let data = redact_release_configurations(response.data.clone());
            Ok(render_openapi_response_with_data(
                &openapi.writer,
                &response,
                data,
            ))
        }
        ReleaseCommand::Rollback {
            env,
            release_id,
            to_release_id,
            operator,
        } => {
            let operator = operator_for_mutation(
                operator.as_deref(),
                &openapi.context,
                openapi.context.output,
            )?;
            let mut path = append_optional_query(
                format!(
                    "/openapi/v1/envs/{}/releases/{}/rollback",
                    encode_path_segment(&env),
                    release_id
                ),
                "operator",
                operator.as_deref(),
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

fn execute_auth_self_check(
    cli: &Cli,
    output: OutputFormat,
    path: &str,
) -> Result<RenderedOutput, CliError> {
    let openapi = openapi_context(cli, output)?;
    if !openapi.context.auth_mode.is_user_token() {
        return Err(CliError::invalid_input(
            "auth whoami and auth capabilities require user-token auth mode; use a Portal user access token starting with apollo_pat_",
            openapi.context.output,
        ));
    }
    let response = openapi.client.request("GET", path, None)?;
    Ok(openapi
        .writer
        .render_success(&response, render_user_token_current_table(&response.data)))
}

fn render_user_token_current_table(data: &Value) -> String {
    let user = data
        .get("userId")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let token_name = data
        .get("tokenName")
        .and_then(Value::as_str)
        .unwrap_or("<unnamed>");
    let token_prefix = data
        .get("tokenPrefix")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let expires = data
        .get("expires")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let action_count = data
        .get("actions")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);

    [
        format!("User: {}", user),
        format!("Token: {} ({})", token_name, token_prefix),
        format!("Expires: {}", expires),
        format!(
            "Operations: {}",
            render_scope(data, "allOperations", "operations")
        ),
        format!("Apps: {}", render_scope(data, "allApps", "appIds")),
        format!("Envs: {}", render_scope(data, "allEnvs", "envs")),
        format!(
            "Namespaces: {}",
            render_namespace_scope(data.get("allNamespaces"), data.get("namespaces"))
        ),
        format!("Actions: {}", action_count),
    ]
    .join("\n")
}

fn render_scope(data: &Value, all_key: &str, list_key: &str) -> String {
    if data.get(all_key).and_then(Value::as_bool).unwrap_or(false) {
        return "<all>".to_owned();
    }
    let values = data
        .get(list_key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if values.is_empty() {
        "<none>".to_owned()
    } else {
        values.join(", ")
    }
}

fn render_namespace_scope(all_value: Option<&Value>, namespaces: Option<&Value>) -> String {
    if all_value.and_then(Value::as_bool).unwrap_or(false) {
        return "<all>".to_owned();
    }
    let values = namespaces
        .and_then(Value::as_array)
        .map(|namespaces| {
            namespaces
                .iter()
                .map(|namespace| {
                    let app = namespace
                        .get("appId")
                        .and_then(Value::as_str)
                        .unwrap_or("*");
                    let env = namespace.get("env").and_then(Value::as_str).unwrap_or("*");
                    let cluster = namespace
                        .get("clusterName")
                        .and_then(Value::as_str)
                        .unwrap_or("*");
                    let name = namespace
                        .get("namespaceName")
                        .and_then(Value::as_str)
                        .unwrap_or("*");
                    format!("{}/{}/{}/{}", app, env, cluster, name)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if values.is_empty() {
        "<none>".to_owned()
    } else {
        values.join(", ")
    }
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
    let mut context = resolve_context(cli, &loaded, writer_output)?;
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
    let auth_mode = if env_token_is_set() {
        AuthMode::from_token_value(token.expose_secret())
    } else {
        context.auth_mode
    };
    context.auth_mode = auth_mode;
    Ok(OpenApiCommandContext {
        context,
        writer: OutputWriter::new(writer_output),
        client: OpenApiClient::new(server, token, auth_mode, writer_output),
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
    let auth_mode = AuthMode::from_token_value(token.expose_secret());
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
        auth_mode,
        operator,
        credential: None,
    };

    Ok(Some(OpenApiCommandContext {
        context,
        writer: OutputWriter::new(writer_output),
        client: OpenApiClient::new(server, token, auth_mode, writer_output),
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
    let auth_mode = AuthMode::from_token_value(token.expose_secret());
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
        auth_mode,
        operator: None,
        credential: None,
    };
    Ok(OpenApiCommandContext {
        context,
        writer: OutputWriter::new(writer_output),
        client: OpenApiClient::new(server, token, auth_mode, writer_output),
    })
}

fn render_openapi_response(writer: &OutputWriter, response: &OpenApiResponse) -> RenderedOutput {
    writer.render_success(response, response.render_table())
}

fn render_openapi_response_with_data(
    writer: &OutputWriter,
    response: &OpenApiResponse,
    data: Value,
) -> RenderedOutput {
    let response = response.with_data(data);
    render_openapi_response(writer, &response)
}

fn filter_apps_by_ids(data: Value, app_ids: &str) -> Value {
    let selected = app_ids
        .split(',')
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
        .collect::<HashSet<_>>();
    if selected.is_empty() {
        return data;
    }

    filter_value_array(data, |app| {
        app.get("appId")
            .and_then(Value::as_str)
            .is_some_and(|app_id| selected.contains(app_id))
    })
}

fn ensure_consumer_token_app_authorized(
    openapi: &OpenApiCommandContext,
    app_id: &str,
) -> Result<(), CliError> {
    if openapi.context.auth_mode.is_user_token() {
        return Ok(());
    }

    let response = openapi
        .client
        .request("GET", "/openapi/v1/apps/authorized", None)?;
    if app_id_in_authorized_apps(&response.data, app_id) {
        return Ok(());
    }

    Err(CliError::invalid_input(
        &format!(
            "consumer-token profile is not authorized for app `{app_id}`; check the token's authorized apps or use user-token mode"
        ),
        openapi.context.output,
    ))
}

fn app_id_in_authorized_apps(data: &Value, app_id: &str) -> bool {
    data.as_array()
        .map(|items| items.as_slice())
        .or_else(|| {
            data.get("content")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
        })
        .unwrap_or_default()
        .iter()
        .any(|app| {
            app.get("appId")
                .and_then(Value::as_str)
                .is_some_and(|authorized_app_id| authorized_app_id == app_id)
        })
}

fn redact_config_item_values(mut data: Value) -> Value {
    redact_item_values_in_list(&mut data);
    data
}

fn redact_nested_item_values(mut data: Value) -> Value {
    redact_nested_item_values_in_value(&mut data);
    data
}

fn redact_release_configurations(mut data: Value) -> Value {
    redact_release_configurations_in_value(&mut data);
    data
}

fn filter_value_array<F>(mut data: Value, predicate: F) -> Value
where
    F: Fn(&Value) -> bool,
{
    match &mut data {
        Value::Array(items) => items.retain(predicate),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get_mut("content") {
                items.retain(predicate);
            }
        }
        _ => {}
    }
    data
}

fn redact_item_values_in_list(data: &mut Value) {
    match data {
        Value::Array(items) => redact_item_values(items),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get_mut("content") {
                redact_item_values(items);
            } else {
                redact_item_value_fields(data);
            }
        }
        _ => {}
    }
}

fn redact_nested_item_values_in_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                redact_nested_item_values_in_value(item);
            }
        }
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get_mut("items") {
                redact_item_values(items);
            }
            for value in map.values_mut() {
                redact_nested_item_values_in_value(value);
            }
        }
        _ => {}
    }
}

fn redact_item_values(items: &mut [Value]) {
    for item in items {
        redact_item_value_fields(item);
    }
}

fn redact_item_value_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in ["value", "oldValue", "newValue"] {
                if map.contains_key(key) {
                    map.insert(key.to_owned(), Value::String("[REDACTED]".to_owned()));
                }
            }
            for value in map.values_mut() {
                redact_item_value_fields(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_item_value_fields(value);
            }
        }
        _ => {}
    }
}

fn redact_release_configurations_in_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("configurations") {
                map.insert(
                    "configurations".to_owned(),
                    Value::String("[REDACTED]".to_owned()),
                );
            }
            for value in map.values_mut() {
                redact_release_configurations_in_value(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_release_configurations_in_value(value);
            }
        }
        _ => {}
    }
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

fn operator_for_mutation(
    command_operator: Option<&str>,
    context: &RuntimeContext,
    output: OutputFormat,
) -> Result<Option<String>, CliError> {
    if context.auth_mode.is_user_token() {
        if command_operator.is_some_and(|operator| !operator.trim().is_empty()) {
            return Err(CliError::invalid_input(
                "--operator is only used with consumer-token auth; user-token requests use the token owner as operator",
                output,
            ));
        }
        return Ok(None);
    }

    required_operator(command_operator, context, output).map(Some)
}

fn append_optional_query(path: String, key: &str, value: Option<&str>) -> String {
    match value {
        Some(value) => append_query(path, key, value),
        None => path,
    }
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

fn item_read_path(scope: &NamespaceScopeArgs, key: &str) -> String {
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

fn item_write_path(scope: &NamespaceScopeArgs, key: &str) -> String {
    format!(
        "{}/items/{}",
        namespace_path(scope),
        encode_path_segment(key)
    )
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
    existing_profile: Option<&ProfileConfig>,
    interactive: bool,
    output: OutputFormat,
) -> Result<String, CliError> {
    cli.global
        .server
        .clone()
        .and_then(non_blank)
        .or_else(|| std::env::var("APOLLO_SERVER").ok().and_then(non_blank))
        .or_else(|| {
            if options.overwrite {
                existing_profile.and_then(|profile| profile.server.clone().and_then(non_blank))
            } else {
                None
            }
        })
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
    auth_mode: AuthMode,
    existing_profile: Option<&ProfileConfig>,
    interactive: bool,
    output: OutputFormat,
) -> Result<Option<String>, CliError> {
    if let Some(operator) = options.operator.clone().and_then(non_blank) {
        if auth_mode.is_user_token() {
            return Err(CliError::invalid_input(
                "--operator is only used with consumer-token auth; user-token requests use the token owner as operator",
                output,
            ));
        }
        return Ok(Some(operator));
    }
    if auth_mode.is_user_token() {
        return Ok(None);
    }
    if options.overwrite
        && let Some(operator) =
            existing_profile.and_then(|profile| profile.operator.clone().and_then(non_blank))
    {
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

fn reject_auth_mode_change_without_token(
    explicit: Option<AuthMode>,
    token: Option<&Sensitive>,
    existing_profile: Option<&ProfileConfig>,
    output: OutputFormat,
) -> Result<(), CliError> {
    if token.is_some() {
        return Ok(());
    }
    let Some(explicit) = explicit else {
        return Ok(());
    };
    let Some(existing_profile) = existing_profile else {
        return Ok(());
    };
    let existing = existing_profile.resolved_auth_mode();
    if explicit != existing {
        return Err(CliError::invalid_input(
            "provide a new token when changing --auth-mode for a profile with an existing stored credential",
            output,
        ));
    }
    Ok(())
}

fn preserved_setup_credential(
    existing_profile: Option<&ProfileConfig>,
    token: Option<&Sensitive>,
) -> Option<CredentialRef> {
    let existing_profile = existing_profile?;
    if token.is_some() {
        return existing_profile.credential.clone();
    }
    existing_profile.credential.clone()
}

fn replaced_credential_to_delete(
    profile_name: &str,
    existing_profile: Option<&ProfileConfig>,
    replacement: &CredentialRef,
) -> Option<CredentialRef> {
    let existing_profile = existing_profile?;
    let previous = existing_profile
        .credential
        .clone()
        .unwrap_or_else(|| credential::implicit_native_ref(profile_name));
    (previous != *replacement).then_some(previous)
}

fn delete_replaced_credential(
    config_path: &std::path::Path,
    credential_ref: Option<CredentialRef>,
    output: OutputFormat,
) -> Result<(), CliError> {
    if let Some(credential_ref) = credential_ref {
        credential::delete(config_path, &credential_ref)
            .map_err(|error| CliError::credential_store_unavailable(&error, output))?;
    }
    Ok(())
}

fn resolve_auth_mode(
    explicit: Option<AuthMode>,
    token: Option<&Sensitive>,
    existing: Option<AuthMode>,
    default: AuthMode,
    output: OutputFormat,
) -> Result<AuthMode, CliError> {
    if let Some(auth_mode) = explicit {
        validate_auth_mode_for_token(auth_mode, token, output)?;
        return Ok(auth_mode);
    }

    if let Some(token) = token {
        return Ok(AuthMode::from_token_value(token.expose_secret()));
    }

    Ok(existing.unwrap_or(default))
}

fn validate_auth_mode_for_token(
    auth_mode: AuthMode,
    token: Option<&Sensitive>,
    output: OutputFormat,
) -> Result<(), CliError> {
    let Some(token) = token else {
        return Ok(());
    };
    let token_is_user_token = token.expose_secret().trim().starts_with(USER_TOKEN_PREFIX);
    match (auth_mode, token_is_user_token) {
        (AuthMode::UserToken, false) => Err(CliError::invalid_input(
            "user-token auth mode requires a Portal user access token starting with apollo_pat_",
            output,
        )),
        (AuthMode::ConsumerToken, true) => Err(CliError::invalid_input(
            "consumer-token auth mode cannot use a Portal user access token starting with apollo_pat_",
            output,
        )),
        _ => Ok(()),
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
    if interactive && prompt_yes_no("Store an Apollo token now?", false, output)? {
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
            let auth_mode = profile.auth_mode.as_deref().unwrap_or("consumer-token");
            lines.push(format!(
                "{} {}  {}  {}  {}",
                marker, profile.name, profile.server, output, auth_mode
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
    #[serde(rename = "authMode")]
    auth_mode: Option<String>,
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
            auth_mode: Some(profile.resolved_auth_mode().to_string()),
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
        lines.push(format!("Auth mode: {}", self.context.auth_mode));
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
    #[serde(rename = "authMode")]
    auth_mode: String,
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
            auth_mode: context.auth_mode.to_string(),
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
    #[serde(rename = "authMode")]
    auth_mode: String,
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
            format!("Auth mode: {}", self.auth_mode),
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
                "Run `apollo auth login --token-stdin` after creating a Portal user access token, or use `--auth-mode consumer-token` for legacy consumer tokens.".to_owned(),
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
    #[serde(rename = "authMode")]
    auth_mode: Option<String>,
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
        let mut lines = vec![format!(
            "Profile: {}\nStatus: {}\nSource: {}",
            self.profile.as_deref().unwrap_or("<none>"),
            state,
            self.source
        )];
        if let Some(auth_mode) = &self.auth_mode {
            lines.push(format!("Auth mode: {}", auth_mode));
        }
        lines.join("\n")
    }
}

#[derive(Serialize)]
struct AuthLoginResponse {
    stored: bool,
    profile: String,
    #[serde(rename = "authMode")]
    auth_mode: String,
    backend: String,
    key: String,
}

impl AuthLoginResponse {
    fn render_table(&self) -> String {
        format!(
            "Credential stored for profile '{}'.\nAuth mode: {}\nBackend: {}\nKey: {}",
            self.profile, self.auth_mode, self.backend, self.key
        )
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::cli::OutputFormat;
    use crate::config::{CredentialRef, ProfileConfig};

    #[test]
    fn read_prompt_line_reports_eof_as_aborted_input() {
        let mut reader = Cursor::new(Vec::<u8>::new());
        let error =
            super::read_prompt_line(&mut reader, OutputFormat::Json).expect_err("EOF should fail");

        assert_eq!(error.exit_code(), 1);
    }

    #[test]
    fn replaced_credential_to_delete_uses_implicit_native_for_legacy_profiles() {
        let existing = ProfileConfig {
            server: Some("https://apollo.example.com".to_owned()),
            ..ProfileConfig::default()
        };
        let replacement = CredentialRef {
            backend: "file".to_owned(),
            key: "dev".to_owned(),
        };

        assert_eq!(
            super::replaced_credential_to_delete("dev", Some(&existing), &replacement),
            Some(CredentialRef {
                backend: "native".to_owned(),
                key: "dev".to_owned(),
            })
        );
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
