use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ipc_types::SOCKET_NAME;

const APP_DIR: &str = "wl";
const STATE_FILE: &str = "state.json";
const ROTATION_STATE_FILE: &str = "rotation.json";

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

/// Get state directory: `$XDG_STATE_HOME/wl/` (default `~/.local/state/wl/`).
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

/// Get cache directory: `$XDG_CACHE_HOME/wl/` (default `~/.cache/wl/`).
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

/// Get socket path: `$XDG_RUNTIME_DIR/wl.sock`.
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

/// Remove all files in the cache directory (including upscale cache).
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

// ---------------------------------------------------------------------------
// Upscale cache (T013)
// ---------------------------------------------------------------------------

const UPSCALE_DIR: &str = "upscale";
const UPSCALE_INDEX: &str = "index.json";
const MAX_UPSCALE_ENTRIES: usize = 50;

/// Get upscale cache directory: `$XDG_CACHE_HOME/wl/upscale/`.
pub fn upscale_cache_dir() -> PathBuf {
    cache_dir().join(UPSCALE_DIR)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpscaleCacheIndex {
    pub entries: Vec<UpscaleCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscaleCacheEntry {
    pub source_path: String,
    pub source_mtime_secs: u64,
    pub source_size: u64,
    pub scale_factor: u8,
    pub cached_filename: String,
    pub created_at: u64,
}

impl UpscaleCacheIndex {
    /// Look up a cache entry by source identity.
    pub fn lookup(
        &self,
        source_path: &str,
        mtime: u64,
        size: u64,
        scale: u8,
    ) -> Option<&UpscaleCacheEntry> {
        self.entries.iter().find(|e| {
            e.source_path == source_path
                && e.source_mtime_secs == mtime
                && e.source_size == size
                && e.scale_factor == scale
        })
    }

    /// Insert a new entry, evicting the oldest if over the limit.
    pub fn insert(&mut self, entry: UpscaleCacheEntry) {
        // Remove any existing entry with the same identity
        self.entries.retain(|e| {
            !(e.source_path == entry.source_path
                && e.source_mtime_secs == entry.source_mtime_secs
                && e.source_size == entry.source_size
                && e.scale_factor == entry.scale_factor)
        });

        self.entries.push(entry);

        // Evict oldest entries beyond limit
        while self.entries.len() > MAX_UPSCALE_ENTRIES {
            let oldest_idx = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.created_at)
                .map(|(i, _)| i)
                .unwrap();
            let evicted = self.entries.remove(oldest_idx);
            let evicted_path = upscale_cache_dir().join(&evicted.cached_filename);
            let _ = std::fs::remove_file(evicted_path);
        }
    }
}

/// Load upscale cache index from disk.
pub fn load_upscale_index() -> UpscaleCacheIndex {
    let path = upscale_cache_dir().join(UPSCALE_INDEX);
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => UpscaleCacheIndex::default(),
    }
}

/// Save upscale cache index to disk.
pub fn save_upscale_index(index: &UpscaleCacheIndex) -> Result<(), std::io::Error> {
    let dir = upscale_cache_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(UPSCALE_INDEX);
    let json = serde_json::to_string_pretty(index)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)
}

// ---------------------------------------------------------------------------
// Upscale preferences (T017)
// ---------------------------------------------------------------------------

const UPSCALE_PREFS_FILE: &str = "upscale-prefs.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpscalePrefs {
    pub enabled: bool,
    #[serde(default)]
    pub custom_cmd: Option<String>,
    #[serde(default)]
    pub scale: Option<u8>,
}

/// Load upscale preferences from disk.
pub fn load_upscale_prefs() -> UpscalePrefs {
    let path = state_dir().join(UPSCALE_PREFS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => UpscalePrefs::default(),
    }
}

/// Save upscale preferences to disk.
pub fn save_upscale_prefs(prefs: &UpscalePrefs) -> Result<(), std::io::Error> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(UPSCALE_PREFS_FILE);
    let json = serde_json::to_string_pretty(prefs)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)
}

// ---------------------------------------------------------------------------
// Rotation state persistence
// ---------------------------------------------------------------------------

/// Persisted rotation state, saved to `rotation.json` in the state directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPersist {
    pub directories: Vec<String>,
    pub interval_secs: u64,
    pub candidates: Vec<String>,
    pub current_index: usize,
    pub resize_mode: String,
    #[serde(default)]
    pub transition_type: Option<String>,
    #[serde(default)]
    pub transition_duration: Option<f32>,
    #[serde(default)]
    pub upscale_mode: Option<String>,
    #[serde(default)]
    pub upscale_cmd: Option<String>,
    #[serde(default)]
    pub upscale_scale: Option<u8>,
    #[serde(default)]
    pub no_notify: bool,
    #[serde(default)]
    pub notify_path: Option<String>,
}

/// Load rotation state from disk. Returns `None` if file does not exist.
pub fn load_rotation_state() -> Option<RotationPersist> {
    let path = state_dir().join(ROTATION_STATE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).ok(),
        Err(_) => None,
    }
}

/// Save rotation state to disk.
pub fn save_rotation_state(state: &RotationPersist) -> Result<(), std::io::Error> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(ROTATION_STATE_FILE);
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)
}

/// Delete rotation state file from disk.
pub fn delete_rotation_state() {
    let path = state_dir().join(ROTATION_STATE_FILE);
    let _ = std::fs::remove_file(path);
}
