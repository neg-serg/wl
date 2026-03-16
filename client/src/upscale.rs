use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::BufReader;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use wl_common::cache::{
    UpscaleCacheEntry, load_upscale_index, save_upscale_index, upscale_cache_dir,
};
use wl_common::ipc_types::*;

use crate::ipc::IpcClient;

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
pub fn is_animated_gif(path: &Path) -> bool {
    let is_gif = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gif"))
        .unwrap_or(false);
    if !is_gif {
        return false;
    }

    // Use image crate to check frame count (needs BufReader for AnimationDecoder)
    use image::codecs::gif::GifDecoder;
    use image::AnimationDecoder;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let reader = BufReader::new(file);
    let decoder = match GifDecoder::new(reader) {
        Ok(d) => d,
        Err(_) => return false,
    };
    // If we can get more than 1 frame, it's animated
    decoder.into_frames().count() > 1
}

/// Get image dimensions without fully decoding.
pub fn get_image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    image::image_dimensions(path).map_err(|e| format!("failed to read image dimensions: {e}"))
}

// ---------------------------------------------------------------------------
// Resolution query (T006)
// ---------------------------------------------------------------------------

/// Query the daemon for the maximum physical resolution among target outputs.
pub async fn query_max_physical_resolution(
    outputs: &Option<Vec<String>>,
) -> Result<(u32, u32), String> {
    let mut client = IpcClient::connect()
        .await
        .map_err(|_| "daemon is not running (needed for resolution query)".to_string())?;

    let response = client
        .send_command(&IpcCommand::Query)
        .await
        .map_err(|e| format!("query failed: {e}"))?;

    match response {
        IpcResponse::QueryResult { outputs: infos } => {
            let filtered: Vec<&OutputInfo> = if let Some(target_names) = outputs {
                infos
                    .iter()
                    .filter(|o| target_names.iter().any(|n| n == &o.name))
                    .collect()
            } else {
                infos.iter().collect()
            };

            let mut max_w = 0u32;
            let mut max_h = 0u32;
            for info in &filtered {
                if let Some((w, h)) = info.physical_resolution {
                    if w * h > max_w * max_h {
                        max_w = w;
                        max_h = h;
                    }
                }
            }

            if max_w == 0 || max_h == 0 {
                Err("no output with physical resolution found".to_string())
            } else {
                Ok((max_w, max_h))
            }
        }
        IpcResponse::Error { message } => Err(message),
        _ => Err("unexpected response from daemon".to_string()),
    }
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

    // Multi-pass: use temp files for intermediates
    let cache_dir = upscale_cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);

    let mut current_input = input.to_path_buf();
    let mut temp_files = Vec::new();

    for (i, &pass_scale) in passes.iter().enumerate() {
        let is_last = i == passes.len() - 1;
        let pass_output = if is_last {
            output.to_path_buf()
        } else {
            let temp = cache_dir.join(format!("_temp_pass_{i}.png"));
            temp_files.push(temp.clone());
            temp
        };

        realesrgan_single_pass(&current_input, &pass_output, pass_scale)?;
        current_input = pass_output;
    }

    // Clean up temp files
    for temp in temp_files {
        let _ = std::fs::remove_file(temp);
    }

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
        return Err(format!("custom upscaler exited with status {status}"));
    }

    validate_output_file(output)?;
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

/// Get source file identity (mtime_secs, file_size) for cache keying.
pub fn source_identity(path: &Path) -> Result<(u64, u64), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot stat source file: {e}"))?;
    let mtime = meta
        .modified()
        .map_err(|e| format!("cannot get mtime: {e}"))?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok((mtime, meta.len()))
}

/// Generate a deterministic cache filename from source identity.
pub fn cache_filename(source_path: &str, mtime: u64, size: u64, scale: u8) -> String {
    let mut hasher = DefaultHasher::new();
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
pub async fn upscale_image(
    path: &str,
    upscale_cmd: &Option<String>,
    forced_scale: &Option<u8>,
    outputs: &Option<Vec<String>>,
) -> String {
    match upscale_image_inner(path, upscale_cmd, forced_scale, outputs).await {
        Ok(upscaled) => upscaled,
        Err(e) => {
            eprintln!("Warning: upscaling failed ({e}), using original image");
            path.to_string()
        }
    }
}

async fn upscale_image_inner(
    path: &str,
    upscale_cmd: &Option<String>,
    forced_scale: &Option<u8>,
    outputs: &Option<Vec<String>>,
) -> Result<String, String> {
    let img_path = Path::new(path);

    // Skip SVG
    if is_svg(img_path) {
        eprintln!("Skipping upscale: SVG images are resolution-independent");
        return Ok(path.to_string());
    }

    // Skip animated GIF
    if is_animated_gif(img_path) {
        eprintln!("Skipping upscale: animated GIFs are not supported");
        return Ok(path.to_string());
    }

    // Get source dimensions
    let (src_w, src_h) = get_image_dimensions(img_path)?;

    // Determine scale factor
    let scale = if let Some(&forced) = forced_scale.as_ref() {
        // Forced scale — no daemon query needed
        forced
    } else {
        // Query daemon for target physical resolution
        let (target_w, target_h) = query_max_physical_resolution(outputs).await?;

        match compute_scale_factor(src_w, src_h, target_w, target_h) {
            Some(s) => s,
            None => {
                eprintln!(
                    "Skipping upscale: image already meets target resolution ({src_w}x{src_h})"
                );
                return Ok(path.to_string());
            }
        }
    };

    // Canonicalize path for cache keying
    let canonical = std::fs::canonicalize(img_path)
        .unwrap_or_else(|_| img_path.to_path_buf());
    let canonical_str = canonical.to_string_lossy().to_string();

    // Get source identity for cache
    let (mtime, size) = source_identity(&canonical)?;

    // Check cache
    let mut index = load_upscale_index();
    let cached = index.lookup(&canonical_str, mtime, size, scale);
    if let Some(entry) = cached {
        let cached_path = upscale_cache_dir().join(&entry.cached_filename);
        if cached_path.exists() {
            let filename = img_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            eprintln!("Using cached upscale: {filename}");
            return Ok(cached_path.to_string_lossy().to_string());
        }
    }

    // Ensure cache directory exists
    let cache_dir = upscale_cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("cannot create cache directory: {e}"))?;

    // Generate output filename
    let out_filename = cache_filename(&canonical_str, mtime, size, scale);
    let out_path = cache_dir.join(&out_filename);

    let filename = img_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    eprintln!("Upscaling {filename} ({src_w}x{src_h} \u{2192} {scale}x)...");
    let start = Instant::now();

    // Run upscaler
    if let Some(cmd) = upscale_cmd {
        invoke_custom_cmd(cmd, &canonical, &out_path)?;
    } else {
        invoke_realesrgan(&canonical, &out_path, scale)?;
    }

    let elapsed = start.elapsed().as_secs_f32();

    // Read result dimensions
    let result_dims = get_image_dimensions(&out_path).unwrap_or((0, 0));
    eprintln!(
        "Upscaled in {elapsed:.1}s \u{2192} {}x{}",
        result_dims.0, result_dims.1
    );

    // Update cache
    let entry = UpscaleCacheEntry {
        source_path: canonical_str,
        source_mtime_secs: mtime,
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

    Ok(out_path.to_string_lossy().to_string())
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
