use std::collections::{BTreeMap, HashMap};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::{AuthMode, Cli, OutputFormat};
use crate::error::CliError;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AppConfig {
    pub active_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProfileConfig {
    pub server: Option<String>,
    pub output: Option<OutputFormat>,
    pub auth_mode: Option<AuthMode>,
    pub operator: Option<String>,
    pub credential: Option<CredentialRef>,
}

impl ProfileConfig {
    pub fn resolved_auth_mode(&self) -> AuthMode {
        self.auth_mode.unwrap_or_default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CredentialRef {
    pub backend: String,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: AppConfig,
}

#[derive(Clone, Debug)]
pub struct RuntimeContext {
    pub profile: Option<String>,
    pub server: Option<String>,
    pub output: OutputFormat,
    pub auth_mode: AuthMode,
    pub operator: Option<String>,
    pub credential: Option<CredentialRef>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Platform {
    MacOs,
    Linux,
    Windows,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

pub fn load_config(format: OutputFormat) -> Result<LoadedConfig, CliError> {
    let path = config_path(format)?;
    if !path.exists() {
        return Ok(LoadedConfig {
            path,
            config: AppConfig::default(),
        });
    }

    let body = fs::read_to_string(&path)
        .map_err(|error| CliError::invalid_config(&path, &error.to_string(), format))?;
    let config: AppConfig = toml::from_str(&body)
        .map_err(|error| CliError::invalid_config(&path, &error.to_string(), format))?;

    Ok(LoadedConfig { path, config })
}

pub fn save_config(path: &Path, config: &AppConfig, format: OutputFormat) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| CliError::invalid_config(path, &error.to_string(), format))?;
    }

    let body = toml::to_string_pretty(config)
        .map_err(|error| CliError::invalid_config(path, &error.to_string(), format))?;
    fs::write(path, body)
        .map_err(|error| CliError::invalid_config(path, &error.to_string(), format))?;
    Ok(())
}

pub fn resolve_context(
    cli: &Cli,
    loaded: &LoadedConfig,
    format: OutputFormat,
) -> Result<RuntimeContext, CliError> {
    let (selected_profile, profile_config) = selected_profile_and_config(cli, loaded, format)?;
    let output = resolve_output(cli, loaded, format)?;

    let server = cli
        .global
        .server
        .clone()
        .and_then(non_blank)
        .or_else(|| env_var_non_blank("APOLLO_SERVER"))
        .or_else(|| {
            profile_config
                .as_ref()
                .and_then(|profile| profile.server.clone().and_then(non_blank))
        });
    let auth_mode = profile_config
        .as_ref()
        .map(ProfileConfig::resolved_auth_mode)
        .unwrap_or_default();
    let operator = profile_config
        .as_ref()
        .and_then(|profile| profile.operator.clone().and_then(non_blank));
    let credential = profile_config.and_then(|profile| profile.credential);

    Ok(RuntimeContext {
        profile: selected_profile,
        server,
        output,
        auth_mode,
        operator,
        credential,
    })
}

pub fn resolve_output(
    cli: &Cli,
    loaded: &LoadedConfig,
    format: OutputFormat,
) -> Result<OutputFormat, CliError> {
    let (_, profile_config) = selected_profile_and_config(cli, loaded, format)?;

    Ok(cli
        .global
        .output
        .or_else(read_env_output)
        .or_else(|| profile_config.as_ref().and_then(|profile| profile.output))
        .unwrap_or(OutputFormat::Table))
}

fn config_path(format: OutputFormat) -> Result<PathBuf, CliError> {
    let vars = collect_env_vars();
    config_path_for_platform(Platform::current(), &vars)
        .map_err(|message| CliError::missing_config_base(&message, format))
}

fn config_path_for_platform(
    platform: Platform,
    vars: &HashMap<String, OsString>,
) -> Result<PathBuf, String> {
    if let Some(cli_home) = vars
        .get("APOLLO_CLI_HOME")
        .filter(|value| !value.is_empty())
        .cloned()
        .map(PathBuf::from)
    {
        if !cli_home.is_absolute() {
            return Err("APOLLO_CLI_HOME must be an absolute path".to_owned());
        }
        return Ok(cli_home.join("config.toml"));
    }

    let home = || {
        vars.get("HOME")
            .filter(|value| !value.is_empty())
            .cloned()
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set".to_owned())
    };

    match platform {
        Platform::MacOs => Ok(home()?
            .join("Library")
            .join("Application Support")
            .join("apollo")
            .join("config.toml")),
        Platform::Linux => {
            if let Some(xdg_config_home) = vars
                .get("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .cloned()
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
            {
                Ok(xdg_config_home.join("apollo").join("config.toml"))
            } else {
                Ok(home()?.join(".config").join("apollo").join("config.toml"))
            }
        }
        Platform::Windows => vars
            .get("APPDATA")
            .filter(|value| !value.is_empty())
            .cloned()
            .map(PathBuf::from)
            .ok_or_else(|| "APPDATA is not set".to_owned())
            .map(|path| path.join("apollo").join("config.toml")),
    }
}

fn collect_env_vars() -> HashMap<String, OsString> {
    env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect()
}

pub(crate) fn read_env_output() -> Option<OutputFormat> {
    env::var("APOLLO_OUTPUT")
        .ok()
        .and_then(|value| OutputFormat::parse(&value))
}

fn selected_profile_and_config(
    cli: &Cli,
    loaded: &LoadedConfig,
    format: OutputFormat,
) -> Result<(Option<String>, Option<ProfileConfig>), CliError> {
    let selected_profile = cli
        .global
        .profile
        .clone()
        .and_then(non_blank)
        .or_else(|| env_var_non_blank("APOLLO_PROFILE"))
        .or_else(|| loaded.config.active_profile.clone().and_then(non_blank));

    let profile_config = match &selected_profile {
        Some(profile_name) => Some(
            loaded
                .config
                .profiles
                .get(profile_name)
                .cloned()
                .ok_or_else(|| CliError::profile_not_found(profile_name, format))?,
        ),
        None => None,
    };

    Ok((selected_profile, profile_config))
}

fn env_var_non_blank(name: &str) -> Option<String> {
    env::var(name).ok().and_then(non_blank)
}

fn non_blank(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

impl OutputFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "table" => Some(Self::Table),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Table => "table",
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{AppConfig, Platform, config_path_for_platform};

    #[test]
    fn config_path_uses_macos_convention() {
        let vars = HashMap::from([(String::from("HOME"), OsString::from("/Users/tester"))]);
        let path = config_path_for_platform(Platform::MacOs, &vars).expect("macos path");
        assert_eq!(
            path,
            PathBuf::from("/Users/tester/Library/Application Support/apollo/config.toml")
        );
    }

    #[test]
    fn apollo_cli_home_overrides_platform_config_path() {
        let vars = HashMap::from([
            (String::from("HOME"), OsString::from("/Users/tester")),
            (
                String::from("APOLLO_CLI_HOME"),
                OsString::from("/tmp/apollo-cli-smoke"),
            ),
        ]);

        for platform in [Platform::MacOs, Platform::Linux, Platform::Windows] {
            assert_eq!(
                config_path_for_platform(platform, &vars).unwrap(),
                PathBuf::from("/tmp/apollo-cli-smoke/config.toml")
            );
        }
    }

    #[test]
    fn apollo_cli_home_must_be_absolute() {
        let vars = HashMap::from([(
            String::from("APOLLO_CLI_HOME"),
            OsString::from("relative/apollo-cli"),
        )]);

        assert_eq!(
            config_path_for_platform(Platform::Linux, &vars).unwrap_err(),
            "APOLLO_CLI_HOME must be an absolute path"
        );
    }

    #[test]
    fn config_path_uses_linux_xdg_when_present() {
        let vars = HashMap::from([(
            String::from("XDG_CONFIG_HOME"),
            OsString::from("/tmp/config-home"),
        )]);
        let path = config_path_for_platform(Platform::Linux, &vars).expect("linux xdg path");
        assert_eq!(path, PathBuf::from("/tmp/config-home/apollo/config.toml"));
    }

    #[test]
    fn config_path_treats_empty_linux_xdg_as_unset() {
        let vars = HashMap::from([
            (String::from("HOME"), OsString::from("/home/tester")),
            (String::from("XDG_CONFIG_HOME"), OsString::from("")),
        ]);
        let path = config_path_for_platform(Platform::Linux, &vars).expect("linux xdg path");
        assert_eq!(
            path,
            PathBuf::from("/home/tester/.config/apollo/config.toml")
        );
    }

    #[test]
    fn config_path_treats_relative_linux_xdg_as_unset() {
        let vars = HashMap::from([
            (String::from("HOME"), OsString::from("/home/tester")),
            (String::from("XDG_CONFIG_HOME"), OsString::from(".config")),
        ]);
        let path = config_path_for_platform(Platform::Linux, &vars).expect("linux xdg path");
        assert_eq!(
            path,
            PathBuf::from("/home/tester/.config/apollo/config.toml")
        );
    }

    #[test]
    fn config_path_treats_empty_home_as_unset() {
        let vars = HashMap::from([(String::from("HOME"), OsString::from(""))]);
        let error = config_path_for_platform(Platform::MacOs, &vars).expect_err("missing home");
        assert_eq!(error, "HOME is not set");

        let vars = HashMap::from([
            (String::from("HOME"), OsString::from("")),
            (String::from("XDG_CONFIG_HOME"), OsString::from("")),
        ]);
        let error = config_path_for_platform(Platform::Linux, &vars).expect_err("missing home");
        assert_eq!(error, "HOME is not set");
    }

    #[test]
    fn config_path_uses_windows_appdata_convention() {
        let vars = HashMap::from([(
            String::from("APPDATA"),
            OsString::from(r"C:\Users\tester\AppData\Roaming"),
        )]);
        let path = config_path_for_platform(Platform::Windows, &vars).expect("windows path");
        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\tester\AppData\Roaming")
                .join("apollo")
                .join("config.toml")
        );
    }

    #[test]
    fn config_path_treats_empty_windows_appdata_as_unset() {
        let vars = HashMap::from([(String::from("APPDATA"), OsString::from(""))]);
        let error = config_path_for_platform(Platform::Windows, &vars).expect_err("appdata");
        assert_eq!(error, "APPDATA is not set");
    }

    #[test]
    fn profile_auth_mode_round_trips_as_kebab_case() {
        let config: AppConfig = toml::from_str(
            r#"
[profiles.dev]
server = "https://apollo.example.com"
auth_mode = "user-token"
"#,
        )
        .expect("parse config");

        let profile = config.profiles.get("dev").expect("profile");
        assert_eq!(profile.auth_mode.expect("auth mode").as_str(), "user-token");

        let body = toml::to_string_pretty(&config).expect("serialize config");
        assert!(body.contains("auth_mode = \"user-token\""));
    }

    #[test]
    fn missing_profile_auth_mode_resolves_as_consumer_token() {
        let config: AppConfig = toml::from_str(
            r#"
[profiles.dev]
server = "https://apollo.example.com"
"#,
        )
        .expect("parse config");

        let profile = config.profiles.get("dev").expect("profile");
        assert!(profile.auth_mode.is_none());
        assert_eq!(profile.resolved_auth_mode().as_str(), "consumer-token");
    }
}
