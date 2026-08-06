#[cfg(unix)]
mod unix_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt;
    use serde_json::json;
    use tea_coding_tools::{BashConfig, BashOutputDirectory, BashShell, BashTool};
    use tea_control::CancellationScope;
    use tea_tools::{StaticResourceResolver, ToolExecutionEvent, ToolRegistry};

    use crate::common::{TestDirectory, invocation};

    #[tokio::test(flavor = "current_thread")]
    async fn infinite_output_remains_bounded_until_cancellation() {
        let temp = TestDirectory::new();
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
        let cancellation = CancellationScope::new();
        let mut stream = registry
            .execute(
                invocation(
                    "bash",
                    json!({"command":"while :; do printf 0123456789; done"}),
                ),
                cancellation.clone(),
            )
            .unwrap();
        let mut progress = 0;
        while let Some(event) = stream.next().await {
            if matches!(event, ToolExecutionEvent::Progress(_)) {
                progress += 1;
            }
            if progress == 8 {
                cancellation.cancel();
            }
            if matches!(event, ToolExecutionEvent::Failed(_)) {
                break;
            }
        }
        assert_eq!(progress, 8);
        let total_spill = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("bash-output-")
            })
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>();
        assert_eq!(total_spill, 0, "cancelled output spill was not cleaned up");
    }
}
