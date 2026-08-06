# tea-coding-tools

Workspace-scoped native coding tool adapters for `tea-rs`.

The package is `tea-coding-tools`; Rust code imports it as `tea_coding_tools`. It implements the reference Coding CLI's workspace-confined `read`, `grep`, `find`, `ls`, `write`, `edit`, and `bash` executors, plus optional client `web_search` and `web_fetch` adapters. It depends only on inward control, protocol, and tool contracts and does not depend on runtime composition, provider, persistence, CLI, or terminal crates.

The crate exposes `WorkspaceRoot`, `ResolvedExistingPath`, and `ResolvedMutationPath` as the mandatory boundary for path-based executors. A workspace capability is constructed only from a canonical existing directory; model-supplied paths must be bounded UTF-8 relative paths and cannot contain parent traversal, absolute/platform prefixes, backslashes, or control characters. Existing targets and nearest existing mutation ancestors are canonicalized after symlink resolution and must remain beneath that root. Returned display paths are normalized and never reveal the host root.

Path-based host APIs cannot atomically combine validation with a later open or rename. Executors call `revalidate_existing` before open, verify the opened handle identity, and call `revalidate_mutation` immediately before mutation commit. These checks compare canonical resolution and filesystem identity and fail closed after detectable target, ancestor, symlink, or root replacement. Descriptor-relative operations should replace path-based operations where a safe portable implementation is practical.

The workspace tools are:

- `read`: reads at most 32 KiB of UTF-8 text with optional 1-based line offset/limit; rejects binary, invalid UTF-8, directories, and oversized files.
- `write`: creates or replaces at most 192 KiB of UTF-8 text through a mode-`0600` sibling temporary file, flush/sync, pre-commit revalidation, atomic rename where supported, and permission preservation for existing files.
- `edit`: performs exact text replacement, defaults to exactly one match, accepts an explicit positive expected count, detects stale source content, and never mutates on zero/ambiguous/mismatched/oversized input.
- `bash`: runs only through a host-configured absolute shell and workspace cwd. Stdout/stderr are drained concurrently, each retains at most 16 KiB in memory, excess output spills up to 8 MiB to a host-configured state directory, and at most 64 progress events are emitted. Spill files are mode `0600` on Unix and inherit the state-directory ACL on Windows; unreferenced spill files are deleted on failure, cancellation, or stream drop.

`grep`, `find`, and `ls` are read-only workspace search/list tools. `web_search`
and `web_fetch` are client adapters with explicit network resources and must be
enabled and supplied with a host provider; they are not implicitly active.

Every executor constructor requires a `WorkspaceRoot`; specs declare effects, resources, timeout, idempotency, retry safety, and serial/parallel scheduling. Failures expose bounded path-independent messages and a machine-readable detail code. The synchronous filesystem phase checks cooperative cancellation before any side effect and does not claim mid-system-call cancellation.

`bash` uses an owned Unix process group with TERM→KILL cleanup. Windows uses `CREATE_NEW_PROCESS_GROUP` plus the system `taskkill /T /F` process-tree equivalent without workspace `unsafe` code. Cancellation, timeout, output-limit failure, and direct stream drop terminate and reap the owned tree where the platform permits; cancellation/timeout after spawn report `uncertain: true` and are never retryable. Captured canonical strings retain bounded terminal controls for model/JSON serialization except invalid UTF-8/NUL normalization; terminal renderers must sanitize controls rather than writing tool strings directly.
