use std::fmt;

use regex::Regex;
use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";

#[derive(Default)]
pub struct Redactor;

impl Redactor {
    pub fn redact_text(&self, value: &str) -> String {
        let authorization = Regex::new("(?i)(authorization\\s*:\\s*bearer\\s+)[^\\s]+")
            .expect("authorization regex");
        let consumer_token =
            Regex::new("(?i)(consumer\\s+token\\s+)[^\\s]+").expect("consumer token regex");

        let value = authorization.replace_all(value, format!("${{1}}{}", REDACTED));
        consumer_token
            .replace_all(&value, format!("${{1}}{}", REDACTED))
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
