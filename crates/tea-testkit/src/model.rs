use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures_util::{StreamExt, future, stream};
use tea_model::{
    BoxModelStream, ModelCancellation, ModelCompletion, ModelEvent, ModelFailure, ModelFailureCode,
    ModelProvider, ModelRequest, ModelRequestError, ModelResponseInfo, ModelSpec,
    ModelStreamSummary, ModelStreamValidator, ModelStreamViolation, ProviderId, Utf8Delta,
};
use tea_protocol::{RetryClass, StopReason};
use thiserror::Error;

/// One deterministic action in a scripted model response.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptStep {
    /// Emit one normalized event immediately when polled.
    Event(Box<ModelEvent>),
    /// Remain pending until the request cancellation scope is cancelled, then
    /// emit a terminal cancelled failure.
    AwaitCancellation,
}

impl ScriptStep {
    /// Creates a step that emits one normalized model event.
    #[must_use]
    pub fn event(event: ModelEvent) -> Self {
        Self::Event(Box::new(event))
    }
}

impl From<ModelEvent> for ScriptStep {
    fn from(event: ModelEvent) -> Self {
        Self::event(event)
    }
}

/// Immutable ordered fake-provider script.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedModelResponse {
    steps: Vec<ScriptStep>,
}

impl ScriptedModelResponse {
    /// Creates a response from explicit ordered steps.
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = ScriptStep>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
        }
    }

    /// Creates a response from normalized events.
    #[must_use]
    pub fn events(events: impl IntoIterator<Item = ModelEvent>) -> Self {
        Self::new(events.into_iter().map(ScriptStep::event))
    }

    /// Creates a deterministic successful text response.
    #[must_use]
    pub fn text(chunks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut events = vec![ModelEvent::Started(ModelResponseInfo::new())];
        for chunk in chunks {
            let chunk = chunk.into();
            if let Ok(delta) = Utf8Delta::new(chunk) {
                events.push(ModelEvent::TextDelta(delta));
            } else {
                events.push(Self::failure_event(
                    ModelFailureCode::Internal,
                    "script contains an invalid text delta",
                ));
                return Self::events(events);
            }
        }
        events.push(ModelEvent::Completed(ModelCompletion::completed()));
        Self::events(events)
    }

    /// Creates a terminal provider failure after a start event.
    #[must_use]
    pub fn failure(code: ModelFailureCode, message: impl Into<String>) -> Self {
        Self::events([
            ModelEvent::Started(ModelResponseInfo::new()),
            Self::failure_event(code, message),
        ])
    }

    /// Creates one terminal failed event using safe bounded diagnostics.
    #[must_use]
    pub fn failure_event(code: ModelFailureCode, message: impl Into<String>) -> ModelEvent {
        ModelEvent::Failed(safe_failure(code, message.into()))
    }

    /// Creates a terminal context-overflow response.
    #[must_use]
    pub fn context_overflow(message: impl Into<String>) -> Self {
        Self::failure(ModelFailureCode::ContextOverflow, message)
    }

    /// Creates a stream that starts and then waits for cooperative cancellation.
    #[must_use]
    pub fn await_cancellation() -> Self {
        Self::new([
            ScriptStep::event(ModelEvent::Started(ModelResponseInfo::new())),
            ScriptStep::AwaitCancellation,
        ])
    }

    /// Returns immutable ordered script steps.
    #[must_use]
    pub fn steps(&self) -> &[ScriptStep] {
        &self.steps
    }
}

/// Deterministic FIFO model provider with request capture.
///
/// Streams are driven directly by polling and never create background tasks,
/// access the network, or sleep on wall-clock time.
#[derive(Debug, Clone)]
pub struct ScriptedModelProvider {
    provider_id: ProviderId,
    models: Vec<ModelSpec>,
    scripts: Arc<Mutex<VecDeque<ScriptedModelResponse>>>,
    captured_requests: Arc<Mutex<Vec<ModelRequest>>>,
}

impl ScriptedModelProvider {
    /// Creates a provider with advertised models and FIFO response scripts.
    #[must_use]
    pub fn new(
        provider_id: ProviderId,
        models: Vec<ModelSpec>,
        scripts: impl IntoIterator<Item = ScriptedModelResponse>,
    ) -> Self {
        Self {
            provider_id,
            models,
            scripts: Arc::new(Mutex::new(scripts.into_iter().collect())),
            captured_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns a snapshot of captured requests in call order.
    ///
    /// # Errors
    ///
    /// Returns an error only if another thread panicked while holding the
    /// internal capture lock.
    pub fn captured_requests(&self) -> Result<Vec<ModelRequest>, ScriptedProviderError> {
        self.captured_requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| ScriptedProviderError::StatePoisoned)
    }

    /// Returns the number of unconsumed FIFO scripts.
    ///
    /// # Errors
    ///
    /// Returns an error only if another thread panicked while holding the
    /// internal script lock.
    pub fn remaining_scripts(&self) -> Result<usize, ScriptedProviderError> {
        self.scripts
            .lock()
            .map(|scripts| scripts.len())
            .map_err(|_| ScriptedProviderError::StatePoisoned)
    }
}

impl ModelProvider for ScriptedModelProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    fn models(&self) -> &[ModelSpec] {
        &self.models
    }

    fn stream(&self, request: ModelRequest, cancellation: ModelCancellation) -> BoxModelStream {
        let capture_result = self
            .captured_requests
            .lock()
            .map(|mut requests| requests.push(request));
        let script_result = self.scripts.lock().map(|mut scripts| scripts.pop_front());

        let response = match (capture_result, script_result) {
            (Ok(()), Ok(Some(response))) => response,
            (Ok(()), Ok(None)) => ScriptedModelResponse::failure(
                ModelFailureCode::Internal,
                "scripted provider has no remaining response",
            ),
            _ => ScriptedModelResponse::failure(
                ModelFailureCode::Internal,
                "scripted provider state lock is poisoned",
            ),
        };

        let state = ScriptState {
            steps: response.steps.into(),
            cancellation,
            started: false,
            done: false,
        };
        Box::pin(stream::unfold(state, |mut state| async move {
            if state.done {
                return None;
            }
            if state.started && state.cancellation.is_cancelled() {
                state.done = true;
                return Some((
                    ScriptedModelResponse::failure_event(
                        ModelFailureCode::Cancelled,
                        "model request was cancelled",
                    ),
                    state,
                ));
            }
            match state.steps.pop_front() {
                Some(ScriptStep::Event(event)) => {
                    if matches!(event.as_ref(), ModelEvent::Started(_)) {
                        state.started = true;
                    }
                    Some((*event, state))
                }
                Some(ScriptStep::AwaitCancellation) => {
                    state.cancellation.cancelled().await;
                    state.done = true;
                    Some((
                        ScriptedModelResponse::failure_event(
                            ModelFailureCode::Cancelled,
                            "model request was cancelled",
                        ),
                        state,
                    ))
                }
                None => None,
            }
        }))
    }
}

#[derive(Debug)]
struct ScriptState {
    steps: VecDeque<ScriptStep>,
    cancellation: ModelCancellation,
    started: bool,
    done: bool,
}

/// Terminal kind observed by a provider conformance run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTerminalKind {
    /// Stream ended successfully.
    Completed,
    /// Stream ended with a typed failure.
    Failed,
}

/// Summary produced by reusable provider conformance collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConformanceReport {
    summary: ModelStreamSummary,
    terminal_kind: ModelTerminalKind,
    stop_reason: Option<StopReason>,
    failure_code: Option<ModelFailureCode>,
}

impl ModelConformanceReport {
    /// Returns total normalized events.
    #[must_use]
    pub const fn event_count(&self) -> usize {
        self.summary.event_count()
    }

    /// Returns completed tool-call count.
    #[must_use]
    pub const fn completed_tool_calls(&self) -> usize {
        self.summary.completed_tool_calls()
    }

    /// Returns the observed terminal kind.
    #[must_use]
    pub const fn terminal_kind(&self) -> ModelTerminalKind {
        self.terminal_kind
    }

    /// Returns successful normalized stop reason.
    #[must_use]
    pub const fn stop_reason(&self) -> Option<&StopReason> {
        self.stop_reason.as_ref()
    }

    /// Returns terminal failure code.
    #[must_use]
    pub const fn failure_code(&self) -> Option<ModelFailureCode> {
        self.failure_code
    }
}

/// Events and report from one fully collected provider stream.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectedModelStream {
    events: Vec<ModelEvent>,
    report: ModelConformanceReport,
}

impl CollectedModelStream {
    /// Returns normalized events in source order.
    #[must_use]
    pub fn events(&self) -> &[ModelEvent] {
        &self.events
    }

    /// Returns the validated conformance report.
    #[must_use]
    pub const fn report(&self) -> &ModelConformanceReport {
        &self.report
    }
}

/// Collects and validates one normalized model stream.
///
/// # Errors
///
/// Returns the first stream grammar violation.
pub async fn collect_model_stream(
    mut stream: BoxModelStream,
) -> Result<CollectedModelStream, ModelConformanceError> {
    let mut events = Vec::new();
    let mut validator = ModelStreamValidator::new();
    let mut terminal_kind = None;
    let mut stop_reason = None;
    let mut failure_code = None;

    while let Some(event) = stream.next().await {
        validator.observe(&event)?;
        match &event {
            ModelEvent::Completed(completion) => {
                terminal_kind = Some(ModelTerminalKind::Completed);
                stop_reason = Some(completion.stop_reason().clone());
            }
            ModelEvent::Failed(failure) => {
                terminal_kind = Some(ModelTerminalKind::Failed);
                failure_code = Some(failure.code());
            }
            _ => {}
        }
        events.push(event);
    }

    let summary = validator.finish()?;
    let terminal_kind = terminal_kind.ok_or(ModelConformanceError::MissingTerminalSummary)?;
    Ok(CollectedModelStream {
        events,
        report: ModelConformanceReport {
            summary,
            terminal_kind,
            stop_reason,
            failure_code,
        },
    })
}

/// Validates a request against an advertised model and runs one provider case.
///
/// # Errors
///
/// Returns an error for missing model advertisement, request mismatch, or
/// normalized stream grammar violation.
pub async fn run_model_provider_case<P>(
    provider: &P,
    request: ModelRequest,
) -> Result<CollectedModelStream, ModelConformanceError>
where
    P: ModelProvider + ?Sized,
{
    let model = provider
        .model(request.model_id())
        .ok_or(ModelConformanceError::ModelNotAdvertised)?;
    if model.provider_id() != provider.provider_id() {
        return Err(ModelConformanceError::ProviderModelMismatch);
    }
    request.validate_for(model)?;
    collect_model_stream(provider.stream(request, ModelCancellation::new())).await
}

/// Polls one provider case, cooperatively cancels it, and awaits termination.
///
/// # Errors
///
/// Returns an error for missing model advertisement, request mismatch, or
/// normalized stream grammar violation.
pub async fn run_cancelled_model_provider_case<P>(
    provider: &P,
    request: ModelRequest,
) -> Result<CollectedModelStream, ModelConformanceError>
where
    P: ModelProvider + ?Sized,
{
    let model = provider
        .model(request.model_id())
        .ok_or(ModelConformanceError::ModelNotAdvertised)?;
    if model.provider_id() != provider.provider_id() {
        return Err(ModelConformanceError::ProviderModelMismatch);
    }
    request.validate_for(model)?;
    let cancellation = ModelCancellation::new();
    let stream = provider.stream(request, cancellation.clone());
    let (result, ()) = future::join(collect_model_stream(stream), async move {
        cancellation.cancel();
    })
    .await;
    result
}

/// Error returned by reusable model-provider conformance utilities.
#[derive(Debug, Error)]
pub enum ModelConformanceError {
    /// Provider did not advertise the request model.
    #[error("provider did not advertise the requested model")]
    ModelNotAdvertised,
    /// Advertised model identifies a different provider.
    #[error("advertised model provider does not match provider adapter")]
    ProviderModelMismatch,
    /// Request is incompatible with the advertised model.
    #[error("model request failed validation: {0}")]
    InvalidRequest(#[from] ModelRequestError),
    /// Normalized stream grammar is invalid.
    #[error("model stream grammar violation: {0}")]
    StreamGrammar(#[from] ModelStreamViolation),
    /// Internal collection summary did not observe a terminal event.
    #[error("model conformance report is missing terminal summary")]
    MissingTerminalSummary,
}

/// Error reading synchronized fake-provider inspection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScriptedProviderError {
    /// Another thread panicked while holding a fake-provider state lock.
    #[error("scripted provider state is poisoned")]
    StatePoisoned,
}

fn safe_failure(code: ModelFailureCode, message: String) -> ModelFailure {
    ModelFailure::new(code, message, retry_for(code))
        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

const fn retry_for(code: ModelFailureCode) -> RetryClass {
    match code {
        ModelFailureCode::RateLimited | ModelFailureCode::Unavailable => RetryClass::AfterBackoff,
        ModelFailureCode::Transport => RetryClass::Immediate,
        ModelFailureCode::InvalidRequest
        | ModelFailureCode::ContextOverflow
        | ModelFailureCode::Authentication
        | ModelFailureCode::PermissionDenied
        | ModelFailureCode::MalformedResponse
        | ModelFailureCode::Cancelled
        | ModelFailureCode::Internal => RetryClass::Never,
    }
}
