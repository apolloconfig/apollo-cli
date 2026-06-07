use assert_cmd::Command;
use predicates::prelude::predicate;
use serde_json::Value;

#[test]
fn help_lists_v0_command_groups_and_global_flags() {
    Command::cargo_bin("apollo")
        .expect("apollo binary")
        .arg("--help")
        .assert()
        .success()
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

    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 output");
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
