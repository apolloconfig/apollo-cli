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
    assert_eq!(server.request().path, "/openapi/v1/apps");
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
    assert_eq!(app_server.request().path, "/openapi/v1/apps");

    let app_get_server = TestServer::json(r#"{"appId":"demo"}"#);
    write_config(&home, &profile_config(&app_get_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args(["--output", "json", "app", "get", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""appId": "demo""#));
    assert_eq!(app_get_server.request().path, "/openapi/v1/apps/demo");

    let env_server = TestServer::json(r#"["DEV","FAT"]"#);
    write_config(&home, &profile_config(&env_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
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
fn namespace_config_and_release_commands_map_to_openapi_paths() {
    let namespace_server = TestServer::json(r#"[{"namespaceName":"application"}]"#);
    let home = temp_home();
    write_config(&home, &profile_config(&namespace_server.url()));

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
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

    let config_server = TestServer::json(r#"{"key":"timeout","value":"3000"}"#);
    write_config(&home, &profile_config(&config_server.url()));
    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
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
        .env("APOLLO_TOKEN", "consumer-token")
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
        .env("APOLLO_TOKEN", "consumer-token")
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
fn mutating_commands_require_yes_before_network_call() {
    let home = temp_home();
    write_config(&home, &profile_config("http://127.0.0.1:9"));

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
        .args([
            "--output", "json", "config", "set", "--env", "DEV", "--app", "demo", "timeout", "3000",
        ])
        .assert()
        .failure();

    assert!(assert.get_output().stdout.is_empty());
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let json: Value = serde_json::from_str(&stderr).expect("json stderr");
    assert_eq!(json["error"]["code"], "confirmation_required");
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
    assert_eq!(body["dataChangeLastModifiedBy"], "apollo-bot");
    assert_eq!(body["dataChangeCreatedBy"], "apollo-bot");
    assert!(body.get("comment").is_none());
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
        &profile_config_with_operator(&get_server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
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
    assert_eq!(body["dataChangeCreatedBy"], "apollo-bot");
}

#[test]
fn namespace_create_with_yes_sends_namespace_instance_payload() {
    let server = TestServer::sequence(vec![
        (200, "application/json", r#"{"name":"settings.json"}"#),
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
            "settings.json",
        ])
        .assert()
        .success();

    let requests = server.requests(2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/openapi/v1/apps/demo/appnamespaces");
    let app_namespace_body: Value = serde_json::from_str(&requests[0].body).expect("json body");
    assert_eq!(app_namespace_body["appId"], "demo");
    assert_eq!(app_namespace_body["name"], "settings");
    assert_eq!(app_namespace_body["format"], "json");
    assert_eq!(app_namespace_body["isPublic"], false);
    assert_eq!(app_namespace_body["dataChangeCreatedBy"], "apollo-bot");

    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(namespace_body[0]["appId"], "demo");
    assert_eq!(namespace_body[0]["env"], "DEV");
    assert_eq!(namespace_body[0]["clusterName"], "default");
    assert_eq!(namespace_body[0]["appNamespaceName"], "settings.json");
}

#[test]
fn namespace_create_with_public_flag_sends_public_namespace_payload() {
    let server = TestServer::sequence(vec![
        (
            200,
            "application/json",
            r#"{"name":"public.application.yml"}"#,
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

    let requests = server.requests(2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/openapi/v1/apps/demo/appnamespaces");
    let app_namespace_body: Value = serde_json::from_str(&requests[0].body).expect("json body");
    assert_eq!(app_namespace_body["appId"], "demo");
    assert_eq!(app_namespace_body["name"], "application");
    assert_eq!(app_namespace_body["format"], "yml");
    assert_eq!(app_namespace_body["isPublic"], true);
    assert_eq!(app_namespace_body["dataChangeCreatedBy"], "apollo-bot");

    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/openapi/v1/namespaces?operator=apollo-bot"
    );
    let namespace_body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(namespace_body[0]["appId"], "demo");
    assert_eq!(namespace_body[0]["env"], "DEV");
    assert_eq!(namespace_body[0]["clusterName"], "default");
    assert_eq!(
        namespace_body[0]["appNamespaceName"],
        "public.application.yml"
    );
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
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
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
        "/openapi/v1/envs/DEV/apps/demo/clusters/default/namespaces/application/items/synchronize?operator=apollo-bot"
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
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
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
            r#"{"content":[{"key":"token-value","value":"consumer-token"}],"page":0,"size":500,"total":1}"#,
        ),
        (200, "application/json", r#"{"message":"consumer-token"}"#),
    ]);
    let home = temp_home();
    write_config(
        &home,
        &profile_config_with_operator(&server.url(), "apollo-bot"),
    );

    let assert = base_command(&home)
        .env("APOLLO_TOKEN", "consumer-token")
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
    assert!(!stdout.contains("consumer-token"));

    let requests = server.requests(2);
    let body: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(body["syncItems"][0]["key"], "token-value");
    assert_eq!(body["syncItems"][0]["value"], "consumer-token");
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

    fn sequence(responses: Vec<(u16, &'static str, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let (request_tx, request_rx) = mpsc::channel();
        thread::spawn(move || {
            for (status, content_type, body) in responses {
                let (stream, _) = listener.accept().expect("accept request");
                let request = read_request(stream, status, content_type, body);
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
}

fn read_request(
    mut stream: TcpStream,
    status: u16,
    content_type: &str,
    response_body: &str,
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

    let response = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
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

fn normalize_toml(body: &str) -> String {
    body.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
        + "\n"
}
