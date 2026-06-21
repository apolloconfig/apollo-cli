use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::predicate;
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
fn init_uses_apollo_server_environment_override() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_SERVER", "https://apollo-env.example.com")
        .args(["--output", "json", "init"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["server"], "https://apollo-env.example.com");
    let config = fs::read_to_string(config_path(&home)).expect("config");
    assert!(config.contains("server = \"https://apollo-env.example.com\""));
}

#[test]
fn profile_add_uses_apollo_server_environment_override() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_SERVER", "https://apollo-env.example.com")
        .args(["--output", "json", "profile", "add", "dev"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["server"], "https://apollo-env.example.com");
    let config = fs::read_to_string(config_path(&home)).expect("config");
    assert!(config.contains("server = \"https://apollo-env.example.com\""));
}

#[test]
fn profile_show_ignores_blank_environment_overrides() {
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
        .env("APOLLO_PROFILE", "")
        .env("APOLLO_SERVER", "   ")
        .args(["--output", "json", "profile", "show"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["context"]["profile"], "dev");
    assert_eq!(json["context"]["server"], "https://apollo-dev.example.com");
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
fn profile_list_uses_active_profile_output_for_rendering() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "json"
"#,
    );

    let assert = base_command(&home)
        .args(["profile", "list"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["activeProfile"], "dev");
    assert_eq!(json["profiles"][0]["name"], "dev");
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
fn profile_list_recovers_when_active_profile_is_stale() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "deleted"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .args(["--output", "json", "profile", "list"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["activeProfile"], "deleted");
    assert_eq!(json["profiles"][0]["name"], "dev");
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
fn profile_use_honors_active_profile_output_config() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "json"

[profiles.prod]
server = "https://apollo-prod.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .args(["profile", "use", "prod"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["activeProfile"], "prod");
}

#[test]
fn profile_use_recovers_when_active_profile_is_stale() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "deleted"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "table"
"#,
    );

    base_command(&home)
        .args(["profile", "use", "dev"])
        .assert()
        .success();

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("active_profile = \"dev\""));
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

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "profile_not_found");
    assert_eq!(json["error"]["profile"], "prod");
}

#[test]
fn init_creates_local_profile_and_file_credential_without_printing_token() {
    let home = temp_home();

    let assert = base_command(&home)
        .write_stdin("secret-from-stdin\n")
        .args([
            "--output",
            "json",
            "init",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["profile"], "local");
    assert_eq!(json["activeProfile"], "local");
    assert_eq!(json["server"], "http://127.0.0.1:8070");
    assert_eq!(json["credential"]["backend"], "file");
    assert!(!stdout.contains("secret-from-stdin"));

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("active_profile = \"local\""));
    assert!(config.contains("[profiles.local]"));
    assert!(config.contains("server = \"http://127.0.0.1:8070\""));
    assert!(config.contains("output = \"json\""));
    assert!(config.contains("operator = \"apollo\""));
    assert!(config.contains("backend = \"file\""));
    assert!(!config.contains("secret-from-stdin"));
    assert!(credential_file_path(&home, "local").exists());
}

#[test]
fn init_defaults_to_user_token_auth_without_operator_when_no_token_is_provided() {
    let home = temp_home();

    let assert = base_command(&home)
        .args(["--output", "json", "init"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["profile"], "local");
    assert_eq!(json["authMode"], "user-token");
    assert!(json["operator"].is_null());
    assert!(json["credential"].is_null());

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("auth_mode = \"user-token\""));
    assert!(!config.contains("operator ="));
}

#[test]
fn profile_add_explicit_consumer_token_mode_keeps_operator() {
    let home = temp_home();

    let assert = base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "https://apollo-dev.example.com",
            "profile",
            "add",
            "dev",
            "--auth-mode",
            "consumer-token",
            "--operator",
            "apollo-bot",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["profile"], "dev");
    assert_eq!(json["authMode"], "consumer-token");
    assert_eq!(json["operator"], "apollo-bot");

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("auth_mode = \"consumer-token\""));
    assert!(config.contains("operator = \"apollo-bot\""));
}

#[test]
fn init_with_apollo_output_json_does_not_persist_implicit_output() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_OUTPUT", "json")
        .write_stdin("secret-from-stdin\n")
        .args(["init", "--token-stdin", "--store-token-in-file"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["activeProfile"], "local");
    assert_eq!(json["output"], "table");

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(!config.contains("output = \"json\""));
}

#[test]
fn init_ignores_blank_setup_values_and_uses_defaults() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_OUTPUT", "json")
        .write_stdin("secret-from-stdin\n")
        .args([
            "--profile",
            "",
            "--server",
            "",
            "init",
            "--name",
            "",
            "--operator",
            "",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["profile"], "local");
    assert_eq!(json["server"], "http://127.0.0.1:8070");
    assert_eq!(json["operator"], "apollo");
    assert_eq!(json["output"], "table");

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("[profiles.local]"));
    assert!(config.contains("server = \"http://127.0.0.1:8070\""));
    assert!(config.contains("operator = \"apollo\""));
    assert!(!config.contains("[profiles.\"\"]"));
    assert!(!config.contains("output = \"json\""));
}

#[test]
fn profile_add_trims_setup_values_before_persisting() {
    let home = temp_home();

    let assert = base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "  https://apollo-dev.example.com/  ",
            "profile",
            "add",
            "  dev  ",
            "--auth-mode",
            "consumer-token",
            "--operator",
            "  alice  ",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["profile"], "dev");
    assert_eq!(json["server"], "https://apollo-dev.example.com/");
    assert_eq!(json["operator"], "alice");

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("[profiles.dev]"));
    assert!(config.contains("server = \"https://apollo-dev.example.com/\""));
    assert!(config.contains("operator = \"alice\""));
    assert!(!config.contains("  dev  "));
    assert!(!config.contains("  alice  "));
}

#[test]
fn profile_add_creates_profile_without_switching_active_profile() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "local"

[profiles.local]
server = "http://127.0.0.1:8070"
output = "json"
"#,
    );

    let assert = base_command(&home)
        .write_stdin("secret-from-stdin\n")
        .args([
            "--server",
            "https://apollo-dev.example.com",
            "--output",
            "json",
            "profile",
            "add",
            "dev",
            "--operator",
            "dev-operator",
            "--token-stdin",
            "--store-token-in-file",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["profile"], "dev");
    assert_eq!(json["activeProfile"], "local");
    assert_eq!(json["server"], "https://apollo-dev.example.com");
    assert_eq!(json["operator"], "dev-operator");
    assert_eq!(json["credential"]["backend"], "file");
    assert!(!stdout.contains("secret-from-stdin"));

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("active_profile = \"local\""));
    assert!(config.contains("[profiles.local]"));
    assert!(config.contains("[profiles.dev]"));
    assert!(config.contains("server = \"https://apollo-dev.example.com\""));
    assert!(config.contains("operator = \"dev-operator\""));
    assert!(!config.contains("secret-from-stdin"));
    assert!(credential_file_path(&home, "dev").exists());
}

#[test]
fn profile_add_honors_active_profile_output_config() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "local"

[profiles.local]
server = "http://127.0.0.1:8070"
output = "json"
"#,
    );

    let assert = base_command(&home)
        .args([
            "--server",
            "https://apollo-prod.example.com",
            "profile",
            "add",
            "prod",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["profile"], "prod");
    assert_eq!(json["activeProfile"], "local");
}

#[test]
fn profile_add_with_use_sets_active_profile() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "local"

[profiles.local]
server = "http://127.0.0.1:8070"
output = "json"
"#,
    );

    base_command(&home)
        .args([
            "--server",
            "https://apollo-prod.example.com",
            "profile",
            "add",
            "prod",
            "--use",
        ])
        .assert()
        .success();

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("active_profile = \"prod\""));
    assert!(config.contains("[profiles.prod]"));
}

#[test]
fn profile_add_overwrite_preserves_existing_credential_without_new_token() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-old.example.com"
output = "table"
operator = "apollo-bot"

[profiles.dev.credential]
backend = "file"
key = "dev"
"#,
    );
    fs::create_dir_all(credential_file_path(&home, "dev").parent().expect("parent"))
        .expect("credential dir");
    fs::write(credential_file_path(&home, "dev"), "secret-from-file\n").expect("credential file");

    let assert = base_command(&home)
        .args([
            "--server",
            "https://apollo-new.example.com",
            "profile",
            "add",
            "dev",
            "--overwrite",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("Auth mode: consumer-token"));
    assert!(stdout.contains("Operator: apollo-bot"));
    assert!(stdout.contains("Credential backend: file"));
    assert!(stdout.contains("Credential key: dev"));

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("server = \"https://apollo-new.example.com\""));
    assert!(config.contains("auth_mode = \"consumer-token\""));
    assert!(config.contains("operator = \"apollo-bot\""));
    assert!(config.contains("backend = \"file\""));
    assert!(config.contains("key = \"dev\""));
    assert!(credential_file_path(&home, "dev").exists());
}

#[test]
fn profile_add_overwrite_preserves_existing_output_without_global_output() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-old.example.com"
output = "json"

[profiles.dev.credential]
backend = "file"
key = "dev"
"#,
    );
    fs::create_dir_all(credential_file_path(&home, "dev").parent().expect("parent"))
        .expect("credential dir");
    fs::write(credential_file_path(&home, "dev"), "apollo_pat_secret\n").expect("credential file");

    let assert = base_command(&home)
        .args([
            "--server",
            "https://apollo-new.example.com",
            "profile",
            "add",
            "dev",
            "--overwrite",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["output"], "json");
    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("server = \"https://apollo-new.example.com\""));
    assert!(config.contains("output = \"json\""));
}

#[test]
fn profile_add_overwrite_disables_implicit_native_credential_without_new_token() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-old.example.com"
auth_mode = "consumer-token"
operator = "apollo-bot"
"#,
    );

    let assert = base_command(&home)
        .args([
            "--server",
            "https://apollo-new.example.com",
            "profile",
            "add",
            "dev",
            "--overwrite",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("Credential backend: none"));

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("server = \"https://apollo-new.example.com\""));
    assert!(config.contains("backend = \"none\""));
    assert!(config.contains("key = \"dev\""));
}

#[test]
fn profile_add_sets_new_profile_active_when_active_profile_is_stale() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "deleted"

[profiles.other]
server = "https://apollo-other.example.com"
output = "table"
"#,
    );

    let assert = base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "https://apollo-dev.example.com",
            "profile",
            "add",
            "dev",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["activeProfile"], "dev");
    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("active_profile = \"dev\""));
}

#[test]
fn profile_add_overwrite_rejects_auth_mode_change_without_new_token() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-old.example.com"
auth_mode = "consumer-token"
operator = "apollo-bot"

[profiles.dev.credential]
backend = "file"
key = "dev"
"#,
    );
    fs::create_dir_all(credential_file_path(&home, "dev").parent().expect("parent"))
        .expect("credential dir");
    fs::write(credential_file_path(&home, "dev"), "consumer-token\n").expect("credential file");

    let assert = base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "https://apollo-new.example.com",
            "profile",
            "add",
            "dev",
            "--overwrite",
            "--auth-mode",
            "user-token",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("--auth-mode"));
    assert!(stderr.contains("token"));

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("auth_mode = \"consumer-token\""));
    assert!(config.contains("operator = \"apollo-bot\""));
}

#[test]
fn profile_add_overwrite_rejects_auth_mode_change_for_implicit_native_credential() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-old.example.com"
auth_mode = "consumer-token"
operator = "apollo-bot"
"#,
    );

    let assert = base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "https://apollo-new.example.com",
            "profile",
            "add",
            "dev",
            "--overwrite",
            "--auth-mode",
            "user-token",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("--auth-mode"));
    assert!(stderr.contains("token"));

    let config = fs::read_to_string(config_path(&home)).expect("config file");
    assert!(config.contains("auth_mode = \"consumer-token\""));
    assert!(!config.contains("auth_mode = \"user-token\""));
}

#[test]
fn profile_add_refuses_existing_profile_without_overwrite() {
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "dev"

[profiles.dev]
server = "https://apollo-dev.example.com"
output = "json"
"#,
    );

    base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "https://apollo-new.example.com",
            "profile",
            "add",
            "dev",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("profile_already_exists"));
}

#[test]
fn profile_add_rejects_blank_profile_name_non_interactively() {
    let home = temp_home();

    let assert = base_command(&home)
        .args([
            "--output",
            "json",
            "--server",
            "https://apollo-dev.example.com",
            "profile",
            "add",
            "",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("provide a profile name"));
}

#[test]
fn profile_add_rejects_blank_server_non_interactively() {
    let home = temp_home();

    let assert = base_command(&home)
        .args(["--output", "json", "--server", "", "profile", "add", "dev"])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("provide a server"));
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
