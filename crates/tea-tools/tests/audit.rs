use std::str::FromStr;

use serde_json::json;
use tea_tools::{
    TOOL_AUDIT_METADATA_NAMESPACE, ToolAuditMetadata, ToolAuditResource, ToolEffect,
    ToolResourceAccess, ToolSource, ToolSourceKind, ToolTrust, ToolVersion,
};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn source() -> ToolSource {
    ToolSource::new(
        ToolSourceKind::Mcp,
        "workspace.files",
        ToolTrust::Workspace,
        DIGEST,
    )
    .unwrap()
}

#[test]
fn tool_source_is_canonical_bounded_and_strictly_serializable() {
    let source = source();
    assert_eq!(source.kind(), ToolSourceKind::Mcp);
    assert_eq!(source.source_id(), "workspace.files");
    assert_eq!(source.trust(), ToolTrust::Workspace);
    assert_eq!(source.descriptor_digest(), DIGEST);
    assert_eq!(
        serde_json::to_value(&source).unwrap(),
        json!({
            "kind":"mcp",
            "sourceId":"workspace.files",
            "trust":"workspace",
            "descriptorDigest":DIGEST
        })
    );
    assert_eq!(
        serde_json::from_value::<ToolSource>(serde_json::to_value(&source).unwrap()).unwrap(),
        source
    );

    for source_id in ["", "Workspace.files", "workspace/files", "workspace\nfiles"] {
        assert!(
            ToolSource::new(ToolSourceKind::Mcp, source_id, ToolTrust::Workspace, DIGEST).is_err()
        );
    }
    assert!(
        ToolSource::new(
            ToolSourceKind::Mcp,
            "x".repeat(257),
            ToolTrust::Workspace,
            DIGEST
        )
        .is_err()
    );
    for digest in [
        "a",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
    ] {
        assert!(
            ToolSource::new(
                ToolSourceKind::Mcp,
                "workspace.files",
                ToolTrust::Workspace,
                digest
            )
            .is_err()
        );
    }

    assert!(serde_json::from_str::<ToolSource>(
        r#"{"kind":"future","sourceId":"workspace.files","trust":"workspace","descriptorDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<ToolSource>(
        r#"{"kind":"mcp","sourceId":"workspace.files","trust":"future","descriptorDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<ToolSource>(
        r#"{"kind":"mcp","sourceId":"workspace.files","trust":"workspace","descriptorDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","extra":true}"#
    )
    .is_err());
    assert!(serde_json::from_str::<ToolSource>(
        r#"{"kind":"mcp","kind":"native","sourceId":"workspace.files","trust":"workspace","descriptorDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
    )
    .is_err());
}

#[test]
fn audit_metadata_is_canonical_redacted_and_namespaced() {
    let audit = ToolAuditMetadata::new(
        ToolVersion::from_str("1.2.3").unwrap(),
        source(),
        [ToolEffect::FsWrite, ToolEffect::FsRead, ToolEffect::FsRead],
        [
            ToolAuditResource::new(
                "file",
                "file:/workspace/notes.txt?[REDACTED]",
                ToolResourceAccess::Write,
            )
            .unwrap(),
            ToolAuditResource::new(
                "credential",
                "credential:[REDACTED]",
                ToolResourceAccess::Read,
            )
            .unwrap(),
        ],
    )
    .unwrap();

    assert_eq!(audit.effects(), &[ToolEffect::FsRead, ToolEffect::FsWrite]);
    assert_eq!(audit.resources().len(), 2);
    let metadata = audit.to_protocol_metadata().unwrap();
    assert_eq!(metadata.len(), 1);
    assert_eq!(
        metadata.get(TOOL_AUDIT_METADATA_NAMESPACE),
        Some(&json!({
            "toolVersion":"1.2.3",
            "source":{
                "kind":"mcp",
                "sourceId":"workspace.files",
                "trust":"workspace",
                "descriptorDigest":DIGEST
            },
            "effects":["fs.read","fs.write"],
            "resources":[
                {
                    "scheme":"credential",
                    "redactedPresentation":"credential:[REDACTED]",
                    "access":"read"
                },
                {
                    "scheme":"file",
                    "redactedPresentation":"file:/workspace/notes.txt?[REDACTED]",
                    "access":"write"
                }
            ]
        }))
    );
    let encoded = serde_json::to_string(&audit).unwrap();
    assert_eq!(
        serde_json::from_str::<ToolAuditMetadata>(&encoded).unwrap(),
        audit
    );
}

#[test]
fn audit_metadata_rejects_unbounded_or_ambiguous_json() {
    assert!(ToolAuditResource::new("Bad", "file:/safe", ToolResourceAccess::Read).is_err());
    assert!(ToolAuditResource::new("file", "", ToolResourceAccess::Read).is_err());
    assert!(
        ToolAuditResource::new("file", "file:/unsafe\npath", ToolResourceAccess::Read).is_err()
    );
    assert!(
        ToolAuditMetadata::new(ToolVersion::from_str("1.0.0").unwrap(), source(), [], []).is_err()
    );
    assert!(
        serde_json::from_str::<ToolAuditResource>(
            r#"{"scheme":"file","redactedPresentation":"file:/safe","access":"read","extra":true}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<ToolAuditResource>(
        r#"{"scheme":"file","scheme":"secret","redactedPresentation":"file:/safe","access":"read"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<ToolAuditMetadata>(&format!(
        r#"{{"toolVersion":"1.0.0","source":{{"kind":"mcp","sourceId":"workspace.files","trust":"workspace","descriptorDigest":"{DIGEST}"}},"effects":["fs.read"],"resources":[],"extra":true}}"#
    ))
    .is_err());
    assert!(serde_json::from_str::<ToolAuditMetadata>(&format!(
        r#"{{"toolVersion":"1.0.0","toolVersion":"1.0.1","source":{{"kind":"mcp","sourceId":"workspace.files","trust":"workspace","descriptorDigest":"{DIGEST}"}},"effects":["fs.read"],"resources":[]}}"#
    ))
    .is_err());
}
