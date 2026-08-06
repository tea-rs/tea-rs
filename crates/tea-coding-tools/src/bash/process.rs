use std::process::Stdio;

use async_stream::stream;
use serde_json::json;
use tea_control::CancellationScope;
use tea_protocol::{ContentBlock, ProtocolMetadata};
use tea_tools::{
    BoxToolExecutionStream, ToolExecutionEvent, ToolExecutionFailure, ToolProgress, ToolResult,
};
use tokio::io::{AsyncReadExt, BufReader};

use super::command::BashConfig;
use super::output::{MAX_PROGRESS_EVENTS, OutputCapture, OutputKind};
use crate::{FileToolError, FileToolErrorCode, WorkspaceRoot};

const CHUNK_BYTES: usize = 8 * 1024;

#[allow(clippy::too_many_lines)] // One owner must select child, pipes, timeout, and cancellation.
pub(crate) fn execute_process(
    workspace: WorkspaceRoot,
    config: BashConfig,
    command_text: String,
    cancellation: CancellationScope,
) -> BoxToolExecutionStream {
    Box::pin(stream! {
        if cancellation.is_cancelled() {
            yield ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled());
            return;
        }
        let mut command = tokio::process::Command::new(config.shell().executable());
        command
            .arg(config.shell().command_argument())
            .arg(command_text)
            .current_dir(workspace.host_path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        super::platform::configure(&mut command);
        let Ok(mut child) = command.spawn() else {
            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), false);
            return;
        };
        let Some(process_id) = child.id() else {
            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
            return;
        };
        let mut process_guard = ProcessTreeGuard::new(process_id);
        let Some(stdout) = child.stdout.take() else {
            let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
            return;
        };
        let Some(stderr) = child.stderr.take() else {
            let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
            return;
        };
        let mut stdout = BufReader::new(stdout);
        let mut stderr = BufReader::new(stderr);
        let mut stdout_open = true;
        let mut stderr_open = true;
        let mut status = None;
        let mut capture = OutputCapture::new();
        let mut progress_events = 0_usize;
        let timeout = tokio::time::sleep(config.timeout());
        tokio::pin!(timeout);

        while status.is_none() || stdout_open || stderr_open {
            let mut stdout_buffer = [0_u8; CHUNK_BYTES];
            let mut stderr_buffer = [0_u8; CHUNK_BYTES];
            tokio::select! {
                () = cancellation.cancelled() => {
                    let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                    yield cancelled_after_spawn();
                    return;
                }
                () = &mut timeout => {
                    let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                    yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true)
                        .with_timeout_details();
                    return;
                }
                result = stdout.read(&mut stdout_buffer), if stdout_open => {
                    match result {
                        Ok(0) => stdout_open = false,
                        Ok(count) => {
                            if let Err(error) = capture.push(
                                OutputKind::Stdout,
                                &stdout_buffer[..count],
                                config.output_directory().path(),
                            ) {
                                let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                                yield failed(error, true);
                                return;
                            }
                            if progress_events < MAX_PROGRESS_EVENTS {
                                progress_events += 1;
                                yield progress("received stdout", count);
                            }
                        }
                        Err(_) => {
                            let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
                            return;
                        }
                    }
                }
                result = stderr.read(&mut stderr_buffer), if stderr_open => {
                    match result {
                        Ok(0) => stderr_open = false,
                        Ok(count) => {
                            if let Err(error) = capture.push(
                                OutputKind::Stderr,
                                &stderr_buffer[..count],
                                config.output_directory().path(),
                            ) {
                                let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                                yield failed(error, true);
                                return;
                            }
                            if progress_events < MAX_PROGRESS_EVENTS {
                                progress_events += 1;
                                yield progress("received stderr", count);
                            }
                        }
                        Err(_) => {
                            let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
                            return;
                        }
                    }
                }
                result = child.wait(), if status.is_none() => {
                    if let Ok(exit) = result {
                        status = Some(exit);
                    } else {
                        let _ = terminate_owned(&mut child, process_id, &mut process_guard).await;
                        yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
                        return;
                    }
                }
            }
        }

        let Some(status) = status else {
            yield failed(FileToolError::new(FileToolErrorCode::ProcessFailure), true);
            return;
        };
        let captured = match capture.finish() {
            Ok(captured) => captured,
            Err(error) => {
                yield failed(error, true);
                return;
            }
        };
        let exit_code = status.code();
        let text = render_model_text(&captured.stdout, &captured.stderr, exit_code);
        let output = json!({
            "stdout":captured.stdout,
            "stderr":captured.stderr,
            "exitCode":exit_code,
            "success":status.success(),
            "truncated":captured.truncated,
            "overflowReference":captured.overflow_reference
        });
        process_guard.disarm();
        let result = ContentBlock::text(text)
            .ok()
            .and_then(|content| ToolResult::new(vec![content], output).ok());
        match result {
            Some(result) => yield ToolExecutionEvent::Finished(result),
            None => yield failed(FileToolError::new(FileToolErrorCode::Internal), true),
        }
    })
}

async fn terminate_owned(
    child: &mut tokio::process::Child,
    process_id: u32,
    guard: &mut ProcessTreeGuard,
) -> Result<(), FileToolError> {
    let result = super::platform::terminate_tree(child, process_id).await;
    guard.disarm();
    result
}

struct ProcessTreeGuard {
    process_id: u32,
    armed: bool,
}

impl ProcessTreeGuard {
    const fn new(process_id: u32) -> Self {
        Self {
            process_id,
            armed: true,
        }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        if self.armed {
            super::platform::kill_tree_on_drop(self.process_id);
        }
    }
}

fn progress(message: &str, bytes: usize) -> ToolExecutionEvent {
    ToolProgress::new(message, u64::try_from(bytes).unwrap_or(u64::MAX), None).map_or_else(
        |_| failed(FileToolError::new(FileToolErrorCode::Internal), true),
        ToolExecutionEvent::Progress,
    )
}

fn render_model_text(stdout: &str, stderr: &str, exit_code: Option<i32>) -> String {
    let stdout = if stdout.is_empty() { "(empty)" } else { stdout };
    let stderr = if stderr.is_empty() { "(empty)" } else { stderr };
    format!("exit code: {exit_code:?}\nstdout:\n{stdout}\nstderr:\n{stderr}")
}

fn failed(error: FileToolError, spawned: bool) -> ToolExecutionEvent {
    let details = ProtocolMetadata::from_entries([(
        "dev.tea-rs.coding-tools",
        json!({"code":error.code().as_str(),"spawned":spawned,"uncertain":spawned}),
    )])
    .unwrap_or_default();
    ToolExecutionEvent::Failed(
        ToolExecutionFailure::execution(error.message())
            .unwrap_or_else(|_| ToolExecutionFailure::internal_contract())
            .with_details(details),
    )
}

fn cancelled_after_spawn() -> ToolExecutionEvent {
    let details = ProtocolMetadata::from_entries([(
        "dev.tea-rs.coding-tools",
        json!({"code":"cancelled","spawned":true,"uncertain":true}),
    )])
    .unwrap_or_default();
    ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled().with_details(details))
}

trait TimeoutDetails {
    fn with_timeout_details(self) -> Self;
}

impl TimeoutDetails for ToolExecutionEvent {
    fn with_timeout_details(self) -> Self {
        let details = ProtocolMetadata::from_entries([(
            "dev.tea-rs.coding-tools",
            json!({"code":"timeout","spawned":true,"uncertain":true}),
        )])
        .unwrap_or_default();
        match self {
            Self::Failed(failure) => Self::Failed(failure.with_details(details)),
            event => event,
        }
    }
}
