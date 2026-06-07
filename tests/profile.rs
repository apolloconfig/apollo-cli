use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn profile_show_uses_active_profile_config_and_redacts_unknown_token_fields() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
operator = "dev-operator"
token = "super-secret-token"

[profiles.dev.credential]
backend = "keychain"
key = "apollo/dev"
"#,
    );

    let assert = base_command(&home)
        .args(["--output", "json", "profile", "show"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["context"]["profile"], "dev");
    assert_eq!(json["context"]["server"], "https://apollo-dev.example.com");
    assert_eq!(json["context"]["operator"], "dev-operator");
    assert_eq!(json["context"]["credential"]["backend"], "keychain");
    assert_eq!(json["context"]["credential"]["key"], "apollo/dev");
    assert!(!stdout.contains("super-secret-token"));
}

#[test]
fn profile_show_prefers_env_over_active_profile_config() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .env("APOLLO_SERVER", "https://apollo-env.example.com")
        .args(["--output", "json", "profile", "show"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["context"]["server"], "https://apollo-env.example.com");
}

#[test]
fn profile_show_uses_apollo_output_for_json_rendering() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .env("APOLLO_OUTPUT", "json")
        .args(["profile", "show"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["context"]["profile"], "dev");
    assert_eq!(json["context"]["output"], "json");
}

#[test]
fn profile_show_prefers_flags_over_environment() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"

[profiles.prod]
server = "https://apollo-prod.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .env("APOLLO_PROFILE", "dev")
        .env("APOLLO_SERVER", "https://apollo-env.example.com")
        .args([
            "--profile",
            "prod",
            "--server",
            "https://apollo-flag.example.com",
            "--output",
            "json",
            "profile",
            "show",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["context"]["profile"], "prod");
    assert_eq!(json["context"]["server"], "https://apollo-flag.example.com");
}

#[test]
fn profile_list_returns_profiles_with_active_marker() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"

[profiles.prod]
server = "https://apollo-prod.example.com"
output = "json"
"#,
    );

    let assert = base_command(&home)
        .args(["--output", "json", "profile", "list"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    let profiles = json["profiles"].as_array().expect("profiles array");

    assert_eq!(profiles.len(), 2);
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "dev" && profile["active"] == true)
    );
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "prod" && profile["active"] == false)
    );
}

#[test]
fn profile_use_updates_active_profile() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"

[profiles.prod]
server = "https://apollo-prod.example.com"
output = "json"
"#,
    );

    base_command(&home)
        .args(["profile", "use", "prod"])
        .assert()
        .success();

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("active_profile = \"prod\""));
}

#[test]
fn profile_use_returns_structured_error_for_unknown_profile() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .args(["--output", "json", "profile", "use", "prod"])
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["error"]["code"], "profile_not_found");
    assert_eq!(json["error"]["profile"], "prod");
}

fn base_command(home: &TempDir) -> Command {
    let mut command = Command::cargo_bin("apollo").expect("apollo binary");
    command.env_remove("APOLLO_PROFILE");
    command.env_remove("APOLLO_SERVER");
    command.env_remove("APOLLO_OUTPUT");
    command.env_remove("XDG_CONFIG_HOME");
    command.env_remove("APPDATA");
    command.env("HOME", home.path());
    if cfg!(target_os = "linux") {
        command.env("XDG_CONFIG_HOME", home.path().join(".config"));
    }
    if cfg!(target_os = "windows") {
        command.env("APPDATA", home.path().join("AppData").join("Roaming"));
    }
    command
}

fn temp_home() -> TempDir {
    tempfile::tempdir().expect("temp home")
}

fn config_path(home: &TempDir) -> PathBuf {
    config_root(home).join("apollo").join("config.toml")
}

fn config_root(home: &TempDir) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.path().join("Library").join("Application Support")
    } else if cfg!(target_os = "windows") {
        home.path().join("AppData").join("Roaming")
    } else {
        home.path().join(".config")
    }
}

fn write_config(home: &TempDir, body: &str) {
    let config_path = config_path(home);
    fs::create_dir_all(
        config_path
            .parent()
            .expect("config path parent should exist"),
    )
    .expect("create config dir");
    fs::write(config_path, normalize_toml(body)).expect("write config");
}

fn normalize_toml(body: &str) -> String {
    body.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
        + "\n"
}
