use apollo_cli::redaction::{Redactor, Sensitive};
use serde_json::json;

#[test]
fn redacts_token_like_text_and_authorization_headers() {
    let redactor = Redactor;

    assert_eq!(
        redactor.redact_text("Authorization: Bearer secret-token"),
        "Authorization: Bearer [REDACTED]"
    );
    assert_eq!(
        redactor.redact_text("Authorization: secret-token"),
        "Authorization: [REDACTED]"
    );
    assert_eq!(
        redactor.redact_text("consumer token secret-token"),
        "consumer token [REDACTED]"
    );
    assert_eq!(
        redactor.redact_text(r#"{"token":"secret-token","message":"visible"}"#),
        r#"{"token":"[REDACTED]","message":"visible"}"#
    );
}

#[test]
fn redacts_nested_token_like_json_fields() {
    let redactor = Redactor;
    let redacted = redactor.redact_json(json!({
        "profile": "dev",
        "token": "secret-token",
        "nested": {
            "authorization": "Bearer secret-token",
            "value": "visible"
        }
    }));

    assert_eq!(redacted["profile"], "dev");
    assert_eq!(redacted["token"], "[REDACTED]");
    assert_eq!(redacted["nested"]["authorization"], "[REDACTED]");
    assert_eq!(redacted["nested"]["value"], "visible");
}

#[test]
fn debug_output_for_sensitive_values_is_redacted() {
    let token = Sensitive::new("secret-token");
    assert_eq!(format!("{:?}", token), "[REDACTED]");
}
