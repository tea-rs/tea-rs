use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::process::Child;

use crate::{FileToolError, FileToolErrorCode};

pub(crate) fn configure(command: &mut tokio::process::Command) {
    command.process_group(0);
}

pub(crate) fn kill_tree_on_drop(process_id: u32) {
    if let Ok(raw) = i32::try_from(process_id) {
        let _ = killpg(Pid::from_raw(raw), Signal::SIGKILL);
    }
}

pub(crate) async fn terminate_tree(
    child: &mut Child,
    process_id: u32,
) -> Result<(), FileToolError> {
    let pid = i32::try_from(process_id)
        .map(Pid::from_raw)
        .map_err(|_| FileToolError::new(FileToolErrorCode::ProcessFailure))?;
    let _ = killpg(pid, Signal::SIGTERM);
    tokio::time::sleep(Duration::from_millis(250)).await;
    let _ = killpg(pid, Signal::SIGKILL);
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        Ok(None) => child
            .wait()
            .await
            .map(|_| ())
            .map_err(|_| FileToolError::new(FileToolErrorCode::ProcessFailure)),
        Err(_) => Err(FileToolError::new(FileToolErrorCode::ProcessFailure)),
    }
}
