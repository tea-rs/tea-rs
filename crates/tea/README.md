# tea

Ergonomic embedding facade and product profile wiring for `tea-rs`.

The published package is `tea-rs`; Rust code imports it as `tea`. It owns replaceable inward-facing ports (model provider, tool registry, policy engine, session store, clock, ID source, event sink, context providers, prompt compiler) and exposes an in-process command sender, bounded event subscription, session snapshots, and health inspection. It contains no product prompt, live provider, UI, filesystem, process, network, or database adapter.

## Core SDK namespaces

Consumers can name every core contract through the facade without adding the
workspace crate split to their import paths:

```rust
use tea::{
    AgentRuntimeBuilder,
    model::{ModelProvider, ModelSpec},
    policy::ActorId,
    profile::AgentProfile,
    protocol::{AgentCommand, CommandEnvelope},
};
```

The complete core namespaces are `tea::context`, `tea::control`, `tea::kernel`,
`tea::model`, `tea::policy`, `tea::profile`, `tea::protocol`, `tea::session`,
and `tea::tools`. Concrete providers, SQLite, MCP, native coding tools, and
product surfaces remain separate adapter dependencies selected by the host.

## Runtime builder

`AgentRuntimeBuilder` wires the model, tools, policy rules, session store, context providers, clock, ID source, and prompt compiler, then registers one or more `AgentProfile` values. At `build`, the runtime precomputes one immutable `ProfileBinding` per profile: a filtered `ToolRegistry` containing only the profile's active tools, a `PolicyEngine` composed from the profile's resolved policy rules plus the platform `UnknownEffectPolicy`, an ordered context-provider list, and converted `RunLimits` and `PromptBudget`.

## Profiles and bindings

A profile is a declarative, versioned, serializable description from `tea-profile`. The runtime resolves its tool and policy rule references against builder-owned registrations; an unresolved reference fails construction. The kernel remains product-agnostic: the runtime constructs a fresh `AgentKernel` borrowing runtime-owned ports for the duration of one async invocation.

## Commands and events

`AgentRuntime::send` accepts a canonical `CommandEnvelope` and dispatches `CreateSession`, `Prompt`, `Steer`, `FollowUp`, `Abort`, `ResolveApproval`, `SetModel`, `SetProfile`, `CompactSession`, and `ForkSession`. `subscribe(session_id)` returns a bounded receiver of `EventEnvelope` values; a full channel applies backpressure to the run, and a dropped receiver is removed. New sessions are branch-aware; legacy unbranched sessions remain readable but cannot fork implicitly.

`attach_session` validates stored profile/model compatibility. Optional `SessionCatalog` wiring provides listing and bounded display names separately from append semantics. `snapshot`, `session_state`, `session_stats`, and `health` expose immutable host queries without creating a second authoritative transcript.

The runtime never creates a Tokio runtime, calls `block_on`, sleeps in tests, or uses wall-clock entropy.

## SDK integration

Use the independent [Tea SDK documentation](https://github.com/tea-hq/tea-docs/blob/main/src/content/docs/sdk/quick-start.md)
for application setup, provider adapters, tools, storage, and MCP integration.
This crate README describes the API surface; it does not require consumers to
enter or build the Tea workspace.
