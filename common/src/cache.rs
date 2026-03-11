// NOTE: serde_json must be added to common/Cargo.toml:
//   serde_json = "1"

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ipc_types::SOCKET_NAME;

const APP_DIR: &str = "swww-vulkan";
const STATE_FILE: &str = "state.json";

// ---------------------------------------------------------------------------
// Session state types
// ---------------------------------------------------------------------------

/// Session state persisted to disk for the `restore` command.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub outputs: HashMap<String, OutputSessionState>,
}

/// Per-output state saved across daemon restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSessionState {
    pub wallpaper_path: String,
    /// One of "crop", "fit", "no".
    pub resize_mode: String,
}

// ---------------------------------------------------------------------------
// XDG directory resolution
// ---------------------------------------------------------------------------

/// Get state directory: `$XDG_STATE_HOME/swww-vulkan/` (default `~/.local/state/swww-vulkan/`).
pub fn state_dir() -> PathBuf {
    let base = dirs::state_dir()
        .or_else(|| std::env::var_os("XDG_STATE_HOME").map(PathBuf::from))
        .unwrap_or_else(|| {
            let mut p = dirs::home_dir().expect("cannot determine home directory");
            p.push(".local");
            p.push("state");
            p
        });
    base.join(APP_DIR)
}

/// Get cache directory: `$XDG_CACHE_HOME/swww-vulkan/` (default `~/.cache/swww-vulkan/`).
pub fn cache_dir() -> PathBuf {
    let base = dirs::cache_dir()
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .unwrap_or_else(|| {
            let mut p = dirs::home_dir().expect("cannot determine home directory");
            p.push(".cache");
            p
        });
    base.join(APP_DIR)
}

/// Get socket path: `$XDG_RUNTIME_DIR/swww-vulkan.sock`.
///
/// Fallback: derive the runtime directory from `$WAYLAND_DISPLAY` if it
/// contains a path separator, otherwise fall back to `/run/user/<uid>/`.
pub fn socket_path() -> PathBuf {
    if let Some(runtime) =
        dirs::runtime_dir().or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
    {
        return runtime.join(SOCKET_NAME);
    }

    // Try to derive a directory from $WAYLAND_DISPLAY (some compositors set an
    // absolute path).
    if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
        let wayland_path = PathBuf::from(&wayland);
        if wayland_path.is_absolute()
            && let Some(parent) = wayland_path.parent()
        {
            return parent.join(SOCKET_NAME);
        }
    }

    // Last resort: construct /run/user/<uid>/ from /proc.
    if let Ok(uid) = std::fs::read_to_string("/proc/self/loginuid") {
        let uid = uid.trim();
        let p = PathBuf::from(format!("/run/user/{uid}"));
        if p.is_dir() {
            return p.join(SOCKET_NAME);
        }
    }

    // Absolute fallback — /tmp is always writable.
    PathBuf::from("/tmp").join(SOCKET_NAME)
}

// ---------------------------------------------------------------------------
// Session persistence
// ---------------------------------------------------------------------------

/// Load session state from `state.json` inside the state directory.
///
/// Returns `Default` if the file does not exist.
pub fn load_session_state() -> Result<SessionState, std::io::Error> {
    let path = state_dir().join(STATE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let state: SessionState = serde_json::from_str(&contents)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            Ok(state)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SessionState::default()),
        Err(e) => Err(e),
    }
}

/// Save session state to `state.json` inside the state directory.
///
/// Creates the state directory if it does not already exist.
pub fn save_session_state(state: &SessionState) -> Result<(), std::io::Error> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(STATE_FILE);
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)
}

// ---------------------------------------------------------------------------
// Cache management
// ---------------------------------------------------------------------------

/// Remove all files in the cache directory.
///
/// The directory itself is preserved. If the directory does not exist this is a
/// no-op.
pub fn clear_cache() -> Result<(), std::io::Error> {
    let dir = cache_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    for entry in entries {
        let entry = entry?;
        let ft = entry.file_type()?;
        let path = entry.path();
        if ft.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }

    Ok(())
}
