# tea-model

Provider-neutral model specifications, requests, cancellation, streaming events, failures, and adapter ports for `tea-rs`.

The Cargo package is `tea-model`; Rust code imports it as `tea_model`. It contains no live provider adapter, HTTP client, credentials, retry loop, agent loop, or persistence implementation.

## Responsibilities

- validated `ModelSpec` values and capability declarations;
- immutable provider-neutral `ModelRequest` turn snapshots;
- model-visible tool names, descriptions, and bounded object JSON Schemas;
- provider-neutral reasoning effort and budget;
- project-owned cooperative `ModelCancellation`;
- normalized model events, failures, stop reasons, usage, and exact cost;
- object-safe `ModelProvider` and `ModelStream` ports;
- deterministic `ModelStreamValidator` grammar checks.

## Example

```rust
use std::str::FromStr;

use tea_model::{
    ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId,
};
use tea_protocol::{ModelId, TokenCount};

let model = ModelSpec::new(
    ModelId::from_str("example/model")?,
    ProviderId::from_str("example")?,
    ModelDisplayName::from_str("Example Model")?,
    TokenCount::new(32_000)?,
    TokenCount::new(8_000)?,
    ModelCapabilities::text().with_reasoning().with_tools(true),
)?;

assert!(model.capabilities().supports_parallel_tool_calls());
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Stream grammar

A fully consumed conforming stream:

1. emits exactly one `ModelEvent::Started` first;
2. emits zero or more text, thinking, and tool-call events;
3. emits exactly one terminal `Completed` or `Failed` event;
4. emits nothing after termination.

Tool calls use a response-local bounded index and opaque provider call ID. A tool index cannot be reused in one response. Argument deltas are incomplete strings and are never executable. Only `ToolCallCompleted` carries parsed, bounded JSON object arguments. Successful termination is rejected while any tool call remains incomplete.

`Completed` is limited to successful normalized stop reasons: completed, length, or tool use. Cancellation and provider/runtime errors use typed terminal `Failed` events. Internal source chains, raw HTTP bodies, credentials, and SDK errors are not stored in `ModelFailure`.

## Cancellation and ownership

`ModelCancellation` is a compatibility alias of `tea_control::CancellationScope`. The shared control crate wraps Tokio-util internally without exposing `CancellationToken`. Providers receive cancellation separately from the immutable request.

Provider streams are lazy and own their resources. Implementations must not create nested runtimes or detached tasks. Dropping a stream abandons it; explicit cancellation is cooperative, and terminal cancellation should be emitted only after stream-owned resources are cleaned up.

## Adapter responsibilities

Provider adapters must:

- translate canonical messages and tool schemas;
- validate requests against advertised model capabilities;
- normalize streaming output and failures;
- preserve provider continuation signatures only behind bounded namespaced metadata;
- normalize usage, exact cost, and stop reasons;
- report setup and streaming failures as terminal events rather than panics;
- pass the reusable conformance utilities in `tea-testkit` using mocked transports before any live API test.

The public API intentionally contains no OpenAI, Anthropic, Vercel AI SDK, HTTP, SSE, or WebSocket types.
