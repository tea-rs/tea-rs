#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Anthropic Messages streaming provider adapter for `tea-rs`.
//!
//! The adapter maps provider-neutral requests and tool definitions to the
//! Anthropic Messages API, then reduces Server-Sent Events into normalized
//! `tea_model::ModelEvent` values.

pub(crate) const WEB_SEARCH_CONTINUATION_FORMAT: &str = "anthropic.messages.web_search.v1";

pub mod catalog;
pub mod credential;
pub mod error;
pub mod provider;
pub mod request;
pub mod sse;
pub mod stream;

pub use credential::{
    AnthropicConfig, AnthropicWebSearchConfig, ApiKey, CredentialResolver,
    DEFAULT_WEB_SEARCH_MAX_USES, DEFAULT_WEB_SEARCH_TOOL_TYPE, EnvCredentialResolver,
    MapCredentialResolver, PROVIDER_ID,
};
pub use error::{AnthropicError, AnthropicErrorCode};
pub use provider::{AnthropicProvider, AnthropicProviderBuilder};
pub use tea_provider_http::{ProviderHttpConfig, UserAgent};
