use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use wl_common::cache::{UpscaleCacheEntry, UpscaleCacheIndex, upscale_cache_dir};
use wl_common::ipc_types::*;

use crate::ipc::IpcClient;

/// Query the daemon for output dimensions and return the maximum (width, height)
/// among targeted outputs. Falls back to 3840x2160 if daemon is not reachable.
pub async fn query_max_resolution(outputs: &Option<Vec<String>>) -> (u32, u32) {
    let result = async {
        let mut client = IpcClient::connect().await.ok()?;
        let response = client.send_command(&IpcCommand::Query).await.ok()?;
        match response {
            IpcResponse::QueryResult { outputs: infos } => {
                let mut max_w: u32 = 0;
                let mut max_h: u32 = 0;
                for info in &infos {
                    // Filter by target outputs if specified.
                    if let Some(targets) = outputs
                        && !targets.iter().any(|t| t == &info.name)
                    {
                        continue;
                    }
                    if let Some((w, h)) = info.dimensions {
                        max_w = max_w.max(w);
                        max_h = max_h.max(h);
                    }
                }
                if max_w > 0 && max_h > 0 {
                    Some((max_w, max_h))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    .await;

    result.unwrap_or_else(|| {
        eprintln!(
            "Warning: could not query daemon for monitor resolution, assuming 4K (3840x2160)"
        );
        (3840, 2160)
    })
}

/// Compute the minimum scale factor (2 or 4) needed to upscale the source image
/// to meet the target resolution. Returns `None` if the source already meets or
/// exceeds the target.
pub fn compute_scale_factor(
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
) -> Option<u8> {
    let source_max = source_w.max(source_h);
    let target_max = target_w.max(target_h);

    if source_max >= target_max {
        None
    } else if source_max * 2 >= target_max {
        Some(2)
    } else {
        Some(4)
    }
}

/// Get image dimensions without fully decoding the image.
pub fn get_image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    image::image_dimensions(path).map_err(|e| {
        format!(
            "failed to read image dimensions for '{}': {e}",
            path.display()
        )
    })
}

/// Check if a file is an animated GIF (has .gif extension).
pub fn is_animated_gif(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"))
}

/// Check if a file is an SVG image.
fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"))
}

/// Run a single pass of Real-ESRGAN with scale 2 or 4.
fn realesrgan_single_pass(input: &Path, output: &Path, scale: u8) -> Result<(), String> {
    let status = Command::new("realesrgan-ncnn-vulkan")
        .arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("-s")
        .arg(scale.to_string())
        .status();

    match status {
        Ok(s) if s.success() => {
            if output.exists() {
                Ok(())
            } else {
                Err("upscaler exited successfully but output file was not created".to_string())
            }
        }
        Ok(s) => {
            let code = s.code().unwrap_or(-1);
            Err(format!("upscaler failed (exit code {code})"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "'realesrgan-ncnn-vulkan' not found. Install it or specify a custom upscaler with --upscale-cmd.\nSee: https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan".to_string(),
        ),
        Err(e) => Err(format!("failed to run upscaler: {e}")),
    }
}

/// Decompose a total scale factor into a sequence of passes (each 2 or 4).
/// E.g. 8 → [4, 2], 16 → [4, 4], 4 → [4], 2 → [2].
fn decompose_scale(total: u8) -> Vec<u8> {
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

/// Invoke Real-ESRGAN, using multiple passes for scales > 4.
fn invoke_realesrgan(input: &Path, output: &Path, scale: u8) -> Result<(), String> {
    let passes = decompose_scale(scale);
    if passes.len() == 1 {
        return realesrgan_single_pass(input, output, passes[0]);
    }

    // Multi-pass: use temp files for intermediate results.
    let cache_dir = output.parent().unwrap_or(Path::new("/tmp"));
    let mut current_input = input.to_path_buf();

    for (i, &pass_scale) in passes.iter().enumerate() {
        let is_last = i == passes.len() - 1;
        let pass_output = if is_last {
            output.to_path_buf()
        } else {
            cache_dir.join(format!("_upscale_pass_{i}.png"))
        };

        eprintln!("  Pass {}/{}: {pass_scale}x...", i + 1, passes.len());
        realesrgan_single_pass(&current_input, &pass_output, pass_scale)?;

        // Clean up previous intermediate file.
        if i > 0 {
            let _ = std::fs::remove_file(&current_input);
        }

        current_input = pass_output;
    }

    Ok(())
}

/// Invoke a custom upscaler command.
fn invoke_custom_cmd(cmd: &str, input: &Path, output: &Path) -> Result<(), String> {
    let input_str = input.to_string_lossy();
    let output_str = output.to_string_lossy();

    let full_cmd = if cmd.contains("{input}") && cmd.contains("{output}") {
        cmd.replace("{input}", &input_str)
            .replace("{output}", &output_str)
    } else {
        format!("{cmd} {input_str} {output_str}")
    };

    let status = Command::new("sh")
        .arg("-c")
        .arg(&full_cmd)
        .status()
        .map_err(|e| format!("failed to run custom upscaler: {e}"))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(format!("custom upscaler failed (exit code {code})"));
    }

    if !output.exists() {
        return Err(
            "custom upscaler exited successfully but output file was not created".to_string(),
        );
    }

    Ok(())
}

/// Get (mtime_secs, file_size) for a file.
fn source_identity(path: &Path) -> Result<(u64, u64), String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("failed to read metadata for '{}': {e}", path.display()))?;
    let mtime = meta
        .modified()
        .map_err(|e| format!("failed to get mtime: {e}"))?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok((mtime, meta.len()))
}

/// Generate a deterministic cache filename from the cache key components.
fn cache_filename(source_path: &str, mtime: u64, size: u64, scale: u8) -> String {
    let mut hasher = DefaultHasher::new();
    source_path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    size.hash(&mut hasher);
    scale.hash(&mut hasher);
    format!("{:016x}.png", hasher.finish())
}

/// Main upscale orchestrator. Returns the path to use for the wallpaper:
/// either the upscaled cached/new file, or the original on skip/error.
pub async fn upscale_image(
    path: &str,
    upscale_cmd: &Option<String>,
    forced_scale: &Option<u8>,
    outputs: &Option<Vec<String>>,
) -> String {
    let source = Path::new(path);

    // Skip SVG images — they are resolution-independent.
    if is_svg(source) {
        eprintln!("Skipping upscale: SVG images are resolution-independent");
        return path.to_string();
    }

    // Skip animated GIFs.
    if is_animated_gif(source) {
        eprintln!("Skipping upscale: animated GIFs not supported");
        return path.to_string();
    }

    // Get source image dimensions.
    let (src_w, src_h) = match get_image_dimensions(source) {
        Ok(dims) => dims,
        Err(e) => {
            eprintln!("Warning: {e}, using original image");
            return path.to_string();
        }
    };

    // Canonicalize source path for cache keying.
    let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let canonical_str = canonical.to_string_lossy().to_string();

    // Get source identity for cache.
    let (mtime, size) = match source_identity(source) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Warning: {e}, using original image");
            return path.to_string();
        }
    };

    // Determine scale factor: forced or auto-detected from monitor resolution.
    let scale = if let Some(forced) = forced_scale {
        *forced
    } else {
        // Query daemon for target resolution.
        let (target_w, target_h) = query_max_resolution(outputs).await;

        match compute_scale_factor(src_w, src_h, target_w, target_h) {
            Some(s) => s,
            None => {
                eprintln!(
                    "Skipping upscale: image resolution ({src_w}x{src_h}) already meets target ({target_w}x{target_h})"
                );
                return path.to_string();
            }
        }
    };

    let cache_dir = upscale_cache_dir();

    // Check cache.
    let mut cache = UpscaleCacheIndex::load(&cache_dir);
    if let Some(cached_path) = cache.lookup(&cache_dir, &canonical_str, mtime, size, scale) {
        eprintln!("Using cached upscaled image: {}", cached_path.display());
        return cached_path.to_string_lossy().to_string();
    }

    // Prepare output path.
    let filename = cache_filename(&canonical_str, mtime, size, scale);
    let output_path = cache_dir.join(&filename);

    // Ensure cache directory exists.
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        eprintln!("Warning: failed to create upscale cache directory: {e}");
    }

    // Print progress.
    let target_w_scaled = src_w * scale as u32;
    let target_h_scaled = src_h * scale as u32;
    let source_name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    eprintln!(
        "Upscaling {source_name} ({src_w}x{src_h} \u{2192} {target_w_scaled}x{target_h_scaled}, scale={scale}x)..."
    );

    let start = Instant::now();

    // Invoke upscaler.
    let result = if let Some(cmd) = upscale_cmd {
        invoke_custom_cmd(cmd, source, &output_path)
    } else {
        invoke_realesrgan(source, &output_path, scale)
    };

    match result {
        Ok(()) => {
            let elapsed = start.elapsed().as_secs_f32();
            // Read actual output dimensions.
            let (rw, rh) =
                get_image_dimensions(&output_path).unwrap_or((target_w_scaled, target_h_scaled));
            eprintln!("Upscaling complete in {elapsed:.1}s ({rw}x{rh})");

            // Insert into cache.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let entry = UpscaleCacheEntry {
                source_path: canonical_str,
                source_mtime_secs: mtime,
                source_size: size,
                scale_factor: scale,
                cached_filename: filename,
                created_at: now,
            };
            cache.insert(entry, &cache_dir);

            output_path.to_string_lossy().to_string()
        }
        Err(e) => {
            eprintln!("Warning: {e}, using original image");
            path.to_string()
        }
    }
}
