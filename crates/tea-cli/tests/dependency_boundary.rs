use std::collections::BTreeSet;

fn workspace_packages() -> Vec<serde_json::Value> {
    let metadata = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .unwrap();
    assert!(metadata.status.success());
    serde_json::from_slice::<serde_json::Value>(&metadata.stdout).unwrap()["packages"]
        .as_array()
        .unwrap()
        .clone()
}

fn internal_dependencies(package: &serde_json::Value) -> BTreeSet<&str> {
    package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|dependency| dependency["kind"].is_null())
        .filter_map(|dependency| dependency["name"].as_str())
        .filter(|name| name.starts_with("tea"))
        .collect()
}

#[test]
fn product_crates_follow_the_declared_dependency_direction() {
    let packages = workspace_packages();
    let expected = [
        (
            "tea-coding-tools",
            BTreeSet::from([
                "tea-control",
                "tea-model",
                "tea-protocol",
                "tea-provider-http",
                "tea-tools",
            ]),
        ),
        (
            "tea-coding",
            BTreeSet::from([
                "tea-rs",
                "tea-coding-tools",
                "tea-context",
                "tea-kernel",
                "tea-mcp",
                "tea-model",
                "tea-policy",
                "tea-profile",
                "tea-protocol",
                "tea-provider-openai",
                "tea-session",
                "tea-session-sqlite",
                "tea-tools",
            ]),
        ),
        (
            "tea-cli",
            BTreeSet::from([
                "tea-rs",
                "tea-coding",
                "tea-coding-tools",
                "tea-mcp",
                "tea-model",
                "tea-policy",
                "tea-protocol",
                "tea-provider-anthropic",
                "tea-provider-http",
                "tea-provider-openai",
                "tea-session",
                "tea-session-sqlite",
                "tea-tools",
            ]),
        ),
    ];
    for (name, dependencies) in expected {
        let package = packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap();
        assert_eq!(
            internal_dependencies(package),
            dependencies,
            "package {name}"
        );
    }

    for package in packages {
        let name = package["name"].as_str().unwrap();
        if matches!(name, "tea-cli" | "tea-coding" | "tea-coding-tools") {
            continue;
        }
        assert!(
            internal_dependencies(&package).is_disjoint(&BTreeSet::from([
                "tea-cli",
                "tea-coding",
                "tea-coding-tools",
            ])),
            "inward package {name} depends on a product crate"
        );
    }
}

#[test]
fn inward_crates_do_not_depend_on_cli_or_terminal_crates() {
    for package in workspace_packages() {
        let name = package["name"].as_str().unwrap();
        if name == "tea-cli" {
            continue;
        }
        let dependencies = package["dependencies"].as_array().unwrap();
        assert!(
            dependencies.iter().all(|dependency| {
                !matches!(
                    dependency["name"].as_str(),
                    Some("ratatui" | "crossterm" | "unicode-width" | "clap")
                )
            }),
            "inward package {name} depends on a CLI/TUI crate"
        );
    }
}
