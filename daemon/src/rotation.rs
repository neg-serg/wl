use std::path::PathBuf;
use std::time::{Duration, Instant};

use wl_common::cache::RotationPersist;
use wl_common::ipc_types::{ResizeMode, TransitionParams};

/// Parameters for starting rotation, extracted from IPC command.
pub struct RotateStartParams {
    pub directories: Vec<PathBuf>,
    pub interval_secs: u64,
    pub resize: ResizeMode,
    pub transition: TransitionParams,
    pub upscale_mode: Option<String>,
    pub upscale_cmd: Option<String>,
    pub upscale_scale: Option<u8>,
    pub no_notify: bool,
    pub notify_path: PathBuf,
}

/// Runtime rotation state held by the daemon.
pub struct RotationState {
    pub directories: Vec<PathBuf>,
    pub interval: Duration,
    pub candidates: Vec<PathBuf>,
    pub current_index: usize,
    pub next_rotation: Instant,
    pub resize: ResizeMode,
    pub transition: TransitionParams,
    pub upscale_mode: Option<String>,
    pub upscale_cmd: Option<String>,
    pub upscale_scale: Option<u8>,
    pub no_notify: bool,
    pub notify_path: PathBuf,
}

impl RotationState {
    /// Scan directories and build a new shuffled cycle.
    pub fn new_cycle(directories: &[PathBuf]) -> Vec<PathBuf> {
        let mut candidates = wl_common::scan::scan_directories(directories);
        fisher_yates_shuffle(&mut candidates);
        candidates
    }

    /// Advance to the next image in the cycle.
    /// Returns `Some(path)` if available, or `None` if cycle is exhausted
    /// (caller should call `reshuffle()` then retry).
    pub fn advance(&mut self) -> Option<PathBuf> {
        if self.current_index < self.candidates.len() {
            let path = self.candidates[self.current_index].clone();
            self.current_index += 1;
            Some(path)
        } else {
            None
        }
    }

    /// Rescan directories and start a fresh shuffle cycle.
    pub fn reshuffle(&mut self) {
        self.candidates = Self::new_cycle(&self.directories);
        self.current_index = 0;
    }

    /// Get the next image, reshuffling if the current cycle is exhausted.
    pub fn next_image(&mut self) -> Option<PathBuf> {
        if let Some(path) = self.advance() {
            return Some(path);
        }
        // Cycle exhausted — rescan and reshuffle
        self.reshuffle();
        self.advance()
    }

    /// Reset the rotation timer to fire after one full interval from now.
    pub fn reset_timer(&mut self) {
        self.next_rotation = Instant::now() + self.interval;
    }

    /// Time remaining until the next rotation.
    pub fn time_until_next(&self) -> Duration {
        self.next_rotation
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO)
    }

    /// Convert to persistence format.
    pub fn to_persist(&self) -> RotationPersist {
        RotationPersist {
            directories: self
                .directories
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            interval_secs: self.interval.as_secs(),
            candidates: self
                .candidates
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            current_index: self.current_index,
            resize_mode: match self.resize {
                ResizeMode::Crop => "crop".to_string(),
                ResizeMode::Fit => "fit".to_string(),
                ResizeMode::No => "no".to_string(),
                ResizeMode::Center => "center".to_string(),
            },
            transition_type: serialize_transition_type(self.transition.transition_type),
            transition_duration: Some(self.transition.duration_secs),
            upscale_mode: self.upscale_mode.clone(),
            upscale_cmd: self.upscale_cmd.clone(),
            upscale_scale: self.upscale_scale,
            no_notify: self.no_notify,
            notify_path: Some(self.notify_path.to_string_lossy().to_string()),
        }
    }

    /// Restore from persistence format.
    pub fn from_persist(p: &RotationPersist) -> Self {
        let directories: Vec<PathBuf> = p.directories.iter().map(PathBuf::from).collect();
        let candidates: Vec<PathBuf> = p.candidates.iter().map(PathBuf::from).collect();

        let resize = match p.resize_mode.as_str() {
            "crop" => ResizeMode::Crop,
            "fit" => ResizeMode::Fit,
            "no" => ResizeMode::No,
            "center" => ResizeMode::Center,
            _ => ResizeMode::Crop,
        };

        let mut transition = TransitionParams::default();
        if let Some(ref tt) = p.transition_type {
            transition.transition_type = deserialize_transition_type(tt);
        }
        if let Some(dur) = p.transition_duration {
            transition.duration_secs = dur;
        }

        let notify_path = p
            .notify_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_default();

        Self {
            directories,
            interval: Duration::from_secs(p.interval_secs),
            candidates,
            current_index: p.current_index,
            next_rotation: Instant::now(), // Will be set by caller
            resize,
            transition,
            upscale_mode: p.upscale_mode.clone(),
            upscale_cmd: p.upscale_cmd.clone(),
            upscale_scale: p.upscale_scale,
            no_notify: p.no_notify,
            notify_path,
        }
    }

    /// Save rotation state to disk.
    pub fn save(&self) {
        let persist = self.to_persist();
        if let Err(e) = wl_common::cache::save_rotation_state(&persist) {
            tracing::warn!("failed to save rotation state: {e}");
        }
    }
}

/// Fisher-Yates shuffle using OS randomness.
fn fisher_yates_shuffle(items: &mut [PathBuf]) {
    if items.len() <= 1 {
        return;
    }
    for i in (1..items.len()).rev() {
        let mut buf = [0u8; 8];
        getrandom::fill(&mut buf).expect("failed to get random bytes");
        let j = u64::from_ne_bytes(buf) as usize % (i + 1);
        items.swap(i, j);
    }
}

fn serialize_transition_type(tt: wl_common::ipc_types::TransitionType) -> Option<String> {
    use wl_common::ipc_types::TransitionType;
    match tt {
        TransitionType::None => None,
        TransitionType::Wipe => Some("wipe".to_string()),
        TransitionType::Wave => Some("wave".to_string()),
        TransitionType::Outer => Some("outer".to_string()),
        TransitionType::Pixelate => Some("pixelate".to_string()),
        TransitionType::Burn => Some("burn".to_string()),
        TransitionType::Glitch => Some("glitch".to_string()),
        TransitionType::Disintegrate => Some("disintegrate".to_string()),
        TransitionType::Dreamy => Some("dreamy".to_string()),
        TransitionType::GlitchMemories => Some("glitch-memories".to_string()),
        TransitionType::Morph => Some("morph".to_string()),
        TransitionType::Hexagonalize => Some("hexagonalize".to_string()),
        TransitionType::CrossZoom => Some("cross-zoom".to_string()),
        TransitionType::FluidDistortion => Some("fluid-distortion".to_string()),
        TransitionType::FluidDrain => Some("fluid-drain".to_string()),
        TransitionType::FluidRipple => Some("fluid-ripple".to_string()),
        TransitionType::FluidVortex => Some("fluid-vortex".to_string()),
        TransitionType::FluidWave => Some("fluid-wave".to_string()),
        TransitionType::InkBleed => Some("ink-bleed".to_string()),
        TransitionType::LavaLamp => Some("lava-lamp".to_string()),
        TransitionType::ChromaticAberration => Some("chromatic-aberration".to_string()),
        TransitionType::LensDistortion => Some("lens-distortion".to_string()),
        TransitionType::CrtShutdown => Some("crt-shutdown".to_string()),
        TransitionType::PerlinWipe => Some("perlin-wipe".to_string()),
        TransitionType::RadialBlur => Some("radial-blur".to_string()),
        TransitionType::Random => Some("random".to_string()),
    }
}

fn deserialize_transition_type(s: &str) -> wl_common::ipc_types::TransitionType {
    use wl_common::ipc_types::TransitionType;
    match s {
        "wipe" => TransitionType::Wipe,
        "wave" => TransitionType::Wave,
        "outer" => TransitionType::Outer,
        "pixelate" => TransitionType::Pixelate,
        "burn" => TransitionType::Burn,
        "glitch" => TransitionType::Glitch,
        "disintegrate" => TransitionType::Disintegrate,
        "dreamy" => TransitionType::Dreamy,
        "glitch-memories" => TransitionType::GlitchMemories,
        "morph" => TransitionType::Morph,
        "hexagonalize" => TransitionType::Hexagonalize,
        "cross-zoom" => TransitionType::CrossZoom,
        "fluid-distortion" => TransitionType::FluidDistortion,
        "fluid-drain" => TransitionType::FluidDrain,
        "fluid-ripple" => TransitionType::FluidRipple,
        "fluid-vortex" => TransitionType::FluidVortex,
        "fluid-wave" => TransitionType::FluidWave,
        "ink-bleed" => TransitionType::InkBleed,
        "lava-lamp" => TransitionType::LavaLamp,
        "chromatic-aberration" => TransitionType::ChromaticAberration,
        "lens-distortion" => TransitionType::LensDistortion,
        "crt-shutdown" => TransitionType::CrtShutdown,
        "perlin-wipe" => TransitionType::PerlinWipe,
        "radial-blur" => TransitionType::RadialBlur,
        "random" => TransitionType::Random,
        _ => TransitionType::None,
    }
}
