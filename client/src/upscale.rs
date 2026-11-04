//! CLI-side upscaling: resolves the target resolution with the daemon, then
//! runs the shared upscaler from `wl_common::upscale` on a blocking thread.

use std::path::Path;

use wl_common::ipc_types::*;
use wl_common::upscale::{UpscaleOptions, UpscaleOutcome, compute_scale_factor};

use crate::ipc::IpcClient;

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
                if let Some((w, h)) = info.physical_resolution
                    && w * h > max_w * max_h
                {
                    max_w = w;
                    max_h = h;
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

/// Upscale `path` for the requested outputs, returning the path to display.
///
/// Failures are non-fatal: the original image is used instead.
pub async fn upscale_image(
    path: &str,
    upscale_cmd: &Option<String>,
    forced_scale: &Option<u8>,
    outputs: &Option<Vec<String>>,
) -> String {
    match upscale_image_inner(path, upscale_cmd, forced_scale, outputs).await {
        Ok(result) => result,
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
    let source = Path::new(path).to_path_buf();
    let filename = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();

    // Without a forced factor the scale depends on the output resolution.
    let target = if forced_scale.is_some() {
        (0, 0)
    } else {
        query_max_physical_resolution(outputs).await?
    };

    let cmd = upscale_cmd.clone();
    let forced = *forced_scale;
    let (src_w, src_h) = wl_common::upscale::get_image_dimensions(&source)?;

    if forced.is_none() && compute_scale_factor(src_w, src_h, target.0, target.1).is_none() {
        eprintln!("Skipping upscale: image already meets target resolution ({src_w}x{src_h})");
        return Ok(path.to_string());
    }

    let outcome = tokio::task::spawn_blocking(move || {
        wl_common::upscale::upscale_source(
            &source,
            &UpscaleOptions {
                custom_cmd: cmd.as_deref(),
                forced_scale: forced,
                target,
            },
        )
    })
    .await
    .map_err(|e| format!("upscale worker failed: {e}"))??;

    match outcome {
        UpscaleOutcome::Skipped { reason } => {
            eprintln!("Skipping upscale: {reason}");
            Ok(path.to_string())
        }
        UpscaleOutcome::Upscaled {
            path: out_path,
            cached,
            elapsed,
            result_dimensions,
            scale,
        } => {
            if cached {
                eprintln!("Using cached upscale: {filename}");
            } else {
                eprintln!(
                    "Upscaled {filename} in {:.1}s ({}x, {}x{})",
                    elapsed.as_secs_f32(),
                    scale,
                    result_dimensions.0,
                    result_dimensions.1
                );
            }
            Ok(out_path.to_string_lossy().to_string())
        }
    }
}
