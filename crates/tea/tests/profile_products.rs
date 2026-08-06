use crate::common;

use std::str::FromStr;
use std::sync::Arc;

use common::{TestIds, TestSessionIds, build_runtime};
use tea_model::{ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId};
use tea_protocol::{ModelId, TokenCount};
use tea_testkit::ScriptedModelProvider;

fn provider() -> Arc<ScriptedModelProvider> {
    let provider_id = ProviderId::from_str("fake").unwrap();
    let model = ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        provider_id.clone(),
        ModelDisplayName::from_str("Fake Model").unwrap(),
        TokenCount::new(32_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text().with_tools(true),
    )
    .unwrap();
    Arc::new(ScriptedModelProvider::new(provider_id, vec![model], []))
}

#[test]
fn product_profiles_share_the_facade_contract_and_bind_distinct_behavior() {
    let runtime = build_runtime(
        provider(),
        Arc::new(TestIds::default()),
        Arc::new(TestSessionIds::default()),
    )
    .unwrap();
    let coding = runtime.binding(&"coding-agent".parse().unwrap()).unwrap();
    let desktop = runtime
        .binding(&"desktop-assistant".parse().unwrap())
        .unwrap();

    assert_eq!(coding.model_id(), desktop.model_id());
    assert_eq!(coding.actor_id(), desktop.actor_id());
    assert_eq!(coding.workspace_id(), desktop.workspace_id());
    assert_ne!(coding.environment(), desktop.environment());
    assert_ne!(coding.run_limits(), desktop.run_limits());

    let coding_tools = coding
        .active_tool_specs()
        .iter()
        .map(|spec| spec.name().as_str())
        .collect::<Vec<_>>();
    let desktop_tools = desktop
        .active_tool_specs()
        .iter()
        .map(|spec| spec.name().as_str())
        .collect::<Vec<_>>();
    assert_eq!(coding_tools, ["read_file", "write_file"]);
    assert_eq!(desktop_tools, ["clipboard_read", "write_file"]);
}
