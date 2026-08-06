# tea-mcp

`tea-mcp` is the outward Model Context Protocol adapter for `tea-rs`. It provides
bounded, redacting server configuration and health contracts, an owned stdio
client transport, deterministic frozen tool discovery, and `ToolExecutor`
integration for discovered bindings.

The package is `tea-mcp`; Rust code imports it as `tea_mcp`.

## Dependency boundary

This crate may adapt MCP protocol values into `tea-tools` contracts. It must not
depend on `tea-kernel`, `tea-policy`, `tea-session`, `tea-coding`, or `tea-cli`,
and none of those inward contract crates expose MCP SDK or transport types.

Configuration contains an exact absolute executable, exact argument vector,
environment variable names only, disabled-by-default host tool declarations,
and deterministic lifecycle bounds. Debug output redacts executable and argument
values. The stdio client starts that executable without a shell or ambient
environment, bounds JSONL frames and stderr, validates request/response
correlation, and owns process-tree shutdown. Configuration and process ownership
do not imply that a stdio process is sandboxed.

Discovery walks bounded `tools/list` pages under one absolute deadline and maps
only enabled host-declared tools into an immutable alias-sorted catalog. Remote
descriptor JSON and authoritative host policy JSON are canonicalized and hashed
with SHA-256; the digest scopes `ToolSource` and `ToolVersion`. Input/output
schemas compile through the existing offline validator, remote annotations stay
untrusted diagnostics, and every binding resolves an explicit `mcp-server`
execute resource before it can be passed to `McpToolExecutor`.

`McpManager::tool_executor` binds an exact discovered tool to the managed stdio
connection. Execution validates request/response correlation, bounds results and
progress, propagates cooperative cancellation, and shuts down owned processes.
MCP processes are executable code and are not a sandbox.
