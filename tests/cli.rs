use assert_cmd::Command;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn help_lists_v0_command_groups_and_global_flags() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .arg("help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Configure the first local profile and token",
        ))
        .stdout(predicate::str::contains(
            "Log in, log out, inspect local auth, and check user tokens",
        ))
        .stdout(predicate::str::contains(
            "Manage named server/token/operator profiles",
        ))
        .stdout(predicate::str::contains("Read Apollo application metadata"))
        .stdout(predicate::str::contains(
            "List environments and clusters for an app",
        ))
        .stdout(predicate::str::contains(
            "List, inspect, and create namespaces",
        ))
        .stdout(predicate::str::contains(
            "Read, change, diff, and sync namespace items",
        ))
        .stdout(predicate::str::contains(
            "Create, list, and roll back releases",
        ))
        .stdout(predicate::str::contains(
            "Send a raw Apollo Portal OpenAPI request",
        ))
        .stdout(predicate::str::contains("Use a named Apollo CLI profile"))
        .stdout(predicate::str::contains(
            "Override the Apollo Portal base URL",
        ))
        .stdout(predicate::str::contains("Render output as json or table"))
        .stdout(predicate::str::contains(
            "Skip confirmation prompts for mutating OpenAPI requests",
        ));
}

#[test]
fn openapi_command_without_token_returns_structured_json_error() {
    let home = temp_home();
    let assert = base_command(&home)
        .args([
            "--server",
            "http://127.0.0.1:9",
            "--output",
            "json",
            "app",
            "list",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let output = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 output");
    let json: Value = serde_json::from_str(&output).expect("valid json output");

    assert_eq!(json["error"]["code"], "authentication_failed");
    assert_eq!(json["error"]["category"], "authentication_failed");
    assert_eq!(json["error"]["command"], "auth");
}

#[test]
fn global_flags_are_accepted_before_subcommands() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args([
            "--profile",
            "dev",
            "--server",
            "https://apollo.example.com",
            "--output",
            "table",
            "--yes",
            "config",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("config"));
}

#[test]
fn apollo_output_controls_early_config_errors() {
    let home = temp_home();
    write_config(&home, "active_profile = [not valid toml\n");

    let assert = base_command(&home)
        .env("APOLLO_OUTPUT", "json")
        .args(["profile", "list"])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "invalid_config");
}

#[test]
fn auth_help_lists_v0_auth_commands() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args(["auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Store a token for the active profile",
        ))
        .stdout(predicate::str::contains(
            "Show local auth state without contacting the server",
        ))
        .stdout(predicate::str::contains(
            "Verify the current user token and show its owner",
        ))
        .stdout(predicate::str::contains(
            "List server capabilities for the current user token",
        ))
        .stdout(predicate::str::contains(
            "Remove the stored token for the active profile",
        ));
}

#[test]
fn setup_commands_accept_auth_mode_flag() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--auth-mode"));

    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args(["profile", "add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--auth-mode"));

    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args(["auth", "login", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--auth-mode"))
        .stdout(predicate::str::contains("user-token"))
        .stdout(predicate::str::contains("consumer-token"));
}

#[test]
fn profile_help_lists_add_command() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args(["profile", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Add or update a named profile"))
        .stdout(predicate::str::contains("List configured profiles"))
        .stdout(predicate::str::contains("Show the active profile"))
        .stdout(predicate::str::contains("Set the active profile"));
}

fn base_command(home: &TempDir) -> Command {
    let mut command = Command::cargo_bin("apollo").expect("apollo binary");
    command.env_remove("APOLLO_PROFILE");
    command.env_remove("APOLLO_SERVER");
    command.env_remove("APOLLO_OUTPUT");
    command.env_remove("APOLLO_TOKEN");
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

fn config_path(home: &TempDir) -> std::path::PathBuf {
    config_root(home).join("apollo").join("config.toml")
}

fn config_root(home: &TempDir) -> std::path::PathBuf {
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
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config dir");
    std::fs::write(config_path, body).expect("write config");
}
