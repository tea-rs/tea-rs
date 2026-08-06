use serde_json::json;
use tea_policy::{PolicyRedactor, RedactionError};

#[test]
fn nested_sensitive_keys_are_redacted_case_insensitively() {
    let input = json!({
        "path":"/workspace/file",
        "apiKey":"secret-1",
        "nested": {
            "Authorization":"Bearer token",
            "password":"secret-2",
            "safe":"visible"
        },
        "array":[{"cookie":"secret-3"}, "visible"]
    });
    let original = input.clone();
    let redacted = PolicyRedactor.redact_arguments(&input).unwrap();

    assert_eq!(input, original);
    assert_eq!(redacted.value()["path"], "/workspace/file");
    assert_eq!(redacted.value()["apiKey"], "[REDACTED]");
    assert_eq!(redacted.value()["nested"]["Authorization"], "[REDACTED]");
    assert_eq!(redacted.value()["nested"]["safe"], "visible");
    assert_eq!(redacted.value()["array"][0]["cookie"], "[REDACTED]");
}

#[test]
fn key_normalization_covers_common_secret_variants() {
    let input = json!({
        "api_key":"a",
        "private-key":"b",
        "accessToken":"c",
        "refresh_token":"d",
        "clientSecret":"e"
    });
    let redacted = PolicyRedactor.redact_arguments(&input).unwrap();
    assert!(
        redacted
            .value()
            .as_object()
            .unwrap()
            .values()
            .all(|value| value == "[REDACTED]")
    );
}

#[test]
fn redaction_output_remains_bounded_and_utf8_safe() {
    let oversized = json!({"safe":"界".repeat(100_000)});
    assert_eq!(
        PolicyRedactor.redact_arguments(&oversized).unwrap_err(),
        RedactionError::OutputOutOfBounds
    );

    let mut nested = json!({"value":true});
    for _ in 0..40 {
        nested = json!({"next":nested});
    }
    assert_eq!(
        PolicyRedactor.redact_arguments(&nested).unwrap_err(),
        RedactionError::OutputOutOfBounds
    );
}

#[test]
fn resource_presentation_redacts_sensitive_schemes_and_query_values() {
    let redactor = PolicyRedactor;
    assert_eq!(
        redactor.redact_resource("credential", "vault://production/token"),
        "credential:[REDACTED]"
    );
    assert_eq!(
        redactor.redact_resource("url", "https://example.test/path?token=secret"),
        "url:https://example.test/path?[REDACTED]"
    );
    assert_eq!(
        redactor.redact_resource("file", "/workspace/notes.txt"),
        "file:/workspace/notes.txt"
    );
}
