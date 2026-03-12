use serde::{Deserialize, Serialize};

/// Maximum IPC payload size: 64 KiB.
pub const MAX_IPC_PAYLOAD: usize = 65536;

/// Unix domain socket name used by the daemon.
pub const SOCKET_NAME: &str = "swww-vulkan.sock";

// ---------------------------------------------------------------------------
// Commands (client -> daemon)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcCommand {
    Img {
        path: String,
        outputs: Option<Vec<String>>,
        resize: ResizeMode,
        transition: TransitionParams,
    },
    Clear {
        outputs: Option<Vec<String>>,
        color: [u8; 3],
    },
    Query,
    Restore,
    Kill,
    Pause {
        outputs: Option<Vec<String>>,
    },
    ClearCache,
}

// ---------------------------------------------------------------------------
// Transition parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TransitionType {
    Fade,
    Wipe,
    Wave,
    Outer,
    Pixelate,
    Burn,
    Glitch,
    Disintegrate,
    Dreamy,
    GlitchMemories,
    Morph,
    Hexagonalize,
    CrossZoom,
    FilmBurn,
    CircleCrop,
    Random,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TransitionParams {
    pub transition_type: TransitionType,
    pub duration_secs: f32,
    pub step: u8,
    pub fps: u32,
    pub angle: f32,
    pub position: (f32, f32),
    pub bezier: [f32; 4],
    pub wave: (f32, f32),
}

impl Default for TransitionParams {
    fn default() -> Self {
        Self {
            transition_type: TransitionType::None,
            duration_secs: 3.0,
            step: 90,
            fps: 240,
            angle: 45.0,
            position: (0.5, 0.5),
            bezier: [0.25, 0.1, 0.25, 1.0],
            wave: (20.0, 20.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Resize mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResizeMode {
    #[default]
    Crop,
    Fit,
    No,
}

// ---------------------------------------------------------------------------
// Responses (daemon -> client)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok,
    Error { message: String },
    QueryResult { outputs: Vec<OutputInfo> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputInfo {
    pub name: String,
    pub wallpaper_path: Option<String>,
    pub dimensions: Option<(u32, u32)>,
    pub state: OutputState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputState {
    Idle,
    Transitioning,
    Playing { frame: u32, total: u32 },
}

// ---------------------------------------------------------------------------
// Image format classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Gif,
    WebP,
    Bmp,
    Tiff,
    Pnm,
    Tga,
    Farbfeld,
    Svg,
}
