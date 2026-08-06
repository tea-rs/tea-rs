use crate::common;

use std::fs;

use serde_json::json;
use tea_control::CancellationScope;
use tea_tools::ToolExecutionEvent;

use common::{TestDirectory, invocation, write_registry};

#[test]
fn traversal_and_replaced_parent_symlinks_fail_closed() {
    let temp = TestDirectory::new();
    let workspace = temp.workspace();

    assert!(
        workspace.resolve_existing("../outside-secret").is_err(),
        "parent traversal must fail before filesystem access"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let parent = temp.path().join("parent");
        let outside = temp.path().with_extension("outside");
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        let target = workspace.resolve_mutation("parent/new.txt").unwrap();

        fs::remove_dir(&parent).unwrap();
        symlink(&outside, &parent).unwrap();
        assert!(workspace.revalidate_mutation(&target).is_err());
        assert!(!outside.join("new.txt").exists());

        fs::remove_dir_all(outside).unwrap();
    }
}

#[cfg(unix)]
mod unix_tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt as _;
    use tea_coding_tools::{BashConfig, BashOutputDirectory, BashShell, BashTool};
    use tea_tools::{StaticResourceResolver, ToolRegistry};

    use super::*;
    use common::execute;

    fn bash_registry(temp: &TestDirectory) -> ToolRegistry {
        let config = BashConfig::new(
            BashShell::new("/bin/sh", "-c").unwrap(),
            BashOutputDirectory::new(temp.path()).unwrap(),
            Duration::from_secs(5),
        )
        .unwrap();
        let mut registry = ToolRegistry::new();
        registry
            .register(
                BashTool::spec().unwrap(),
                Arc::new(StaticResourceResolver::new([]).unwrap()),
                Arc::new(BashTool::new(temp.workspace(), config)),
            )
            .unwrap();
        registry
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_shell_bytes_are_canonical_and_json_framing_safe() {
        let temp = TestDirectory::new();
        let events = execute(
            &bash_registry(&temp),
            "bash",
            json!({"command":r"printf '\377\000\033[31m'"}),
        )
        .await;
        let result = match events.last().unwrap() {
            ToolExecutionEvent::Finished(result) => result,
            event => panic!("unexpected terminal event: {event:?}"),
        };
        let stdout = result.output()["stdout"].as_str().unwrap();
        assert!(stdout.contains('\u{fffd}'));
        assert!(!stdout.contains('\0'));
        assert!(stdout.contains('\u{1b}'));

        let encoded = serde_json::to_vec(result.output()).unwrap();
        assert!(!encoded.contains(&0));
        assert!(!encoded.contains(&0x1b));
        assert!(serde_json::from_slice::<serde_json::Value>(&encoded).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn created_files_and_output_spills_are_owner_only() {
        let temp = TestDirectory::new();
        let write = write_registry(temp.workspace());
        let events = write
            .execute(
                invocation("write", json!({"path":"private.txt","content":"secret"})),
                CancellationScope::new(),
            )
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(matches!(
            events.as_slice(),
            [ToolExecutionEvent::Finished(_)]
        ));
        let mode = fs::metadata(temp.path().join("private.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "new atomic files must not be group/world accessible"
        );

        let events = execute(
            &bash_registry(&temp),
            "bash",
            json!({"command":"yes x | head -c 20000"}),
        )
        .await;
        let result = match events.last().unwrap() {
            ToolExecutionEvent::Finished(result) => result,
            event => panic!("unexpected terminal event: {event:?}"),
        };
        assert_eq!(result.output()["truncated"], true);
        assert!(result.output()["stdout"].as_str().unwrap().len() <= 16 * 1024);
        let reference = result.output()["overflowReference"].as_str().unwrap();
        assert!(!reference.contains(['/', '\\']));
        let spill_mode = fs::metadata(temp.path().join(reference))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            spill_mode & 0o077,
            0,
            "spill files must not be group/world accessible"
        );
    }
}
