use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::OutputFormat;
use crate::mutation::MutationPlan;
use crate::redaction::Redactor;

#[derive(Debug, Eq, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Eq, PartialEq)]
pub struct RenderedOutput {
    pub stream: OutputStream,
    pub body: String,
}

impl RenderedOutput {
    pub fn stdout(body: String) -> Self {
        Self {
            stream: OutputStream::Stdout,
            body: ensure_trailing_newline(body),
        }
    }
}

pub struct OutputWriter {
    format: OutputFormat,
}

impl OutputWriter {
    pub fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    pub fn render_error(&self, error: &StructuredError) -> RenderedOutput {
        let redactor = Redactor;
        match self.format {
            OutputFormat::Json => {
                let value = redactor.redact_json(json!({ "error": error }));
                RenderedOutput {
                    stream: OutputStream::Stderr,
                    body: ensure_trailing_newline(
                        serde_json::to_string_pretty(&value)
                            .expect("structured error json serialization"),
                    ),
                }
            }
            OutputFormat::Table => {
                let mut lines = vec![error.message.clone()];
                if let Some(operation) = &error.operation {
                    lines.push(operation.render_table());
                }
                if let Some(command) = &error.command {
                    lines.push(format!("Command: {}", command));
                }
                RenderedOutput {
                    stream: OutputStream::Stderr,
                    body: ensure_trailing_newline(redactor.redact_text(&lines.join("\n"))),
                }
            }
        }
    }

    pub fn render_success<T: Serialize>(&self, value: &T, table_body: String) -> RenderedOutput {
        let redactor = Redactor;
        match self.format {
            OutputFormat::Json => {
                let value =
                    serde_json::to_value(value).expect("structured success json serialization");
                let value = redactor.redact_json(value);
                RenderedOutput::stdout(
                    serde_json::to_string_pretty(&value)
                        .expect("structured success json serialization"),
                )
            }
            OutputFormat::Table => RenderedOutput::stdout(redactor.redact_text(&table_body)),
        }
    }

    pub fn render_mutation_success<T: Serialize>(
        &self,
        operation: &MutationPlan,
        value: &T,
        table_body: String,
    ) -> RenderedOutput {
        if self.format == OutputFormat::Table {
            return self.render_success(value, table_body);
        }

        let mut value = serde_json::to_value(value).expect("structured success json serialization");
        let operation = serde_json::to_value(operation).expect("mutation plan json serialization");
        match &mut value {
            Value::Object(fields) => {
                fields.insert("operation".to_owned(), operation);
            }
            value => {
                *value = json!({
                    "operation": operation,
                    "data": value.take(),
                });
            }
        }
        let redactor = Redactor;
        let value = redactor.redact_json(value);
        RenderedOutput::stdout(
            serde_json::to_string_pretty(&value)
                .expect("structured mutation success json serialization"),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StructuredError {
    pub code: &'static str,
    pub category: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<MutationPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

fn ensure_trailing_newline(mut body: String) -> String {
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body
}
