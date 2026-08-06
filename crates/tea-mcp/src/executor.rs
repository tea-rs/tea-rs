use std::{fmt, time::Duration};

use serde_json::{Map, Value, json};
use tea_control::CancellationScope;
use tea_protocol::ProtocolMetadata;
use tea_tools::{
    BoxToolExecutionStream, CompiledToolSchema, ToolExecutionEvent, ToolExecutionFailure,
    ToolExecutor, ToolSpec, ValidatedToolInvocation,
};
use tokio::time::Instant;

use crate::{
    McpError, McpErrorCode, McpRemoteToolName, McpStdioClient,
    content::{MappedCallResult, map_call_result},
    progress::ProgressMapper,
    reconnect::{McpConnectionLease, McpConnectionSlot},
    transport::{SdkToolCall, SdkToolCallEvent},
};

/// Executes one exact frozen MCP binding through the ordinary tool stream.
#[derive(Clone)]
pub struct McpToolExecutor {
    connection: McpConnectionSlot,
    remote_name: McpRemoteToolName,
    spec: ToolSpec,
    input: CompiledToolSchema,
    output: CompiledToolSchema,
    maximum_result_bytes: usize,
    cancellation_timeout: Duration,
}

impl McpToolExecutor {
    /// Binds an initialized client to one immutable discovered tool binding.
    ///
    /// # Errors
    ///
    /// Rejects a binding from another server or a schema that no longer
    /// compiles at the execution boundary.
    pub fn new(client: &McpStdioClient, binding: &crate::McpToolBinding) -> Result<Self, McpError> {
        let expected_source = format!("mcp.{}", client.server_id().as_str());
        if binding.spec().source().source_id() != expected_source {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        Self::bind(
            McpConnectionSlot::ready(client.execution_handle()?),
            binding,
            client.limits().max_result_bytes(),
            client.lifecycle().cancellation_timeout(),
        )
    }

    pub(crate) fn managed(
        connection: McpConnectionSlot,
        binding: &crate::McpToolBinding,
        maximum_result_bytes: usize,
        cancellation_timeout: Duration,
    ) -> Result<Self, McpError> {
        Self::bind(
            connection,
            binding,
            maximum_result_bytes,
            cancellation_timeout,
        )
    }

    fn bind(
        connection: McpConnectionSlot,
        binding: &crate::McpToolBinding,
        maximum_result_bytes: usize,
        cancellation_timeout: Duration,
    ) -> Result<Self, McpError> {
        let spec = binding.spec().clone();
        let input = CompiledToolSchema::compile(spec.input_schema().clone())
            .map_err(|_| McpError::new(McpErrorCode::Schema))?;
        let output = CompiledToolSchema::compile(spec.output_schema().clone())
            .map_err(|_| McpError::new(McpErrorCode::Schema))?;
        Ok(Self {
            connection,
            remote_name: binding.remote_name().clone(),
            spec,
            input,
            output,
            maximum_result_bytes,
            cancellation_timeout,
        })
    }

    fn validate_invocation(
        &self,
        invocation: &ValidatedToolInvocation,
    ) -> Result<Map<String, Value>, McpError> {
        if invocation.spec() != &self.spec {
            return Err(McpError::new(McpErrorCode::Configuration));
        }
        self.input
            .validate(invocation.arguments())
            .map_err(|_| McpError::new(McpErrorCode::Configuration))?;
        invocation
            .arguments()
            .as_object()
            .cloned()
            .ok_or_else(|| McpError::new(McpErrorCode::Configuration))
    }

    async fn start_call(
        &self,
        invocation: &ValidatedToolInvocation,
        cancellation: &CancellationScope,
    ) -> Result<StartedCall, DispatchedError> {
        let arguments = self
            .validate_invocation(invocation)
            .map_err(DispatchedError::before)?;
        if cancellation.is_cancelled() {
            return Err(DispatchedError::before(McpError::new(
                McpErrorCode::Cancellation,
            )));
        }
        let timeout = Duration::from_millis(invocation.spec().execution().timeout().as_millis());
        let deadline = Instant::now() + timeout;
        let lease = self
            .connection
            .acquire_call()
            .map_err(DispatchedError::before)?;
        let client = lease.connection().clone();
        let permit = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(DispatchedError::before(McpError::new(McpErrorCode::Cancellation)));
            }
            () = tokio::time::sleep_until(deadline) => {
                return Err(DispatchedError::before(McpError::new(McpErrorCode::Timeout)));
            }
            permit = client.acquire() => permit.map_err(DispatchedError::before)?,
        };

        let begin = client.begin_tool_call(self.remote_name.as_str().to_owned(), arguments, permit);
        tokio::pin!(begin);
        let call = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                client.abort_service();
                return Err(DispatchedError::after(McpError::new(McpErrorCode::Cancellation)));
            }
            () = tokio::time::sleep_until(deadline) => {
                client.abort_service();
                return Err(DispatchedError::after(McpError::new(McpErrorCode::Timeout)));
            }
            result = &mut begin => result.map_err(DispatchedError::after)?,
        };
        Ok(StartedCall {
            call,
            deadline,
            _lease: lease,
        })
    }

    fn map_terminal(
        &self,
        result: Result<rmcp::model::CallToolResult, McpError>,
    ) -> ToolExecutionEvent {
        match result
            .and_then(|result| map_call_result(result, self.maximum_result_bytes, &self.output))
        {
            Ok(MappedCallResult::Success(result)) => ToolExecutionEvent::Finished(*result),
            Ok(MappedCallResult::RemoteError) => {
                failure_event(McpError::new(McpErrorCode::Execution), DispatchPhase::After)
            }
            Err(error) => failure_event(error, DispatchPhase::After),
        }
    }
}

impl fmt::Debug for McpToolExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpToolExecutor")
            .field("remote_name", &self.remote_name)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl ToolExecutor for McpToolExecutor {
    fn execute(
        &self,
        invocation: ValidatedToolInvocation,
        cancellation: CancellationScope,
    ) -> BoxToolExecutionStream {
        let executor = self.clone();
        Box::pin(async_stream::stream! {
            let StartedCall { mut call, deadline, _lease } = match executor
                .start_call(&invocation, &cancellation)
                .await
            {
                Ok(started) => started,
                Err(error) => {
                    yield failure_event(error.error, error.phase);
                    return;
                }
            };
            let mut progress = ProgressMapper::default();
            loop {
                let event = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {
                        let _ = call.cancel(executor.cancellation_timeout).await;
                        yield failure_event(
                            McpError::new(McpErrorCode::Cancellation),
                            DispatchPhase::After,
                        );
                        return;
                    }
                    () = tokio::time::sleep_until(deadline) => {
                        let _ = call.cancel(executor.cancellation_timeout).await;
                        yield failure_event(
                            McpError::new(McpErrorCode::Timeout),
                            DispatchPhase::After,
                        );
                        return;
                    }
                    event = call.next() => event,
                };
                match event {
                    SdkToolCallEvent::Progress(notification) => match progress.map(notification) {
                        Ok(progress) => yield ToolExecutionEvent::Progress(progress),
                        Err(error) => {
                            let _ = call.cancel(executor.cancellation_timeout).await;
                            yield failure_event(error, DispatchPhase::After);
                            return;
                        }
                    },
                    SdkToolCallEvent::ProgressOverflow => {
                        let _ = call.cancel(executor.cancellation_timeout).await;
                        yield failure_event(
                            McpError::new(McpErrorCode::OutputBound),
                            DispatchPhase::After,
                        );
                        return;
                    }
                    SdkToolCallEvent::Result(result) => {
                        yield executor.map_terminal(result);
                        return;
                    }
                }
            }
        })
    }
}

struct StartedCall {
    call: SdkToolCall,
    deadline: Instant,
    _lease: McpConnectionLease,
}

struct DispatchedError {
    error: McpError,
    phase: DispatchPhase,
}

impl DispatchedError {
    const fn before(error: McpError) -> Self {
        Self {
            error,
            phase: DispatchPhase::Before,
        }
    }

    const fn after(error: McpError) -> Self {
        Self {
            error,
            phase: DispatchPhase::After,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DispatchPhase {
    Before,
    After,
}

impl DispatchPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before_dispatch",
            Self::After => "after_dispatch",
        }
    }
}

fn failure_event(error: McpError, phase: DispatchPhase) -> ToolExecutionEvent {
    let details = ProtocolMetadata::from_entries([(
        "dev.tea-rs.mcp",
        json!({"code":error.code(), "phase":phase.as_str()}),
    )])
    .unwrap_or_default();
    let failure = match error.code() {
        McpErrorCode::Cancellation => ToolExecutionFailure::cancelled(),
        McpErrorCode::Schema
        | McpErrorCode::Protocol
        | McpErrorCode::InvalidResult
        | McpErrorCode::OutputBound => ToolExecutionFailure::invalid_output(),
        _ => ToolExecutionFailure::execution(error.to_string())
            .unwrap_or_else(|_| ToolExecutionFailure::internal_contract()),
    };
    ToolExecutionEvent::Failed(failure.with_details(details))
}
