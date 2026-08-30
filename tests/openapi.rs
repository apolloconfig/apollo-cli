use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::predicate;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn api_get_calls_openapi_with_consumer_token() {
    let server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "api",
            "get",
            "/openapi/v1/apps",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"][0]["appId"], "demo");
    assert!(!stdout.contains("consumer-token"));

    let request = server.request();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/openapi/v1/apps");
    assert!(
        request
            .headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("authorization: consumer-token"))
    );
}

#[test]
fn api_get_calls_openapi_with_user_token_bearer_header_from_env() {
    let server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "api",
            "get",
            "/openapi/v1/apps",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"][0]["appId"], "demo");
    assert!(!stdout.contains("apollo_pat_test_token"));

    let request = server.request();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/openapi/v1/apps");
    assert!(
        request.headers.iter().any(
            |header| header.eq_ignore_ascii_case("authorization: Bearer apollo_pat_test_token")
        )
    );
}

#[test]
fn api_passthrough_rejects_dot_segment_openapi_paths() {
    let home = temp_home();

    for path in ["/openapi/v1/../apps", "/openapi/v1/%2e%2e/apps"] {
        let assert = base_command(&home)
            .env("APOLLO_TOKEN", "consumer-token")
            .args([
                "--server",
                "http://127.0.0.1:9",
                "--output",
                "json",
                "api",
                "get",
                path,
            ])
            .assert()
            .failure();

        assert!(assert.get_output().stdout.is_empty());
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
        let json: Value = serde_json::from_str(&stderr).expect("json stderr");
        assert_eq!(json["error"]["code"], "invalid_input");
        assert!(stderr.contains("must not contain . or .. path segments"));
    }
}

#[test]
fn api_passthrough_rejects_backslash_path_separators() {
    let home = temp_home();

    for path in ["/openapi/v1/..\\apps", "/openapi/v1/%5capps"] {
        let assert = base_command(&home)
            .env("APOLLO_TOKEN", "consumer-token")
            .args([
                "--server",
                "http://127.0.0.1:9",
                "--output",
                "json",
                "api",
                "get",
                path,
            ])
            .assert()
            .failure();

        assert!(assert.get_output().stdout.is_empty());
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
        let json: Value = serde_json::from_str(&stderr).expect("json stderr");
        assert_eq!(json["error"]["code"], "invalid_input");
        assert!(stderr.contains("backslash path separators"));
    }
}

#[test]
fn stored_user_token_profile_uses_bearer_header() {
    let server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    base_command(&home)
        .args(["--output", "json", "app", "list"])
        .assert()
        .success();

    let request = server.request();
    assert!(request.headers.iter().any(|header| {
        header.eq_ignore_ascii_case("authorization: Bearer apollo_pat_stored_token")
    }));
}

#[test]
fn stored_consumer_token_profile_uses_raw_authorization_header() {
    let server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "consumer-token"),
    );
    write_file_credential(&home, "dev", "consumer-stored-token");

    base_command(&home)
        .args(["--output", "json", "app", "list"])
        .assert()
        .success();

    let request = server.request();
    assert!(
        request
            .headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("authorization: consumer-stored-token"))
    );
}

#[test]
fn auth_whoami_calls_user_token_current_endpoint() {
    let server = TestServer::json(
        r#"{"authType":"USER_TOKEN","userId":"alice","tokenId":7,"tokenName":"local-cli","tokenPrefix":"apollo_pat_abc","rateLimit":10,"expires":"2030-01-01T00:00:00Z","dataChangeCreatedTime":"2026-01-01T00:00:00Z","denyAll":false,"allOperations":true,"operations":[],"allApps":true,"appIds":[],"allEnvs":true,"envs":[],"allNamespaces":true,"namespaces":[],"actions":[]}"#,
    );
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "auth",
            "whoami",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["userId"], "alice");
    assert!(!stdout.contains("apollo_pat_test_token"));

    let request = server.request();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/openapi/v1/user-tokens/current");
    assert!(
        request.headers.iter().any(
            |header| header.eq_ignore_ascii_case("authorization: Bearer apollo_pat_test_token")
        )
    );
}

#[test]
fn auth_capabilities_table_summarizes_user_token_scope() {
    let server = TestServer::json(
        r#"{"authType":"USER_TOKEN","userId":"alice","tokenId":7,"tokenName":"local-cli","tokenPrefix":"apollo_pat_abc","rateLimit":10,"expires":"2030-01-01T00:00:00Z","dataChangeCreatedTime":"2026-01-01T00:00:00Z","denyAll":false,"allOperations":false,"operations":["config:read"],"allApps":false,"appIds":["demo"],"allEnvs":true,"envs":[],"allNamespaces":true,"namespaces":[],"actions":[{"id":"item.list","method":"GET","path":"/openapi/v1/...","requiredOperations":["config:read"],"grantedOperations":["config:read"],"operationMatch":"ANY","resourceScope":"item","description":"Page items"}]}"#,
    );
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args(["--server", &server.url(), "auth", "capabilities"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("User: alice"));
    assert!(stdout.contains("Token: local-cli"));
    assert!(stdout.contains("Operations: config:read"));
    assert!(stdout.contains("Apps: demo"));
    assert!(stdout.contains("Actions: 1"));
    assert!(!stdout.contains("apollo_pat_test_token"));

    assert_eq!(
        server.request().path,
        "/openapi/v1/user-tokens/current/capabilities"
    );
}

#[test]
fn auth_self_check_requires_user_token_mode() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--server",
            "http://127.0.0.1:9",
            "--output",
            "json",
            "auth",
            "whoami",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("user-token"));
}

#[test]
fn openapi_env_auth_with_explicit_server_ignores_stale_active_profile() {
    let server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();
    write_config(
        &home,
        r#"
active_profile = "missing"
"#,
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--server", &server.url(), "--output", "json", "app", "list"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["data"][0]["appId"], "demo");
    assert_eq!(server.request().path, "/openapi/v1/apps/authorized");
}

#[test]
fn openapi_env_auth_with_explicit_server_does_not_require_config_home() {
    let server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "api",
            "get",
            "/openapi/v1/apps",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"][0]["appId"], "demo");
    assert_eq!(server.request().path, "/openapi/v1/apps");
}

#[test]
fn app_and_env_commands_call_openapi_endpoints() {
    let app_server = TestServer::json(r#"[{"appId":"demo"}]"#);
    let home = temp_home();
    write_config(&home, &profile_config(&app_server.url()));

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""appId": "demo""#));
    assert_eq!(app_server.request().path, "/openapi/v1/apps/authorized");

    let app_ids_server = TestServer::json(r#"[{"appId":"demo"}]"#);
    write_config(&home, &profile_config(&app_ids_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "list", "--app-ids", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""appId": "demo""#));
    assert_eq!(app_ids_server.request().path, "/openapi/v1/apps/authorized");

    let app_get_server = TestServer::sequence(vec![
        (200, "application/json", r#"[{"appId":"demo"}]"#),
        (200, "application/json", r#"{"appId":"demo"}"#),
    ]);
    write_config(&home, &profile_config(&app_get_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "get", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""appId": "demo""#));
    let requests = app_get_server.requests(2);
    assert_eq!(requests[0].path, "/openapi/v1/apps/authorized");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo");

    let env_server = TestServer::json(r#"["DEV","FAT"]"#);
    write_config(&home, &profile_config(&env_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args(["--output", "json", "env", "list", "--app", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DEV"));
    assert_eq!(
        env_server.request().path,
        "/openapi/v1/apps/demo/envclusters"
    );
}

#[test]
fn consumer_token_app_list_filters_authorized_apps_locally() {
    let server = TestServer::json(r#"[{"appId":"demo"},{"appId":"other"}]"#);
    let home = temp_home();
    write_config(&home, &profile_config(&server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--output",
            "json",
            "app",
            "list",
            "--app-ids",
            "demo,missing",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(server.request().path, "/openapi/v1/apps/authorized");
    assert_eq!(json["data"].as_array().expect("data array").len(), 1);
    assert_eq!(json["data"][0]["appId"], "demo");
    assert!(!stdout.contains("other"));
}

#[test]
fn user_token_app_list_filters_visible_apps_locally() {
    let server = TestServer::json(r#"[{"appId":"demo"},{"appId":"other"}]"#);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "app",
            "list",
            "--app-ids",
            "demo,missing",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(server.request().path, "/openapi/v1/apps");
    assert_eq!(json["data"].as_array().expect("data array").len(), 1);
    assert_eq!(json["data"][0]["appId"], "demo");
    assert!(!stdout.contains("other"));
}

#[test]
fn consumer_token_app_get_checks_authorized_apps() {
    let app_get_server = TestServer::sequence(vec![
        (200, "application/json", r#"[{"appId":"demo"}]"#),
        (200, "application/json", r#"{"appId":"demo","name":"Demo"}"#),
    ]);
    let home = temp_home();
    write_config(&home, &profile_config(&app_get_server.url()));

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "get", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""name": "Demo""#));

    let requests = app_get_server.requests(2);
    assert_eq!(requests[0].path, "/openapi/v1/apps/authorized");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo");
}

#[test]
fn consumer_token_app_get_rejects_unauthorized_apps() {
    let home = temp_home();

    let app_get_server = TestServer::json(r#"[{"appId":"other"}]"#);
    write_config(&home, &profile_config(&app_get_server.url()));
    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "get", "demo"])
        .assert()
        .failure();
    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("not authorized"));
    assert_eq!(app_get_server.request().path, "/openapi/v1/apps/authorized");
}

#[test]
fn consumer_token_scoped_reads_fail_closed_without_namespace_scope() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    for args in [
        vec![
            "--output", "json", "config", "get", "--env", "DEV", "--app", "demo", "timeout",
        ],
        vec![
            "--output", "json", "config", "list", "--env", "DEV", "--app", "demo",
        ],
        vec![
            "--output",
            "json",
            "config",
            "diff",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
        ],
        vec![
            "--yes",
            "--output",
            "json",
            "config",
            "apply",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
        ],
        vec!["--output", "json", "env", "list", "--app", "demo"],
        vec![
            "--output",
            "json",
            "namespace",
            "list",
            "--env",
            "DEV",
            "--app",
            "demo",
        ],
        vec![
            "--output",
            "json",
            "namespace",
            "get",
            "--env",
            "DEV",
            "--app",
            "demo",
            "application",
        ],
        vec![
            "--output", "json", "release", "list", "--env", "DEV", "--app", "demo",
        ],
    ] {
        let assert = base_command(&home)
            .env("APOLLO_TOKEN", "consumer-token")
            .args(args)
            .assert()
            .failure();

        assert!(assert.get_output().stdout.is_empty());
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
        let json: Value = serde_json::from_str(&stderr).expect("json stderr");
        assert_eq!(json["error"]["code"], "invalid_input");
        assert!(stderr.contains("consumer-token mode cannot safely verify"));
    }

    server.assert_no_request();
}

#[test]
fn namespace_config_and_release_commands_map_to_openapi_paths() {
    let namespace_server = TestServer::json(r#"[{"namespaceName":"application"}]"#);
    let home = temp_home();
    write_config(&home, &profile_config(&namespace_server.url()));

    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "namespace",
            "list",
            "--env",
            "DEV",
            "--app",
            "demo",
        ])
        .assert()
        .success();
    assert_eq!(
        namespace_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces"
    );

    let namespace_get_server = TestServer::json(r#"{"namespaceName":"settings"}"#);
    write_config(&home, &profile_config(&namespace_get_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "namespace",
            "get",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .success();
    assert_eq!(
        namespace_get_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/settings"
    );

    let config_server = TestServer::json(r#"{"key":"timeout","value":"3000"}"#);
    write_config(&home, &profile_config(&config_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output", "json", "config", "get", "--env", "DEV", "--app", "demo", "timeout",
        ])
        .assert()
        .success();
    assert_eq!(
        config_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/timeout"
    );

    let config_list_server = TestServer::json(r#"{"content":[],"page":0,"size":20,"total":0}"#);
    write_config(&home, &profile_config(&config_list_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output", "json", "config", "list", "--env", "DEV", "--app", "demo",
        ])
        .assert()
        .success();
    assert_eq!(
        config_list_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items?page=0&size=20"
    );

    let release_server = TestServer::json(r#"[{"id":1,"name":"release-1"}]"#);
    write_config(&home, &profile_config(&release_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output", "json", "release", "list", "--env", "DEV", "--app", "demo",
        ])
        .assert()
        .success();
    assert_eq!(
        release_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/releases/active?page=0&size=20"
    );
}

#[test]
fn list_commands_redact_broad_config_values() {
    let config_server = TestServer::json(
        r#"{"content":[{"key":"db.password","value":"s3cr3t"}],"page":0,"size":20,"total":1}"#,
    );
    let home = temp_home();
    write_config(&home, &profile_config(&config_server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output", "json", "config", "list", "--env", "DEV", "--app", "demo",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["content"][0]["value"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));
    assert_eq!(
        config_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items?page=0&size=20"
    );

    let namespace_server = TestServer::json(
        r#"[{"namespaceName":"application","items":[{"key":"db.password","value":"s3cr3t"}]}]"#,
    );
    write_config(&home, &profile_config(&namespace_server.url()));
    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "namespace",
            "list",
            "--env",
            "DEV",
            "--app",
            "demo",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"][0]["items"][0]["value"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));

    let release_server = TestServer::json(
        r#"[{"id":1,"name":"release","configurations":{"db.password":"s3cr3t"}}]"#,
    );
    write_config(&home, &profile_config(&release_server.url()));
    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output", "json", "release", "list", "--env", "DEV", "--app", "demo",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"][0]["configurations"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));
}

#[test]
fn single_read_commands_redact_broad_config_values() {
    let namespace_server = TestServer::json(
        r#"{"namespaceName":"application","items":[{"key":"db.password","value":"s3cr3t"}]}"#,
    );
    let home = temp_home();
    write_config(&home, &profile_config(&namespace_server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "namespace",
            "get",
            "--env",
            "DEV",
            "--app",
            "demo",
            "application",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["items"][0]["value"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));

    let release_server =
        TestServer::json(r#"{"id":1,"name":"release","configurations":{"db.password":"s3cr3t"}}"#);
    write_config(
        &home,
        &profile_config_with_operator(&release_server.url(), "apollo-bot"),
    );
    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes", "--output", "json", "release", "create", "--env", "DEV", "--app", "demo",
            "--title", "v1",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["configurations"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));

    let diff_server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"content":[{"key":"timeout","value":"3000"}],"page":0,"size":500,"total":1}"#,
        ),
        (
            200,
            "application/json",
            r#"{"createItems":[{"key":"db.password","value":"s3cr3t"}],"updateItems":[{"key":"plain","oldValue":"old","newValue":"s3cr3t"}]}"#,
        ),
    ]);
    write_config(
        &home,
        &profile_config_with_auth_mode(&diff_server.url(), "user-token"),
    );
    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "config",
            "diff",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["createItems"][0]["value"], "[REDACTED]");
    assert_eq!(json["data"]["updateItems"][0]["oldValue"], "[REDACTED]");
    assert_eq!(json["data"]["updateItems"][0]["newValue"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));
}

#[test]
fn mutating_commands_require_yes_before_network_call() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(&home, &profile_config(&server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--output", "json", "config", "set", "--env", "DEV", "--app", "demo", "timeout", "3000",
        ])
        .assert()
        .code(1);

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "confirmation_required");
    assert_eq!(json["error"]["operation"]["operation"], "config.set");
    assert_eq!(json["error"]["operation"]["profile"], "dev");
    assert_eq!(json["error"]["operation"]["server"], server.url());
    assert_eq!(json["error"]["operation"]["target"]["app"], "demo");
    assert_eq!(json["error"]["operation"]["target"]["env"], "DEV");
    assert_eq!(json["error"]["operation"]["key"], "timeout");
    assert_eq!(json["error"]["operation"]["keyCount"], 1);
    server.assert_no_request();
}

#[test]
fn namespace_create_requires_initial_confirmation_before_preflight_requests() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(&home, &profile_config(&server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "application.yml",
        ])
        .assert()
        .code(1);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "confirmation_required");
    assert_eq!(
        json["error"]["operation"]["target"]["namespace"],
        "application.yml"
    );
    server.assert_no_request();
}

#[test]
fn yes_in_table_mode_prints_target_summary_before_mutation() {
    let server = TestServer::json(r#"{"key":"timeout","value":"3000"}"#);
    let home = temp_home();
    write_config(&home, &profile_config(&server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-secret")
        .args([
            "--yes",
            "config",
            "set",
            "--env",
            "PROD",
            "--app",
            "demo",
            "db.password",
            "s3cr3t",
        ])
        .assert()
        .success();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(stderr.contains("Mutation plan:"));
    assert!(stderr.contains("Operation: config.set"));
    assert!(stderr.contains("Target: app=demo env=PROD cluster=default namespace=application"));
    assert!(stderr.contains("Key: db.password"));
    assert!(!stderr.contains("s3cr3t"));
    assert!(!stderr.contains("consumer-secret"));

    let request = server.request();
    assert!(request.body.contains("s3cr3t"));
}

#[test]
fn table_mutation_with_empty_response_prints_success_message() {
    let server = TestServer::new(200, "application/json", "");
    let home = temp_home();
    write_config(&home, &profile_config(&server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "config",
            "set",
            "--env",
            "LOCAL",
            "--app",
            "demo",
            "feature.demo",
            "try-it",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert_eq!(stdout, "Mutation 'config.set' completed successfully.\n");
    assert!(!stdout.contains("null"));
}

#[test]
fn api_mutation_json_plan_sanitizes_path_query_and_body() {
    let server = TestServer::empty();
    let home = temp_home();
    let path = "/openapi/v1/tokens/consumer-secret/apps?operator=alice&token=consumer-secret";

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-secret")
        .args([
            "--server",
            &server.url(),
            "--yes",
            "--output",
            "json",
            "api",
            "post",
            path,
            "--body",
            r#"{"password":"s3cr3t"}"#,
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["operation"]["operation"], "api.post");
    assert_eq!(json["operation"]["request"]["method"], "POST");
    assert_eq!(
        json["operation"]["request"]["path"],
        "/openapi/v1/tokens/[REDACTED]/apps"
    );
    assert_eq!(
        json["operation"]["request"]["queryParameters"],
        serde_json::json!(["operator", "token"])
    );
    assert!(!stdout.contains("consumer-secret"));
    assert!(!stdout.contains("s3cr3t"));

    let request = server.request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, path);
    assert!(request.body.contains("s3cr3t"));
}

#[test]
fn user_token_config_set_does_not_require_or_send_operator() {
    let server = TestServer::json(r#"{"key":"timeout","value":"3000"}"#);
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "--yes",
            "config",
            "set",
            "--env",
            "DEV",
            "--app",
            "demo",
            "timeout",
            "3000",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["status"], 200);
    assert_eq!(json["operation"]["operation"], "config.set");
    assert_eq!(json["operation"]["target"]["namespace"], "application");
    assert_eq!(json["operation"]["key"], "timeout");
    assert_eq!(json["data"]["value"], "3000");

    let request = server.request();
    assert_eq!(
        request.path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/timeout?createIfNotExists=true"
    );
    let body: Value = serde_json::from_str(&request.body).expect("json body");
    assert_eq!(body["key"], "timeout");
    assert_eq!(body["value"], "3000");
    assert_eq!(body["type"], 0);
    assert!(body.get("dataChangeCreatedBy").is_none());
    assert!(body.get("dataChangeLastModifiedBy").is_none());
}

#[test]
fn user_token_mutating_command_rejects_explicit_operator() {
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--server",
            "http://127.0.0.1:9",
            "--output",
            "json",
            "--yes",
            "config",
            "set",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--operator",
            "apollo-bot",
            "timeout",
            "3000",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("operator"));
    assert!(stderr.contains("user-token"));
}

#[test]
fn mutating_confirmation_uses_profile_output_before_resolving_credentials() {
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_output("http://127.0.0.1:9", "json"),
    );

    let assert = base_command(&home)
        .args([
            "config", "set", "--env", "DEV", "--app", "demo", "timeout", "3000",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "confirmation_required");
}

#[test]
fn openapi_error_redacts_sensitive_response_body_to_stderr() {
    let server = TestServer::new(500, "application/json", r#"{"message":"consumer-token"}"#);
    let home = temp_home();
    write_config(&home, &profile_config(&server.url()));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "list"])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");

    assert_eq!(json["error"]["code"], "server_error");
    assert!(!stderr.contains("consumer-token"));
    assert!(stderr.contains("[REDACTED]"));
}

#[test]
fn user_token_unauthorized_error_includes_actionable_hint() {
    let server = TestServer::new(401, "text/plain", "Unauthorized user token");
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "api",
            "get",
            "/openapi/v1/apps",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "authentication_failed");
    assert!(stderr.contains("user token"));
    assert!(stderr.contains("expired"));
    assert!(!stderr.contains("apollo_pat_test_token"));
}

#[test]
fn openapi_client_does_not_follow_auth_redirects() {
    let server = TestServer::sequence_with_headers(vec![
        (
            302,
            "text/plain",
            "",
            vec![("Location", "/signin?token=apollo_pat_test_token")],
        ),
        (200, "text/html", "<html>signin</html>", Vec::new()),
    ]);
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "auth",
            "whoami",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "server_error");
    assert!(stderr.contains("HTTP 302"));
    assert!(stderr.contains("redirected to /signin?token=[REDACTED]"));
    assert!(stderr.contains("user token authentication failed"));
    assert!(!stderr.contains("<html>signin</html>"));
    assert!(!stderr.contains("apollo_pat_test_token"));

    let request = server.request();
    assert_eq!(request.path, "/openapi/v1/user-tokens/current");
}

#[test]
fn openapi_success_redacts_exact_token_from_response_body() {
    let server = TestServer::json(r#"{"message":"consumer-token","consumer-token":"value"}"#);
    let home = temp_home();

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--server",
            &server.url(),
            "--output",
            "json",
            "api",
            "get",
            "/openapi/v1/apps",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");

    assert_eq!(json["data"]["message"], "[REDACTED]");
    assert_eq!(json["data"]["[REDACTED]"], "value");
    assert!(!stdout.contains("consumer-token"));
}

#[test]
fn config_set_with_yes_sends_update_payload() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes", "--output", "json", "config", "set", "--env", "DEV", "--app", "demo",
            "timeout", "3000",
        ])
        .assert()
        .success();

    let request = server.request();
    assert_eq!(request.method, "PUT");
    assert_eq!(
        request.path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/timeout?createIfNotExists=true"
    );
    let body: Value = serde_json::from_str(&request.body).expect("json body");
    assert_eq!(body["key"], "timeout");
    assert_eq!(body["value"], "3000");
    assert_eq!(body["type"], 0);
    assert_eq!(body["dataChangeLastModifiedBy"], "apollo-bot");
    assert_eq!(body["dataChangeCreatedBy"], "apollo-bot");
    assert!(body.get("comment").is_none());
}

#[test]
fn config_set_with_yes_sends_item_type_when_provided() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "config",
            "set",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--type",
            "3",
            "payload",
            r#"{"enabled":true}"#,
        ])
        .assert()
        .success();

    let request = server.request();
    let body: Value = serde_json::from_str(&request.body).expect("json body");
    assert_eq!(body["key"], "payload");
    assert_eq!(body["value"], r#"{"enabled":true}"#);
    assert_eq!(body["type"], 3);
}

#[test]
fn config_set_with_yes_sends_comment_when_provided() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "config",
            "set",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--comment",
            "update timeout",
            "timeout",
            "3000",
        ])
        .assert()
        .success();

    let request = server.request();
    let body: Value = serde_json::from_str(&request.body).expect("json body");
    assert_eq!(body["comment"], "update timeout");
}

#[test]
fn config_item_commands_use_encoded_items_for_path_sensitive_keys() {
    let get_server = TestServer::json(r#"{"key":"logging/level","value":"debug"}"#);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&get_server.url(), "user-token"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "config",
            "get",
            "--env",
            "DEV",
            "--app",
            "demo",
            "logging/level",
        ])
        .assert()
        .success();
    assert_eq!(
        get_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/encodedItems/bG9nZ2luZy9sZXZlbA"
    );

    let set_server = TestServer::empty();
    write_config(
        &home,
        &profile_config_with_operator(&set_server.url(), "apollo-bot"),
    );
    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "config",
            "set",
            "--env",
            "DEV",
            "--app",
            "demo",
            "logging/level",
            "debug",
        ])
        .assert()
        .success();
    assert_eq!(
        set_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/encodedItems/bG9nZ2luZy9sZXZlbA?createIfNotExists=true"
    );

    let delete_server = TestServer::empty();
    write_config(
        &home,
        &profile_config_with_operator(&delete_server.url(), "apollo-bot"),
    );
    let delete_assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "config",
            "delete",
            "--env",
            "DEV",
            "--app",
            "demo",
            "logging/level",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(delete_assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("delete json");
    assert_eq!(json["operation"]["operation"], "config.delete");
    assert_eq!(json["operation"]["key"], "logging/level");
    assert_eq!(
        delete_server.request().path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/encodedItems/bG9nZ2luZy9sZXZlbA?operator=apollo-bot"
    );
}

#[test]
fn config_set_falls_back_to_create_when_update_reports_missing_item() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"item not found"}"#),
        (
            200,
            "application/json",
            r#"{"key":"timeout","value":"3000"}"#,
        ),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes", "--output", "json", "config", "set", "--env", "DEV", "--app", "demo",
            "timeout", "3000",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""key": "timeout""#));

    let requests = server.requests(2);
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/timeout?createIfNotExists=true"
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items?operator=apollo-bot"
    );
    let body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(body["key"], "timeout");
    assert_eq!(body["value"], "3000");
    assert_eq!(body["type"], 0);
    assert_eq!(body["dataChangeCreatedBy"], "apollo-bot");
}

#[test]
fn namespace_create_with_yes_sends_namespace_instance_payload() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (200, "application/json", r#"{"name":"settings.json"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings.json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("namespace create json");
    assert_eq!(json["operation"]["operation"], "namespace.create");
    assert_eq!(json["operation"]["target"]["app"], "demo");
    assert_eq!(json["operation"]["target"]["namespace"], "settings.json");
    assert_eq!(json["operation"]["publicNamespace"], false);
    assert_eq!(json["operation"]["appendNamespacePrefix"], true);

    let requests = server.requests(3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/settings.json"
    );

    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    let app_namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(app_namespace_body["appId"], "demo");
    assert_eq!(app_namespace_body["name"], "settings");
    assert_eq!(app_namespace_body["format"], "json");
    assert_eq!(app_namespace_body["isPublic"], false);
    assert_eq!(app_namespace_body["appendNamespacePrefix"], true);
    assert_eq!(app_namespace_body["dataChangeCreatedBy"], "apollo-bot");

    assert_eq!(requests[2].method, "POST");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[2].body).expect("json body");
    assert_eq!(namespace_body[0]["appId"], "demo");
    assert_eq!(namespace_body[0]["env"], "DEV");
    assert_eq!(namespace_body[0]["clusterName"], "default");
    assert_eq!(namespace_body[0]["appNamespaceName"], "settings.json");
}

#[test]
fn namespace_create_infers_txt_appnamespace_format() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (200, "application/json", r#"{"name":"message.txt"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "message.txt",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    let app_namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(app_namespace_body["name"], "message");
    assert_eq!(app_namespace_body["format"], "txt");
    let namespace_body: Value = serde_json::from_str(&requests[2].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "message.txt");
}

#[test]
fn namespace_create_treats_apollo_missing_appnamespace_400_as_absent() {
    let server = TestServer::sequence(vec![
        (
            400,
            "application/json",
            r#"{"message":"appNamespace not exist for appId:demo namespaceName:settings"}"#,
        ),
        (200, "application/json", r#"{"name":"settings"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/settings"
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    assert_eq!(requests[2].method, "POST");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
}

#[test]
fn namespace_create_recovers_when_server_reports_failure_after_creation() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (200, "application/json", r#"{"name":"settings"}"#),
        (
            400,
            "application/json",
            r#"{"message":"create namespace failed for: DEV/default/settings"}"#,
        ),
        (
            200,
            "application/json",
            r#"{"namespaceName":"settings","items":[{"key":"db.password","value":"s3cr3t"}]}"#,
        ),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""namespaceName": "settings""#));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["items"][0]["value"], "[REDACTED]");
    assert!(!stdout.contains("s3cr3t"));

    let requests = server.requests(4);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[2].method, "POST");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    assert_eq!(requests[3].method, "GET");
    assert_eq!(
        requests[3].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/settings"
    );
}

#[test]
fn namespace_create_does_not_mask_auth_redirect_after_namespace_post() {
    let server = TestServer::sequence_with_headers(vec![
        (
            404,
            "application/json",
            r#"{"message":"not found"}"#,
            Vec::new(),
        ),
        (
            200,
            "application/json",
            r#"{"name":"settings"}"#,
            Vec::new(),
        ),
        (302, "text/plain", "", vec![("Location", "/signin")]),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    base_command(&home)
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("HTTP 302"))
        .stderr(predicate::str::contains("redirected to /signin"));

    let requests = server.requests(3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, "/openapi/v1/namespaces");
}

#[test]
fn namespace_create_rejects_existing_namespace_before_posting() {
    let server = TestServer::sequence(vec![
        (200, "application/json", r#"{"name":"settings"}"#),
        (
            200,
            "application/json",
            r#"{"namespaceName":"settings","items":[]}"#,
        ),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("namespace already exists"));

    let requests = server.requests(2);
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/settings"
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/settings"
    );
}

#[test]
fn namespace_create_reuses_existing_appnamespace() {
    let server = TestServer::sequence(vec![
        (200, "application/json", r#"{"name":"application"}"#),
        (
            404,
            "application/json",
            r#"{"message":"namespace not exist"}"#,
        ),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "application",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/application"
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application"
    );

    assert_eq!(requests[2].method, "POST");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[2].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "application");
}

#[test]
fn namespace_create_continues_when_user_token_cannot_read_existing_namespace() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"name":"application","isPublic":false}"#,
        ),
        (403, "application/json", r#"{"message":"forbidden"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    base_command(&home)
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "application",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application"
    );
    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, "/openapi/v1/namespaces");
}

#[test]
fn namespace_create_continues_when_user_token_cannot_read_appnamespace_lookup() {
    let server = TestServer::sequence(vec![
        (403, "application/json", r#"{"message":"forbidden"}"#),
        (403, "application/json", r#"{"message":"forbidden"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    base_command(&home)
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/settings"
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/settings"
    );
    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, "/openapi/v1/namespaces");
    let namespace_body: Value = serde_json::from_str(&requests[2].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "settings");
}

#[test]
fn namespace_create_reuses_existing_prefixed_public_appnamespace() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (
            200,
            "application/json",
            r#"[{"name":"FX.application.yml","isPublic":true}]"#,
        ),
        (
            404,
            "application/json",
            r#"{"message":"namespace not exist"}"#,
        ),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "application.yml",
        ])
        .assert()
        .success();

    let requests = server.requests(4);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/application.yml"
    );

    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");

    assert_eq!(requests[2].method, "GET");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/FX.application.yml"
    );

    assert_eq!(requests[3].method, "POST");
    assert_eq!(
        requests[3].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[3].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "FX.application.yml");
}

#[test]
fn namespace_create_reuses_prefixed_public_when_unprefixed_private_exists() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"name":"application.yml","isPublic":false}"#,
        ),
        (
            200,
            "application/json",
            r#"[{"name":"FX.application.yml","isPublic":true}]"#,
        ),
        (
            404,
            "application/json",
            r#"{"message":"namespace not exist"}"#,
        ),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "application.yml",
        ])
        .assert()
        .success();

    let requests = server.requests(4);
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/application.yml"
    );
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/FX.application.yml"
    );
    let namespace_body: Value = serde_json::from_str(&requests[3].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "FX.application.yml");
}

#[test]
fn namespace_create_allows_prefixed_public_when_only_unprefixed_private_exists() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"name":"application.yml","isPublic":false}"#,
        ),
        (200, "application/json", r#"[]"#),
        (200, "application/json", r#"{"orgId":"FX"}"#),
        (200, "application/json", r#"{"name":"FX.application.yml"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "application.yml",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("namespace create json");
    assert_eq!(
        json["operation"]["target"]["namespace"],
        "FX.application.yml"
    );

    let requests = server.requests(5);
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/application.yml"
    );
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[2].path, "/openapi/v1/apps/demo");
    assert_eq!(requests[3].method, "POST");
    assert_eq!(
        requests[3].path,
        "/openapi/v1/apps/demo/appnamespaces?appendNamespacePrefix=true"
    );
    assert_eq!(requests[4].method, "POST");
    assert_eq!(
        requests[4].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[4].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "FX.application.yml");
}

#[test]
fn namespace_create_does_not_reuse_suffix_colliding_public_appnamespace() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (
            200,
            "application/json",
            r#"[{"name":"FX.foo.application.yml","isPublic":true}]"#,
        ),
        (200, "application/json", r#"{"orgId":"FX"}"#),
        (200, "application/json", r#"{"name":"FX.application.yml"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "application.yml",
        ])
        .assert()
        .success();

    let requests = server.requests(5);
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[2].path, "/openapi/v1/apps/demo");
    assert_eq!(requests[3].method, "POST");
    assert_eq!(
        requests[3].path,
        "/openapi/v1/apps/demo/appnamespaces?appendNamespacePrefix=true"
    );
    let namespace_body: Value = serde_json::from_str(&requests[4].body).expect("json body");
    assert_eq!(namespace_body[0]["appNamespaceName"], "FX.application.yml");
}

#[test]
fn namespace_create_stops_if_apollo_returns_an_unapproved_prefixed_name() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (200, "application/json", r#"[]"#),
        (200, "application/json", r#"{"orgId":"FX"}"#),
        (
            200,
            "application/json",
            r#"{"name":"OTHER.application.yml"}"#,
        ),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "application.yml",
        ])
        .assert()
        .code(1);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(stderr.contains("OTHER.application.yml"));
    assert!(stderr.contains("FX.application.yml"));
    assert!(stderr.contains("namespace instance was not created"));

    let requests = server.requests(4);
    assert_eq!(requests[3].method, "POST");
    assert_eq!(
        requests[3].path,
        "/openapi/v1/apps/demo/appnamespaces?appendNamespacePrefix=true"
    );
}

#[test]
fn namespace_create_treats_empty_appnamespace_lookup_as_missing() {
    let server = TestServer::sequence(vec![
        (200, "application/json", "{}"),
        (200, "application/json", r#"{"name":"application"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "application",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/application"
    );

    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    let app_namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(app_namespace_body["name"], "application");

    assert_eq!(requests[2].method, "POST");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
}

#[test]
fn namespace_create_rejects_unprefixed_public_flag_for_existing_private_appnamespace() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"name":"application","isPublic":false}"#,
        ),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "--no-append-namespace-prefix",
            "application",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("private"));
    assert!(stderr.contains("--public"));

    let request = server.request();
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.path,
        "/openapi/v1/apps/demo/appnamespaces/application"
    );
}

#[test]
fn namespace_create_rejects_private_create_for_existing_public_appnamespace() {
    let server = TestServer::sequence(vec![(
        200,
        "application/json",
        r#"{"name":"application","isPublic":true}"#,
    )]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "application",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("public"));
    assert!(stderr.contains("--public"));

    let request = server.request();
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.path,
        "/openapi/v1/apps/demo/appnamespaces/application"
    );
}

#[test]
fn namespace_create_with_public_flag_sends_public_namespace_payload() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (200, "application/json", r#"{"name":"application.yml"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--public",
            "--comment",
            "shared settings",
            "--no-append-namespace-prefix",
            "application.yml",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/application.yml"
    );

    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/apps/demo/appnamespaces?appendNamespacePrefix=false"
    );
    let app_namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(app_namespace_body["appId"], "demo");
    assert_eq!(app_namespace_body["name"], "application");
    assert_eq!(app_namespace_body["format"], "yml");
    assert_eq!(app_namespace_body["isPublic"], true);
    assert_eq!(app_namespace_body["appendNamespacePrefix"], false);
    assert_eq!(app_namespace_body["comment"], "shared settings");
    assert_eq!(app_namespace_body["dataChangeCreatedBy"], "apollo-bot");

    assert_eq!(requests[2].method, "POST");
    assert_eq!(
        requests[2].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[2].body).expect("json body");
    assert_eq!(namespace_body[0]["appId"], "demo");
    assert_eq!(namespace_body[0]["env"], "DEV");
    assert_eq!(namespace_body[0]["clusterName"], "default");
    assert_eq!(namespace_body[0]["appNamespaceName"], "application.yml");
}

#[test]
fn user_token_namespace_create_omits_operator_fields() {
    let server = TestServer::sequence(vec![
        (404, "application/json", r#"{"message":"not found"}"#),
        (200, "application/json", r#"{"name":"settings"}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    base_command(&home)
        .args([
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "settings",
        ])
        .assert()
        .success();

    let requests = server.requests(3);
    for request in &requests {
        assert!(request.headers.iter().any(|header| {
            header.eq_ignore_ascii_case("authorization: Bearer apollo_pat_stored_token")
        }));
    }

    assert_eq!(
        requests[0].path,
        "/openapi/v1/apps/demo/appnamespaces/settings"
    );

    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/openapi/v1/apps/demo/appnamespaces");
    let app_namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert!(app_namespace_body.get("dataChangeCreatedBy").is_none());

    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, "/openapi/v1/namespaces");
}

#[test]
fn user_token_release_writes_omit_operator_fields() {
    let server = TestServer::sequence(vec![
        (200, "application/json", r#"{"id":42}"#),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    let create_assert = base_command(&home)
        .args([
            "--yes", "--output", "json", "release", "create", "--env", "DEV", "--app", "demo",
            "--title", "release",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(create_assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("create json");
    assert_eq!(json["operation"]["operation"], "release.create");
    assert_eq!(json["operation"]["target"]["app"], "demo");
    assert_eq!(json["operation"]["target"]["env"], "DEV");
    assert_eq!(json["operation"]["releaseTitle"], "release");
    assert_eq!(json["operation"]["emergency"], false);

    let rollback_assert = base_command(&home)
        .args([
            "--yes",
            "--output",
            "json",
            "release",
            "rollback",
            "--env",
            "DEV",
            "42",
            "--to-release-id",
            "40",
        ])
        .assert()
        .success();

    let stdout =
        String::from_utf8(rollback_assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("rollback json");
    assert_eq!(json["operation"]["operation"], "release.rollback");
    assert_eq!(json["operation"]["target"]["env"], "DEV");
    assert_eq!(json["operation"]["releaseId"], 42);
    assert_eq!(json["operation"]["toReleaseId"], 40);

    let requests = server.requests(2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/releases"
    );
    let release_body: Value = serde_json::from_str(&requests[0].body).expect("json body");
    assert!(release_body.get("releasedBy").is_none());

    assert_eq!(requests[1].method, "PUT");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/releases/42/rollback?toReleaseId=40"
    );
}

#[test]
fn user_token_namespace_and_release_reject_explicit_operator_without_network_call() {
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode("http://127.0.0.1:9", "user-token"),
    );
    write_file_credential(&home, "dev", "apollo_pat_stored_token");

    for args in [
        vec![
            "--yes",
            "--output",
            "json",
            "namespace",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--operator",
            "apollo-bot",
            "settings",
        ],
        vec![
            "--yes",
            "--output",
            "json",
            "release",
            "create",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--operator",
            "apollo-bot",
            "--title",
            "release",
        ],
        vec![
            "--yes",
            "--output",
            "json",
            "release",
            "rollback",
            "--env",
            "DEV",
            "--operator",
            "apollo-bot",
            "42",
        ],
    ] {
        let assert = base_command(&home).args(args).assert().failure();

        assert!(assert.get_output().stdout.is_empty());
        let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
        let json: Value = serde_json::from_str(&stderr).expect("json stderr");
        assert_eq!(json["error"]["code"], "invalid_input");
        assert!(stderr.contains("operator"));
        assert!(stderr.contains("user-token"));
    }
}

#[test]
fn config_apply_with_yes_uses_synchronize_endpoint() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"content":[{"key":"timeout","value":"3000"}],"page":0,"size":500,"total":1}"#,
        ),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--yes",
            "--output",
            "json",
            "config",
            "apply",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("apply json");
    assert_eq!(json["operation"]["operation"], "config.apply");
    assert_eq!(json["operation"]["source"]["env"], "DEV");
    assert_eq!(json["operation"]["target"]["env"], "FAT");
    assert_eq!(json["operation"]["target"]["namespace"], "application");

    let requests = server.requests(2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items?page=0&size=500"
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/synchronize"
    );
    let body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(body["syncToNamespaces"][0]["appId"], "demo");
    assert_eq!(body["syncToNamespaces"][0]["env"], "FAT");
    assert_eq!(body["syncToNamespaces"][0]["clusterName"], "default");
    assert_eq!(body["syncToNamespaces"][0]["namespaceName"], "application");
    assert_eq!(body["syncItems"][0]["key"], "timeout");
    assert_eq!(body["syncItems"][0]["value"], "3000");
}

#[test]
fn config_apply_rejects_cross_namespace_target_before_syncing() {
    let server = TestServer::empty();
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--yes",
            "--output",
            "json",
            "config",
            "apply",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
            "--target-namespace",
            "other",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "invalid_input");
    assert!(stderr.contains("--target-namespace"));
}

#[test]
fn config_diff_populates_sync_items_from_source_namespace() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"content":[{"key":"timeout","value":"3000"}],"page":0,"size":500,"total":1}"#,
        ),
        (200, "application/json", "{}"),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "config",
            "diff",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
        ])
        .assert()
        .success();

    let requests = server.requests(2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items?page=0&size=500"
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/diff"
    );
    let body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(body["syncItems"][0]["key"], "timeout");
    assert_eq!(body["syncItems"][0]["value"], "3000");
}

#[test]
fn config_diff_keeps_source_sync_items_unredacted_while_redacting_output() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"content":[{"key":"token-value","value":"source-secret"}],"page":0,"size":500,"total":1}"#,
        ),
        (
            200,
            "application/json",
            r#"{"message":"apollo_pat_test_token"}"#,
        ),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_auth_mode(&server.url(), "user-token"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "apollo_pat_test_token")
        .args([
            "--output",
            "json",
            "config",
            "diff",
            "--env",
            "DEV",
            "--app",
            "demo",
            "--target-env",
            "FAT",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(json["data"]["message"], "[REDACTED]");
    assert!(!stdout.contains("apollo_pat_test_token"));

    let requests = server.requests(2);
    let body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(body["syncItems"][0]["key"], "token-value");
    assert_eq!(body["syncItems"][0]["value"], "source-secret");
}

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: Vec<String>,
    body: String,
}

struct TestServer {
    addr: SocketAddr,
    request_rx: Receiver<CapturedRequest>,
}

type TestResponse = (u16, &'static str, &'static str);
type TestResponseWithHeaders = (
    u16,
    &'static str,
    &'static str,
    Vec<(&'static str, &'static str)>,
);

impl TestServer {
    fn json(body: &'static str) -> Self {
        Self::new(200, "application/json", body)
    }

    fn empty() -> Self {
        Self::new(200, "application/json", "{}")
    }

    fn new(status: u16, content_type: &'static str, body: &'static str) -> Self {
        Self::sequence(vec![(status, content_type, body)])
    }

    fn sequence(responses: Vec<TestResponse>) -> Self {
        Self::sequence_with_headers(
            responses
                .into_iter()
                .map(|(status, content_type, body)| (status, content_type, body, Vec::new()))
                .collect(),
        )
    }

    fn sequence_with_headers(responses: Vec<TestResponseWithHeaders>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let (request_tx, request_rx) = mpsc::channel();
        thread::spawn(move || {
            for (status, content_type, body, response_headers) in responses {
                let (stream, _) = listener.accept().expect("accept request");
                let request = read_request(stream, status, content_type, body, &response_headers);
                request_tx.send(request).expect("send captured request");
            }
        });
        Self { addr, request_rx }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn request(self) -> CapturedRequest {
        self.request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for captured request")
    }

    fn requests(self, count: usize) -> Vec<CapturedRequest> {
        (0..count)
            .map(|_| {
                self.request_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("timed out waiting for captured request")
            })
            .collect()
    }

    fn assert_no_request(self) {
        assert!(
            self.request_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "expected no OpenAPI request"
        );
    }
}

fn read_request(
    mut stream: TcpStream,
    status: u16,
    content_type: &str,
    response_body: &str,
    response_headers: &[(&str, &str)],
) -> CapturedRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if request_complete(&buffer) {
            break;
        }
    }

    let request = String::from_utf8_lossy(&buffer).to_string();
    let (head, body) = request.split_once("\r\n\r\n").unwrap_or((&request, ""));
    let mut lines = head.lines();
    let request_line = lines.next().expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("method").to_owned();
    let path = parts.next().expect("path").to_owned();
    let headers = lines.map(ToOwned::to_owned).collect();

    let extra_headers = response_headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let response = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        extra_headers,
        response_body.len(),
        response_body
    );
    stream
        .write_all(response.as_bytes())
        .expect("write response");

    CapturedRequest {
        method,
        path,
        headers,
        body: body.to_owned(),
    }
}

fn request_complete(buffer: &[u8]) -> bool {
    let request = String::from_utf8_lossy(buffer);
    let Some((head, body)) = request.split_once("\r\n\r\n") else {
        return false;
    };
    let content_length = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    match content_length {
        Some(length) => body.len() >= length,
        None => true,
    }
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

fn write_file_credential(home: &TempDir, key: &str, token: &str) {
    let path = credential_file_path(home, key);
    fs::create_dir_all(path.parent().expect("credential parent")).expect("credential dir");
    fs::write(path, token).expect("credential file");
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

fn profile_config(server: &str) -> String {
    profile_config_with_operator(server, "apollo-bot")
}

fn profile_config_with_output(server: &str, output: &str) -> String {
    format!(
        r#"
active_profile = "dev"

[profiles.dev]
server = "{}"
output = "{}"
operator = "apollo-bot"
"#,
        server, output
    )
}

fn profile_config_with_operator(server: &str, operator: &str) -> String {
    format!(
        r#"
active_profile = "dev"

[profiles.dev]
server = "{}"
output = "table"
operator = "{}"
"#,
        server, operator
    )
}

fn profile_config_with_auth_mode(server: &str, auth_mode: &str) -> String {
    format!(
        r#"
active_profile = "dev"

[profiles.dev]
server = "{}"
output = "table"
auth_mode = "{}"

[profiles.dev.credential]
backend = "file"
key = "dev"
"#,
        server, auth_mode
    )
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
