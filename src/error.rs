use crate::cli::OutputFormat;
use crate::mutation::MutationPlan;
use crate::output::{OutputWriter, RenderedOutput, StructuredError};

#[derive(Debug)]
pub enum CliErrorKind {
    Parse {
        message: String,
    },
    InvalidConfig {
        path: String,
        message: String,
    },
    MissingConfigBase {
        message: String,
    },
    ConfirmationRequired {
        message: String,
        operation: Option<Box<MutationPlan>>,
    },
    CredentialStoreUnavailable {
        message: String,
    },
    InvalidInput {
        message: String,
    },
    AuthenticationRequired {
        message: String,
    },
    Network {
        path: String,
        message: String,
    },
    HttpStatus {
        status: u16,
        path: String,
        message: String,
    },
    ProfileNotFound {
        profile: String,
    },
    ProfileAlreadyExists {
        profile: String,
        command: String,
    },
}

#[derive(Debug)]
pub struct CliError {
    kind: CliErrorKind,
    format: OutputFormat,
}

impl CliError {
    pub fn parse(message: String, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::Parse { message },
            format,
        }
    }

    pub fn invalid_config(path: &std::path::Path, message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::InvalidConfig {
                path: path.display().to_string(),
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn missing_config_base(message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::MissingConfigBase {
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn profile_not_found(profile: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::ProfileNotFound {
                profile: profile.to_owned(),
            },
            format,
        }
    }

    pub fn profile_already_exists(profile: &str, command: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::ProfileAlreadyExists {
                profile: profile.to_owned(),
                command: command.to_owned(),
            },
            format,
        }
    }

    pub fn confirmation_required(message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::ConfirmationRequired {
                message: message.to_owned(),
                operation: None,
            },
            format,
        }
    }

    pub fn confirmation_required_with_plan(
        message: &str,
        operation: MutationPlan,
        format: OutputFormat,
    ) -> Self {
        Self {
            kind: CliErrorKind::ConfirmationRequired {
                message: message.to_owned(),
                operation: Some(Box::new(operation)),
            },
            format,
        }
    }

    pub fn credential_store_unavailable(message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::CredentialStoreUnavailable {
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn invalid_input(message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::InvalidInput {
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn authentication_required(message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::AuthenticationRequired {
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn network(path: &str, message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::Network {
                path: path.to_owned(),
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn http_status(status: u16, path: &str, message: &str, format: OutputFormat) -> Self {
        Self {
            kind: CliErrorKind::HttpStatus {
                status,
                path: path.to_owned(),
                message: message.to_owned(),
            },
            format,
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self.kind {
            CliErrorKind::Parse { .. } => 2,
            CliErrorKind::InvalidConfig { .. }
            | CliErrorKind::MissingConfigBase { .. }
            | CliErrorKind::ConfirmationRequired { .. }
            | CliErrorKind::CredentialStoreUnavailable { .. }
            | CliErrorKind::InvalidInput { .. }
            | CliErrorKind::AuthenticationRequired { .. }
            | CliErrorKind::Network { .. }
            | CliErrorKind::HttpStatus { .. }
            | CliErrorKind::ProfileNotFound { .. }
            | CliErrorKind::ProfileAlreadyExists { .. } => 1,
        }
    }

    pub fn http_status_code(&self) -> Option<u16> {
        match self.kind {
            CliErrorKind::HttpStatus { status, .. } => Some(status),
            _ => None,
        }
    }

    pub fn http_status_message(&self) -> Option<&str> {
        match &self.kind {
            CliErrorKind::HttpStatus { message, .. } => Some(message.as_str()),
            _ => None,
        }
    }

    pub fn render(&self) -> RenderedOutput {
        match &self.kind {
            CliErrorKind::Parse { message } => {
                OutputWriter::new(self.format).render_error(&StructuredError {
                    code: "parse_error",
                    category: "invalid_input",
                    message: message.clone(),
                    operation: None,
                    command: None,
                    follow_up_issue: Some(5631),
                    path: None,
                    profile: None,
                })
            }
            CliErrorKind::InvalidConfig { path, message } => OutputWriter::new(self.format)
                .render_error(&StructuredError {
                    code: "invalid_config",
                    category: "invalid_input",
                    message: format!("Invalid Apollo CLI config at {}: {}", path, message),
                    operation: None,
                    command: None,
                    follow_up_issue: None,
                    path: Some(path.clone()),
                    profile: None,
                }),
            CliErrorKind::MissingConfigBase { message } => OutputWriter::new(self.format)
                .render_error(&StructuredError {
                    code: "missing_config_base",
                    category: "invalid_input",
                    message: format!("Cannot resolve Apollo CLI config path: {}", message),
                    operation: None,
                    command: None,
                    follow_up_issue: None,
                    path: None,
                    profile: None,
                }),
            CliErrorKind::ConfirmationRequired { message, operation } => {
                OutputWriter::new(self.format).render_error(&StructuredError {
                    code: "confirmation_required",
                    category: "confirmation_required",
                    message: message.clone(),
                    operation: operation.as_deref().cloned(),
                    command: None,
                    follow_up_issue: Some(5626),
                    path: None,
                    profile: None,
                })
            }
            CliErrorKind::CredentialStoreUnavailable { message } => OutputWriter::new(self.format)
                .render_error(&StructuredError {
                    code: "credential_store_unavailable",
                    category: "unsupported_operation",
                    message: message.clone(),
                    operation: None,
                    command: Some("auth".to_owned()),
                    follow_up_issue: Some(5630),
                    path: None,
                    profile: None,
                }),
            CliErrorKind::InvalidInput { message } => {
                OutputWriter::new(self.format).render_error(&StructuredError {
                    code: "invalid_input",
                    category: "invalid_input",
                    message: message.clone(),
                    operation: None,
                    command: None,
                    follow_up_issue: None,
                    path: None,
                    profile: None,
                })
            }
            CliErrorKind::AuthenticationRequired { message } => OutputWriter::new(self.format)
                .render_error(&StructuredError {
                    code: "authentication_failed",
                    category: "authentication_failed",
                    message: message.clone(),
                    operation: None,
                    command: Some("auth".to_owned()),
                    follow_up_issue: Some(5630),
                    path: None,
                    profile: None,
                }),
            CliErrorKind::Network { path, message } => {
                OutputWriter::new(self.format).render_error(&StructuredError {
                    code: "network_error",
                    category: "network",
                    message: format!("OpenAPI request to {} failed: {}", path, message),
                    operation: None,
                    command: None,
                    follow_up_issue: None,
                    path: Some(path.clone()),
                    profile: None,
                })
            }
            CliErrorKind::HttpStatus {
                status,
                path,
                message,
            } => {
                let (code, category) = http_status_code_and_category(*status);
                OutputWriter::new(self.format).render_error(&StructuredError {
                    code,
                    category,
                    message: format!(
                        "OpenAPI request to {} returned HTTP {}: {}",
                        path, status, message
                    ),
                    operation: None,
                    command: None,
                    follow_up_issue: None,
                    path: Some(path.clone()),
                    profile: None,
                })
            }
            CliErrorKind::ProfileNotFound { profile } => OutputWriter::new(self.format)
                .render_error(&StructuredError {
                    code: "profile_not_found",
                    category: "not_found",
                    message: format!("Profile '{}' was not found.", profile),
                    operation: None,
                    command: Some("profile".to_owned()),
                    follow_up_issue: Some(5629),
                    path: None,
                    profile: Some(profile.clone()),
                }),
            CliErrorKind::ProfileAlreadyExists { profile, command } => {
                OutputWriter::new(self.format).render_error(&StructuredError {
                    code: "profile_already_exists",
                    category: "invalid_input",
                    message: format!(
                        "Profile '{}' already exists. Re-run with --overwrite to replace it.",
                        profile
                    ),
                    operation: None,
                    command: Some(command.clone()),
                    follow_up_issue: None,
                    path: None,
                    profile: Some(profile.clone()),
                })
            }
        }
    }
}

fn http_status_code_and_category(status: u16) -> (&'static str, &'static str) {
    match status {
        401 => ("authentication_failed", "authentication_failed"),
        403 => ("permission_denied", "permission_denied"),
        404 => ("not_found", "not_found"),
        409 => ("conflict", "conflict"),
        412 => ("precondition_failed", "precondition_failed"),
        400..=499 => ("invalid_input", "invalid_input"),
        _ => ("server_error", "server"),
    }
}
