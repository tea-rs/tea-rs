use std::str::FromStr as _;

use tea_protocol::ReasoningEffort;

#[test]
fn reasoning_effort_has_stable_wire_values_and_order() {
    let cases = [
        (ReasoningEffort::Off, "off"),
        (ReasoningEffort::Minimal, "minimal"),
        (ReasoningEffort::Low, "low"),
        (ReasoningEffort::Medium, "medium"),
        (ReasoningEffort::High, "high"),
        (ReasoningEffort::ExtraHigh, "xhigh"),
        (ReasoningEffort::Maximum, "max"),
    ];

    for (effort, wire) in cases {
        assert_eq!(effort.as_str(), wire);
        assert_eq!(effort.to_string(), wire);
        assert_eq!(ReasoningEffort::from_str(wire).unwrap(), effort);
        assert_eq!(
            serde_json::to_string(&effort).unwrap(),
            format!(r#""{wire}""#)
        );
        assert_eq!(
            serde_json::from_str::<ReasoningEffort>(&format!(r#""{wire}""#)).unwrap(),
            effort
        );
    }

    assert_eq!(ReasoningEffort::ALL, cases.map(|(effort, _)| effort));
    assert!(ReasoningEffort::High < ReasoningEffort::ExtraHigh);
    assert!(ReasoningEffort::ExtraHigh < ReasoningEffort::Maximum);
    assert!(ReasoningEffort::from_str("ultra").is_err());
}

#[test]
fn shortcut_levels_exclude_explicit_extended_levels() {
    assert_eq!(
        ReasoningEffort::SHORTCUT_LEVELS,
        [
            ReasoningEffort::Off,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ]
    );
}
