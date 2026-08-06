# tea-policy

Pure policy evaluation, approvals, grants, expiry/revocation, and redacted presentation for `tea-rs`.

The package is `tea-policy`; Rust code imports it as `tea_policy`. Its normal
dependency graph is Tokio-free and contains no futures, provider SDK, tool
executor, filesystem, process, network, database, or wall-clock implementation.

## Policy input

`PolicyInput` can only be built from `ValidatedToolInvocation`. It snapshots:

- actor and product profile;
- session, optional run, and optional workspace;
- tool call, canonical tool name, and semantic version;
- schema-validated arguments;
- declared effects and resolved resources;
- execution surface/target and bounded environment metadata;
- caller-supplied canonical evaluation time;
- bounded candidate grants.

Policy never reads the clock. The caller supplies `now`, making expiry behavior deterministic.

## Ordered composition

Rules execute in fixed authority order:

```text
Platform -> Organization -> Product -> Workspace
```

Decision restriction is monotonic:

```text
Allow < Redirect < Ask < Deny < HardDeny
```

A lower layer can narrow but cannot broaden a previous decision. `HardDeny` terminates immediately. Empty or fully abstaining engines fail closed. Rule traces contain only bounded rule IDs, layers, and decisions—not raw arguments.

`UnknownEffectPolicy` hard-denies namespaced effects this runtime does not understand. `CodingWorkspacePolicy` and `DesktopPolicy` provide deterministic product-policy examples. Process decisions may inspect validated arguments: `git status` can be allowed while destructive commands ask for approval, even when both share one executor category. Tool names are not used to infer side effects.

## Grants

`PolicyGrant` is serializable and constrained by:

- actor and profile;
- tool name and exact semantic version;
- effect subset;
- resource scheme, locator prefix, and optional access;
- once, run, session-resource, or expiring persistent-resource scope;
- issuance, expiration, and optional revocation timestamps.

A grant can satisfy `Ask` only. It never overrides `Deny` or `HardDeny`. Persistent grants must expire, and all grants require non-empty effect/resource constraints.

This crate defines grant values and deterministic evaluation inputs but does not
own storage. `tea-session` provides the append-only approval/grant journal
contract, and `tea-session-sqlite` provides its durable SQLite implementation.

## Approval and redaction

`ApprovalRequest` snapshots bounded policy context plus `ApprovalPresentation`. Arguments are recursively redacted using explicit case-insensitive key normalization for tokens, passwords, secrets, API keys, authorization, cookies, and private keys. Credential/secret resources and URL query values are hidden. Original arguments are unchanged.

`ApprovalResolution` uses canonical protocol decisions and may attach a matching grant only to `AllowSession`. Request/response time and direct deserialization boundaries are validated. Denial projects to canonical machine-readable `approval_denied` tool failure.

Approval decisions remain canonical protocol transitions. Rich approval
artifacts and grant journal facts are persisted by the selected session store,
while this crate remains responsible for validation, matching, expiry, and
redaction.

## Tool feature isolation

`tea-policy` depends on `tea-tools` with default features disabled, loading only metadata/value contracts. Tool execution, schema compiler, model projection, futures, and shared Tokio cancellation are feature-gated out of the normal policy graph. Policy tests enable execution only as development fixtures to create real validated invocations.
