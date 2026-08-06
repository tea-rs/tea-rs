use std::str::FromStr;

use serde_json::json;
use tea_policy::{
    ActorId, ExecutionSurface, GrantId, PolicyEnvironment, PolicyExecutionTarget, WorkspaceId,
};
use tea_protocol::ProtocolMetadata;

#[test]
fn policy_identifiers_are_bounded_and_canonical() {
    assert_eq!(
        ActorId::from_str("user:alice").unwrap().as_str(),
        "user:alice"
    );
    assert!(ActorId::from_str("").is_err());
    assert!(ActorId::from_str("User Alice").is_err());
    assert!(ActorId::from_str(&"a".repeat(129)).is_err());

    assert_eq!(
        WorkspaceId::from_str("workspace/main").unwrap().as_str(),
        "workspace/main"
    );
    assert!(WorkspaceId::from_str("../secret").is_err());
    assert!(WorkspaceId::from_str("workspace\nmain").is_err());
}

#[test]
fn grant_id_requires_canonical_uuid_v7() {
    let value = "0195a0b1-5e69-70ac-807e-0aa7aa000047";
    let id = GrantId::from_str(value).unwrap();
    assert_eq!(id.to_string(), value);
    assert!(GrantId::from_str("550e8400-e29b-41d4-a716-446655440000").is_err());
    assert_eq!(
        serde_json::from_str::<GrantId>(&format!("\"{value}\"")).unwrap(),
        id
    );
}

#[test]
fn environment_round_trips_with_safe_metadata() {
    let metadata = ProtocolMetadata::from_entries([(
        "com.example.environment".to_owned(),
        json!({"region":"test"}),
    )])
    .unwrap();
    let environment = PolicyEnvironment::new(
        ExecutionSurface::Desktop,
        PolicyExecutionTarget::Sandbox,
        metadata.clone(),
    );
    assert_eq!(environment.surface(), ExecutionSurface::Desktop);
    assert_eq!(environment.target(), PolicyExecutionTarget::Sandbox);
    assert_eq!(environment.metadata(), &metadata);
    let json = serde_json::to_string(&environment).unwrap();
    assert_eq!(
        serde_json::from_str::<PolicyEnvironment>(&json).unwrap(),
        environment
    );
}
