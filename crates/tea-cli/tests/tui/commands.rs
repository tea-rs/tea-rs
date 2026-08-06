use tea_cli::tui::{CommandCatalog, SlashCommand};
use tea_protocol::ReasoningEffort;

#[test]
fn builtins_templates_and_skills_parse_without_prefix_guessing() {
    let catalog = CommandCatalog::new(["review"], ["rust-check"]).unwrap();
    assert_eq!(catalog.parse("/new").unwrap(), SlashCommand::New);
    assert_eq!(catalog.parse("/mcp").unwrap(), SlashCommand::Mcp);
    assert_eq!(
        catalog.parse("/mcp reconnect fixture").unwrap(),
        SlashCommand::McpReconnect("fixture".parse().unwrap())
    );
    assert_eq!(
        catalog.parse("/model fake/model").unwrap(),
        SlashCommand::Model(Some("fake/model".parse().unwrap()))
    );
    assert_eq!(
        catalog.parse("/reasoning").unwrap(),
        SlashCommand::Reasoning(None)
    );
    for effort in ReasoningEffort::ALL {
        assert_eq!(
            catalog
                .parse(&format!("/reasoning {}", effort.as_str()))
                .unwrap(),
            SlashCommand::Reasoning(Some(effort))
        );
    }
    assert_eq!(
        catalog.parse("/review src/lib.rs").unwrap(),
        SlashCommand::Template {
            name: "review".to_owned(),
            arguments: vec!["src/lib.rs".to_owned()],
        }
    );
    assert_eq!(
        catalog.parse("/skill:rust-check --all").unwrap(),
        SlashCommand::Skill("/skill:rust-check --all".to_owned())
    );
    assert!(catalog.parse("/unknown").is_err());
    assert!(catalog.parse("/mcp reconnect").is_err());
    assert!(catalog.parse("/reasoning extreme").is_err());
    assert!(catalog.parse("/reasoning low high").is_err());
}

#[test]
fn completion_is_sorted_bounded_and_includes_declarative_resources() {
    let catalog = CommandCatalog::new(["review", "release"], ["rust-check"]).unwrap();
    assert_eq!(
        catalog.complete("/re", 8),
        ["/reasoning", "/release", "/resume", "/review"]
    );
    assert_eq!(catalog.complete("/skill:", 1), ["/skill:rust-check"]);
    assert_eq!(catalog.complete("/mc", 8), ["/mcp"]);
}

#[test]
fn image_commands_preserve_paths_and_reject_unbounded_arguments() {
    let catalog = CommandCatalog::new(Vec::<String>::new(), Vec::<String>::new()).unwrap();

    assert_eq!(
        catalog.parse("/image fixtures/my image.png").unwrap(),
        SlashCommand::Image("fixtures/my image.png".to_owned())
    );
    assert_eq!(
        catalog.parse("/image remove 4").unwrap(),
        SlashCommand::ImageRemove(4)
    );
    assert_eq!(
        catalog.parse("/image clear").unwrap(),
        SlashCommand::ImageClear
    );

    for input in [
        "/image",
        "/image remove",
        "/image remove 0",
        "/image remove 5",
        "/image remove 1 extra",
        "/image clear extra",
        "/image bad\npath.png",
    ] {
        assert!(catalog.parse(input).is_err(), "accepted {input:?}");
    }
    assert!(
        catalog
            .parse(&format!("/image {}", "x".repeat(4097)))
            .is_err()
    );
}
