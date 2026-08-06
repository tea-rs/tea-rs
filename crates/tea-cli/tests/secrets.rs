use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use clap::Parser as _;
use tea_cli::args::CliArgs;
use tea_cli::{BootstrapEnvironment, CliBootstrap, CliFailure};
use tea_coding::mcp_config::resolve_mcp_environment;
use tea_model::{ModelCapabilities, ModelDisplayName, ModelSpec, ProviderId};
use tea_protocol::{ModelId, SessionId, TokenCount};
use tea_session::SessionArchive;
use tea_testkit::{ScriptedModelProvider, ScriptedModelResponse};

const SECRET: &str = "sk-seeded-cli-credential-must-never-persist";

#[derive(Clone)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl Write for SharedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn temp_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tea-cli-secret-{label}-{}",
        uuid::Uuid::now_v7().hyphenated()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn model() -> ModelSpec {
    ModelSpec::new(
        ModelId::from_str("fake/model").unwrap(),
        ProviderId::from_str("fake").unwrap(),
        ModelDisplayName::from_str("Fake Model").unwrap(),
        TokenCount::new(32_000).unwrap(),
        TokenCount::new(4_000).unwrap(),
        ModelCapabilities::text().with_tools(true),
    )
    .unwrap()
}

fn bootstrap(root: &Path) -> CliBootstrap {
    let environment = BootstrapEnvironment::new(
        root,
        Some(root.to_path_buf()),
        BTreeMap::from([
            ("TEA_OPENAI_API_KEY".to_owned(), SECRET.to_owned()),
            ("TEA_OPENAI_MODEL".to_owned(), "fake/model".to_owned()),
            ("MCP_ALLOWED_TOKEN".to_owned(), SECRET.to_owned()),
        ]),
    );
    let provider = Arc::new(ScriptedModelProvider::new(
        ProviderId::from_str("fake").unwrap(),
        vec![model()],
        [ScriptedModelResponse::text(["safe answer"])],
    ));
    CliBootstrap::new(environment).with_provider(provider)
}

fn args(root: &Path, session_id: Option<SessionId>) -> CliArgs {
    let mut values = vec![
        "tea".to_owned(),
        "--json".to_owned(),
        "--provider".to_owned(),
        "fake".to_owned(),
        "--model".to_owned(),
        "fake/model".to_owned(),
        "--trust".to_owned(),
        "ignore".to_owned(),
        "--cwd".to_owned(),
        root.display().to_string(),
        "--config-dir".to_owned(),
        root.join("config").display().to_string(),
        "--state-dir".to_owned(),
        root.join("state").display().to_string(),
        "--data-dir".to_owned(),
        root.join("data").display().to_string(),
    ];
    if let Some(session_id) = session_id {
        values.push("--session".to_owned());
        values.push(session_id.to_string());
    } else {
        values.push("--new".to_owned());
        values.push("safe prompt".to_owned());
    }
    CliArgs::try_parse_from(values).unwrap()
}

fn assert_secret_absent(label: &str, bytes: &[u8]) {
    assert!(
        !bytes
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes()),
        "secret leaked through {label}"
    );
}

fn scan_tree(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            scan_tree(&path);
        } else {
            assert_secret_absent(&path.display().to_string(), &fs::read(&path).unwrap());
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn credentials_never_enter_debug_events_settings_sqlite_or_exports() {
    let root = temp_root("durable");
    let bootstrap = bootstrap(&root);
    assert_secret_absent("bootstrap Debug", format!("{bootstrap:?}").as_bytes());
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/settings.json"),
        r#"{"schemaVersion":1,"mcpServers":[{"id":"secret-test","transport":{"type":"stdio","executable":"/test/server"},"inheritedEnvironment":["MCP_ALLOWED_TOKEN"]}]}"#,
    )
    .unwrap();
    assert_secret_absent(
        "MCP settings document",
        &fs::read(root.join("config/settings.json")).unwrap(),
    );

    let output = Arc::new(Mutex::new(Vec::new()));
    Box::pin(tea_cli::modes::json::run(
        &args(&root, None),
        &bootstrap,
        &mut io::empty(),
        true,
        Box::new(SharedOutput(Arc::clone(&output))),
    ))
    .await
    .unwrap();
    let output = output.lock().unwrap().clone();
    assert_secret_absent("JSON events", &output);
    let header: serde_json::Value = serde_json::from_slice(
        output
            .split(|byte| *byte == b'\n')
            .find(|line| !line.is_empty())
            .unwrap(),
    )
    .unwrap();
    let session_id = SessionId::from_str(header["sessionId"].as_str().unwrap()).unwrap();

    let (service, _) = bootstrap
        .build_async(&args(&root, Some(session_id)))
        .await
        .unwrap();
    let mcp_environment = resolve_mcp_environment(
        &service.settings().mcp_servers[0],
        bootstrap.mcp_environment_resolver(),
    )
    .unwrap();
    assert_secret_absent(
        "MCP child environment Debug",
        format!("{mcp_environment:?}").as_bytes(),
    );
    service.open_session(session_id).await.unwrap();
    assert_secret_absent("service Debug", format!("{service:?}").as_bytes());
    assert_secret_absent(
        "settings snapshot",
        &serde_json::to_vec(service.settings()).unwrap(),
    );
    let snapshot = service.session_snapshot(session_id).await.unwrap();
    assert_secret_absent("snapshot Debug", format!("{snapshot:?}").as_bytes());
    let archive = SessionArchive::from_snapshot(&snapshot).unwrap();
    assert_secret_absent("session export", &serde_json::to_vec(&archive).unwrap());
    service.shutdown().await;

    scan_tree(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn runtime_error_conversion_drops_sensitive_adapter_diagnostics() {
    let runtime = tea::RuntimeError::new(tea::RuntimeErrorCode::ProviderFailure, SECRET);
    let coding: tea_coding::CodingError = runtime.into();
    assert_secret_absent("coding error", format!("{coding:?}").as_bytes());
    let cli: CliFailure = coding.into();
    assert_secret_absent("CLI error", format!("{cli:?}").as_bytes());
}

#[test]
fn committed_json_fixtures_contain_no_credential_material() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for entry in fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            for nested in fs::read_dir(path).unwrap() {
                assert_fixture(&nested.unwrap().path());
            }
        } else {
            assert_fixture(&path);
        }
    }
}

fn assert_fixture(path: &Path) {
    let text = String::from_utf8_lossy(&fs::read(path).unwrap()).to_ascii_lowercase();
    for marker in [
        "tea_openai_api_key",
        "\"apikey\"",
        "authorization",
        "bearer ",
        "sk-",
    ] {
        assert!(
            !text.contains(marker),
            "credential marker {marker:?} in {}",
            path.display()
        );
    }
}
