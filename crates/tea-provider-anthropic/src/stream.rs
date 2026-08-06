//! Maps Anthropic Messages stream payloads to normalized model events.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde_json::{Value, json};
use tea_model::{
    HostedToolCompleted, HostedToolStarted, ModelCompletion, ModelEvent, ModelFailure,
    ModelFailureCode, ModelResponseInfo, ModelSourceCitation, ModelStreamIndex, ProviderResponseId,
    ProviderToolCallId, ToolArgumentsDelta, ToolCallCompleted, ToolCallStarted, Utf8Delta,
};
use tea_protocol::{
    ExternalSource, HostedToolError, HostedToolOutcome, ProviderContinuation, RetryClass,
    SourceCitation, StopReason, TokenCount, Usage,
};
use tea_provider_http::normalize_provider_error;

use crate::WEB_SEARCH_CONTINUATION_FORMAT;
use crate::credential::PROVIDER_ID;
use crate::error::{AnthropicError, AnthropicErrorCode};

#[derive(Clone)]
struct ToolCallAccumulator {
    index: u16,
    provider_call_id: String,
    name: String,
    arguments: String,
}

#[derive(Clone)]
struct HostedToolAccumulator {
    index: u16,
    provider_call_id: String,
    name: String,
    arguments: String,
    raw_block: Value,
    completed: bool,
}

/// Stateful reducer for Anthropic Messages Server-Sent Events.
#[derive(Default)]
pub struct AnthropicReducer {
    started: bool,
    terminal_emitted: bool,
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    tool_calls: BTreeMap<u16, ToolCallAccumulator>,
    hosted_tools: BTreeMap<u16, HostedToolAccumulator>,
    source_owners: BTreeMap<String, String>,
}

impl fmt::Debug for AnthropicReducer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnthropicReducer")
            .field("started", &self.started)
            .field("terminal_emitted", &self.terminal_emitted)
            .field("has_input_tokens", &self.input_tokens.is_some())
            .field("has_cache_read_tokens", &self.cache_read_tokens.is_some())
            .field("has_cache_write_tokens", &self.cache_write_tokens.is_some())
            .field("tool_call_count", &self.tool_calls.len())
            .field("hosted_tool_count", &self.hosted_tools.len())
            .field("source_owner_count", &self.source_owners.len())
            .finish_non_exhaustive()
    }
}

impl AnthropicReducer {
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

    /// Maps one parsed Messages API event into normalized model events.
    ///
    /// # Errors
    ///
    /// Returns an error when an event cannot be normalized safely.
    pub fn map_chunk(&mut self, value: &Value) -> Result<Vec<ModelEvent>, AnthropicError> {
        let mut events = Vec::new();
        if !self.started {
            events.push(ModelEvent::Started(ModelResponseInfo::new()));
            self.started = true;
        }
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "error" => {
                self.terminal_emitted = true;
                events.push(ModelEvent::Failed(map_stream_error(value.get("error"))));
            }
            "message_start" => self.map_message_start(value, &mut events)?,
            "content_block_start" => self.map_content_start(value, &mut events)?,
            "content_block_delta" => self.map_content_delta(value, &mut events)?,
            "message_delta" => self.map_message_delta(value, &mut events)?,
            "message_stop" if !self.terminal_emitted => {
                self.ensure_hosted_tools_complete()?;
                events.extend(self.complete_tool_calls()?);
                events.push(self.complete(StopReason::Completed, None)?);
            }
            _ => {}
        }
        Ok(events)
    }

    /// Completes a truncated or gateway-terminated stream deterministically.
    ///
    /// # Errors
    ///
    /// Returns an error when an accumulated tool input is not valid JSON.
    pub fn finish(&mut self) -> Result<Vec<ModelEvent>, AnthropicError> {
        if self.terminal_emitted {
            Ok(Vec::new())
        } else {
            self.ensure_hosted_tools_complete()?;
            let mut events = self.complete_tool_calls()?;
            events.push(self.complete(StopReason::Completed, None)?);
            Ok(events)
        }
    }

    fn map_message_start(
        &mut self,
        value: &Value,
        events: &mut [ModelEvent],
    ) -> Result<(), AnthropicError> {
        let message = value
            .get("message")
            .ok_or_else(|| malformed("message_start missing message"))?;
        if let Some(usage) = message.get("usage") {
            self.capture_input_usage(usage);
        }
        let mut info = ModelResponseInfo::new();
        if let Some(response_id) = message
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| ProviderResponseId::from_str(id).ok())
        {
            info = info.with_response_id(response_id);
        }
        if let Some(model) = message
            .get("model")
            .and_then(Value::as_str)
            .and_then(|model| tea_protocol::ModelId::from_str(model).ok())
        {
            info = info.with_response_model(model);
        }
        if matches!(events.first(), Some(ModelEvent::Started(_))) {
            events[0] = ModelEvent::Started(info);
        }
        Ok(())
    }

    fn map_content_start(
        &mut self,
        value: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), AnthropicError> {
        let index = stream_index(value)?;
        let block = value
            .get("content_block")
            .ok_or_else(|| malformed("content block missing"))?;
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    Self::push_text(text, events)?;
                }
                if let Some(citations) = block.get("citations").and_then(Value::as_array) {
                    for citation in citations {
                        self.push_citation(citation, events)?;
                    }
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("tool use id missing"))?;
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("tool use name missing"))?;
                let provider_id = ProviderToolCallId::from_str(id)
                    .map_err(|_| malformed("tool use id is invalid"))?;
                events.push(ModelEvent::ToolCallStarted(
                    ToolCallStarted::new(index, provider_id.clone(), name)
                        .map_err(|_| malformed("tool use is invalid"))?,
                ));
                let mut accumulator = ToolCallAccumulator {
                    index: index.get(),
                    provider_call_id: id.to_owned(),
                    name: name.to_owned(),
                    arguments: String::new(),
                };
                if let Some(input) = block.get("input") {
                    if !input.is_object() {
                        return Err(malformed("tool use input must be an object"));
                    }
                    if !input.as_object().is_some_and(serde_json::Map::is_empty) {
                        let arguments = serde_json::to_string(input)
                            .map_err(|_| malformed("tool use input could not serialize"))?;
                        accumulator.arguments.push_str(&arguments);
                        events.push(ModelEvent::ToolArgumentsDelta(
                            ToolArgumentsDelta::new(index, provider_id, arguments)
                                .map_err(|_| malformed("tool input delta is invalid"))?,
                        ));
                    }
                }
                self.tool_calls.insert(index.get(), accumulator);
            }
            Some("server_tool_use") => self.start_hosted_tool(index, block, events)?,
            Some("web_search_tool_result") => self.complete_hosted_tool(block, events)?,
            _ => {}
        }
        Ok(())
    }

    fn map_content_delta(
        &mut self,
        value: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), AnthropicError> {
        let index = stream_index(value)?;
        let delta = value
            .get("delta")
            .ok_or_else(|| malformed("content delta missing"))?;
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                let text = delta
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("text delta missing text"))?;
                Self::push_text(text, events)?;
            }
            Some("input_json_delta") => {
                let partial = delta
                    .get("partial_json")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("tool input delta missing partial json"))?;
                if !partial.is_empty() {
                    if let Some(accumulator) = self.tool_calls.get_mut(&index.get()) {
                        accumulator.arguments.push_str(partial);
                        let provider_id =
                            ProviderToolCallId::from_str(&accumulator.provider_call_id)
                                .map_err(|_| malformed("tool use id is invalid"))?;
                        events.push(ModelEvent::ToolArgumentsDelta(
                            ToolArgumentsDelta::new(index, provider_id, partial.to_owned())
                                .map_err(|_| malformed("tool input delta is invalid"))?,
                        ));
                    } else if let Some(accumulator) = self.hosted_tools.get_mut(&index.get()) {
                        accumulator.arguments.push_str(partial);
                    } else {
                        return Err(malformed("tool input delta arrived before tool start"));
                    }
                }
            }
            Some("citations_delta") => {
                let citation = delta
                    .get("citation")
                    .ok_or_else(|| malformed("citation delta missing citation"))?;
                self.push_citation(citation, events)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn map_message_delta(
        &mut self,
        value: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), AnthropicError> {
        let stop = match value
            .get("delta")
            .and_then(|delta| delta.get("stop_reason"))
            .and_then(Value::as_str)
        {
            Some("max_tokens") => StopReason::Length,
            Some("tool_use") => StopReason::ToolUse,
            Some("pause_turn") => StopReason::PauseTurn,
            _ => StopReason::Completed,
        };
        let output_tokens = value
            .get("usage")
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_u64);
        self.ensure_hosted_tools_complete()?;
        for event in self.complete_tool_calls()? {
            events.push(event);
        }
        events.push(self.complete(stop, output_tokens)?);
        Ok(())
    }

    fn capture_input_usage(&mut self, usage: &Value) {
        self.input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
        self.cache_read_tokens = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
        self.cache_write_tokens = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64);
    }

    fn push_text(text: &str, events: &mut Vec<ModelEvent>) -> Result<(), AnthropicError> {
        if !text.is_empty() {
            events.push(ModelEvent::TextDelta(
                Utf8Delta::new(text.to_owned()).map_err(|_| malformed("text delta is invalid"))?,
            ));
        }
        Ok(())
    }

    fn start_hosted_tool(
        &mut self,
        index: ModelStreamIndex,
        block: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), AnthropicError> {
        let id = block
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("server tool use id missing"))?;
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("server tool use name missing"))?;
        if name != "web_search" {
            return Err(malformed("unsupported Anthropic server tool use"));
        }
        let input = block
            .get("input")
            .ok_or_else(|| malformed("server tool use input missing"))?;
        let input = input
            .as_object()
            .ok_or_else(|| malformed("server tool use input must be an object"))?;
        if self.tool_calls.contains_key(&index.get())
            || self.hosted_tools.contains_key(&index.get())
            || self
                .hosted_tools
                .values()
                .any(|tool| tool.provider_call_id == id)
        {
            return Err(malformed("server tool use reused an identity"));
        }
        let provider_id = ProviderToolCallId::from_str(id)
            .map_err(|_| malformed("server tool use id is invalid"))?;
        let arguments = if input.is_empty() {
            String::new()
        } else {
            serde_json::to_string(input)
                .map_err(|_| malformed("server tool use input could not serialize"))?
        };
        self.hosted_tools.insert(
            index.get(),
            HostedToolAccumulator {
                index: index.get(),
                provider_call_id: id.to_owned(),
                name: name.to_owned(),
                arguments,
                raw_block: block.clone(),
                completed: false,
            },
        );
        events.push(ModelEvent::HostedToolStarted(
            HostedToolStarted::new(index, provider_id, name)
                .map_err(|_| malformed("server tool use is invalid"))?,
        ));
        Ok(())
    }

    fn complete_hosted_tool(
        &mut self,
        block: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), AnthropicError> {
        let tool_use_id = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("web search result tool id missing"))?;
        let index = self
            .hosted_tools
            .iter()
            .find_map(|(index, tool)| (tool.provider_call_id == tool_use_id).then_some(*index))
            .ok_or_else(|| malformed("web search result has no matching server tool use"))?;
        let tool = self
            .hosted_tools
            .get(&index)
            .expect("hosted tool index was resolved above");
        if tool.completed {
            return Err(malformed("web search result completed a server tool twice"));
        }
        let arguments = hosted_arguments(tool)?;
        let (outcome, sources) = parse_web_search_result(block)?;
        let mut server_block = tool.raw_block.clone();
        server_block
            .as_object_mut()
            .expect("server tool block was validated as an object")
            .insert("input".to_owned(), arguments.clone());
        let continuation = ProviderContinuation::new(
            PROVIDER_ID,
            WEB_SEARCH_CONTINUATION_FORMAT,
            json!({"content_blocks":[server_block, block.clone()]}),
        )
        .map_err(|_| malformed("web search continuation exceeded bounds"))?;
        let provider_id = ProviderToolCallId::from_str(&tool.provider_call_id)
            .map_err(|_| malformed("server tool use id is invalid"))?;
        let completed = HostedToolCompleted::new(
            ModelStreamIndex::new(tool.index)
                .map_err(|_| malformed("server tool stream index is invalid"))?,
            provider_id,
            tool.name.clone(),
            arguments,
            outcome,
            sources.clone(),
            Some(continuation),
        )
        .map_err(|_| malformed("web search completion is invalid"))?;

        for source in &sources {
            self.source_owners
                .insert(source.url().to_owned(), tool_use_id.to_owned());
        }
        self.hosted_tools
            .get_mut(&index)
            .expect("hosted tool index was resolved above")
            .completed = true;
        events.push(ModelEvent::HostedToolCompleted(completed));
        Ok(())
    }

    fn push_citation(
        &mut self,
        raw: &Value,
        events: &mut Vec<ModelEvent>,
    ) -> Result<(), AnthropicError> {
        if raw.get("type").and_then(Value::as_str) != Some("web_search_result_location") {
            return Ok(());
        }
        let source = external_source(raw)?;
        let cited_text = raw
            .get("cited_text")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("web search citation text missing"))?;
        let continuation = ProviderContinuation::new(
            PROVIDER_ID,
            WEB_SEARCH_CONTINUATION_FORMAT,
            json!({"citation":raw.clone()}),
        )
        .map_err(|_| malformed("web search citation continuation exceeded bounds"))?;
        let citation = SourceCitation::new(source)
            .with_cited_text(cited_text)
            .map(|citation| citation.with_continuation(continuation))
            .map_err(|_| malformed("web search citation is invalid"))?;
        let provider_call_id = self
            .source_owners
            .get(citation.source().url())
            .map(|id| {
                ProviderToolCallId::from_str(id)
                    .map_err(|_| malformed("web search citation owner is invalid"))
            })
            .transpose()?;
        let citation = ModelSourceCitation::new(provider_call_id, citation)
            .map_err(|_| malformed("web search citation is invalid"))?;
        events.push(ModelEvent::SourceCitation(citation));
        Ok(())
    }

    fn ensure_hosted_tools_complete(&self) -> Result<(), AnthropicError> {
        if self.hosted_tools.values().any(|tool| !tool.completed) {
            return Err(malformed("server tool use has no web search result"));
        }
        Ok(())
    }

    fn complete_tool_calls(&self) -> Result<Vec<ModelEvent>, AnthropicError> {
        self.tool_calls
            .values()
            .map(|tool| {
                let arguments = if tool.arguments.is_empty() {
                    Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&tool.arguments)
                        .map_err(|_| malformed("tool input did not parse"))?
                };
                let index = ModelStreamIndex::new(tool.index)
                    .map_err(|_| malformed("tool stream index is invalid"))?;
                let provider_id = ProviderToolCallId::from_str(&tool.provider_call_id)
                    .map_err(|_| malformed("tool use id is invalid"))?;
                ToolCallCompleted::new(index, provider_id, tool.name.clone(), arguments)
                    .map(ModelEvent::ToolCallCompleted)
                    .map_err(|_| malformed("tool completion is invalid"))
            })
            .collect()
    }

    fn complete(
        &mut self,
        stop: StopReason,
        output_tokens: Option<u64>,
    ) -> Result<ModelEvent, AnthropicError> {
        let mut completion =
            ModelCompletion::new(stop).map_err(|_| malformed("completion reason is invalid"))?;
        if let (Some(input), Some(output)) = (self.input_tokens, output_tokens)
            && let (Ok(input), Ok(output)) = (TokenCount::new(input), TokenCount::new(output))
        {
            let mut usage = Usage::new(input, output);
            if let Some(cache_read) = self
                .cache_read_tokens
                .and_then(|value| TokenCount::new(value).ok())
            {
                usage = usage.with_cache_read(cache_read);
            }
            if let Some(cache_write) = self
                .cache_write_tokens
                .and_then(|value| TokenCount::new(value).ok())
            {
                usage = usage.with_cache_write(cache_write);
            }
            if usage.total_tokens().is_ok() {
                completion = completion.with_usage(usage);
            }
        }
        self.terminal_emitted = true;
        Ok(ModelEvent::Completed(completion))
    }
}

fn hosted_arguments(tool: &HostedToolAccumulator) -> Result<Value, AnthropicError> {
    let arguments = if tool.arguments.is_empty() {
        tool.raw_block
            .get("input")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
    } else {
        serde_json::from_str(&tool.arguments)
            .map_err(|_| malformed("server tool input did not parse"))?
    };
    if !arguments.is_object() {
        return Err(malformed("server tool input must be an object"));
    }
    Ok(arguments)
}

fn parse_web_search_result(
    block: &Value,
) -> Result<(HostedToolOutcome, Vec<ExternalSource>), AnthropicError> {
    let content = block
        .get("content")
        .ok_or_else(|| malformed("web search result content missing"))?;
    match content {
        Value::Array(results) => {
            let sources = results
                .iter()
                .map(|result| {
                    if result.get("type").and_then(Value::as_str) != Some("web_search_result") {
                        return Err(malformed("web search result item is invalid"));
                    }
                    external_source(result)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((HostedToolOutcome::Success, sources))
        }
        Value::Object(error)
            if error.get("type").and_then(Value::as_str)
                == Some("web_search_tool_result_error") =>
        {
            let code = error
                .get("error_code")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("web search result error code missing"))?;
            let error = HostedToolError::new(code, web_search_error_message(code))
                .map_err(|_| malformed("web search result error is invalid"))?;
            Ok((HostedToolOutcome::Error(error), Vec::new()))
        }
        _ => Err(malformed("web search result content is invalid")),
    }
}

fn external_source(value: &Value) -> Result<ExternalSource, AnthropicError> {
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("web search source URL missing"))?;
    let mut source =
        ExternalSource::new(url).map_err(|_| malformed("web search source URL is invalid"))?;
    match value.get("title") {
        Some(Value::String(title)) => {
            source = source
                .with_title(title)
                .map_err(|_| malformed("web search source title is invalid"))?;
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err(malformed("web search source title is invalid")),
    }
    Ok(source)
}

fn web_search_error_message(code: &str) -> &'static str {
    match code {
        "too_many_requests" => "Anthropic web search rate limit exceeded",
        "invalid_tool_input" => "Anthropic web search input was invalid",
        "max_uses_exceeded" => "Anthropic web search maximum uses exceeded",
        "query_too_long" => "Anthropic web search query was too long",
        "request_too_large" => "Anthropic web search request was too large",
        "unavailable" => "Anthropic web search was unavailable",
        _ => "Anthropic web search failed",
    }
}

/// Maps an Anthropic HTTP failure to a normalized model failure.
#[must_use]
pub fn map_http_failure(status: u16, body: &str) -> ModelFailure {
    let (code, retry) = match status {
        401 => (ModelFailureCode::Authentication, RetryClass::Never),
        403 => (ModelFailureCode::PermissionDenied, RetryClass::Never),
        400 | 404 | 413 | 422 => (ModelFailureCode::InvalidRequest, RetryClass::Never),
        429 => (ModelFailureCode::RateLimited, RetryClass::AfterBackoff),
        408 => (ModelFailureCode::Transport, RetryClass::Immediate),
        500..=599 => (ModelFailureCode::Unavailable, RetryClass::AfterBackoff),
        _ => (ModelFailureCode::Internal, RetryClass::Never),
    };
    ModelFailure::safe(code, normalize_provider_error(Some(status), body), retry)
        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

/// Maps an Anthropic in-stream error payload to a normalized model failure.
#[must_use]
pub fn map_stream_error(error: Option<&Value>) -> ModelFailure {
    let error = error.unwrap_or(&Value::Null);
    let serialized = serde_json::to_string(error).unwrap_or_default();
    let message = normalize_provider_error(None, &serialized);
    let (code, retry) = match error.get("type").and_then(Value::as_str) {
        Some("authentication_error") => (ModelFailureCode::Authentication, RetryClass::Never),
        Some("permission_error") => (ModelFailureCode::PermissionDenied, RetryClass::Never),
        Some("rate_limit_error") => (ModelFailureCode::RateLimited, RetryClass::AfterBackoff),
        Some("overloaded_error" | "api_error") => {
            (ModelFailureCode::Unavailable, RetryClass::AfterBackoff)
        }
        _ => (ModelFailureCode::MalformedResponse, RetryClass::Never),
    };
    ModelFailure::safe(code, message, retry)
        .unwrap_or_else(|_| ModelFailure::internal_adapter_failure())
}

fn stream_index(value: &Value) -> Result<ModelStreamIndex, AnthropicError> {
    let raw = value
        .get("index")
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed("content block index missing"))?;
    let index = u16::try_from(raw).map_err(|_| malformed("content block index is too large"))?;
    ModelStreamIndex::new(index).map_err(|_| malformed("content block index is invalid"))
}

fn malformed(message: &'static str) -> AnthropicError {
    AnthropicError::new(AnthropicErrorCode::MalformedResponse, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPAQUE_SENTINEL: &str = "opaque-continuation-must-not-appear-in-debug";

    #[test]
    fn reducer_debug_redacts_accumulated_payloads() {
        let mut reducer = AnthropicReducer::new();
        reducer.tool_calls.insert(
            0,
            ToolCallAccumulator {
                index: 0,
                provider_call_id: "tool_0".to_owned(),
                name: "read".to_owned(),
                arguments: OPAQUE_SENTINEL.to_owned(),
            },
        );
        reducer.hosted_tools.insert(
            1,
            HostedToolAccumulator {
                index: 1,
                provider_call_id: "search_0".to_owned(),
                name: "web_search".to_owned(),
                arguments: String::new(),
                raw_block: json!({"opaque": OPAQUE_SENTINEL}),
                completed: true,
            },
        );
        reducer
            .source_owners
            .insert(OPAQUE_SENTINEL.to_owned(), "search_0".to_owned());

        let debug = format!("{reducer:?}");
        assert!(!debug.contains(OPAQUE_SENTINEL), "{debug}");
    }
}
