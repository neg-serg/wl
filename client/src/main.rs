mod cli;
mod daemon;
mod ipc;
mod random;
mod upscale;

use clap::Parser;

use swww_vulkan_common::cache::{UpscalePrefs, load_upscale_prefs, save_upscale_prefs};
use swww_vulkan_common::ipc_types::*;

use crate::cli::*;
use crate::ipc::{IpcClient, IpcError};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Init => {
            daemon::init().await?;
            Ok(())
        }
        Commands::Kill => {
            daemon::kill().await?;
            Ok(())
        }
        Commands::Img {
            path,
            outputs,
            resize,
            transition_type,
            transition_duration,
            transition_step,
            transition_fps,
            transition_angle,
            transition_pos,
            transition_bezier,
            transition_wave,
            upscale,
            upscale_cmd,
            upscale_scale,
        } => {
            let position = parse_position(&transition_pos)?;
            let bezier = parse_bezier(&transition_bezier)?;
            let wave = parse_wave(&transition_wave)?;
            let parsed_outputs = parse_outputs(&outputs);

            // Resolve upscale mode from CLI flag + persistent prefs.
            let prefs = load_upscale_prefs();
            let (should_upscale, effective_cmd, effective_scale) =
                resolve_upscale(&upscale, &upscale_cmd, &upscale_scale, &prefs);

            let final_path = if should_upscale {
                upscale::upscale_image(&path, &effective_cmd, &effective_scale, &parsed_outputs)
                    .await
            } else {
                path
            };

            let cmd = IpcCommand::Img {
                path: final_path,
                outputs: parsed_outputs,
                resize: resize.into(),
                transition: TransitionParams {
                    transition_type: transition_type.into(),
                    duration_secs: transition_duration,
                    step: transition_step,
                    fps: transition_fps,
                    angle: transition_angle,
                    position,
                    bezier,
                    wave,
                },
            };

            send_and_check(cmd).await
        }
        Commands::Clear { color, outputs } => {
            let rgb = parse_color(&color)?;
            let cmd = IpcCommand::Clear {
                outputs: parse_outputs(&outputs),
                color: rgb,
            };
            send_and_check(cmd).await
        }
        Commands::Query => {
            let mut client = connect_or_error().await?;
            let response = client
                .send_command(&IpcCommand::Query)
                .await
                .map_err(|e| format!("query failed: {e}"))?;

            match response {
                IpcResponse::QueryResult { outputs } => {
                    for info in outputs {
                        let path = info.wallpaper_path.as_deref().unwrap_or("(none)");
                        let dims = info
                            .dimensions
                            .map(|(w, h)| format!("({w}x{h})"))
                            .unwrap_or_default();
                        let state = match info.state {
                            OutputState::Idle => "[idle]".to_string(),
                            OutputState::Transitioning => "[transitioning]".to_string(),
                            OutputState::Playing { frame, total } => {
                                format!("[playing frame {frame}/{total}]")
                            }
                        };
                        println!("{}: {path} {dims} {state}", info.name);
                    }
                    Ok(())
                }
                IpcResponse::Error { message } => Err(message),
                _ => Err("unexpected response".to_string()),
            }
        }
        Commands::Restore => send_and_check(IpcCommand::Restore).await,
        Commands::Pause { outputs } => {
            send_and_check(IpcCommand::Pause {
                outputs: parse_outputs(&outputs),
            })
            .await
        }
        Commands::ClearCache => send_and_check(IpcCommand::ClearCache).await,
        Commands::Random {
            directories,
            outputs,
            resize,
            transition_type,
            transition_duration,
            transition_step,
            transition_fps,
            transition_angle,
            transition_pos,
            transition_bezier,
            transition_wave,
            upscale,
            upscale_cmd,
            upscale_scale,
            no_greeter_sync,
            greeter_path,
            no_notify,
            notify_path,
        } => {
            // Scan directories for image candidates.
            let candidates = random::scan_directories(&directories);
            if candidates.is_empty() {
                return Err("no image files found in specified directories".to_string());
            }

            // Pick a random wallpaper.
            let picked = random::pick_random(&candidates).to_path_buf();
            let path = picked.to_string_lossy().to_string();

            // Parse transition parameters.
            let position = parse_position(&transition_pos)?;
            let bezier = parse_bezier(&transition_bezier)?;
            let wave = parse_wave(&transition_wave)?;
            let parsed_outputs = parse_outputs(&outputs);

            // Resolve upscale mode.
            let prefs = load_upscale_prefs();
            let (should_upscale, effective_cmd, effective_scale) =
                resolve_upscale(&upscale, &upscale_cmd, &upscale_scale, &prefs);

            let final_path = if should_upscale {
                upscale::upscale_image(&path, &effective_cmd, &effective_scale, &parsed_outputs)
                    .await
            } else {
                path.clone()
            };

            // Ensure daemon is running.
            if connect_or_error().await.is_err() {
                daemon::init().await?;
            }

            // Send wallpaper command to daemon.
            let cmd = IpcCommand::Img {
                path: final_path,
                outputs: parsed_outputs,
                resize: resize.into(),
                transition: TransitionParams {
                    transition_type: transition_type.into(),
                    duration_secs: transition_duration,
                    step: transition_step,
                    fps: transition_fps,
                    angle: transition_angle,
                    position,
                    bezier,
                    wave,
                },
            };

            send_and_check(cmd).await?;

            // Run post-apply hooks.
            let expanded_greeter = expand_tilde(&greeter_path);
            let expanded_notify = expand_tilde(&notify_path);

            if !no_greeter_sync {
                random::greeter_sync(&picked, std::path::Path::new(&expanded_greeter));
            }
            if !no_notify {
                random::write_notify(&picked, std::path::Path::new(&expanded_notify));
            }

            // Print selected wallpaper path to stdout.
            println!("{}", picked.display());
            Ok(())
        }
    }
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}

async fn connect_or_error() -> Result<IpcClient, String> {
    IpcClient::connect().await.map_err(|e| match e {
        IpcError::DaemonNotRunning => {
            "daemon is not running. Start it with 'swww-vulkan init'.".to_string()
        }
        other => format!("failed to connect: {other}"),
    })
}

async fn send_and_check(cmd: IpcCommand) -> Result<(), String> {
    let mut client = connect_or_error().await?;
    let response = client
        .send_command(&cmd)
        .await
        .map_err(|e| format!("command failed: {e}"))?;

    match response {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message } => Err(message),
        _ => Err("unexpected response from daemon".to_string()),
    }
}

/// Resolve whether to upscale and with what parameters, based on CLI flags and persistent prefs.
/// Returns (should_upscale, effective_cmd, effective_scale).
fn resolve_upscale(
    mode: &Option<UpscaleMode>,
    cli_cmd: &Option<String>,
    cli_scale: &Option<u8>,
    prefs: &UpscalePrefs,
) -> (bool, Option<String>, Option<u8>) {
    // If --upscale-cmd or --upscale-scale provided without --upscale, treat as "once".
    let effective_mode = if mode.is_none() && (cli_cmd.is_some() || cli_scale.is_some()) {
        Some(UpscaleMode::Once)
    } else {
        mode.clone()
    };

    match effective_mode {
        Some(UpscaleMode::Always) => {
            // Save prefs with current CLI params (full replace).
            let new_prefs = UpscalePrefs {
                enabled: true,
                custom_cmd: cli_cmd.clone(),
                scale: *cli_scale,
            };
            save_upscale_prefs(&new_prefs);
            (true, cli_cmd.clone(), *cli_scale)
        }
        Some(UpscaleMode::Off) => {
            // Disable persistent mode.
            let new_prefs = UpscalePrefs {
                enabled: false,
                custom_cmd: None,
                scale: None,
            };
            save_upscale_prefs(&new_prefs);
            (false, None, None)
        }
        Some(UpscaleMode::Once) => {
            // Upscale this image only, don't change prefs.
            (true, cli_cmd.clone(), *cli_scale)
        }
        Some(UpscaleMode::Never) => {
            // Skip upscaling, don't change prefs.
            (false, None, None)
        }
        None => {
            // No --upscale flag: use persistent prefs.
            if prefs.enabled {
                (true, prefs.custom_cmd.clone(), prefs.scale)
            } else {
                (false, None, None)
            }
        }
    }
}
