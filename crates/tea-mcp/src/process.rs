use std::{
    ffi::OsString,
    process::{Command, Stdio},
    time::Duration,
};

use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};

use crate::{McpError, McpErrorCode, McpExecutableIdentity, McpLifecyclePolicy, McpStdioConfig};

pub(crate) struct SpawnedStdioProcess {
    pub(crate) owner: OwnedProcess,
    pub(crate) stdin: ChildStdin,
    pub(crate) stdout: ChildStdout,
    pub(crate) stderr: ChildStderr,
}

pub(crate) async fn spawn(
    config: &McpStdioConfig,
    environment: Vec<(OsString, OsString)>,
    executable_identity: &McpExecutableIdentity,
    startup_timeout: Duration,
    kill_timeout: Duration,
) -> Result<SpawnedStdioProcess, McpError> {
    executable_identity.verify(config.executable())?;
    let mut command = Command::new(config.executable());
    command
        .args(config.arguments())
        .env_clear()
        .envs(environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_tree(&mut command);
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    let child = tokio::time::timeout(startup_timeout, async move { command.spawn() })
        .await
        .map_err(|_| McpError::new(McpErrorCode::Timeout))?
        .map_err(|_| McpError::new(McpErrorCode::Startup))?;
    let process_id = child
        .id()
        .ok_or_else(|| McpError::new(McpErrorCode::Startup))?;
    let mut owner = OwnedProcess::new(child, process_id);
    let Some(stdin) = owner.child.stdin.take() else {
        owner.kill_immediately(kill_timeout).await;
        return Err(McpError::new(McpErrorCode::Startup));
    };
    let Some(stdout) = owner.child.stdout.take() else {
        owner.kill_immediately(kill_timeout).await;
        return Err(McpError::new(McpErrorCode::Startup));
    };
    let Some(stderr) = owner.child.stderr.take() else {
        owner.kill_immediately(kill_timeout).await;
        return Err(McpError::new(McpErrorCode::Startup));
    };
    Ok(SpawnedStdioProcess {
        owner,
        stdin,
        stdout,
        stderr,
    })
}

pub(crate) struct OwnedProcess {
    child: Child,
    process_id: u32,
    guard: ProcessTreeGuard,
}

impl OwnedProcess {
    fn new(child: Child, process_id: u32) -> Self {
        Self {
            child,
            process_id,
            guard: ProcessTreeGuard::new(process_id),
        }
    }

    pub(crate) fn exited(&mut self) -> Result<bool, McpError> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|_| McpError::new(McpErrorCode::ServerExit))
    }

    pub(crate) async fn shutdown(
        &mut self,
        lifecycle: McpLifecyclePolicy,
    ) -> Result<ProcessShutdown, McpError> {
        if wait_for_exit(&mut self.child, lifecycle.graceful_shutdown_timeout()).await? {
            terminate_remaining_descendants(self.process_id).await;
            self.guard.disarm();
            return Ok(ProcessShutdown { forced: false });
        }

        let _ = send_term(self.process_id).await;
        if wait_for_exit(&mut self.child, lifecycle.termination_timeout()).await? {
            let _ = send_kill(self.process_id).await;
            self.guard.disarm();
            return Ok(ProcessShutdown { forced: true });
        }

        let _ = send_kill(self.process_id).await;
        if !wait_for_exit(&mut self.child, lifecycle.kill_timeout()).await? {
            return Err(McpError::new(McpErrorCode::Shutdown));
        }
        let _ = send_kill(self.process_id).await;
        self.guard.disarm();
        Ok(ProcessShutdown { forced: true })
    }

    pub(crate) async fn kill_immediately(&mut self, timeout: Duration) {
        let _ = send_kill(self.process_id).await;
        if wait_for_exit(&mut self.child, timeout)
            .await
            .unwrap_or(false)
        {
            let _ = send_kill(self.process_id).await;
            self.guard.disarm();
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessShutdown {
    pub(crate) forced: bool,
}

struct ProcessTreeGuard {
    process_id: u32,
    armed: bool,
}

impl ProcessTreeGuard {
    const fn new(process_id: u32) -> Self {
        Self {
            process_id,
            armed: true,
        }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        if self.armed {
            kill_tree_on_drop(self.process_id);
        }
    }
}

async fn wait_for_exit(child: &mut Child, timeout: Duration) -> Result<bool, McpError> {
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(_)) => Ok(true),
        Ok(Err(_)) => Err(McpError::new(McpErrorCode::Shutdown)),
        Err(_) => Ok(false),
    }
}

async fn terminate_remaining_descendants(process_id: u32) {
    let _ = send_term(process_id).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    let _ = send_kill(process_id).await;
}

#[cfg(unix)]
fn configure_process_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_tree(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn send_term(process_id: u32) -> std::future::Ready<Result<(), McpError>> {
    std::future::ready(send_unix_signal(
        process_id,
        nix::sys::signal::Signal::SIGTERM,
    ))
}

#[cfg(unix)]
fn send_kill(process_id: u32) -> std::future::Ready<Result<(), McpError>> {
    std::future::ready(send_unix_signal(
        process_id,
        nix::sys::signal::Signal::SIGKILL,
    ))
}

#[cfg(unix)]
fn send_unix_signal(process_id: u32, signal: nix::sys::signal::Signal) -> Result<(), McpError> {
    use nix::{sys::signal::killpg, unistd::Pid};

    let raw = i32::try_from(process_id).map_err(|_| McpError::new(McpErrorCode::Shutdown))?;
    let _ = killpg(Pid::from_raw(raw), signal);
    Ok(())
}

#[cfg(unix)]
fn kill_tree_on_drop(process_id: u32) {
    let _ = send_unix_signal(process_id, nix::sys::signal::Signal::SIGKILL);
}

#[cfg(windows)]
async fn send_term(process_id: u32) -> Result<(), McpError> {
    taskkill(process_id, false).await
}

#[cfg(windows)]
async fn send_kill(process_id: u32) -> Result<(), McpError> {
    taskkill(process_id, true).await
}

#[cfg(windows)]
async fn taskkill(process_id: u32, force: bool) -> Result<(), McpError> {
    let mut command = tokio::process::Command::new("taskkill");
    command
        .args(["/PID", &process_id.to_string(), "/T"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if force {
        command.arg("/F");
    }
    command
        .status()
        .await
        .map(|_| ())
        .map_err(|_| McpError::new(McpErrorCode::Shutdown))
}

#[cfg(windows)]
fn kill_tree_on_drop(process_id: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &process_id.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
