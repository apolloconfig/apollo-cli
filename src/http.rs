use serde::{Serialize, Serializer, ser::SerializeStruct};
use serde_json::Value;
use std::fmt;
use std::time::Duration;

use crate::cli::{AuthMode, OutputFormat};
use crate::error::CliError;
use crate::redaction::{Redactor, Sensitive};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_BODY_CHARS: usize = 4096;

#[derive(Clone)]
pub struct OpenApiResponse {
    pub status: u16,
    pub data: Value,
    redaction_token: Sensitive,
}

impl OpenApiResponse {
    pub fn render_table(&self) -> String {
        serde_json::to_string_pretty(&self.redacted_data()).expect("openapi response table json")
    }

    pub fn with_data(&self, data: Value) -> Self {
        Self {
            status: self.status,
            data,
            redaction_token: self.redaction_token.clone(),
        }
    }

    fn redacted_data(&self) -> Value {
        redact_exact_token_value(self.data.clone(), self.redaction_token.expose_secret())
    }
}

impl fmt::Debug for OpenApiResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenApiResponse")
            .field("status", &self.status)
            .field("data", &self.redacted_data())
            .finish()
    }
}

impl Serialize for OpenApiResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("OpenApiResponse", 2)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("data", &self.redacted_data())?;
        state.end()
    }
}

pub struct OpenApiClient {
    server: String,
    token: Sensitive,
    auth_mode: AuthMode,
    format: OutputFormat,
    client: reqwest::blocking::Client,
}

impl OpenApiClient {
    pub fn new(
        server: String,
        token: Sensitive,
        auth_mode: AuthMode,
        format: OutputFormat,
    ) -> Self {
        Self {
            server: server.trim_end_matches('/').to_owned(),
            token,
            auth_mode,
            format,
            client: reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("OpenAPI HTTP client should build"),
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
            .header(reqwest::header::AUTHORIZATION, self.authorization_header())
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request
            .timeout(REQUEST_TIMEOUT)
            .send()
            .map_err(|error| CliError::network(&path, &error.to_string(), self.format))?;
        let status = response.status();
        let redirect_location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = response
            .text()
            .map_err(|error| CliError::network(&path, &error.to_string(), self.format))?;

        if !status.is_success() {
            let mut body = sanitize_error_body(&body, self.token.expose_secret());
            if body.is_empty()
                && let Some(location) = redirect_location
            {
                body = format!(
                    "redirected to {}",
                    redact_exact_token(location, self.token.expose_secret())
                );
            }
            if self.auth_mode.is_user_token()
                && (matches!(status.as_u16(), 401 | 403) || status.is_redirection())
            {
                body = append_user_token_hint(body);
            }
            return Err(CliError::http_status(
                status.as_u16(),
                &path,
                &body,
                self.format,
            ));
        }

        let data = if body.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&body).unwrap_or(Value::String(body))
        };

        Ok(OpenApiResponse {
            status: status.as_u16(),
            data,
            redaction_token: self.token.clone(),
        })
    }

    fn authorization_header(&self) -> String {
        match self.auth_mode {
            AuthMode::UserToken => format!("Bearer {}", self.token.expose_secret()),
            AuthMode::ConsumerToken => self.token.expose_secret().to_owned(),
        }
    }
}

fn append_user_token_hint(mut body: String) -> String {
    if !body.is_empty() {
        body.push_str("; ");
    }
    body.push_str(
        "user token authentication failed; verify the Apollo server supports user tokens, the token is not expired or revoked, and the requested resource is within token scope",
    );
    body
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

fn redact_exact_token_value(value: Value, token: &str) -> Value {
    let token = token.trim();
    if token.is_empty() {
        return value;
    }

    match value {
        Value::String(value) => Value::String(value.replace(token, "[REDACTED]")),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_exact_token_value(value, token))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    (
                        key.replace(token, "[REDACTED]"),
                        redact_exact_token_value(value, token),
                    )
                })
                .collect(),
        ),
        value => value,
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
    reject_dot_segments(&path, format)?;
    if path == "/openapi/v1" || path.starts_with("/openapi/v1/") {
        Ok(path)
    } else {
        Err(CliError::invalid_input(
            "OpenAPI path must start with /openapi/v1",
            format,
        ))
    }
}

fn reject_dot_segments(path: &str, format: OutputFormat) -> Result<(), CliError> {
    let path_without_query = path.split('?').next().unwrap_or(path);
    for segment in path_without_query.split('/') {
        let decoded = urlencoding::decode(segment).map_err(|_| {
            CliError::invalid_input("OpenAPI path contains invalid percent-encoding", format)
        })?;
        if decoded == "." || decoded == ".." {
            return Err(CliError::invalid_input(
                "OpenAPI path must not contain . or .. path segments",
                format,
            ));
        }
    }
    Ok(())
}
