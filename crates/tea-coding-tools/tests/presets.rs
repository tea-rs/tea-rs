use tea_coding_tools::{WorkspaceRoot, read_only_workspace_tools};

#[test]
fn read_only_workspace_preset_has_stable_tool_order() {
    let workspace = WorkspaceRoot::new(std::env::current_dir().unwrap()).unwrap();
    let tools = read_only_workspace_tools(&workspace).unwrap();
    let names = tools
        .iter()
        .map(|(spec, _, _)| spec.name().as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, ["read", "grep", "find", "ls"]);
}
