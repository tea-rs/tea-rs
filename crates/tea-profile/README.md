# tea-profile

Tokio-free, versioned product profile schema, validation, composition, and example profiles for `tea-rs`.

The package is `tea-profile`; Rust code imports it as `tea_profile`. Its normal dependency graph contains no model provider, tool executor, session store, clock, filesystem, process, network, database, or async runtime. It depends only on protocol selectors, tool metadata, policy environment types, Serde, and SemVer.

## Profile schema

`AgentProfile` is a versioned, serializable, product-declarative description of how one product configures the runtime. It carries a `ProfileSchemaVersion`, bounded display name and description, `ProfileId`, `ModelId`, ordered unique active `ToolName` selectors, ordered unique policy rule id references, a `ProfilePromptBudget`, `ProfileRunLimits`, a `PolicyEnvironment`, an approval lifetime, and optional `ProfileWorkspaceInstruction` documents.

Every selector, duration, count, and byte bound is validated at construction. Oversized, empty, duplicate, zero, or mismatched-schema values are stable `ProfileError` values. The schema version is checked at every deserialization and composition and is independent of protocol compatibility and crate SemVer.

## Composition

`ProfileOverlay` merges a base profile with optional override fields. `None` overlay fields inherit the base; `Some` fields replace wholesale. A schema-version or profile-id mismatch is a `CompositionConflict`. The composed result revalidates every bound.

## Example profiles

Three validated constructors ship inside this crate: `AgentProfile::minimal_assistant`, `AgentProfile::coding_agent`, and `AgentProfile::desktop_assistant`. They reference canonical selectors and conservative limits. No example profile is wired into the kernel or used as a default; the kernel remains product-agnostic.

Policy rule ids and tool names are declarative references resolved by the runtime against builder-registered rules and tools; the profile never instantiates a rule or owns an executor.
