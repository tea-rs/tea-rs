# tea-tools

Portable tool specifications, offline schema validation, resources, registry behavior, execution streams, and scheduler metadata for `tea-rs`.

The package is `tea-tools`; Rust code imports it as `tea_tools`. It defines
portable contracts and performs no real filesystem, process, network, MCP,
sandbox, or policy operation.

Default features preserve the complete tool runtime. `execution` enables schema compilation, registry, streams, and shared cancellation; `model-projection` enables conversion to `ModelToolDefinition`. Pure policy consumers disable default features to use only tool metadata, resources, results, and invocation values without Tokio/futures/schema dependencies.

## Contract layers

- `ToolSpec`: identity, semantic version, model-facing description and object schemas, effects, prompt/UI hints, timeout, concurrency, idempotency, and retry safety.
- `ToolInvocation`: complete but untrusted object arguments.
- `ValidatedToolInvocation`: constructible only after registry schema validation and pure resource resolution.
- `ToolExecutor`: object-safe lazy stream port receiving only validated invocations and shared cancellation.
- `ToolResult`: model-visible text/images, bounded machine output, safe details, and optional tool-specific usage.
- `ToolRegistry`: deterministic registration, conflict detection, validation, resolution, execution delegation, and output-contract enforcement.

## Schema contract

Input and output schemas use JSON Schema Draft 2020-12. Schemas and values are limited to 256 KiB encoded JSON and depth 32. Validation returns at most 16 sorted normalized diagnostics.

External `$ref` retrieval is rejected. The `jsonschema` crate is built without its default HTTP/file resolver features, so schema compilation never reads local files or makes network requests.

`jsonschema 0.26.2` and `idna 1.0.3` are pinned in the workspace dependency
graph. The direct `idna` dependency constrains a compatible transitive version;
it is not used as a product API. The crate follows the workspace MSRV of Rust
1.97.1.

## Effects and scheduling

Known effects are:

- `fs.read`, `fs.write`, `fs.delete`;
- `process.spawn`, `network.request`;
- `credential.read`, `clipboard.read`;
- `user.interaction`, `external.mutation`.

New effects must be lowercase namespaced dotted values. Unknown effects are preserved but fail closed as `PolicyRequired`, serial, and not automatically retryable. Scheduler classification uses declared effects and execution semantics only, never tool names.

Non-idempotent tools cannot declare automatic retry. A timeout is metadata
consumed by the runtime; this crate validates and exposes it but creates no
timer. An interrupted uncertain operation is not automatically replayable.

## Registry and streams

Registration compiles both schemas atomically. One active name cannot have duplicate or conflicting versions. Registry iteration uses canonical `ToolName` order.

Execution order is:

```text
untrusted invocation
  -> input schema validation
  -> pure resource resolution
  -> validated invocation
  -> executor stream
  -> output schema validation
  -> terminal result/failure
```

Invalid arguments never reach an executor. Streams emit zero or more progress events followed by exactly one successful or failed terminal event. Missing terminal events and invalid output are normalized to typed contract failures. No event may follow terminal.

See `tea-testkit` for fake read/write/process executors and reusable conformance collection.
