# tea-provider-anthropic

Streaming Anthropic Messages API adapter for tea-rs.

Configure the adapter through an injected credential resolver or the process
environment contract used by the CLI:

```bash
export TEA_PROVIDER=anthropic
export TEA_ANTHROPIC_API_KEY='...'
export TEA_ANTHROPIC_MODEL='<anthropic-model-id>'
tea --provider anthropic --model "$TEA_ANTHROPIC_MODEL"
```

`TEA_ANTHROPIC_BASE_URL` defaults to `https://api.anthropic.com`,
`TEA_ANTHROPIC_API_VERSION` defaults to `2023-06-01`, and
`TEA_ANTHROPIC_REQUEST_TIMEOUT_MS` defaults to `60000`. The adapter supports
text, images, function tools, parallel tool calls, usage reporting, and the
Anthropic hosted web-search tool. Hosted web-search options are configured with
`TEA_ANTHROPIC_WEB_SEARCH_TOOL_TYPE` and
`TEA_ANTHROPIC_WEB_SEARCH_MAX_USES` when the host activates that tool.

Extended thinking is not currently supported. Provider-specific request and
stream types remain behind this adapter; hosts consume normalized
`tea_model` events.
