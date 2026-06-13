use assert_cmd::Command;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn help_lists_v0_command_groups_and_global_flags() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("profile"))
        .stdout(predicate::str::contains("app"))
        .stdout(predicate::str::contains("env"))
        .stdout(predicate::str::contains("namespace"))
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("release"))
        .stdout(predicate::str::contains("api"))
        .stdout(predicate::str::contains("--profile"))
        .stdout(predicate::str::contains("--server"))
        .stdout(predicate::str::contains("--output"))
        .stdout(predicate::str::contains("--yes"));
}

#[test]
fn openapi_command_without_token_returns_structured_json_error() {
    let assert = Command::cargo_bin("apollo")
        .expect("apollo binary")
        .env_remove("APOLLO_TOKEN")
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
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("logout"));
}

#[test]
fn profile_help_lists_add_command() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .args(["profile", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("use"));
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
