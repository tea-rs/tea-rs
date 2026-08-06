# tea-protocol

Canonical, provider-neutral protocol types for `tea-rs`.

The crate is intentionally limited to pure domain values and JSON serialization contracts. It does not execute models or tools, persist sessions, evaluate policy, manage asynchronous tasks, or depend on Tokio.

## Status

Protocol **1.0** is implemented. The Cargo package is named `tea-protocol`,
while Rust code imports it as `tea_protocol`.

## Protocol surface

The crate owns four distinct contracts:

- `CommandEnvelope` and `AgentCommand`: requested actions that a host accepts or rejects;
- `EventEnvelope` and `AgentEvent`: observable lifecycle and streaming output;
- `RecordEnvelope` and `SessionRecord`: required append-only facts for replay and recovery;
- `ProtocolErrorEnvelope` and `ProtocolError`: stable machine-readable failures with safe diagnostics.

Shared types include:

- UUIDv7 strong IDs for sessions, runs, turns, messages, tools, approvals, commands, events, records, branches, causation, and correlation;
- decimal-string `SessionSequence` values for authoritative session-local ordering;
- canonical RFC 3339 UTC millisecond timestamps;
- user, assistant, and tool-result messages;
- text, thinking, image, and tool-call content blocks;
- exact decimal cost and JavaScript-safe token usage;
- bounded reverse-domain metadata.

## Example

```rust
use std::str::FromStr;

use tea_protocol::{
    AgentCommand, CommandEnvelope, CommandId, ProfileId, ProtocolMetadata,
    ProtocolTimestamp,
};

let command = CommandEnvelope::new(
    CommandId::from_str("0195a0b1-5e3b-7ef0-8ec1-0aa7aa000001")?,
    None,
    ProtocolTimestamp::from_str("2026-07-23T09:30:12.123Z")?,
    AgentCommand::CreateSession {
        profile_id: ProfileId::from_str("minimal-assistant")?,
        metadata: ProtocolMetadata::default(),
    },
)?;

let json = serde_json::to_string_pretty(&command)?;
assert!(json.contains(r#""type": "create_session""#));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Compatibility rules

Protocol versions are independent from crate SemVer. This crate currently writes `1.0` and accepts known envelopes from protocol major version `1` when every known field remains valid.

Within the same protocol major:

- unknown optional object fields on known types are ignored;
- bounded namespaced metadata is preserved at documented extension points;
- unknown commands are rejected as `unsupported_command`;
- unknown durable records stop replay as `unsupported_record`;
- unknown observable events are skippable only when their validated envelope explicitly declares `compatibility: "skippable_observation"`;
- unknown enum values are rejected unless the specific type documents preservation, such as `StopReason`;
- duplicate JSON object keys are rejected recursively at public envelope and metadata boundaries.

Canonical JSON uses `camelCase` fields, a `type` discriminator, and `snake_case` discriminator values.

## Ordering and durability

`SessionSequence` is the authoritative order for observable session events and durable records. UUID and timestamp ordering must not drive replay.

Events and records are intentionally different:

- text deltas and tool progress can be transient observations;
- final messages, approval transitions, tool execution boundaries, interruption state, branch changes, compaction provenance, and turn checkpoints are durable records;
- every protocol 1.0 durable record kind is required for replay and cannot be skipped.

This crate defines record values only. Store append transactions, expected-sequence
concurrency, reducers, projections, and SQLite persistence are supplied by
`tea-session` and `tea-session-sqlite`.

## Resource limits

Public constants expose the exact limits. Important defaults include:

| Value | Limit |
| --- | ---: |
| Metadata namespaces | 16 |
| Encoded metadata | 16 KiB |
| Metadata depth | 8 |
| Text/thinking block | 1 MiB |
| Inline Base64 image | 6 MiB encoded |
| Tool arguments | 256 KiB, depth 32 |
| Streaming delta | 64 KiB |
| Unknown skippable event | 64 KiB |
| Error technical message | 4 KiB |
| Token count | `Number.MAX_SAFE_INTEGER` |

Transport adapters must additionally enforce frame and request limits before deserialization.

## Security boundary

Constructors, deserialization, and serialization validate wire invariants. Public enums remain matchable by adapters, but directly constructed invalid values cannot cross the JSON boundary.

Protocol errors contain stable codes, English technical diagnostics, safe details, and optional correlation IDs. Internal causes, stack traces, credentials, authorization headers, filesystem secrets, and raw provider bodies are not serialized by default.

See the public [Tea documentation](https://github.com/tea-hq/tea-docs) for
protocol compatibility and session durability guidance.
