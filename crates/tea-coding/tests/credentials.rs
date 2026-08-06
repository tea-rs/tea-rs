use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use tea_coding::config::{CodingSettings, SettingsLayer, merge_settings};
use tea_coding::mcp_config::{
    McpEnvironmentErrorCode, McpLifecycleSettings, McpLimitsSettings, McpServerSettings,
    McpTransportSettings, StaticMcpEnvironmentResolver, resolve_mcp_environment,
};
use tea_coding::{CodingCredentialResolver, McpEnvironmentResolver};
use tea_provider_openai::MapCredentialResolver;

#[test]
fn credential_debug_and_settings_serialization_never_expose_secrets() {
    let secret = "sk-super-secret-value";
    let resolver = MapCredentialResolver::new(BTreeMap::from([
        ("TEA_OPENAI_API_KEY".to_owned(), secret.to_owned()),
        ("TEA_OPENAI_MODEL".to_owned(), "gpt-test".to_owned()),
    ]));
    assert!(!format!("{resolver:?}").contains(secret));
    let coding = CodingCredentialResolver::new(Arc::new(resolver));
    assert!(!format!("{coding:?}").contains(secret));
    let config = coding.resolve().unwrap();
    assert_eq!(config.api_key().as_str(), secret);
    assert!(!format!("{config:?}").contains(secret));
}

fn environment_server(names: Vec<String>) -> tea_mcp::McpServerConfig {
    let layer = SettingsLayer {
        mcp_servers: Some(vec![McpServerSettings {
            id: "environment".to_owned(),
            transport: McpTransportSettings::Stdio {
                executable: "/usr/bin/env".into(),
                arguments: Vec::new(),
            },
            inherited_environment: names,
            tools: Vec::new(),
            limits: McpLimitsSettings::default(),
            lifecycle: McpLifecycleSettings::default(),
            reconnect: None,
        }]),
        ..Default::default()
    };
    merge_settings(CodingSettings::default(), Some(&layer), None, None, None)
        .unwrap()
        .mcp_servers
        .remove(0)
}

#[test]
fn mcp_environment_values_are_late_bounded_and_redacting() {
    let secret = "mcp-secret-value-that-must-never-persist";
    let resolver = StaticMcpEnvironmentResolver::new(BTreeMap::from([
        ("ALLOWED_TOKEN".to_owned(), OsString::from(secret)),
        (
            "UNRELATED_SECRET".to_owned(),
            OsString::from("must-not-be-inherited"),
        ),
    ]))
    .unwrap();
    assert!(!format!("{resolver:?}").contains(secret));

    let server = environment_server(vec!["ALLOWED_TOKEN".to_owned()]);
    assert!(!format!("{server:?}").contains(secret));
    let environment = resolve_mcp_environment(&server, &resolver).unwrap();
    assert_eq!(environment.names().collect::<Vec<_>>(), ["ALLOWED_TOKEN"]);
    assert!(!format!("{environment:?}").contains(secret));

    #[cfg(unix)]
    {
        let mut command = std::process::Command::new("/usr/bin/env");
        command.env("AMBIENT_SECRET", "must-be-cleared");
        environment.apply_to(&mut command);
        let output = command.output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains(&format!("ALLOWED_TOKEN={secret}")));
        assert!(!stdout.contains("UNRELATED_SECRET"));
        assert!(!stdout.contains("AMBIENT_SECRET"));
    }
}

#[test]
fn mcp_environment_missing_errors_expose_only_name_and_code() {
    let secret = "missing-secret-must-not-appear";
    let resolver = StaticMcpEnvironmentResolver::new(BTreeMap::from([(
        "OTHER".to_owned(),
        OsString::from(secret),
    )]))
    .unwrap();
    let server = environment_server(vec!["REQUIRED_TOKEN".to_owned()]);
    let error = resolve_mcp_environment(&server, &resolver).unwrap_err();

    assert_eq!(error.code(), McpEnvironmentErrorCode::MissingVariable);
    assert_eq!(error.name(), "REQUIRED_TOKEN");
    assert!(format!("{error:?}").contains("REQUIRED_TOKEN"));
    assert!(!format!("{error:?}").contains(secret));
    let object: &dyn McpEnvironmentResolver = &resolver;
    assert!(object.resolve("REQUIRED_TOKEN").unwrap().is_none());
}
