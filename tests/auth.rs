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
fn auth_status_reports_environment_token_without_profile() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "secret-from-env")
        .args([
            "--server",
            "https://apollo-dev.example.com",
            "--output",
            "json",
            "auth",
            "status",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], true);
    assert_eq!(json["source"], "env");
    assert_eq!(json["backend"], "env");
    assert_eq!(json["key"], "APOLLO_TOKEN");
    assert!(json["profile"].is_null());
    assert!(!stdout.contains("secret-from-env"));
}

#[test]
fn auth_status_reports_unauthenticated_without_profile_or_token() {
    let home = temp_home();

    let assert = base_command(&home)
        .args(["--output", "json", "auth", "status"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], false);
    assert_eq!(json["source"], "none");
    assert!(json["profile"].is_null());
    assert!(json["authMode"].is_null());
    assert!(json["backend"].is_null());
    assert!(json["key"].is_null());
}

#[test]
fn auth_status_reports_unauthenticated_with_stale_active_profile() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "missing"
"#,
    );

    let assert = base_command(&home)
        .args(["--output", "json", "auth", "status"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], false);
    assert_eq!(json["source"], "none");
    assert!(json["profile"].is_null());
    assert!(json["authMode"].is_null());
    assert!(json["backend"].is_null());
    assert!(json["key"].is_null());
}

#[test]
fn auth_status_reports_environment_token_without_config_home() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "secret-from-env")
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .args(["--output", "json", "auth", "status"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], true);
    assert_eq!(json["source"], "env");
    assert_eq!(json["backend"], "env");
    assert_eq!(json["key"], "APOLLO_TOKEN");
    assert!(json["profile"].is_null());
    assert!(!stdout.contains("secret-from-env"));
}

#[test]
fn auth_status_reports_environment_token_with_stale_active_profile() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "missing"
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
    assert_eq!(json["profile"], "missing");
    assert!(!stdout.contains("secret-from-env"));
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

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "confirmation_required");
    assert!(!stderr.contains("secret-from-stdin"));
    assert!(!credential_file_path(&home, "dev").exists());
}

#[test]
fn auth_login_does_not_persist_apollo_token_from_environment() {
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
        .env("APOLLO_TOKEN", "secret-from-env")
        .args(["--output", "json", "auth", "login", "--store-token-in-file"])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("APOLLO_TOKEN is read-only"));
    assert!(!stderr.contains("secret-from-env"));
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
fn auth_login_auto_detects_user_token_mode_from_prefix() {
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
        .write_stdin("apollo_pat_test_token\n")
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
    assert_eq!(json["authMode"], "user-token");
    assert!(!stdout.contains("apollo_pat_test_token"));

    let config = fs::read_to_string(config_path(&home)).expect("config");
    assert!(config.contains("auth_mode = \"user-token\""));
    assert!(config.contains("backend = \"file\""));
    assert!(!config.contains("apollo_pat_test_token"));
}

#[test]
fn auth_login_deletes_replaced_file_credential() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"

[profiles.dev.credential]
backend = "file"
key = "old-dev"
"#,
    );
    fs::create_dir_all(
        credential_file_path(&home, "old-dev")
            .parent()
            .expect("parent"),
    )
    .expect("credential dir");
    fs::write(credential_file_path(&home, "old-dev"), "old-secret\n").expect("old credential");

    base_command(&home)
        .write_stdin("apollo_pat_test_token\n")
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

    assert!(!credential_file_path(&home, "old-dev").exists());
    assert!(credential_file_path(&home, "dev").exists());
}

#[cfg(unix)]
#[test]
fn auth_login_keeps_replaced_file_credential_when_config_save_fails() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"

[profiles.dev.credential]
backend = "file"
key = "old-dev"
"#,
    );
    fs::create_dir_all(
        credential_file_path(&home, "old-dev")
            .parent()
            .expect("parent"),
    )
    .expect("credential dir");
    fs::write(credential_file_path(&home, "old-dev"), "old-secret\n").expect("old credential");
    fs::set_permissions(config_path(&home), fs::Permissions::from_mode(0o400))
        .expect("make config read-only");

    let assert = base_command(&home)
        .write_stdin("apollo_pat_test_token\n")
        .args([
            "--output",
            "json",
            "auth",
            "login",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    assert_eq!(
        fs::read_to_string(credential_file_path(&home, "old-dev")).expect("old credential"),
        "old-secret\n"
    );
}

#[test]
fn auth_login_user_token_clears_existing_operator() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
auth_mode = "consumer-token"
operator = "apollo-bot"
"#,
    );

    let assert = base_command(&home)
        .write_stdin("apollo_pat_test_token\n")
        .args([
            "--output",
            "json",
            "auth",
            "login",
            "--auth-mode",
            "user-token",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["authMode"], "user-token");

    let config = fs::read_to_string(config_path(&home)).expect("config");
    assert!(config.contains("auth_mode = \"user-token\""));
    assert!(!config.contains("operator ="));
}

#[test]
fn auth_login_rejects_explicit_user_token_mode_with_consumer_token_value() {
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
        .write_stdin("consumer-token\n")
        .args([
            "--output",
            "json",
            "auth",
            "login",
            "--auth-mode",
            "user-token",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("apollo_pat_"));
    assert!(!stderr.contains("consumer-token"));
    assert!(!credential_file_path(&home, "dev").exists());
}

#[test]
fn auth_login_rejects_explicit_consumer_token_mode_with_user_token_value() {
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
        .write_stdin("apollo_pat_test_token\n")
        .args([
            "--output",
            "json",
            "auth",
            "login",
            "--auth-mode",
            "consumer-token",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("consumer-token"));
    assert!(!stderr.contains("apollo_pat_test_token"));
    assert!(!credential_file_path(&home, "dev").exists());
}

#[cfg(unix)]
#[test]
fn auth_login_tightens_existing_file_credential_permissions() {
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
    fs::write(credential_file_path(&home, "dev"), "old-secret\n").expect("credential file");
    fs::set_permissions(
        credential_file_path(&home, "dev"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("set permissive mode");

    base_command(&home)
        .write_stdin("new-secret\n")
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

    assert_eq!(
        fs::metadata(credential_file_path(&home, "dev"))
            .expect("credential metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(credential_file_path(&home, "dev")).expect("credential"),
        "new-secret"
    );
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
fn auth_status_treats_empty_file_credential_as_unauthenticated() {
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
    fs::write(credential_file_path(&home, "dev"), "  \n\t").expect("credential file");

    let assert = base_command(&home)
        .args(["--output", "json", "auth", "status"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["authenticated"], false);
    assert_eq!(json["source"], "file");
    assert_eq!(json["backend"], "file");
    assert_eq!(json["key"], "dev");
}

#[test]
fn auth_login_rejects_file_credential_keys_with_path_separators() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "../dev"

[profiles."../dev"]
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
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "credential_store_unavailable");
    assert!(!stderr.contains("secret-from-stdin"));
    assert!(
        !config_root(&home)
            .join("apollo")
            .join("credentials")
            .exists()
    );
}

#[test]
fn auth_login_rejects_missing_profile_before_storing_token() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "missing"
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
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "profile_not_found");
    assert!(!stderr.contains("secret-from-stdin"));
    assert!(
        !config_root(&home)
            .join("apollo")
            .join("credentials")
            .exists()
    );
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

#[test]
fn auth_logout_warns_when_apollo_token_environment_still_applies() {
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
        .env("APOLLO_TOKEN", "secret-from-env")
        .args(["--output", "json", "auth", "logout"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["loggedOut"], true);
    assert_eq!(json["environmentCredentialStillActive"], true);
    assert!(
        json["message"]
            .as_str()
            .expect("message")
            .contains("APOLLO_TOKEN")
    );
    assert!(!stdout.contains("secret-from-env"));
    assert!(!stdout.contains("secret-from-file"));
    assert!(!credential_file_path(&home, "dev").exists());
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

fn credential_file_path(home: &TempDir, key: &str) -> PathBuf {
    config_root(home)
        .join("apollo")
        .join("credentials")
        .join(format!("{}.token", key))
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
