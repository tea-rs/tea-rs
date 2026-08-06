#[cfg(unix)]
mod unix_tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt;
    use serde_json::json;
    use tea_coding_tools::{BashConfig, BashOutputDirectory, BashShell, BashTool};
    use tea_control::CancellationScope;
    use tea_tools::{
        StaticResourceResolver, ToolExecutionEvent, ToolExecutionFailureCode, ToolRegistry,
    };

    use crate::common::{TestDirectory, invocation};

    fn registry(temp: &TestDirectory) -> ToolRegistry {
        let config = BashConfig::new(
            BashShell::new("/bin/sh", "-c").unwrap(),
            BashOutputDirectory::new(temp.path()).unwrap(),
            Duration::from_secs(30),
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
    async fn cancellation_terminates_the_owned_process_group_and_descendant() {
        let temp = TestDirectory::new();
        let registry = registry(&temp);
        let cancellation = CancellationScope::new();
        let mut stream = registry
            .execute(
                invocation(
                    "bash",
                    json!({"command":"sleep 1 & echo $! > child.pid; echo ready; wait"}),
                ),
                cancellation.clone(),
            )
            .unwrap();
        let first = stream.next().await.unwrap();
        assert!(matches!(first, ToolExecutionEvent::Progress(_)));
        cancellation.cancel();
        let remaining = stream.collect::<Vec<_>>().await;
        match remaining.last().unwrap() {
            ToolExecutionEvent::Failed(failure) => {
                assert_eq!(failure.code(), ToolExecutionFailureCode::Cancelled);
                assert_eq!(
                    failure.details()["dev.tea-rs.coding-tools"]["uncertain"],
                    true
                );
            }
            event => panic!("unexpected terminal: {event:?}"),
        }
        let pid = fs::read_to_string(temp.path().join("child.pid")).unwrap();
        let status = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "grandchild {pid} survived cancellation");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_the_stream_terminates_the_owned_process_group() {
        let temp = TestDirectory::new();
        let registry = registry(&temp);
        let mut stream = registry
            .execute(
                invocation(
                    "bash",
                    json!({"command":"sleep 1 & echo $! > dropped.pid; echo ready; wait"}),
                ),
                CancellationScope::new(),
            )
            .unwrap();
        assert!(matches!(
            stream.next().await,
            Some(ToolExecutionEvent::Progress(_))
        ));
        drop(stream);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let pid = fs::read_to_string(temp.path().join("dropped.pid")).unwrap();
        let status = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "grandchild {pid} survived stream drop");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pre_cancelled_bash_never_spawns() {
        let temp = TestDirectory::new();
        let registry = registry(&temp);
        let cancellation = CancellationScope::new();
        cancellation.cancel();
        let events = registry
            .execute(
                invocation("bash", json!({"command":"touch spawned"})),
                cancellation,
            )
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(
            matches!(events.as_slice(), [ToolExecutionEvent::Failed(failure)] if failure.code() == ToolExecutionFailureCode::Cancelled)
        );
        assert!(!temp.path().join("spawned").exists());
    }
}
