use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn auth_status_uses_apollo_token_without_writing_credentials() {
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
        .env("APOLLO_TOKEN", "secret-from-env")
        .args(["--output", "json", "auth", "status"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], true);
    assert_eq!(json["source"], "env");
    assert_eq!(json["profile"], "dev");
    assert!(!stdout.contains("secret-from-env"));
    assert!(!credential_file_path(&home, "dev").exists());
}

#[test]
fn auth_login_file_fallback_requires_explicit_opt_in() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
"#,
    );

    let assert = base_command(&home)
        .write_stdin("secret-from-stdin\n")
        .args(["--output", "json", "auth", "login", "--token-stdin"])
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["error"]["code"], "confirmation_required");
    assert!(!stdout.contains("secret-from-stdin"));
    assert!(!credential_file_path(&home, "dev").exists());
}

#[test]
fn auth_login_can_store_file_fallback_token_without_printing_it() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
"#,
    );

    let assert = base_command(&home)
        .write_stdin("secret-from-stdin\n")
        .args([
            "--output",
            "json",
            "auth",
            "login",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["stored"], true);
    assert_eq!(json["backend"], "file");
    assert_eq!(json["profile"], "dev");
    assert!(credential_file_path(&home, "dev").exists());
    assert!(!stdout.contains("secret-from-stdin"));

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(credential_file_path(&home, "dev"))
            .expect("credential metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let config = fs::read_to_string(config_path(&home)).expect("config");
    assert!(config.contains("backend = \"file\""));
    assert!(!config.contains("secret-from-stdin"));
}

#[test]
fn auth_status_detects_file_fallback_without_printing_token() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"

[profiles.dev.credential]
backend = "file"
key = "dev"
"#,
    );
    fs::create_dir_all(credential_file_path(&home, "dev").parent().expect("parent"))
        .expect("credential dir");
    fs::write(credential_file_path(&home, "dev"), "secret-from-file\n").expect("credential file");

    let assert = base_command(&home)
        .args(["--output", "json", "auth", "status"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], true);
    assert_eq!(json["source"], "file");
    assert!(!stdout.contains("secret-from-file"));
}

#[test]
fn auth_logout_removes_file_credential_and_keeps_profile_config() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"

[profiles.dev.credential]
backend = "file"
key = "dev"
"#,
    );
    fs::create_dir_all(credential_file_path(&home, "dev").parent().expect("parent"))
        .expect("credential dir");
    fs::write(credential_file_path(&home, "dev"), "secret-from-file\n").expect("credential file");

    base_command(&home)
        .args(["auth", "logout"])
        .assert()
        .success();

    assert!(!credential_file_path(&home, "dev").exists());
    let config = fs::read_to_string(config_path(&home)).expect("config");
    assert!(config.contains("[profiles.dev]"));
    assert!(!config.contains("secret-from-file"));
}

fn base_command(home: &TempDir) -> Command {
    let mut command = Command::cargo_bin("apollo").expect("apollo binary");
    command.env_remove("APOLLO_PROFILE");
    command.env_remove("APOLLO_SERVER");
    command.env_remove("APOLLO_OUTPUT");
    command.env_remove("APOLLO_TOKEN");
    command.env_remove("XDG_CONFIG_HOME");
    command.env_remove("APPDATA");
    command.env("APOLLO_CLI_TEST_DISABLE_NATIVE", "1");
    command.env("HOME", home.path());
    command
}

fn temp_home() -> TempDir {
    tempfile::tempdir().expect("temp home")
}

fn config_path(home: &TempDir) -> PathBuf {
    home.path()
        .join("Library")
        .join("Application Support")
        .join("apollo")
        .join("config.toml")
}

fn credential_file_path(home: &TempDir, key: &str) -> PathBuf {
    home.path()
        .join("Library")
        .join("Application Support")
        .join("apollo")
        .join("credentials")
        .join(format!("{}.token", key))
}

fn write_config(home: &TempDir, body: &str) {
    let config_path = config_path(home);
    fs::create_dir_all(config_path.parent().expect("config parent")).expect("create config dir");
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
