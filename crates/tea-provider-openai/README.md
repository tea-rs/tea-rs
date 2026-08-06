# tea-provider-openai

OpenAI-compatible streaming model provider for `tea-rs`.

The package is `tea-provider-openai`; Rust code imports it as
`tea_provider_openai`. It implements the provider-neutral `tea_model::ModelProvider`
contract for OpenAI Chat Completions and Responses APIs, including streaming
text, reasoning, images, function tools, Responses hosted web search, usage, and
provider continuation data.

## Configuration

`OpenAiConfig` can be constructed directly for an injected credential/configuration
source. `EnvCredentialResolver` provides the process environment contract for
the CLI and examples:

```text
TEA_OPENAI_API_KEY
TEA_OPENAI_MODEL
TEA_OPENAI_BASE_URL       (optional; defaults to https://api.openai.com/v1)
TEA_OPENAI_API_MODE       (optional; chat-completions or responses)
TEA_OPENAI_REQUEST_TIMEOUT_MS (optional)
```

The adapter is gateway-agnostic: changing `TEA_OPENAI_BASE_URL` or using an
injected `OpenAiConfig` selects an OpenAI-compatible endpoint. Credentials are
resolved at request time and are not stored in model requests, events, or
session records.

## Integration

Use `OpenAiProviderBuilder` to create a provider and advertise its model
catalog. The adapter owns HTTP request mapping, SSE parsing, capability
validation, retry classification, and normalization into `tea_model` events;
runtime policy, session storage, and tool execution remain host-selected.

See the public [Tea integration documentation](https://github.com/tea-hq/tea-docs)
for provider selection and application setup.
