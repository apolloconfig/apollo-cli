use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";

static AUTHORIZATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)(authorization\\s*:\\s*(?:bearer\\s+)?)[^\\s]+").unwrap());
static CONSUMER_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)(consumer\\s+token\\s+)[^\\s]+").unwrap());
static USER_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("apollo_pat_[A-Za-z0-9._~-]+").unwrap());
static JSON_SENSITIVE_FIELD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)(\"[^\"]*(?:authorization|token|password|secret)[^\"]*\"\\s*:\\s*\")[^\"]*(\")")
        .unwrap()
});

#[derive(Default)]
pub struct Redactor;

impl Redactor {
    pub fn redact_text(&self, value: &str) -> String {
        let value = AUTHORIZATION_RE.replace_all(value, format!("${{1}}{}", REDACTED));
        let value = CONSUMER_TOKEN_RE
            .replace_all(&value, format!("${{1}}{}", REDACTED))
            .to_string();
        let value = USER_TOKEN_RE.replace_all(&value, REDACTED).to_string();
        JSON_SENSITIVE_FIELD_RE
            .replace_all(&value, format!("${{1}}{}${{2}}", REDACTED))
            .to_string()
    }

    pub fn redact_json(&self, value: Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(self.redact_object(map)),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| self.redact_json(value))
                    .collect(),
            ),
            Value::String(value) => Value::String(self.redact_text(&value)),
            value => value,
        }
    }

    fn redact_object(&self, map: Map<String, Value>) -> Map<String, Value> {
        map.into_iter()
            .map(|(key, value)| {
                if is_sensitive_key(&key) {
                    (key, Value::String(REDACTED.to_owned()))
                } else {
                    (key, self.redact_json(value))
                }
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct Sensitive(String);

impl Sensitive {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Sensitive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("authorization")
        || key.contains("password")
        || key.contains("secret")
}
