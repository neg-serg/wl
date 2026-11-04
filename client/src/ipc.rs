use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use wl_common::ipc_types::*;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum IpcError {
    Io(std::io::Error),
    Bincode(String),
    PayloadTooLarge { size: usize, limit: usize },
    DaemonNotRunning,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Bincode(msg) => write!(f, "bincode error: {msg}"),
            Self::PayloadTooLarge { size, limit } => write!(
                f,
                "IPC payload is {size} bytes, limit is {limit} bytes (split the request, e.g. pass a parent directory)"
            ),
            Self::DaemonNotRunning => {
                write!(f, "daemon is not running (could not connect to socket)")
            }
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
        return Err(IpcError::PayloadTooLarge {
            size: len,
            limit: MAX_IPC_PAYLOAD,
        });
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_framed(stream: &mut UnixStream, data: &[u8]) -> Result<(), IpcError> {
    if data.len() > MAX_IPC_PAYLOAD {
        return Err(IpcError::PayloadTooLarge {
            size: data.len(),
            limit: MAX_IPC_PAYLOAD,
        });
    }
    stream.write_u32(data.len() as u32).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// IPC client
// ---------------------------------------------------------------------------

pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    /// Connect to the running daemon's IPC socket.
    ///
    /// Returns `IpcError::DaemonNotRunning` if the socket does not exist or
    /// the connection is refused.
    pub async fn connect() -> Result<Self, IpcError> {
        let socket_path = wl_common::cache::socket_path();

        let stream = UnixStream::connect(&socket_path)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                    IpcError::DaemonNotRunning
                }
                _ => IpcError::Io(e),
            })?;

        Ok(Self { stream })
    }

    /// Send a command to the daemon and wait for its response.
    pub async fn send_command(&mut self, cmd: &IpcCommand) -> Result<IpcResponse, IpcError> {
        let data = bincode::serialize(cmd).map_err(|e| IpcError::Bincode(e.to_string()))?;
        write_framed(&mut self.stream, &data).await?;

        // A wedged daemon must fail the CLI instead of hanging it forever:
        // image decode/upload and device recovery can take a few seconds, so
        // the reply budget is generous but bounded.
        let response_data = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            read_framed(&mut self.stream),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Err(IpcError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "timed out waiting for the daemon response",
                )));
            }
        };
        let response: IpcResponse =
            bincode::deserialize(&response_data).map_err(|e| IpcError::Bincode(e.to_string()))?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::IpcError;

    #[test]
    fn payload_error_reports_size_and_limit() {
        let error = IpcError::PayloadTooLarge {
            size: 70_000,
            limit: 65_536,
        };
        let text = error.to_string();
        assert!(text.contains("70000"), "{text}");
        assert!(text.contains("65536"), "{text}");
    }
}
