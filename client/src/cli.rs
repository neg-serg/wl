use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "wl", about = "Vulkan wallpaper tool for Wayland")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the wallpaper daemon
    Init,

    /// Set a wallpaper image
    Img {
        /// Path to image file
        path: String,

        /// Comma-separated output names (default: all)
        #[arg(short, long)]
        outputs: Option<String>,

        /// Resize mode
        #[arg(long, default_value = "crop")]
        resize: ResizeArg,

        /// Transition type
        #[arg(long, default_value = "random")]
        transition_type: TransitionTypeArg,

        /// Transition duration in seconds
        #[arg(long, default_value = "0.5")]
        transition_duration: f32,

        /// Frame step size (1-255)
        #[arg(long, default_value = "90")]
        transition_step: u8,

        /// Target transition FPS
        #[arg(long, default_value = "30")]
        transition_fps: u32,

        /// Angle in degrees (for wipe)
        #[arg(long, default_value = "45")]
        transition_angle: f32,

        /// Position as "x,y" normalized or "center" (for grow)
        #[arg(long, default_value = "center")]
        transition_pos: String,

        /// Cubic bezier control points as "a,b,c,d"
        #[arg(long, default_value = ".25,.1,.25,1")]
        transition_bezier: String,

        /// Wave frequency and amplitude as "freq,amp"
        #[arg(long, default_value = "20,20")]
        transition_wave: String,

        /// Upscale low-resolution images using a neural network before display.
        /// Values: once (default), always (persist), never (skip), off (disable persistent)
        #[arg(long, num_args(0..=1), default_missing_value = "once")]
        upscale: Option<UpscaleMode>,

        /// Custom upscaler command (implies --upscale).
        /// Use {input} and {output} placeholders, or paths are appended as arguments.
        #[arg(long)]
        upscale_cmd: Option<String>,

        /// Force a specific upscale factor (2, 4, 8, or 16) instead of auto-detecting.
        /// Values above 4 use multiple upscaling passes. Implies --upscale.
        #[arg(long, value_parser = parse_upscale_scale)]
        upscale_scale: Option<u8>,
    },

    /// Clear wallpaper to solid color
    Clear {
        /// Hex color (e.g., "#1a1b26")
        #[arg(short, long, default_value = "#000000")]
        color: String,

        /// Comma-separated output names (default: all)
        #[arg(short, long)]
        outputs: Option<String>,
    },

    /// Query current wallpaper state
    Query,

    /// Restore wallpapers from previous session
    Restore,

    /// Stop the daemon
    Kill,

    /// Toggle animated wallpaper playback
    Pause {
        /// Comma-separated output names (default: all)
        #[arg(short, long)]
        outputs: Option<String>,
    },

    /// Remove all cached image data
    ClearCache,

    /// Set a random wallpaper from specified directories
    Random {
        /// Directories to scan recursively for image files
        #[arg(required = true, num_args = 1..)]
        directories: Vec<PathBuf>,

        /// Comma-separated output names (default: all)
        #[arg(short, long)]
        outputs: Option<String>,

        /// Resize mode
        #[arg(long, default_value = "crop")]
        resize: ResizeArg,

        /// Transition type
        #[arg(long, default_value = "random")]
        transition_type: TransitionTypeArg,

        /// Transition duration in seconds
        #[arg(long, default_value = "0.5")]
        transition_duration: f32,

        /// Frame step size (1-255)
        #[arg(long, default_value = "90")]
        transition_step: u8,

        /// Target transition FPS
        #[arg(long, default_value = "30")]
        transition_fps: u32,

        /// Angle in degrees (for wipe)
        #[arg(long, default_value = "45")]
        transition_angle: f32,

        /// Position as "x,y" normalized or "center" (for grow)
        #[arg(long, default_value = "center")]
        transition_pos: String,

        /// Cubic bezier control points as "a,b,c,d"
        #[arg(long, default_value = ".25,.1,.25,1")]
        transition_bezier: String,

        /// Wave frequency and amplitude as "freq,amp"
        #[arg(long, default_value = "20,20")]
        transition_wave: String,

        /// Upscale low-resolution images using a neural network before display
        #[arg(long, num_args(0..=1), default_missing_value = "once")]
        upscale: Option<UpscaleMode>,

        /// Custom upscaler command (implies --upscale)
        #[arg(long)]
        upscale_cmd: Option<String>,

        /// Force a specific upscale factor (2, 4, 8, or 16). Implies --upscale.
        #[arg(long, value_parser = parse_upscale_scale)]
        upscale_scale: Option<u8>,

        /// Disable copying wallpaper to greeter cache
        #[arg(long)]
        no_greeter_sync: bool,

        /// Custom greeter cache file path
        #[arg(long, default_value = "~/.cache/greeter-wallpaper")]
        greeter_path: String,

        /// Disable writing wallpaper path to notification file
        #[arg(long)]
        no_notify: bool,

        /// Custom notification file path
        #[arg(long, default_value = "~/.cache/quickshell-wallpaper-path")]
        notify_path: String,
    },
}

#[derive(Clone, ValueEnum)]
pub enum ResizeArg {
    Crop,
    Fit,
    No,
}

#[derive(Clone, ValueEnum)]
pub enum UpscaleMode {
    /// Upscale this image only (default when --upscale is used without a value)
    Once,
    /// Enable persistent upscaling for all future images
    Always,
    /// Skip upscaling for this image (don't change persistent setting)
    Never,
    /// Disable persistent upscaling
    Off,
}

#[derive(Clone, ValueEnum)]
pub enum TransitionTypeArg {
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
    FluidDistortion,
    FluidDrain,
    FluidRipple,
    FluidVortex,
    FluidWave,
    InkBleed,
    LavaLamp,
    ChromaticAberration,
    LensDistortion,
    CrtShutdown,
    PerlinWipe,
    RadialBlur,
    Random,
    None,
}

impl From<ResizeArg> for wl_common::ipc_types::ResizeMode {
    fn from(arg: ResizeArg) -> Self {
        match arg {
            ResizeArg::Crop => Self::Crop,
            ResizeArg::Fit => Self::Fit,
            ResizeArg::No => Self::No,
        }
    }
}

fn parse_upscale_scale(s: &str) -> Result<u8, String> {
    let n: u8 = s.parse().map_err(|_| format!("invalid scale: '{s}'"))?;
    match n {
        2 | 4 | 8 | 16 => Ok(n),
        _ => Err("upscale scale must be 2, 4, 8, or 16".to_string()),
    }
}

impl From<TransitionTypeArg> for wl_common::ipc_types::TransitionType {
    fn from(arg: TransitionTypeArg) -> Self {
        match arg {
            TransitionTypeArg::Wipe => Self::Wipe,
            TransitionTypeArg::Wave => Self::Wave,
            TransitionTypeArg::Outer => Self::Outer,
            TransitionTypeArg::Pixelate => Self::Pixelate,
            TransitionTypeArg::Burn => Self::Burn,
            TransitionTypeArg::Glitch => Self::Glitch,
            TransitionTypeArg::Disintegrate => Self::Disintegrate,
            TransitionTypeArg::Dreamy => Self::Dreamy,
            TransitionTypeArg::GlitchMemories => Self::GlitchMemories,
            TransitionTypeArg::Morph => Self::Morph,
            TransitionTypeArg::Hexagonalize => Self::Hexagonalize,
            TransitionTypeArg::CrossZoom => Self::CrossZoom,
            TransitionTypeArg::FluidDistortion => Self::FluidDistortion,
            TransitionTypeArg::FluidDrain => Self::FluidDrain,
            TransitionTypeArg::FluidRipple => Self::FluidRipple,
            TransitionTypeArg::FluidVortex => Self::FluidVortex,
            TransitionTypeArg::FluidWave => Self::FluidWave,
            TransitionTypeArg::InkBleed => Self::InkBleed,
            TransitionTypeArg::LavaLamp => Self::LavaLamp,
            TransitionTypeArg::ChromaticAberration => Self::ChromaticAberration,
            TransitionTypeArg::LensDistortion => Self::LensDistortion,
            TransitionTypeArg::CrtShutdown => Self::CrtShutdown,
            TransitionTypeArg::PerlinWipe => Self::PerlinWipe,
            TransitionTypeArg::RadialBlur => Self::RadialBlur,
            TransitionTypeArg::Random => Self::Random,
            TransitionTypeArg::None => Self::None,
        }
    }
}

/// Parse comma-separated output names.
pub fn parse_outputs(s: &Option<String>) -> Option<Vec<String>> {
    s.as_ref().map(|s| {
        s.split(',')
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect()
    })
}

/// Parse hex color string like "#1a1b26" to [u8; 3].
pub fn parse_color(s: &str) -> Result<[u8; 3], String> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return Err(format!(
            "invalid hex color: expected 6 hex digits, got '{s}'"
        ));
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| format!("invalid hex color: '{s}'"))?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| format!("invalid hex color: '{s}'"))?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| format!("invalid hex color: '{s}'"))?;
    Ok([r, g, b])
}

/// Parse "x,y" position string to (f32, f32).
pub fn parse_position(s: &str) -> Result<(f32, f32), String> {
    if s == "center" {
        return Ok((0.5, 0.5));
    }
    if s == "top" {
        return Ok((0.5, 0.0));
    }
    if s == "bottom" {
        return Ok((0.5, 1.0));
    }
    if s == "left" {
        return Ok((0.0, 0.5));
    }
    if s == "right" {
        return Ok((1.0, 0.5));
    }

    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(format!("invalid position: expected 'x,y', got '{s}'"));
    }
    let x: f32 = parts[0]
        .trim()
        .parse()
        .map_err(|_| format!("invalid position x: '{}'", parts[0]))?;
    let y: f32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| format!("invalid position y: '{}'", parts[1]))?;
    Ok((x, y))
}

/// Parse "a,b,c,d" bezier string to [f32; 4].
pub fn parse_bezier(s: &str) -> Result<[f32; 4], String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return Err(format!("invalid bezier: expected 4 values, got '{s}'"));
    }
    let mut result = [0.0f32; 4];
    for (i, part) in parts.iter().enumerate() {
        result[i] = part
            .trim()
            .parse()
            .map_err(|_| format!("invalid bezier value: '{part}'"))?;
    }
    Ok(result)
}

/// Parse "freq,amp" wave string to (f32, f32).
pub fn parse_wave(s: &str) -> Result<(f32, f32), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(format!("invalid wave: expected 'freq,amp', got '{s}'"));
    }
    let freq: f32 = parts[0]
        .trim()
        .parse()
        .map_err(|_| format!("invalid wave freq: '{}'", parts[0]))?;
    let amp: f32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| format!("invalid wave amp: '{}'", parts[1]))?;
    Ok((freq, amp))
}
