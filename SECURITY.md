# Security Policy

## Supported versions

The `0.1.x` release line is supported on a best-effort basis while the public
API evolves.

## Reporting a vulnerability

Please use GitHub private vulnerability reporting for this repository when it
is available. Do not open a public issue with exploit details.

If private reporting is unavailable, open a minimal public issue asking for a
private security contact without including the vulnerability details.

Include synthetic reproduction steps, the affected crate or feature, impact,
required preconditions, and a suggested mitigation when safe to do so.

Never include API keys, access tokens, cookies, private source, personal data,
or unredacted provider requests and responses.

## Security boundaries

Tea's policy and approval flows authorize operations; they do not provide an
operating-system sandbox. Native tools normally run with the permissions of
the host process. Use an explicit sandbox, container, restricted subprocess,
or remote execution boundary when stronger isolation is required.

Configured providers, MCP servers, project instructions, and tool output are
untrusted inputs. Review configuration and permissions before running Tea in
an untrusted workspace.

## Secrets and test data

Keep credentials in environment variables or a secret manager. Test fixtures
must use synthetic values and must not contain real conversations, production
payloads, authorization headers, or session data.
