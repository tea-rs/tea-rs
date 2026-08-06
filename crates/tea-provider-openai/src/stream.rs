//! Maps parsed `OpenAI` streaming chunks to normalized `ModelEvent`s.
//!
//! The [`ChunkReducer`] is transport-agnostic: the live `reqwest` stream and
//! the fixture-backed conformance tests both feed parsed JSON payloads through
//! it so the mapping logic is identical. Tool-call argument fragments are
//! accumulated per stream index and completed on `finish_reason`.

use std::collections::BTreeMap;
use std::str::FromStr;

use serde_json::Value;
use tea_model::{
    ModelCompletion, ModelEvent, ModelFailure, ModelFailureCode, ModelResponseInfo,
    ModelStreamIndex, ProviderToolCallId, ToolArgumentsDelta, ToolCallCompleted, ToolCallStarted,
    Utf8Delta,
};
use tea_protocol::{RetryClass, StopReason, TokenCount, Usage};
use tea_provider_http::normalize_provider_error;

use crate::error::{OpenAiError, OpenAiErrorCode};

/// Accumulator for one streaming tool call.
#[derive(Debug, Default, Clone)]
struct ToolCallAccumulator {
    index: u16,
    provider_call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// Stateful reducer mapping `OpenAI` chunk JSON to normalized `ModelEvent`s.
#[derive(Debug, Default)]
pub struct ChunkReducer {
    started: bool,
    tool_calls: BTreeMap<u16, ToolCallAccumulator>,
    stop_reason: Option<StopReason>,
    terminal_emitted: bool,
}

impl ChunkReducer {
    /// Creates an empty reducer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether a terminal event has already been emitted.
    #[must_use]
    pub const fn terminal_emitted(&self) -> bool {
        self.terminal_emitted
    }

    /// Maps one parsed JSON chunk to zero or more normalized events.
    ///
    /// # Errors
    ///
    /// Returns an error when the chunk is malformed or a normalized event
    /// cannot be constructed.
    pub fn map_chunk(&mut self, value: &Value) -> Result<Vec<ModelEvent>, OpenAiError> {
        let mut events = Vec::new();
        if !self.started {
            events.push(ModelEvent::Started(ModelResponseInfo::new()));
            self.started = true;
        }
        if let Some(error) = value.get("error") {
            self.terminal_emitted = true;
            events.push(ModelEvent::Failed(map_stream_error(error)));
            return Ok(events);
        }
        if let Some(usage) = value.get("usage")
            && self.stop_reason.is_some()
            && !self.terminal_emitted
        {
            events.push(self.complete(self.stop_reason.clone(), usage)?);
        }
        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return Ok(events);
        };
        for choice in choices {
            self.map_choice(choice, &mut events)?;
        }
        Ok(events)
    }

    /// Flushes the terminal `Completed` event when the stream ends without a
    /// usage chunk.
    ///
    /// # Errors
    ///
    /// Returns an error when the accumulated tool-call arguments cannot parse.
    pub fn finish(&mut self) -> Result<Option<ModelEvent>, OpenAiError> {
        if self.terminal_emitted {
            return Ok(None);
        }
        if self.stop_reason.is_some() {
            let stop = self.stop_reason.clone();
            Ok(Some(self.complete(stop, &Value::Null)?))
        } else {
            // No finish_reason observed; treat as a successful completion.
            Ok(Some(ModelEvent::Completed(ModelCompletion::completed())))
        }
    }

    fn map_choice(
        &mut self,
        choice: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), OpenAiError> {
        let Some(delta) = choice.get("delta") else {
            return Ok(());
        };
        if let Some(reasoning) = delta.get("reasoning").and_then(Value::as_str)
            && !reasoning.is_empty()
        {
            events.push(ModelEvent::ThinkingDelta(
                Utf8Delta::new(reasoning.to_owned()).map_err(|_| malformed("reasoning delta"))?,
            ));
        }
        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            events.push(ModelEvent::TextDelta(
                Utf8Delta::new(content.to_owned()).map_err(|_| malformed("text delta"))?,
            ));
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                self.map_tool_call_delta(tool_call, events)?;
            }
        }
        if let Some(finish) = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned)
            && !finish.is_empty()
            && self.stop_reason.is_none()
        {
            let stop_reason = match finish.as_str() {
                "length" => StopReason::Length,
                "tool_calls" => StopReason::ToolUse,
                _ => StopReason::Completed,
            };
            self.stop_reason = Some(stop_reason.clone());
            for event in self.complete_tool_calls()? {
                events.push(event);
            }
        }
        Ok(())
    }

    fn map_tool_call_delta(
        &mut self,
        tool_call: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), OpenAiError> {
        let index = tool_call
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| malformed("tool call index missing"))
            .and_then(|value| {
                u16::try_from(value.min(u64::from(u16::MAX)))
                    .map_err(|_| malformed("tool call index out of range"))
            })?;
        let accumulator = self
            .tool_calls
            .entry(index)
            .or_insert_with(|| ToolCallAccumulator {
                index,
                ..Default::default()
            });
        if let Some(id) = tool_call.get("id").and_then(Value::as_str)
            && accumulator.provider_call_id.is_none()
        {
            let provider_id =
                ProviderToolCallId::from_str(id).map_err(|_| malformed("tool call id"))?;
            accumulator.provider_call_id = Some(id.to_owned());
            let name = tool_call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("tool call name missing"))?;
            accumulator.name = Some(name.to_owned());
            let stream_index =
                ModelStreamIndex::new(index).map_err(|_| malformed("stream index"))?;
            events.push(ModelEvent::ToolCallStarted(
                ToolCallStarted::new(stream_index, provider_id, name)
                    .map_err(|_| malformed("tool call started"))?,
            ));
        }
        if let Some(arguments) = tool_call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
            && !arguments.is_empty()
        {
            accumulator.arguments.push_str(arguments);
            let stream_index =
                ModelStreamIndex::new(index).map_err(|_| malformed("stream index"))?;
            let provider_id = accumulator
                .provider_call_id
                .as_ref()
                .and_then(|id| ProviderToolCallId::from_str(id).ok())
                .ok_or_else(|| malformed("tool arguments without a started call"))?;
            events.push(ModelEvent::ToolArgumentsDelta(
                ToolArgumentsDelta::new(stream_index, provider_id, arguments.to_owned())
                    .map_err(|_| malformed("tool arguments delta"))?,
            ));
        }
        Ok(())
    }

    fn complete_tool_calls(&mut self) -> Result<Vec<ModelEvent>, OpenAiError> {
        let mut events = Vec::new();
        // Complete every accumulated tool call in index order.
        let mut completed: Vec<(u16, ToolCallAccumulator)> = self
            .tool_calls
            .iter()
            .map(|(index, accumulator)| (*index, accumulator.clone()))
            .collect();
        completed.sort_by_key(|(index, _)| *index);
        for (_, accumulator) in completed {
            let arguments: Value = if accumulator.arguments.is_empty() {
                Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&accumulator.arguments)
                    .map_err(|_| malformed("tool arguments did not parse"))?
            };
            let stream_index =
                ModelStreamIndex::new(accumulator.index).map_err(|_| malformed("stream index"))?;
            let provider_id = accumulator
                .provider_call_id
                .as_ref()
                .and_then(|id| ProviderToolCallId::from_str(id).ok())
                .ok_or_else(|| malformed("completed tool call without id"))?;
            let name = accumulator.name.clone().unwrap_or_default();
            events.push(ModelEvent::ToolCallCompleted(
                ToolCallCompleted::new(stream_index, provider_id, name, arguments)
                    .map_err(|_| malformed("tool call completed"))?,
            ));
        }
        Ok(events)
    }

    fn complete(
        &mut self,
        stop_reason: Option<StopReason>,
        usage_value: &Value,
    ) -> Result<ModelEvent, OpenAiError> {
        let stop = stop_reason.unwrap_or(StopReason::Completed);
        let mut completion =
            ModelCompletion::new(stop).map_err(|_| malformed("invalid completion reason"))?;
        if let (Some(prompt), Some(completion_tokens)) = (
            usage_value.get("prompt_tokens").and_then(Value::as_u64),
            usage_value.get("completion_tokens").and_then(Value::as_u64),
        ) {
            let prompt_details = usage_value.get("prompt_tokens_details");
            let cached = prompt_details
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64);
            let cache_write = prompt_details
                .and_then(|details| details.get("cache_write_tokens"))
                .and_then(Value::as_u64);
            let billable_input = prompt
                .checked_sub(cached.unwrap_or(0))
                .and_then(|value| value.checked_sub(cache_write.unwrap_or(0)))
                .ok_or_else(|| malformed("cache usage exceeds prompt tokens"))?;
            if let (Ok(input), Ok(output)) = (
                TokenCount::new(billable_input),
                TokenCount::new(completion_tokens),
            ) {
                let mut usage = Usage::new(input, output);
                if let Some(cached) = cached.and_then(|cached| TokenCount::new(cached).ok()) {
                    usage = usage.with_cache_read(cached);
                }
                if let Some(cache_write) =
                    cache_write.and_then(|cache_write| TokenCount::new(cache_write).ok())
                {
                    usage = usage.with_cache_write(cache_write);
                }
                if let Some(reasoning) = usage_value
                    .get("completion_tokens_details")
                    .and_then(|details| details.get("reasoning_tokens"))
                    .and_then(Value::as_u64)
                    .and_then(|reasoning| TokenCount::new(reasoning).ok())
                    && let Ok(with_reasoning) = usage.clone().with_reasoning(reasoning)
                {
                    usage = with_reasoning;
                }
                completion = completion.with_usage(usage);
            }
        }
        self.terminal_emitted = true;
        Ok(ModelEvent::Completed(completion))
    }
}

/// Maps an HTTP error status / transport failure to a normalized `ModelFailure`.
#[must_use]
pub fn map_http_failure(status: u16, body: &str) -> ModelFailure {
    let (code, retry) = match status {
        401 => (ModelFailureCode::Authentication, RetryClass::Never),
        403 => (ModelFailureCode::PermissionDenied, RetryClass::Never),
        400 => (ModelFailureCode::InvalidRequest, RetryClass::Never),
        429 => (ModelFailureCode::RateLimited, RetryClass::AfterBackoff),
        408 => (ModelFailureCode::Transport, RetryClass::Immediate),
        500..=599 if status != 501 => (ModelFailureCode::Unavailable, RetryClass::AfterBackoff),
        _ => (ModelFailureCode::Internal, RetryClass::Never),
    };
    ModelFailure::safe(code, normalize_provider_error(Some(status), body), retry)
        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

/// Maps an `OpenAI` streaming `error` payload to a normalized `ModelFailure`.
#[must_use]
pub fn map_stream_error(error: &Value) -> ModelFailure {
    let serialized = serde_json::to_string(error).unwrap_or_default();
    let message = normalize_provider_error(None, &serialized);
    let error_type = error.get("type").and_then(Value::as_str).unwrap_or("");
    let (code, retry) = match error_type {
        "server_error" | "overloaded" => (ModelFailureCode::Unavailable, RetryClass::AfterBackoff),
        "rate_limit_exceeded" => (ModelFailureCode::RateLimited, RetryClass::AfterBackoff),
        _ => (ModelFailureCode::MalformedResponse, RetryClass::Never),
    };
    ModelFailure::safe(code, message, retry)
        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

fn malformed(message: &str) -> OpenAiError {
    OpenAiError::new(OpenAiErrorCode::MalformedResponse, message)
}
