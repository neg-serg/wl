use std::process::Command;
use std::time::Duration;

use wl_common::ipc_types::IpcCommand;

use crate::ipc::{IpcClient, IpcError};

/// Spawn the daemon process and wait for it to become ready.
pub async fn init() -> Result<(), String> {
    // Check if daemon is already running
    if IpcClient::connect().await.is_ok() {
        return Err("daemon is already running".to_string());
    }

    // Spawn the daemon binary
    let daemon_bin = find_daemon_binary()?;

    let _child = Command::new(&daemon_bin)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to spawn daemon: {e}"))?;

    // Wait for the daemon to be ready (socket appears and accepts connections)
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err("daemon failed to start within 10 seconds".to_string());
        }

        tokio::time::sleep(Duration::from_millis(50)).await;

        if IpcClient::connect().await.is_ok() {
            return Ok(());
        }
    }
}

/// Send a kill command to the running daemon.
pub async fn kill() -> Result<(), String> {
    let mut client = IpcClient::connect().await.map_err(|e| match e {
        IpcError::DaemonNotRunning => {
            "daemon is not running. Start it with 'wl init'.".to_string()
        }
        other => format!("failed to connect to daemon: {other}"),
    })?;

    let response = client
        .send_command(&IpcCommand::Kill)
        .await
        .map_err(|e| format!("failed to send kill command: {e}"))?;

    match response {
        wl_common::ipc_types::IpcResponse::Ok => Ok(()),
        wl_common::ipc_types::IpcResponse::Error { message } => Err(message),
        _ => Err("unexpected response from daemon".to_string()),
    }
}

/// Find the daemon binary path.
/// Looks next to the current executable first, then in PATH.
fn find_daemon_binary() -> Result<String, String> {
    // Try same directory as this binary
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let daemon_path = dir.join("wl-daemon");
        if daemon_path.exists() {
            return Ok(daemon_path.to_string_lossy().to_string());
        }
    }

    // Fall back to PATH
    Ok("wl-daemon".to_string())
}
