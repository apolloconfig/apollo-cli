use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

use crate::cli::OutputFormat;
use crate::error::CliError;
use crate::redaction::{Redactor, Sensitive};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_BODY_CHARS: usize = 4096;

#[derive(Clone, Debug, Serialize)]
pub struct OpenApiResponse {
    pub status: u16,
    pub data: Value,
}

impl OpenApiResponse {
    pub fn render_table(&self) -> String {
        serde_json::to_string_pretty(&self.data).expect("openapi response table json")
    }
}

pub struct OpenApiClient {
    server: String,
    token: Sensitive,
    format: OutputFormat,
    client: reqwest::blocking::Client,
}

impl OpenApiClient {
    pub fn new(server: String, token: Sensitive, format: OutputFormat) -> Self {
        Self {
            server: server.trim_end_matches('/').to_owned(),
            token,
            format,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<OpenApiResponse, CliError> {
        let path = normalize_openapi_path(path, self.format)?;
        let url = format!("{}{}", self.server, path);
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| CliError::invalid_input("unsupported HTTP method", self.format))?;
        let mut request = self
            .client
            .request(method, &url)
            .header(reqwest::header::AUTHORIZATION, self.token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request
            .timeout(REQUEST_TIMEOUT)
            .send()
            .map_err(|error| CliError::network(&path, &error.to_string(), self.format))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| CliError::network(&path, &error.to_string(), self.format))?;

        if !status.is_success() {
            let body = sanitize_error_body(&body, self.token.expose_secret());
            return Err(CliError::http_status(
                status.as_u16(),
                &path,
                &body,
                self.format,
            ));
        }

        let body = redact_exact_token(body, self.token.expose_secret());
        let data = if body.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&body).unwrap_or(Value::String(body))
        };

        Ok(OpenApiResponse {
            status: status.as_u16(),
            data,
        })
    }
}

fn sanitize_error_body(body: &str, token: &str) -> String {
    let redactor = Redactor;
    let redacted = serde_json::from_str::<Value>(body)
        .ok()
        .map(|value| redactor.redact_json(value).to_string())
        .unwrap_or_else(|| redactor.redact_text(body));
    let redacted = redact_exact_token(redacted, token);
    truncate_chars(redacted, MAX_ERROR_BODY_CHARS)
}

fn redact_exact_token(value: String, token: &str) -> String {
    let token = token.trim();
    if token.is_empty() {
        value
    } else {
        value.replace(token, "[REDACTED]")
    }
}

fn truncate_chars(value: String, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

pub fn encode_path_segment(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

pub fn append_query(path: String, key: &str, value: &str) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    format!(
        "{}{}{}={}",
        path,
        separator,
        urlencoding::encode(key),
        urlencoding::encode(value)
    )
}

fn normalize_openapi_path(path: &str, format: OutputFormat) -> Result<String, CliError> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err(CliError::invalid_input(
            "OpenAPI path must be relative, for example /openapi/v1/apps",
            format,
        ));
    }
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{}", path)
    };
    if path == "/openapi/v1" || path.starts_with("/openapi/v1/") {
        Ok(path)
    } else {
        Err(CliError::invalid_input(
            "OpenAPI path must start with /openapi/v1",
            format,
        ))
    }
}
