use serde::Serialize;
use serde_json::Value;

use crate::cli::OutputFormat;
use crate::error::CliError;
use crate::redaction::Sensitive;

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
            .send()
            .map_err(|error| CliError::network(&path, &error.to_string(), self.format))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| CliError::network(&path, &error.to_string(), self.format))?;

        if !status.is_success() {
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
        })
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
