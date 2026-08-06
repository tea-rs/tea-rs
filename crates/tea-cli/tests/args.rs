use clap::{CommandFactory as _, Parser as _};
use tea_cli::args::{CliArgs, SessionSelection, TrustArg};
use tea_protocol::ReasoningEffort;

#[test]
fn shared_flags_parse_and_resolve_session_selection() {
    let args = CliArgs::try_parse_from([
        "tea",
        "--print",
        "--cwd",
        "/tmp",
        "--provider",
        "openai",
        "--model",
        "openai/test",
        "--reasoning-effort",
        "xhigh",
        "--api-key",
        "sk-cli-test",
        "--profile",
        "coding-agent",
        "--tools",
        "read,edit",
        "--context-file",
        "CONTEXT.md",
        "--no-session",
        "--trust",
        "ignore",
        "-vv",
        "hello",
    ])
    .unwrap();
    assert!(args.print);
    assert!(!args.json);
    assert!(!args.rpc);
    assert_eq!(args.tools, ["read", "edit"]);
    assert_eq!(args.context_files, ["CONTEXT.md"]);
    assert_eq!(
        args.reasoning_effort.map(ReasoningEffort::as_str),
        Some("xhigh")
    );
    assert_eq!(args.api_key.as_ref().unwrap().as_str(), "sk-cli-test");
    assert!(!format!("{args:?}").contains("sk-cli-test"));
    assert_eq!(args.trust, TrustArg::Ignore);
    assert_eq!(args.verbose, 2);
    assert_eq!(args.prompt, ["hello"]);
    assert_eq!(
        args.session_selection().unwrap(),
        SessionSelection::NoSession
    );
}

#[test]
fn session_flags_conflict_and_explicit_id_is_validated() {
    assert!(CliArgs::try_parse_from(["tea", "--new", "--continue"]).is_err());
    assert!(CliArgs::try_parse_from(["tea", "--print", "--json"]).is_err());
    assert!(CliArgs::try_parse_from(["tea", "--rpc", "--json"]).is_err());
    let args = CliArgs::try_parse_from(["tea", "--session", "not-an-id"]).unwrap();
    assert!(args.session_selection().is_err());
    assert!(CliArgs::try_parse_from(["tea", "--reasoning-effort", "extreme"]).is_err());
}

#[test]
fn documented_cli_modes_are_available_without_credentials() {
    let help = CliArgs::command().render_long_help().to_string();

    assert!(help.contains("--rpc"));
    assert!(help.contains("--json"));
    assert!(help.contains("--print"));
}
