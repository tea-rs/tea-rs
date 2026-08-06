use crate::common;

use std::str::FromStr;

use common::{module, segment};
use futures_util::FutureExt;
use tea_context::{
    BudgetBehavior, ContextProvider, ContextProviderId, ContextRequest, PromptAuthority,
    StaticContextProvider,
};
use tea_protocol::{ProfileId, ProtocolMetadata, SessionId};

fn request() -> ContextRequest {
    ContextRequest::new(
        ProfileId::from_str("coding").unwrap(),
        SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap(),
        None,
        vec![],
        ProtocolMetadata::default(),
    )
    .unwrap()
}

fn assert_provider(_provider: &dyn ContextProvider) {}

#[test]
fn provider_is_object_safe_send_sync_and_returns_immutable_snapshot() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StaticContextProvider>();
    let provider = StaticContextProvider::new(
        ContextProviderId::from_str("static.test").unwrap(),
        vec![module(
            "product",
            PromptAuthority::Product,
            0,
            vec![segment("identity", "content", BudgetBehavior::Required)],
        )],
    );
    assert_provider(&provider);
    let first = provider.provide(request()).now_or_never().unwrap().unwrap();
    let second = provider.provide(request()).now_or_never().unwrap().unwrap();
    assert_eq!(first, second);
    assert_eq!(provider.id().as_str(), "static.test");
}

#[test]
fn context_request_preserves_profile_session_and_metadata() {
    let request = request();
    assert_eq!(request.profile_id().as_str(), "coding");
    assert_eq!(
        request.session_id().to_string(),
        "0195a0b1-5e3a-7d72-a902-c4e85d828bf1"
    );
    assert!(request.run_id().is_none());
    assert!(request.active_tools().is_empty());
    assert!(request.metadata().is_empty());
}
