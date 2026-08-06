# tea-kernel

Tokio-native, headless model-tool state machine for `tea-rs`.

The package is `tea-kernel`; Rust code imports it as `tea_kernel`. It coordinates the provider-neutral model port, portable tool registry, pure policy engine, append-only session store, cancellation tree, clock/ID sources, and awaited canonical event sink. It contains no product prompt, live provider, UI, filesystem, process, network, or database adapter.

## Durable ordering

The kernel enforces these boundaries:

1. Each provider call receives an immutable request built from the last committed active session projection.
2. A complete assistant message and all matching `ToolCallRequested` records commit before policy or execution.
3. Registry validation and resource resolution produce the exact immutable invocation evaluated by policy and later passed to `execute_validated`.
4. Policy decision and `ToolExecutionStarted` commit before polling an allowed executor.
5. Tool terminal and model-visible tool-result message commit atomically.
6. Policy Ask commits canonical policy/request/checkpoint records and the rich redacted request artifact atomically before returning `WaitingApproval`.
7. Approval resume loads the persisted request and arguments, rejects actor/workspace/profile/environment/tool/resource drift, then commits its canonical/rich resolution, optional grant, and execution start—or denial terminal/result—atomically.

Parallel-safe tool calls may execute concurrently in scheduler lanes; serial and
exclusive calls are ordered by their declared execution semantics. Results are
committed in canonical model source order. Unknown tools and invalid arguments
become durable machine-readable failures and never reach policy or an executor.

## Recovery and limits

`AgentKernel::resume_approval` works with a fresh kernel instance. Event sinks may retain a session cursor so observation sequence remains monotonic across pause/resume. Per-run tool iterations and assistant output usage are reconstructed from durable records rather than hidden process state.

Cancellation and deadlines are selected cooperatively while polling model and tool streams. Partial model output is never committed. Cancellation after a durable tool start records `ToolExecutionInterrupted` with uncertain outcome before the run terminal. A normal new run rejects pending approvals and incomplete/uncertain tools, so non-idempotent work is never replayed implicitly.

Hard limits cover elapsed time, tool iterations, accumulated assistant output bytes, emitted events, queued messages, and steering bytes. The bounded `KernelInputQueue` snapshots steering/follow-ups only between durable turns; an active model request cannot be mutated.

The kernel does not create runtimes, call `block_on`, or sleep in tests. Bounded
model retry is configured by `ModelRetryPolicy`; effect-aware parallel tool
scheduling is built in; automatic compaction is available when the host supplies
a policy and summarizer and is disabled by default. Product prompt compilation,
live providers, and concrete tool adapters remain outside this crate.
