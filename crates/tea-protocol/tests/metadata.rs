use std::collections::BTreeMap;

use serde_json::{Value, json};
use tea_protocol::{
    MAX_METADATA_BYTES, MAX_METADATA_DEPTH, MAX_METADATA_NAMESPACES, ProtocolMetadata,
};

#[test]
fn namespaced_metadata_round_trips_in_stable_order() {
    let metadata = ProtocolMetadata::from_entries([
        ("org.example", json!({"enabled": true})),
        ("com.example.renderer", json!({"renderer": "file_diff"})),
    ])
    .unwrap();

    assert_eq!(metadata.len(), 2);
    assert_eq!(metadata["com.example.renderer"]["renderer"], "file_diff");
    assert_eq!(
        serde_json::to_string(&metadata).unwrap(),
        r#"{"com.example.renderer":{"renderer":"file_diff"},"org.example":{"enabled":true}}"#
    );
    assert_eq!(
        serde_json::from_str::<ProtocolMetadata>(&serde_json::to_string(&metadata).unwrap())
            .unwrap(),
        metadata
    );
}

#[test]
fn invalid_namespaces_are_rejected() {
    for namespace in [
        "",
        "example",
        ".example",
        "com..example",
        "com.example.",
        "com._example.tool",
        "Com.example.tool",
        "com.example.tool!",
    ] {
        assert!(
            ProtocolMetadata::from_entries([(namespace, json!(true))]).is_err(),
            "accepted namespace {namespace:?}"
        );
    }
}

#[test]
fn metadata_limits_namespace_count_encoded_bytes_and_depth() {
    let too_many: BTreeMap<String, Value> = (0..=MAX_METADATA_NAMESPACES)
        .map(|index| (format!("com.example.key{index}"), json!(index)))
        .collect();
    assert!(ProtocolMetadata::try_from(too_many).is_err());

    let oversized = "x".repeat(MAX_METADATA_BYTES);
    assert!(ProtocolMetadata::from_entries([("com.example.large", json!(oversized))]).is_err());

    let mut nested = json!(true);
    for _ in 0..=MAX_METADATA_DEPTH {
        nested = json!({"child": nested});
    }
    assert!(ProtocolMetadata::from_entries([("com.example.deep", nested)]).is_err());
}

#[test]
fn empty_metadata_is_valid() {
    let metadata = ProtocolMetadata::default();
    assert!(metadata.is_empty());
    assert_eq!(serde_json::to_string(&metadata).unwrap(), "{}");
}
