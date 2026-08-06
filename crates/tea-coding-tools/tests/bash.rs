#[cfg(unix)]
mod unix_tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use tea_coding_tools::{BashConfig, BashOutputDirectory, BashShell, BashTool};
    use tea_tools::{
        StaticResourceResolver, ToolExecutionEvent, ToolExecutionFailureCode, ToolRegistry,
        ToolResourceAccess,
    };

    use crate::common::{TestDirectory, execute};

    fn registry(temp: &TestDirectory, timeout: Duration) -> ToolRegistry {
        let shell = BashShell::new("/bin/sh", "-c").unwrap();
        let output = BashOutputDirectory::new(temp.path()).unwrap();
        let config = BashConfig::new(shell, output, timeout).unwrap();
        let mut registry = ToolRegistry::new();
        registry
            .register(
                BashTool::spec().unwrap(),
                Arc::new(
                    StaticResourceResolver::new([BashTool::workspace_resource().unwrap()]).unwrap(),
                ),
                Arc::new(BashTool::new(temp.workspace(), config)),
            )
            .unwrap();
        registry
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_captures_mixed_output_and_nonzero_status() {
        let temp = TestDirectory::new();
        let registry = registry(&temp, Duration::from_secs(5));
        let events = execute(
            &registry,
            "bash",
            json!({"command":"printf out; printf err >&2; exit 7"}),
        )
        .await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ToolExecutionEvent::Progress(_)))
        );
        let result = match events.last().unwrap() {
            ToolExecutionEvent::Finished(result) => result,
            event => panic!("unexpected terminal: {event:?}"),
        };
        assert_eq!(result.output()["stdout"], "out");
        assert_eq!(result.output()["stderr"], "err");
        assert_eq!(result.output()["exitCode"], 7);
        assert_eq!(result.output()["success"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_uses_workspace_and_spills_bounded_output_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDirectory::new();
        fs::write(temp.path().join("marker"), b"yes").unwrap();
        let registry = registry(&temp, Duration::from_secs(5));
        let events = execute(
            &registry,
            "bash",
            json!({"command":"pwd; cat marker; yes x | head -c 20000"}),
        )
        .await;
        let result = match events.last().unwrap() {
            ToolExecutionEvent::Finished(result) => result,
            event => panic!("unexpected terminal: {event:?}"),
        };
        assert!(result.output()["stdout"].as_str().unwrap().contains("yes"));
        assert_eq!(result.output()["truncated"], true);
        let reference = result.output()["overflowReference"].as_str().unwrap();
        assert!(!reference.contains('/'));
        let metadata = fs::metadata(temp.path().join(reference)).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(result.output()["stdout"].as_str().unwrap().len() <= 16 * 1024);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bash_timeout_is_uncertain_and_terminal() {
        let temp = TestDirectory::new();
        let registry = registry(&temp, Duration::from_millis(50));
        let events = execute(&registry, "bash", json!({"command":"sleep 1"})).await;
        match events.last().unwrap() {
            ToolExecutionEvent::Failed(failure) => {
                assert_eq!(failure.code(), ToolExecutionFailureCode::ExecutionFailed);
                assert_eq!(
                    failure.details()["dev.tea-rs.coding-tools"]["code"],
                    "timeout"
                );
                assert_eq!(
                    failure.details()["dev.tea-rs.coding-tools"]["uncertain"],
                    true
                );
            }
            event => panic!("unexpected terminal: {event:?}"),
        }
    }

    #[test]
    fn bash_spec_is_non_retryable_serial_process_spawn() {
        use tea_protocol::ToolIdempotency;
        use tea_tools::{SchedulerClass, ToolEffect, ToolRetrySafety};
        let spec = BashTool::spec().unwrap();
        assert_eq!(spec.effects(), &[ToolEffect::ProcessSpawn]);
        assert_eq!(spec.scheduler_class(), SchedulerClass::Serial);
        assert_eq!(
            spec.execution().idempotency(),
            ToolIdempotency::NonIdempotent
        );
        assert_eq!(spec.execution().retry_safety(), ToolRetrySafety::Never);
        let resource = BashTool::workspace_resource().unwrap();
        assert_eq!(resource.locator(), "/workspace");
        assert_eq!(resource.access(), ToolResourceAccess::Execute);
    }
}
