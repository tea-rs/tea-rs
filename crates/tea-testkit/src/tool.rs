use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use futures_util::{StreamExt, stream};
use serde_json::json;
use tea_control::CancellationScope;
use tea_protocol::ContentBlock;
use tea_tools::{
    BoxToolExecutionStream, ToolExecutionEvent, ToolExecutionFailure, ToolExecutionFailureCode,
    ToolExecutor, ToolProgress, ToolResult, ToolStreamValidator, ToolStreamViolation,
    ValidatedToolInvocation,
};

/// Deterministic in-memory read executor.
#[derive(Debug, Clone)]
pub struct FakeReadTool {
    files: Arc<Mutex<BTreeMap<String, String>>>,
}

impl FakeReadTool {
    /// Creates an in-memory file map.
    #[must_use]
    pub fn new(files: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            files: Arc::new(Mutex::new(files.into_iter().collect())),
        }
    }
}

impl ToolExecutor for FakeReadTool {
    fn execute(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        if cancellation.is_cancelled() {
            return Box::pin(stream::iter([cancelled()]));
        }
        let path = invocation
            .arguments()
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let event = match self.files.lock() {
            Ok(files) => match files.get(path) {
                Some(content) => result_event(content.clone(), json!({"content":content})),
                None => failure_event("fake file was not found"),
            },
            Err(_) => failure_event("fake read state is poisoned"),
        };
        Box::pin(stream::iter([event]))
    }
}

/// Deterministic in-memory write executor with captured mutations.
#[derive(Debug, Clone, Default)]
pub struct FakeWriteTool {
    writes: Arc<Mutex<Vec<(String, String)>>>,
}

impl FakeWriteTool {
    /// Creates an empty write capture.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns captured path/content writes in invocation order.
    ///
    /// # Errors
    ///
    /// Returns an error only when another thread poisoned the capture lock.
    pub fn writes(&self) -> Result<Vec<(String, String)>, FakeToolStateError> {
        self.writes
            .lock()
            .map(|writes| writes.clone())
            .map_err(|_| FakeToolStateError)
    }
}

impl ToolExecutor for FakeWriteTool {
    fn execute(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        if cancellation.is_cancelled() {
            return Box::pin(stream::iter([cancelled()]));
        }
        let path = invocation
            .arguments()
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let content = invocation
            .arguments()
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let event = match self.writes.lock() {
            Ok(mut writes) => {
                writes.push((path.clone(), content.clone()));
                result_event(
                    format!("wrote {path}"),
                    json!({"path":path,"writtenBytes":content.len()}),
                )
            }
            Err(_) => failure_event("fake write state is poisoned"),
        };
        Box::pin(stream::iter([event]))
    }
}

/// Script for a deterministic fake process executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeProcessScript {
    /// Emit progress and successful stdout.
    Complete {
        /// Deterministic captured standard output.
        stdout: String,
    },
    /// Emit a terminal operation failure.
    Fail {
        /// Bounded technical failure diagnostic.
        message: String,
    },
    /// Remain pending until cooperative cancellation.
    AwaitCancellation,
}

/// Deterministic process executor that never spawns a real process.
#[derive(Debug, Clone)]
pub struct FakeProcessTool {
    script: FakeProcessScript,
}

impl FakeProcessTool {
    /// Creates one scripted fake process.
    #[must_use]
    pub const fn new(script: FakeProcessScript) -> Self {
        Self { script }
    }
}

impl ToolExecutor for FakeProcessTool {
    fn execute(
        &self,
        _invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        match self.script.clone() {
            FakeProcessScript::Complete { stdout } => Box::pin(stream::iter([
                ToolExecutionEvent::Progress(
                    ToolProgress::new("process running", 0, None)
                        .expect("static progress is valid"),
                ),
                result_event(stdout.clone(), json!({"stdout":stdout,"exitCode":0})),
            ])),
            FakeProcessScript::Fail { message } => {
                Box::pin(stream::iter([failure_event(&message)]))
            }
            FakeProcessScript::AwaitCancellation => Box::pin(stream::once(async move {
                cancellation.cancelled().await;
                cancelled()
            })),
        }
    }
}

/// Terminal kind observed by tool conformance collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTerminalKind {
    /// Tool finished successfully.
    Finished,
    /// Tool ended with a typed failure.
    Failed,
}

/// Summary of one fully collected tool execution stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolConformanceReport {
    event_count: usize,
    progress_count: usize,
    terminal_kind: ToolTerminalKind,
    failure_code: Option<ToolExecutionFailureCode>,
}

impl ToolConformanceReport {
    /// Returns accepted event count.
    #[must_use]
    pub const fn event_count(self) -> usize {
        self.event_count
    }
    /// Returns progress event count.
    #[must_use]
    pub const fn progress_count(self) -> usize {
        self.progress_count
    }
    /// Returns terminal kind.
    #[must_use]
    pub const fn terminal_kind(self) -> ToolTerminalKind {
        self.terminal_kind
    }
    /// Returns terminal failure code.
    #[must_use]
    pub const fn failure_code(self) -> Option<ToolExecutionFailureCode> {
        self.failure_code
    }
}

/// Events plus validated tool conformance report.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectedToolExecution {
    events: Vec<ToolExecutionEvent>,
    report: ToolConformanceReport,
}

impl CollectedToolExecution {
    /// Returns normalized events in source order.
    #[must_use]
    pub fn events(&self) -> &[ToolExecutionEvent] {
        &self.events
    }
    /// Returns stream conformance summary.
    #[must_use]
    pub const fn report(&self) -> ToolConformanceReport {
        self.report
    }
}

/// Collects and validates one tool execution stream.
///
/// # Errors
///
/// Returns a typed stream grammar violation.
pub async fn collect_tool_execution(
    mut stream: BoxToolExecutionStream,
) -> Result<CollectedToolExecution, ToolConformanceError> {
    let mut validator = ToolStreamValidator::new();
    let mut events = Vec::new();
    let mut progress_count = 0;
    let mut terminal_kind = None;
    let mut failure_code = None;
    while let Some(event) = stream.next().await {
        validator.observe(&event)?;
        match &event {
            ToolExecutionEvent::Progress(_) => progress_count += 1,
            ToolExecutionEvent::Finished(_) => terminal_kind = Some(ToolTerminalKind::Finished),
            ToolExecutionEvent::Failed(failure) => {
                terminal_kind = Some(ToolTerminalKind::Failed);
                failure_code = Some(failure.code());
            }
        }
        events.push(event);
    }
    let event_count = validator.finish()?;
    let terminal_kind = terminal_kind.ok_or(ToolConformanceError::MissingTerminalSummary)?;
    Ok(CollectedToolExecution {
        events,
        report: ToolConformanceReport {
            event_count,
            progress_count,
            terminal_kind,
            failure_code,
        },
    })
}

/// Error returned by tool conformance collection.
#[derive(Debug, thiserror::Error)]
pub enum ToolConformanceError {
    /// Normalized stream grammar violation.
    #[error("tool stream grammar violation: {0}")]
    Stream(#[from] ToolStreamViolation),
    /// Collector did not observe a terminal summary.
    #[error("tool conformance report is missing terminal summary")]
    MissingTerminalSummary,
}

/// Error reading synchronized fake-tool inspection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("fake tool state is poisoned")]
pub struct FakeToolStateError;

fn result_event(text: impl Into<String>, output: serde_json::Value) -> ToolExecutionEvent {
    let content = ContentBlock::text(text.into());
    match content.and_then(|content| {
        ToolResult::new(vec![content], output)
            .map_err(|_| tea_protocol::ContentValidationError::InvalidText)
    }) {
        Ok(result) => ToolExecutionEvent::Finished(result),
        Err(_) => failure_event("fake tool produced an invalid result"),
    }
}

fn failure_event(message: &str) -> ToolExecutionEvent {
    ToolExecutionEvent::Failed(
        ToolExecutionFailure::execution(message)
            .unwrap_or_else(|_| ToolExecutionFailure::internal_contract()),
    )
}

fn cancelled() -> ToolExecutionEvent {
    ToolExecutionEvent::Failed(ToolExecutionFailure::cancelled())
}
