use std::process::Command;

#[test]
fn version_does_not_require_home_or_workspace_access() {
    let output = Command::new(env!("CARGO_BIN_EXE_tea"))
        .arg("--version")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "tea 0.1.0\n");
    assert!(output.stderr.is_empty());
}
