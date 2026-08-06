use crate::common;

use std::fs;
use std::sync::Arc;

use futures_util::StreamExt as _;
use serde_json::json;
use tea_coding_tools::{FindTool, GrepTool, LsTool, WorkspaceFileResourceResolver, WorkspaceRoot};
use tea_control::CancellationScope;
use tea_tools::{ToolExecutionFailureCode, ToolRegistry, ToolResourceAccess};

use common::{TestDirectory, execute, finished};

fn search_registry(workspace: WorkspaceRoot) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for (spec, executor) in [
        (
            GrepTool::spec().unwrap(),
            Arc::new(GrepTool::new(workspace.clone())) as Arc<dyn tea_tools::ToolExecutor>,
        ),
        (
            FindTool::spec().unwrap(),
            Arc::new(FindTool::new(workspace.clone())) as Arc<dyn tea_tools::ToolExecutor>,
        ),
        (
            LsTool::spec().unwrap(),
            Arc::new(LsTool::new(workspace)) as Arc<dyn tea_tools::ToolExecutor>,
        ),
    ] {
        registry
            .register(
                spec,
                Arc::new(WorkspaceFileResourceResolver::new(ToolResourceAccess::Read)),
                executor,
            )
            .unwrap();
    }
    registry
}

fn fixture() -> (TestDirectory, ToolRegistry) {
    let temp = TestDirectory::new();
    fs::create_dir_all(temp.path().join("src/nested")).unwrap();
    fs::write(
        temp.path().join("src/lib.rs"),
        "pub fn Tea() {}\nfn other() {}\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("src/nested/mod.rs"),
        "pub fn tea_leaf() {}\n",
    )
    .unwrap();
    fs::write(temp.path().join("README.md"), "tea docs\n").unwrap();
    let registry = search_registry(temp.workspace());
    (temp, registry)
}

#[tokio::test(flavor = "current_thread")]
async fn grep_returns_sorted_bounded_regex_matches() {
    let (_temp, registry) = fixture();
    let events = execute(
        &registry,
        "grep",
        json!({"pattern":"tea","glob":"*.rs","caseSensitive":false}),
    )
    .await;
    let result = finished(&events);
    let matches = result.output()["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["path"], "src/lib.rs");
    assert_eq!(matches[0]["line"], 1);
    assert_eq!(matches[0]["column"], 8);
    assert_eq!(matches[1]["path"], "src/nested/mod.rs");
    assert_eq!(result.output()["truncated"], false);
}

#[tokio::test(flavor = "current_thread")]
async fn find_matches_paths_and_honors_result_limit() {
    let (_temp, registry) = fixture();
    let events = execute(&registry, "find", json!({"pattern":"*.rs","limit":1})).await;
    let result = finished(&events);
    assert_eq!(result.output()["paths"], json!(["src/lib.rs"]));
    assert_eq!(result.output()["truncated"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn ls_lists_directory_entries_with_bounded_depth() {
    let (_temp, registry) = fixture();
    let events = execute(&registry, "ls", json!({"path":"src","depth":2})).await;
    let result = finished(&events);
    let entries = result.output()["entries"].as_array().unwrap();
    let paths = entries
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["src/lib.rs", "src/nested", "src/nested/mod.rs"]);
    assert_eq!(entries[1]["kind"], "directory");
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_patterns_fail_without_exposing_host_paths() {
    let (temp, registry) = fixture();
    for (name, arguments) in [
        ("grep", json!({"pattern":"["})),
        ("find", json!({"pattern":"["})),
    ] {
        let events = execute(&registry, name, arguments).await;
        let failure = common::failed(&events);
        assert!(
            !failure
                .message()
                .contains(temp.path().to_string_lossy().as_ref())
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn grep_accepts_a_file_as_the_search_root() {
    let (_temp, registry) = fixture();
    let events = execute(
        &registry,
        "grep",
        json!({"pattern":"tea","path":"README.md"}),
    )
    .await;
    let result = finished(&events);
    assert_eq!(result.output()["matches"][0]["path"], "README.md");
}

#[tokio::test(flavor = "current_thread")]
async fn nested_searches_honor_workspace_gitignore_rules() {
    let (temp, registry) = fixture();
    fs::write(temp.path().join(".gitignore"), "ignored.rs\n").unwrap();
    fs::write(temp.path().join("src/ignored.rs"), "hidden tea\n").unwrap();

    let grep = execute(
        &registry,
        "grep",
        json!({"pattern":"hidden tea","path":"src"}),
    )
    .await;
    assert!(
        finished(&grep).output()["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let find = execute(
        &registry,
        "find",
        json!({"pattern":"ignored.rs","path":"src"}),
    )
    .await;
    assert!(
        finished(&find).output()["paths"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn search_tools_do_not_follow_or_return_symlinks() {
    use std::os::unix::fs::symlink;

    let (temp, registry) = fixture();
    let outside = TestDirectory::new();
    fs::write(outside.path().join("secret.txt"), "outside secret\n").unwrap();
    symlink(outside.path(), temp.path().join("src/external")).unwrap();

    let grep = execute(&registry, "grep", json!({"pattern":"outside secret"})).await;
    assert!(
        finished(&grep).output()["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let find = execute(&registry, "find", json!({"pattern":"external"})).await;
    assert!(
        finished(&find).output()["paths"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let ls = execute(&registry, "ls", json!({"path":"src","depth":2})).await;
    assert!(
        finished(&ls).output()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["path"] != "src/external")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn pre_cancelled_search_stops_before_traversal() {
    let (_temp, registry) = fixture();
    let cancellation = CancellationScope::new();
    cancellation.cancel();
    let events = registry
        .execute(
            common::invocation("grep", json!({"pattern":"tea"})),
            cancellation,
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        common::failed(&events).code(),
        ToolExecutionFailureCode::Cancelled
    );
}
