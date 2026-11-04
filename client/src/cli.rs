use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "wl", version, about = "Vulkan wallpaper tool for Wayland")]
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

        /// Neural upscale mode
        #[arg(long)]
        upscale: Option<UpscaleMode>,

        /// Custom upscaler command, run through `sh -c` after substituting {input} and {output}
        #[arg(long)]
        upscale_cmd: Option<String>,

        /// Force upscale factor (2, 4, 8 or 16; default: fit to the output resolution)
        #[arg(long)]
        upscale_scale: Option<u8>,

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

        /// Disable writing wallpaper path to notification file
        #[arg(long)]
        no_notify: bool,

        /// Custom notification file path
        #[arg(long, default_value = "~/.cache/quickshell-wallpaper-path")]
        notify_path: String,
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
    Query {
        /// Print the result as JSON (for status bars and scripts)
        #[arg(long)]
        json: bool,
    },

    /// Alias for query
    List {
        /// Print the result as JSON (for status bars and scripts)
        #[arg(long)]
        json: bool,
    },

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

    /// Auto-rotate wallpapers from directories at a time interval
    Rotate {
        #[command(subcommand)]
        action: RotateAction,
    },

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

        /// Neural upscale mode
        #[arg(long)]
        upscale: Option<UpscaleMode>,

        /// Custom upscaler command, run through `sh -c` after substituting {input} and {output}
        #[arg(long)]
        upscale_cmd: Option<String>,

        /// Force upscale factor (2, 4, 8 or 16; default: fit to the output resolution)
        #[arg(long)]
        upscale_scale: Option<u8>,

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

#[derive(Subcommand)]
pub enum RotateAction {
    /// Start or replace wallpaper rotation
    Start {
        /// Directories to scan recursively for image files
        #[arg(required = true, num_args = 1..)]
        directories: Vec<PathBuf>,

        /// Rotation interval (e.g., "30m", "1h30m", "90", "2h", "45s")
        #[arg(short, long, default_value = "30m")]
        interval: String,

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

        /// Neural upscale mode
        #[arg(long)]
        upscale: Option<UpscaleMode>,

        /// Custom upscaler command, run through `sh -c` after substituting {input} and {output}
        #[arg(long)]
        upscale_cmd: Option<String>,

        /// Force upscale factor (2, 4, 8 or 16; default: fit to the output resolution)
        #[arg(long)]
        upscale_scale: Option<u8>,

        /// Disable writing wallpaper path to notification file
        #[arg(long)]
        no_notify: bool,

        /// Custom notification file path
        #[arg(long, default_value = "~/.cache/quickshell-wallpaper-path")]
        notify_path: String,
    },

    /// Stop active wallpaper rotation
    Stop,

    /// Show current rotation status
    Status {
        /// Print the status as JSON (for status bars and scripts)
        #[arg(long)]
        json: bool,
    },

    /// Skip to next wallpaper immediately
    Next,
}

#[derive(Clone, ValueEnum)]
pub enum UpscaleMode {
    /// Upscale this image only
    Once,
    /// Enable persistent upscaling for all future commands
    Always,
    /// Skip upscaling for this image only
    Never,
    /// Disable persistent upscaling
    Off,
}

#[derive(Clone, ValueEnum)]
pub enum ResizeArg {
    /// Scale to fill the output, cropping the overflow (default)
    Crop,
    /// Scale to fit inside the output, letterboxing with black bars
    Fit,
    /// No scaling: native pixels, centered, overflow cropped
    No,
    /// Same as `no` — native pixels, centered, overflow cropped
    Center,
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
            ResizeArg::Center => Self::Center,
        }
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

/// Output names the daemon does not know about.
///
/// The daemon drops a wallpaper for an unknown name with a warning, which used
/// to make a typo in `--outputs` look like a success; the CLI now fails before
/// decoding and uploading anything.
pub fn unknown_outputs<'a>(requested: &'a [String], known: &[String]) -> Vec<&'a str> {
    requested
        .iter()
        .map(String::as_str)
        .filter(|name| !known.iter().any(|known| known == name))
        .collect()
}

/// Reject transition timing values that cannot produce a sane animation.
///
/// The daemon refuses to get stuck on a degenerate duration, but a negative,
/// NaN or infinite value, or a multi-minute transition, is a user error that
/// should fail at the CLI instead of being silently "completed".
pub fn validate_transition_timing(duration_secs: f32, fps: u32) -> Result<(), String> {
    if !duration_secs.is_finite() || duration_secs < 0.0 {
        return Err(format!(
            "--transition-duration must be a finite, non-negative number of seconds (got {duration_secs})"
        ));
    }
    if duration_secs > 600.0 {
        return Err(format!(
            "--transition-duration is limited to 600 seconds (got {duration_secs})"
        ));
    }
    if fps > 1000 {
        return Err(format!("--transition-fps is limited to 1000 (got {fps})"));
    }
    Ok(())
}

/// Parse hex color string like "#1a1b26" to [u8; 3].
pub fn parse_color(s: &str) -> Result<[u8; 3], String> {
    let s = s.trim_start_matches('#');
    // Byte length is checked before slicing at byte offsets, so reject anything
    // that is not ASCII first: `&s[0..2]` would panic on a multi-byte boundary.
    if !s.is_ascii() || s.len() != 6 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_exposes_version_and_help_metadata() {
        Cli::command().debug_assert();
        let command = Cli::command();
        assert!(command.get_version().is_some(), "wl --version must work");
        assert!(command.get_about().is_some());
    }

    #[test]
    fn rejects_degenerate_transition_timing() {
        for duration in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(
                validate_transition_timing(duration, 30).is_err(),
                "duration {duration} must be rejected"
            );
        }
        assert!(validate_transition_timing(601.0, 30).is_err());
        assert!(validate_transition_timing(1.0, 1001).is_err());
        assert!(validate_transition_timing(0.0, 0).is_ok());
        assert!(validate_transition_timing(600.0, 1000).is_ok());
    }

    #[test]
    fn unknown_outputs_lists_only_missing_names() {
        let requested = vec![
            "DP-2".to_string(),
            "DP-9".to_string(),
            "HDMI-A-1".to_string(),
        ];
        let known = vec!["DP-2".to_string(), "HDMI-A-1".to_string()];

        assert_eq!(unknown_outputs(&requested, &known), vec!["DP-9"]);
        assert!(unknown_outputs(&[], &known).is_empty());
        assert!(unknown_outputs(&known, &known).is_empty());
        assert_eq!(unknown_outputs(&known, &[]).len(), 2);
    }

    #[test]
    fn rejects_multibyte_slices_before_they_can_panic() {
        // Six bytes, but slicing at 2 would split the UTF-8 sequence.
        assert!(parse_color("€aaa").is_err());
        // Six bytes of ASCII still parse.
        assert_eq!(parse_color("#1a2b3c").unwrap(), [0x1a, 0x2b, 0x3c]);
        assert_eq!(parse_color("1a2b3c").unwrap(), [0x1a, 0x2b, 0x3c]);
    }
}
