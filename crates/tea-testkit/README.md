# tea-testkit

Deterministic test doubles and reusable conformance utilities for `tea-rs`.

The Cargo package is `tea-testkit`; Rust code imports it as `tea_testkit`. It performs no network access, credential lookup, wall-clock sleep, nested runtime creation, or detached task spawning.

## Scripted model provider

`ScriptedModelProvider` consumes immutable `ScriptedModelResponse` values in FIFO order and captures every `ModelRequest`. Scripts can:

- emit deterministic text and thinking chunks;
- emit one or several complete tool calls;
- fail before content or during streaming;
- simulate context overflow;
- remain pending until cooperative cancellation;
- represent intentionally malformed normalized event sequences for adapter tests.

Missing scripts and poisoned inspection state become terminal internal failures rather than panics.

```rust
use tea_testkit::ScriptedModelResponse;

let response = ScriptedModelResponse::text(["deterministic ", "answer"]);
assert_eq!(response.steps().len(), 4); // start, two deltas, completion
```

## Conformance utilities

- `collect_model_stream` collects events and applies `ModelStreamValidator`.
- `run_model_provider_case` verifies model advertisement, provider ownership, request capabilities, and stream grammar.
- `run_cancelled_model_provider_case` polls a stream, cancels it from the owning scope, and awaits its terminal failure.
- `ModelConformanceReport` records event count, completed tool count, terminal kind, stop reason, and failure code.

Live adapters should use these helpers with mocked HTTP transports. Unit tests must not depend on a paid API, real credentials, provider availability, or scheduler timing.

## Fake tools

- `FakeReadTool` reads a deterministic in-memory path map.
- `FakeWriteTool` captures schema-validated path/content mutations.
- `FakeProcessTool` emits scripted progress, success, failure, or waits for cancellation without spawning a process.
- `collect_tool_execution` applies terminal stream grammar and returns a conformance report.

Fake tools implement the production `ToolExecutor` port and are intended to run through `ToolRegistry`, so tests exercise argument validation, resource resolution, output validation, and cancellation. They never access the real filesystem, process table, or network.
