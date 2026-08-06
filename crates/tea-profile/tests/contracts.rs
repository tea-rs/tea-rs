//! Public contract tests for the versioned profile schema and stable errors.

use std::str::FromStr;

use tea_profile::{
    CURRENT_PROFILE_SCHEMA_VERSION, ProfileError, ProfileErrorCode, ProfileSchemaVersion,
};

#[test]
fn current_schema_version_is_supported_and_round_trips() {
    let version = ProfileSchemaVersion::current();
    assert!(version.is_supported());
    assert_eq!(version, CURRENT_PROFILE_SCHEMA_VERSION);
    let text = version.to_string();
    let parsed = ProfileSchemaVersion::from_str(&text).unwrap();
    assert_eq!(parsed, version);
}

#[test]
fn unsupported_schema_version_is_rejected() {
    // A future major version is not supported by this crate version.
    let future = ProfileSchemaVersion::from_str("999.0.0").unwrap();
    assert!(!future.is_supported());
    assert!(matches!(
        ProfileError::from_unsupported(future).code(),
        ProfileErrorCode::InvalidVersion
    ));
}

#[test]
fn malformed_schema_version_is_rejected() {
    assert!(ProfileSchemaVersion::from_str("not-a-version").is_err());
    assert!(ProfileSchemaVersion::from_str("1").is_err());
    assert!(ProfileSchemaVersion::from_str("1.0").is_err());
}

#[test]
fn error_codes_are_stable_discriminants() {
    for code in [
        ProfileErrorCode::InvalidVersion,
        ProfileErrorCode::InvalidSelector,
        ProfileErrorCode::BoundsExceeded,
        ProfileErrorCode::DuplicateEntry,
        ProfileErrorCode::CompositionConflict,
        ProfileErrorCode::UnsupportedValue,
    ] {
        let error = ProfileError::new(code, "example");
        assert_eq!(error.code(), code);
        assert!(!error.message().is_empty());
        assert!(!error.message().contains('\0'));
    }
}

#[test]
fn error_message_is_bounded_and_null_free() {
    let long = "x".repeat(8192);
    let error = ProfileError::new(ProfileErrorCode::BoundsExceeded, long);
    assert!(error.message().len() <= 4096);
    assert!(!error.message().is_empty());
}
