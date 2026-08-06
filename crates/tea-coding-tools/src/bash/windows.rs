use tokio::process::Child;

use crate::{FileToolError, FileToolErrorCode};

pub(crate) fn configure(command: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command
        .as_std_mut()
        .creation_flags(CREATE_NEW_PROCESS_GROUP);
}

pub(crate) fn kill_tree_on_drop(process_id: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &process_id.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Uses the Windows system `taskkill /T /F` tree-termination equivalent.
pub(crate) async fn terminate_tree(
    child: &mut Child,
    process_id: u32,
) -> Result<(), FileToolError> {
    let status = tokio::process::Command::new("taskkill")
        .args(["/PID", &process_id.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|_| FileToolError::new(FileToolErrorCode::ProcessFailure))?;
    let waited = child
        .wait()
        .await
        .map_err(|_| FileToolError::new(FileToolErrorCode::ProcessFailure))?;
    if status.success() || waited.success() {
        Ok(())
    } else {
        Err(FileToolError::new(FileToolErrorCode::ProcessFailure))
    }
}
