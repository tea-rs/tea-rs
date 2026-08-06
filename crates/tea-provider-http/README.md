# tea-provider-http

Shared HTTP client identity and construction policy for `tea-rs` model provider adapters.

This crate owns provider-neutral transport settings such as User-Agent validation and timeout-aware
`reqwest::Client` construction. Provider-specific authentication, request mapping, streaming, and
error classification remain in their adapter crates.

HTTP provider builders accept `ProviderHttpConfig` and construct their clients through
`ProviderHttpConfig::build_client`. Each provider keeps a local TCP contract test so shared policy is
verified at the actual request boundary.
