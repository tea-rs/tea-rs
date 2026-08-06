use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use serde_json::{Value, json};
use tea_protocol::{
    AgentErrorCode, CorrelationId, ProtocolError, ProtocolErrorEnvelope, RetryClass,
};

const CORRELATION_ID: &str = "0195a0b1-5e3b-7ef0-8ec1-0aa7aa000001";

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v1.0")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn unsupported_command_fixture_round_trips() {
    let value = fixture("error-unsupported-command.json");
    let envelope: ProtocolErrorEnvelope = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(envelope.error().code(), AgentErrorCode::UnsupportedCommand);
    assert_eq!(envelope.error().retry(), RetryClass::Never);
    assert_eq!(
        envelope.error().correlation_id(),
        Some(&CorrelationId::from_str(CORRELATION_ID).unwrap())
    );
    assert_eq!(serde_json::to_value(envelope).unwrap(), value);
}

#[test]
fn stable_codes_and_retry_classes_have_explicit_wire_values() {
    let cases = [
        (AgentErrorCode::UnsupportedCommand, "unsupported_command"),
        (AgentErrorCode::UnsupportedRecord, "unsupported_record"),
        (
            AgentErrorCode::UnsupportedProtocolVersion,
            "unsupported_protocol_version",
        ),
        (AgentErrorCode::InvalidCommand, "invalid_command"),
        (AgentErrorCode::InvalidInput, "invalid_input"),
        (AgentErrorCode::SequenceConflict, "sequence_conflict"),
        (AgentErrorCode::RateLimited, "rate_limited"),
        (AgentErrorCode::ProviderUnavailable, "provider_unavailable"),
        (AgentErrorCode::Cancelled, "cancelled"),
        (AgentErrorCode::Internal, "internal"),
    ];
    for (code, wire) in cases {
        assert_eq!(serde_json::to_value(code).unwrap(), wire);
    }

    assert_eq!(serde_json::to_value(RetryClass::Never).unwrap(), "never");
    assert_eq!(
        serde_json::to_value(RetryClass::Immediate).unwrap(),
        "immediate"
    );
    assert_eq!(
        serde_json::to_value(RetryClass::AfterBackoff).unwrap(),
        "after_backoff"
    );
}

#[test]
fn error_messages_and_safe_details_are_bounded() {
    let correlation = CorrelationId::from_str(CORRELATION_ID).unwrap();
    assert!(
        ProtocolError::new(AgentErrorCode::InvalidInput, "bad input", RetryClass::Never,)
            .unwrap()
            .with_correlation_id(correlation)
            .correlation_id()
            .is_some()
    );

    assert!(ProtocolError::new(AgentErrorCode::InvalidInput, "", RetryClass::Never,).is_err());
    assert!(
        ProtocolError::new(
            AgentErrorCode::Internal,
            "x".repeat(4097),
            RetryClass::Never,
        )
        .is_err()
    );

    let oversized = json!({
        "protocolVersion":"1.0",
        "type":"protocol_error",
        "error":{
            "code":"invalid_input",
            "message":"bad input",
            "retry":"never",
            "details":{
                "com.example": {"value":"x".repeat(20_000)}
            }
        }
    });
    assert!(serde_json::from_value::<ProtocolErrorEnvelope>(oversized).is_err());
}

#[test]
fn common_error_constructors_are_safe_and_canonical() {
    let correlation = CorrelationId::from_str(CORRELATION_ID).unwrap();
    let error = ProtocolError::unsupported_command(correlation);
    let value = serde_json::to_value(error).unwrap();

    assert_eq!(value["code"], "unsupported_command");
    assert_eq!(value["retry"], "never");
    assert_eq!(
        value["details"]["dev.tea-rs.protocol"]["supportedProtocol"],
        ">=1.0 <2.0"
    );
    assert!(value.get("source").is_none());
    assert!(value.get("stack").is_none());
}
