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

fn package<'a>(packages: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    packages
        .iter()
        .find(|package| package["name"] == name)
        .unwrap_or_else(|| panic!("workspace package {name} is missing"))
}

fn direct_dependency_names(package: &serde_json::Value) -> BTreeSet<&str> {
    package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|dependency| dependency["kind"].is_null())
        .filter_map(|dependency| dependency["name"].as_str())
        .collect()
}

fn direct_internal_dependencies(package: &serde_json::Value) -> BTreeSet<&str> {
    direct_dependency_names(package)
        .into_iter()
        .filter(|name| name.starts_with("tea-"))
        .collect()
}

#[test]
fn facade_depends_only_on_the_core_contract_tier() {
    let packages = workspace_packages();
    let facade = package(&packages, "tea-rs");

    assert_eq!(
        direct_internal_dependencies(facade),
        BTreeSet::from([
            "tea-context",
            "tea-control",
            "tea-kernel",
            "tea-model",
            "tea-policy",
            "tea-profile",
            "tea-protocol",
            "tea-session",
            "tea-tools",
        ])
    );
}

#[test]
fn mcp_adapter_depends_only_on_declared_contract_tier() {
    let packages = workspace_packages();
    let mcp = package(&packages, "tea-mcp");

    assert_eq!(
        direct_internal_dependencies(mcp),
        BTreeSet::from(["tea-control", "tea-protocol", "tea-tools"])
    );
}

#[test]
fn core_contract_crates_do_not_depend_on_adapters_or_host_transports() {
    let packages = workspace_packages();
    let core_contracts = [
        "tea-protocol",
        "tea-control",
        "tea-model",
        "tea-tools",
        "tea-policy",
        "tea-session",
        "tea-context",
        "tea-profile",
        "tea-kernel",
        "tea-rs",
    ];
    let forbidden_internal = BTreeSet::from([
        "tea-cli",
        "tea-coding",
        "tea-coding-tools",
        "tea-mcp",
        "tea-provider-openai",
        "tea-session-sqlite",
        "tea-testkit",
    ]);
    let forbidden_external = BTreeSet::from([
        "clap",
        "crossterm",
        "nix",
        "portable-pty",
        "ratatui",
        "reqwest",
        "rmcp",
        "rusqlite",
        "unicode-width",
    ]);

    for name in core_contracts {
        let dependencies = direct_dependency_names(package(&packages, name));
        assert!(
            dependencies.is_disjoint(&forbidden_internal),
            "core contract crate {name} depends on an adapter or product crate"
        );
        assert!(
            dependencies.is_disjoint(&forbidden_external),
            "core contract crate {name} depends on a host transport implementation"
        );
    }
}
