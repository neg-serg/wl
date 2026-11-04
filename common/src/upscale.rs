//! Upscaling helpers shared by the CLI and the daemon.
//!
//! The daemon applies the same scale/cache/invoke rules to rotation
//! wallpapers as the client does for one-off `wl img`/`wl random` commands.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::cache::{UpscaleCacheEntry, load_upscale_index, save_upscale_index, upscale_cache_dir};

// ---------------------------------------------------------------------------
// Image type detection (T005)
// ---------------------------------------------------------------------------

/// Check if the path is an SVG file (resolution-independent, skip upscaling).
pub fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
        .unwrap_or(false)
}

/// Check if the file is an animated GIF (more than 1 frame).
///
/// Only frame metadata is read: decoding every frame just to count them used to
/// decompress the whole animation on the CLI's hot path.
pub fn is_animated_gif(path: &Path) -> bool {
    let is_gif = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gif"))
        .unwrap_or(false);
    if !is_gif {
        return false;
    }

    crate::image_decode::gif_info(path)
        .map(|info| info.frame_count > 1)
        .unwrap_or(false)
}

/// Get image dimensions without fully decoding.
pub fn get_image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    image::image_dimensions(path).map_err(|e| format!("failed to read image dimensions: {e}"))
}

// ---------------------------------------------------------------------------
// Scale computation (T007)
// ---------------------------------------------------------------------------

/// Compute the minimum scale factor needed (2 or 4), or None if no upscaling needed.
pub fn compute_scale_factor(src_w: u32, src_h: u32, target_w: u32, target_h: u32) -> Option<u8> {
    let src_max = src_w.max(src_h);
    let target_max = target_w.max(target_h);

    if src_max >= target_max {
        return None; // Already large enough
    }

    if src_max * 2 >= target_max {
        Some(2)
    } else {
        Some(4)
    }
}

/// Decompose a total scale factor into passes of 2 or 4.
/// E.g., 8 -> [4, 2], 4 -> [4], 2 -> [2], 16 -> [4, 4].
pub fn decompose_scale(total: u8) -> Vec<u8> {
    let mut remaining = total;
    let mut passes = Vec::new();
    while remaining > 1 {
        if remaining >= 4 {
            passes.push(4);
            remaining /= 4;
        } else {
            passes.push(2);
            remaining /= 2;
        }
    }
    passes
}

// ---------------------------------------------------------------------------
// Upscaler invocation (T008, T009)
// ---------------------------------------------------------------------------

/// Run a single pass of realesrgan-ncnn-vulkan.
fn realesrgan_single_pass(input: &Path, output: &Path, scale: u8) -> Result<(), String> {
    let status = Command::new("realesrgan-ncnn-vulkan")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("-s")
        .arg(scale.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "upscaler not found. Install realesrgan-ncnn-vulkan or specify --upscale-cmd"
                    .to_string()
            } else {
                format!("failed to run realesrgan-ncnn-vulkan: {e}")
            }
        })?;

    if !status.success() {
        // A failed run can still leave a truncated file that would be cached.
        let _ = std::fs::remove_file(output);
        return Err(format!(
            "realesrgan-ncnn-vulkan exited with status {status}"
        ));
    }

    // Validate output exists and is non-empty
    validate_output_file(output)?;
    Ok(())
}

/// Invoke Real-ESRGAN, handling multi-pass for scale > 4.
fn invoke_realesrgan(input: &Path, output: &Path, scale: u8) -> Result<(), String> {
    let passes = decompose_scale(scale);

    if passes.len() == 1 {
        return realesrgan_single_pass(input, output, passes[0]);
    }

    // Multi-pass: intermediates go through uniquely named temp files, and the
    // guard removes them even when a pass fails halfway.
    let cache_dir = upscale_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);

    let mut current_input = input.to_path_buf();
    let mut temps = TempFiles::default();

    for (i, &pass_scale) in passes.iter().enumerate() {
        let is_last = i == passes.len() - 1;
        let pass_output = if is_last {
            output.to_path_buf()
        } else {
            let temp = cache_dir.join(format!("_temp_pass_{}_{i}.png", std::process::id()));
            temps.push(temp.clone());
            temp
        };

        realesrgan_single_pass(&current_input, &pass_output, pass_scale)?;
        current_input = pass_output;
    }

    temps.cleanup();
    Ok(())
}

/// Invoke a custom upscaler command.
fn invoke_custom_cmd(cmd: &str, input: &Path, output: &Path) -> Result<(), String> {
    let input_str = input.to_string_lossy();
    let output_str = output.to_string_lossy();

    let expanded = cmd
        .replace("{input}", &input_str)
        .replace("{output}", &output_str);

    let status = Command::new("sh")
        .arg("-c")
        .arg(&expanded)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("failed to run custom upscaler: {e}"))?;

    if !status.success() {
        let _ = std::fs::remove_file(output);
        return Err(format!("custom upscaler exited with status {status}"));
    }

    validate_output_file(output)?;
    Ok(())
}

/// Temp files created for multi-pass upscaling; removed on drop.
#[derive(Default)]
struct TempFiles {
    paths: Vec<std::path::PathBuf>,
}

impl TempFiles {
    fn push(&mut self, path: std::path::PathBuf) {
        self.paths.push(path);
    }

    fn cleanup(&mut self) {
        for path in self.paths.drain(..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Supported upscale factors.
///
/// `realesrgan`-style upscalers take 2 or 4 per pass; larger factors are
/// composed from those passes. Anything else would silently produce a smaller
/// image than the cache key claims.
const SUPPORTED_SCALES: [u8; 4] = [2, 4, 8, 16];

fn validate_scale(scale: u8) -> Result<(), String> {
    if SUPPORTED_SCALES.contains(&scale) {
        Ok(())
    } else {
        Err(format!(
            "unsupported upscale factor {scale}x (supported: {SUPPORTED_SCALES:?})"
        ))
    }
}

/// Rotate/flip `output` so it already includes the EXIF orientation of
/// `source`.
///
/// Upscalers write fresh pixel data without metadata; without this the daemon
/// decodes the result with `NoTransforms` and displays it sideways.
fn bake_source_orientation(source: &Path, output: &Path) -> Result<(), String> {
    let orientation = crate::image_decode::read_orientation(source).map_err(|e| e.to_string())?;
    if orientation == image::metadata::Orientation::NoTransforms {
        return Ok(());
    }

    let mut image = image::open(output).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    image.save(output).map_err(|e| e.to_string())?;
    Ok(())
}

/// Validate that an output file exists and is non-empty.
fn validate_output_file(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > 0 => Ok(()),
        Ok(_) => {
            let _ = std::fs::remove_file(path);
            Err("upscaler produced empty output file".to_string())
        }
        Err(_) => Err("upscaler did not produce output file".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Cache helpers (T014)
// ---------------------------------------------------------------------------

/// Get source file identity (mtime_ms, file_size) for cache keying.
pub fn source_identity(path: &Path) -> Result<(u64, u64), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot stat source file: {e}"))?;
    let mtime = meta
        .modified()
        .map_err(|e| format!("cannot get mtime: {e}"))?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Ok((mtime, meta.len()))
}

/// Bump when the *meaning* of a cached upscale changes (e.g. the output now has
/// the source's EXIF orientation baked in), so stale artifacts are not reused.
const UPSCALE_CACHE_VERSION: u8 = 2;

/// Generate a deterministic cache filename from source identity.
pub fn cache_filename(source_path: &str, mtime: u64, size: u64, scale: u8) -> String {
    let mut hasher = DefaultHasher::new();
    UPSCALE_CACHE_VERSION.hash(&mut hasher);
    source_path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    size.hash(&mut hasher);
    scale.hash(&mut hasher);
    format!("{:016x}.png", hasher.finish())
}

// ---------------------------------------------------------------------------
// Main orchestrator (T010)
// ---------------------------------------------------------------------------

/// Upscale an image if needed. Returns the path to use (upscaled or original).
///
/// This function never fails fatally — on any error, it returns the original path.
/// Inputs for one upscale attempt.
pub struct UpscaleOptions<'a> {
    /// Custom command with `{input}`/`{output}` placeholders (shell).
    pub custom_cmd: Option<&'a str>,
    /// Force a factor instead of fitting the target resolution.
    pub forced_scale: Option<u8>,
    /// Target (physical) resolution used when no factor is forced.
    pub target: (u32, u32),
}

/// Result of an upscale attempt.
#[derive(Debug)]
pub enum UpscaleOutcome {
    /// The image was left as-is; the reason is user-facing.
    Skipped { reason: String },
    Upscaled {
        path: PathBuf,
        /// True when an existing cache entry was reused.
        cached: bool,
        elapsed: Duration,
        result_dimensions: (u32, u32),
        scale: u8,
    },
}

/// Upscale `source` if needed and return where the result lives.
///
/// This is the shared implementation behind the CLI's `--upscale` handling and
/// the daemon's rotation upscaling. It is blocking (it may run an external
/// upscaler for seconds), so async callers should use `spawn_blocking`.
pub fn upscale_source(
    source: &Path,
    options: &UpscaleOptions<'_>,
) -> Result<UpscaleOutcome, String> {
    if is_svg(source) {
        return Ok(UpscaleOutcome::Skipped {
            reason: "SVG images are resolution-independent".to_string(),
        });
    }
    if is_animated_gif(source) {
        return Ok(UpscaleOutcome::Skipped {
            reason: "animated GIFs are not supported".to_string(),
        });
    }

    let (src_w, src_h) = get_image_dimensions(source)?;

    let scale = match options.forced_scale {
        Some(forced) => forced,
        None => match compute_scale_factor(src_w, src_h, options.target.0, options.target.1) {
            Some(scale) => scale,
            None => {
                return Ok(UpscaleOutcome::Skipped {
                    reason: format!("image already meets the target resolution ({src_w}x{src_h})"),
                });
            }
        },
    };
    validate_scale(scale)?;

    let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let canonical_str = canonical.to_string_lossy().to_string();
    let (mtime, size) = source_identity(&canonical)?;

    let mut index = load_upscale_index();
    if let Some(entry) = index.lookup(&canonical_str, mtime, size, scale) {
        let cached_path = upscale_cache_dir().join(&entry.cached_filename);
        if cached_path.exists() {
            let result_dimensions = get_image_dimensions(&cached_path).unwrap_or((0, 0));
            return Ok(UpscaleOutcome::Upscaled {
                path: cached_path,
                cached: true,
                elapsed: Duration::ZERO,
                result_dimensions,
                scale,
            });
        }
    }

    let cache_dir = upscale_cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("cannot create cache directory: {e}"))?;

    let out_filename = cache_filename(&canonical_str, mtime, size, scale);
    let out_path = cache_dir.join(&out_filename);

    let start = Instant::now();
    match options.custom_cmd {
        Some(cmd) => invoke_custom_cmd(cmd, &canonical, &out_path)?,
        None => invoke_realesrgan(&canonical, &out_path, scale)?,
    }
    let elapsed = start.elapsed();

    // External upscalers drop metadata, so a portrait photo would lose its EXIF
    // orientation and the daemon would show the upscaled file sideways. Bake
    // the source orientation into the cached pixels instead.
    if let Err(e) = bake_source_orientation(&canonical, &out_path) {
        eprintln!("Warning: could not apply the EXIF orientation to the upscaled image: {e}");
    }

    let result_dimensions = get_image_dimensions(&out_path).unwrap_or((0, 0));

    let entry = UpscaleCacheEntry {
        source_path: canonical_str,
        source_mtime_ms: mtime,
        source_size: size,
        scale_factor: scale,
        cached_filename: out_filename,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    index.insert(entry);
    if let Err(e) = save_upscale_index(&index) {
        eprintln!("Warning: failed to save upscale cache index: {e}");
    }

    Ok(UpscaleOutcome::Upscaled {
        path: out_path,
        cached: false,
        elapsed,
        result_dimensions,
        scale,
    })
}

// ---------------------------------------------------------------------------
// Tests (T007)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_scale_factor_skip() {
        // Source already large enough
        assert_eq!(compute_scale_factor(3840, 2160, 3840, 2160), None);
        assert_eq!(compute_scale_factor(4000, 3000, 3840, 2160), None);
    }

    #[test]
    fn test_compute_scale_factor_2x() {
        // Source * 2 >= target
        assert_eq!(compute_scale_factor(1920, 1080, 3840, 2160), Some(2));
        assert_eq!(compute_scale_factor(2000, 1500, 3840, 2160), Some(2));
    }

    #[test]
    fn test_compute_scale_factor_4x() {
        // Source * 2 < target
        assert_eq!(compute_scale_factor(640, 480, 3840, 2160), Some(4));
        assert_eq!(compute_scale_factor(800, 600, 3840, 2160), Some(4));
    }

    #[test]
    fn svg_is_skipped_before_reading_the_file() {
        let options = UpscaleOptions {
            custom_cmd: None,
            forced_scale: Some(2),
            target: (0, 0),
        };
        let outcome = upscale_source(Path::new("/nonexistent/wallpaper.svg"), &options)
            .expect("svg must be skipped without touching the file");
        match outcome {
            UpscaleOutcome::Skipped { reason } => assert!(reason.contains("SVG"), "{reason}"),
            other => panic!("expected skip, got {other:?}"),
        }
    }

    #[test]
    fn animated_gif_is_skipped() {
        let dir = std::env::temp_dir().join(format!("wl-upscale-gif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("anim.gif");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = gif::Encoder::new(file, 2, 2, &[]).unwrap();
            for color in [[255u8, 0, 0, 255], [0, 255, 0, 255]] {
                // from_rgba_speed wants width*height*4 bytes.
                let mut rgba: Vec<u8> = color.iter().copied().cycle().take(2 * 2 * 4).collect();
                let frame = gif::Frame::from_rgba_speed(2, 2, &mut rgba, 10);
                encoder.write_frame(&frame).unwrap();
            }
        }

        let options = UpscaleOptions {
            custom_cmd: None,
            forced_scale: Some(2),
            target: (0, 0),
        };
        let outcome = upscale_source(&path, &options).unwrap();
        match outcome {
            UpscaleOutcome::Skipped { reason } => {
                assert!(reason.contains("animated GIF"), "{reason}")
            }
            other => panic!("expected skip, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn image_at_the_target_resolution_is_skipped() {
        let dir = std::env::temp_dir().join(format!("wl-upscale-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("small.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();

        let options = UpscaleOptions {
            custom_cmd: None,
            forced_scale: None,
            target: (32, 32),
        };
        let outcome = upscale_source(&path, &options).unwrap();
        match outcome {
            UpscaleOutcome::Skipped { reason } => {
                assert!(reason.contains("already meets"), "{reason}")
            }
            other => panic!("expected skip, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unsupported_scale_factors() {
        for good in [2u8, 4, 8, 16] {
            assert!(validate_scale(good).is_ok(), "{good}x must be accepted");
        }
        for bad in [0u8, 1, 3, 5, 6, 7, 9, 255] {
            assert!(validate_scale(bad).is_err(), "{bad}x must be rejected");
        }
        // Every supported factor decomposes into whole 2/4 passes.
        for good in [2u8, 4, 8, 16] {
            let product: u32 = decompose_scale(good)
                .iter()
                .map(|s| u32::from(*s))
                .product();
            assert_eq!(product, u32::from(good));
        }
    }

    #[test]
    fn orientation_is_baked_into_the_upscaled_output() {
        let dir = std::env::temp_dir().join(format!("wl-upscale-exif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 8x4 source tagged "rotate 90".
        let source = crate::test_helpers::jpeg_with_orientation(&dir, Some(6));
        // Pretend the upscaler produced a 16x8 image without metadata.
        let output = dir.join("upscaled.png");
        image::RgbaImage::new(16, 8).save(&output).unwrap();

        bake_source_orientation(&source, &output).unwrap();
        assert_eq!(
            image::image_dimensions(&output).unwrap(),
            (8, 16),
            "the upscaled output kept the unrotated dimensions"
        );

        // A source without orientation metadata must be left untouched.
        let plain = crate::test_helpers::jpeg_with_orientation(&dir, None);
        let untouched = dir.join("untouched.png");
        image::RgbaImage::new(16, 8).save(&untouched).unwrap();
        bake_source_orientation(&plain, &untouched).unwrap();
        assert_eq!(image::image_dimensions(&untouched).unwrap(), (16, 8));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multi_pass_temp_files_are_unique_per_process_and_pass() {
        let name = |i: usize| format!("_temp_pass_{}_{i}.png", std::process::id());
        assert_eq!(name(0), name(0));
        assert_ne!(name(0), name(1));
        assert!(name(0).contains(&std::process::id().to_string()));
    }

    #[test]
    fn test_decompose_scale() {
        assert_eq!(decompose_scale(2), vec![2]);
        assert_eq!(decompose_scale(4), vec![4]);
        assert_eq!(decompose_scale(8), vec![4, 2]);
        assert_eq!(decompose_scale(16), vec![4, 4]);
    }

    #[test]
    fn test_cache_filename_deterministic() {
        let a = cache_filename("/path/img.jpg", 1000, 5000, 4);
        let b = cache_filename("/path/img.jpg", 1000, 5000, 4);
        assert_eq!(a, b);

        let c = cache_filename("/path/img.jpg", 1001, 5000, 4);
        assert_ne!(a, c);
    }
}
