use serde_json::json;
use tea_protocol::{
    CodeChange, CodeChangeHunk, CodeChangeKind, CodeChangeLine, CodeChangeLineKind,
    CodeChangeTruncation, MAX_WEB_FETCH_BODY_BYTES, MAX_WEB_FETCH_REDIRECTS, ToolPresentation,
    WebFetchPresentation, WebFetchRedirect, WebFetchTruncation,
};

fn change() -> CodeChange {
    let lines = vec![
        CodeChangeLine::new(
            CodeChangeLineKind::Context,
            Some(1),
            Some(1),
            "fn answer() {",
        )
        .unwrap(),
        CodeChangeLine::new(CodeChangeLineKind::Deletion, Some(2), None, "    1").unwrap(),
        CodeChangeLine::new(CodeChangeLineKind::Addition, None, Some(2), "    2").unwrap(),
        CodeChangeLine::new(CodeChangeLineKind::Context, Some(3), Some(3), "}").unwrap(),
    ];
    CodeChange::new(
        "src/lib.rs",
        CodeChangeKind::Update,
        vec![CodeChangeHunk::new(1, 3, 1, 3, lines).unwrap()],
        false,
        None,
        Some("--- src/lib.rs\n+++ src/lib.rs\n@@ -1,3 +1,3 @@\n".to_owned()),
        Some(2),
    )
    .unwrap()
}

#[test]
fn code_change_round_trips_with_typed_line_numbers_and_patch() {
    let change = change();
    let presentation = ToolPresentation::CodeChange(change.clone());
    let value = serde_json::to_value(&presentation).unwrap();
    assert_eq!(value["type"], "code_change");
    assert_eq!(value["value"]["hunks"][0]["lines"][1]["kind"], "deletion");
    assert_eq!(value["value"]["hunks"][0]["lines"][2]["newLine"], 2);
    assert_eq!(
        serde_json::from_value::<ToolPresentation>(value).unwrap(),
        presentation
    );
}

#[test]
fn code_change_rejects_inconsistent_truncation_and_unknown_fields() {
    let value = json!({
        "path":"src/lib.rs",
        "kind":"update",
        "hunks":[],
        "truncated":false,
        "truncation":"lines"
    });
    assert!(serde_json::from_value::<CodeChange>(value).is_err());

    let mut value = serde_json::to_value(change()).unwrap();
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<CodeChange>(value).is_err());

    let mut value = serde_json::to_value(change()).unwrap();
    value["hunks"][0]["oldLines"] = json!(9);
    assert!(serde_json::from_value::<CodeChange>(value).is_err());
}

#[test]
fn code_change_line_bounds_and_truncation_reason_are_explicit() {
    assert!(
        CodeChangeLine::new(
            CodeChangeLineKind::Addition,
            None,
            Some(1),
            "x".repeat(1_025),
        )
        .is_err()
    );
    let truncated = CodeChange::new(
        "src/lib.rs",
        CodeChangeKind::Update,
        Vec::new(),
        true,
        Some(CodeChangeTruncation::Lines),
        None,
        None,
    )
    .unwrap();
    assert!(truncated.truncated());
    assert_eq!(truncated.truncation(), Some(CodeChangeTruncation::Lines));
}

fn web_fetch() -> WebFetchPresentation {
    WebFetchPresentation::new(
        "https://Example.COM:443/start#ignored",
        "https://example.com/final",
        "TEXT/PLAIN; charset=UTF-8",
        "bounded extracted body",
    )
    .unwrap()
    .with_title("Fetched document")
    .unwrap()
    .with_truncation(WebFetchTruncation::BodyCharacters)
    .with_redirects(vec![
        WebFetchRedirect::new(
            "https://example.com/start",
            "https://example.com/final",
            302,
        )
        .unwrap(),
    ])
    .unwrap()
}

#[test]
fn web_fetch_presentation_round_trips_normalized_json_without_opaque_state() {
    let fetch = web_fetch();
    assert_eq!(fetch.requested_url(), "https://example.com/start");
    assert_eq!(fetch.mime_type(), "text/plain; charset=utf-8");
    let presentation = ToolPresentation::WebFetch(Box::new(fetch.clone()));
    let value = serde_json::to_value(&presentation).unwrap();
    assert_eq!(value["type"], "web_fetch");
    assert_eq!(value["value"]["truncation"], "body_characters");
    assert_eq!(value["value"]["redirects"][0]["status"], 302);
    assert!(value["value"].get("continuation").is_none());
    assert_eq!(
        serde_json::from_value::<ToolPresentation>(value).unwrap(),
        presentation
    );

    let debug = format!("{fetch:?}");
    assert!(!debug.contains("example.com"));
    assert!(!debug.contains("bounded extracted body"));
}

#[test]
fn web_fetch_presentation_rejects_unbounded_or_provider_owned_fields() {
    let mut opaque =
        serde_json::to_value(ToolPresentation::WebFetch(Box::new(web_fetch()))).unwrap();
    opaque["value"]["continuation"] = json!({
        "provider":"opaque",
        "payload":{"secret":"MUST_NOT_PERSIST"}
    });
    assert!(serde_json::from_value::<ToolPresentation>(opaque).is_err());

    let mut oversized =
        serde_json::to_value(ToolPresentation::WebFetch(Box::new(web_fetch()))).unwrap();
    oversized["value"]["body"] = json!("x".repeat(MAX_WEB_FETCH_BODY_BYTES + 1));
    assert!(serde_json::from_value::<ToolPresentation>(oversized).is_err());

    let mut redirects =
        serde_json::to_value(ToolPresentation::WebFetch(Box::new(web_fetch()))).unwrap();
    redirects["value"]["redirects"] = json!(
        (0..=MAX_WEB_FETCH_REDIRECTS)
            .map(|_| json!({
                "from":"https://example.com/a",
                "to":"https://example.com/b",
                "status":302
            }))
            .collect::<Vec<_>>()
    );
    assert!(serde_json::from_value::<ToolPresentation>(redirects).is_err());
}
