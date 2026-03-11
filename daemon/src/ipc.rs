use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use swww_vulkan_common::ipc_types::*;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum IpcError {
    Io(std::io::Error),
    Bincode(String),
    PayloadTooLarge,
    DaemonAlreadyRunning,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Bincode(msg) => write!(f, "bincode error: {msg}"),
            Self::PayloadTooLarge => write!(f, "IPC payload exceeds maximum size"),
            Self::DaemonAlreadyRunning => write!(f, "daemon is already running"),
        }
    }
}

impl std::error::Error for IpcError {}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Framing helpers
// ---------------------------------------------------------------------------

async fn read_framed(stream: &mut UnixStream) -> Result<Vec<u8>, IpcError> {
    let len = stream.read_u32().await? as usize;
    if len > MAX_IPC_PAYLOAD {
        return Err(IpcError::PayloadTooLarge);
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_framed(stream: &mut UnixStream, data: &[u8]) -> Result<(), IpcError> {
    if data.len() > MAX_IPC_PAYLOAD {
        return Err(IpcError::PayloadTooLarge);
    }
    stream.write_u32(data.len() as u32).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// IPC server
// ---------------------------------------------------------------------------

pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

impl IpcServer {
    /// Bind the IPC socket.
    ///
    /// If a stale socket file exists (i.e. no daemon is listening), it is
    /// removed before binding.  If a live daemon is already listening the call
    /// returns `IpcError::DaemonAlreadyRunning`.
    pub async fn bind() -> Result<Self, IpcError> {
        let socket_path = swww_vulkan_common::cache::socket_path();

        // Check for an existing socket file.
        if socket_path.exists() {
            // Try to connect — if it succeeds another daemon is running.
            match UnixStream::connect(&socket_path).await {
                Ok(_) => return Err(IpcError::DaemonAlreadyRunning),
                Err(_) => {
                    // Stale socket — remove it.
                    let _ = std::fs::remove_file(&socket_path);
                }
            }
        }

        let listener = UnixListener::bind(&socket_path)?;

        // Restrict permissions to owner-only (0600).
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Accept the next incoming connection and read one command from it.
    ///
    /// The caller receives both the deserialized command and the stream so it
    /// can send a response back with [`send_response`].
    pub async fn accept_command(&self) -> Result<(IpcCommand, UnixStream), IpcError> {
        let (mut stream, _addr) = self.listener.accept().await?;

        let payload = read_framed(&mut stream).await?;
        let cmd: IpcCommand =
            bincode::deserialize(&payload).map_err(|e| IpcError::Bincode(e.to_string()))?;

        Ok((cmd, stream))
    }
}

/// Send a response back to the client over an accepted stream.
pub async fn send_response(
    stream: &mut UnixStream,
    response: &IpcResponse,
) -> Result<(), IpcError> {
    let data = bincode::serialize(response).map_err(|e| IpcError::Bincode(e.to_string()))?;
    write_framed(stream, &data).await
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}
