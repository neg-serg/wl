mod cli;
mod daemon;
mod ipc;
mod random;
mod upscale;

use clap::Parser;

use wl_common::ipc_types::*;

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

/// Resolve whether upscaling should happen based on CLI flag and persistent prefs.
/// Returns (should_upscale, effective_cmd, effective_scale).
fn resolve_upscale(
    mode: &Option<UpscaleMode>,
    cmd: &Option<String>,
    scale: &Option<u8>,
    prefs: &mut wl_common::cache::UpscalePrefs,
) -> (bool, Option<String>, Option<u8>) {
    match mode {
        Some(UpscaleMode::Never) => (false, None, None),
        Some(UpscaleMode::Off) => {
            prefs.enabled = false;
            prefs.custom_cmd = None;
            prefs.scale = None;
            if let Err(e) = wl_common::cache::save_upscale_prefs(prefs) {
                eprintln!("Warning: failed to save upscale prefs: {e}");
            }
            eprintln!("Upscale mode: off (saved)");
            (false, None, None)
        }
        Some(UpscaleMode::Always) => {
            prefs.enabled = true;
            prefs.custom_cmd = cmd.clone();
            prefs.scale = *scale;
            if let Err(e) = wl_common::cache::save_upscale_prefs(prefs) {
                eprintln!("Warning: failed to save upscale prefs: {e}");
            }
            eprintln!("Upscale mode: always (saved)");
            (true, cmd.clone(), *scale)
        }
        Some(UpscaleMode::Once) => (true, cmd.clone(), *scale),
        None => {
            // No flag: check persistent prefs
            if prefs.enabled {
                let eff_cmd = cmd.clone().or_else(|| prefs.custom_cmd.clone());
                let eff_scale = scale.or(prefs.scale);
                (true, eff_cmd, eff_scale)
            } else {
                (false, None, None)
            }
        }
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
            upscale: upscale_mode,
            upscale_cmd,
            upscale_scale,
            transition_type,
            transition_duration,
            transition_step,
            transition_fps,
            transition_angle,
            transition_pos,
            transition_bezier,
            transition_wave,
            no_notify,
            notify_path,
        } => {
            let position = parse_position(&transition_pos)?;
            let bezier = parse_bezier(&transition_bezier)?;
            let wave = parse_wave(&transition_wave)?;
            let parsed_outputs = parse_outputs(&outputs);

            let expanded_notify = expand_tilde(&notify_path);
            let notify_path_buf = std::path::PathBuf::from(&path);

            // Resolve upscale
            let mut prefs = wl_common::cache::load_upscale_prefs();
            let (should_upscale, eff_cmd, eff_scale) =
                resolve_upscale(&upscale_mode, &upscale_cmd, &upscale_scale, &mut prefs);

            let final_path = if should_upscale {
                upscale::upscale_image(&path, &eff_cmd, &eff_scale, &parsed_outputs).await
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

            send_and_check(cmd).await?;

            if !no_notify {
                random::write_notify(&notify_path_buf, std::path::Path::new(&expanded_notify));
            }

            Ok(())
        }
        Commands::Clear { color, outputs } => {
            let rgb = parse_color(&color)?;
            let cmd = IpcCommand::Clear {
                outputs: parse_outputs(&outputs),
                color: rgb,
            };
            send_and_check(cmd).await
        }
        Commands::Query | Commands::List => {
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
        Commands::Rotate { action } => handle_rotate(action).await,
        Commands::Random {
            directories,
            outputs,
            resize,
            upscale: upscale_mode,
            upscale_cmd,
            upscale_scale,
            transition_type,
            transition_duration,
            transition_step,
            transition_fps,
            transition_angle,
            transition_pos,
            transition_bezier,
            transition_wave,
            no_greeter_sync,
            greeter_path,
            no_notify,
            notify_path,
        } => {
            let candidates = random::scan_directories(&directories);
            if candidates.is_empty() {
                return Err("no image files found in specified directories".to_string());
            }

            let picked = random::pick_random(&candidates).to_path_buf();
            let path = picked.to_string_lossy().to_string();

            let position = parse_position(&transition_pos)?;
            let bezier = parse_bezier(&transition_bezier)?;
            let wave = parse_wave(&transition_wave)?;
            let parsed_outputs = parse_outputs(&outputs);

            // Ensure daemon is running.
            if connect_or_error().await.is_err() {
                daemon::init().await?;
            }

            // Resolve upscale
            let mut prefs = wl_common::cache::load_upscale_prefs();
            let (should_upscale, eff_cmd, eff_scale) =
                resolve_upscale(&upscale_mode, &upscale_cmd, &upscale_scale, &mut prefs);

            let final_path = if should_upscale {
                upscale::upscale_image(&path, &eff_cmd, &eff_scale, &parsed_outputs).await
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

            send_and_check(cmd).await?;

            let expanded_greeter = expand_tilde(&greeter_path);
            let expanded_notify = expand_tilde(&notify_path);

            if !no_greeter_sync {
                random::greeter_sync(&picked, std::path::Path::new(&expanded_greeter));
            }
            if !no_notify {
                random::write_notify(&picked, std::path::Path::new(&expanded_notify));
            }

            println!("{}", picked.display());
            Ok(())
        }
    }
}

async fn handle_rotate(action: cli::RotateAction) -> Result<(), String> {
    match action {
        cli::RotateAction::Start {
            directories,
            interval,
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
            no_notify,
            notify_path,
        } => {
            // Parse interval
            let duration = wl_common::duration_parse::parse_duration(&interval).map_err(|e| {
                format!(
                    "{e}\n\nExpected formats: \"30m\", \"1h30m\", \"2h\", \"45s\", \"1d\", or a plain number (minutes)"
                )
            })?;

            // Validate directories exist
            for dir in &directories {
                if !dir.is_dir() {
                    return Err(format!("'{}' is not a directory", dir.display()));
                }
            }

            let position = parse_position(&transition_pos)?;
            let bezier = parse_bezier(&transition_bezier)?;
            let wave = parse_wave(&transition_wave)?;

            let upscale_mode_str = upscale.as_ref().map(|m| match m {
                UpscaleMode::Once => "once".to_string(),
                UpscaleMode::Always => "always".to_string(),
                UpscaleMode::Never => "never".to_string(),
                UpscaleMode::Off => "off".to_string(),
            });

            let expanded_notify = expand_tilde(&notify_path);

            let cmd = IpcCommand::RotateStart {
                directories,
                interval_secs: duration.as_secs(),
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
                upscale_mode: upscale_mode_str,
                upscale_cmd,
                upscale_scale,
                no_notify,
                notify_path: expanded_notify,
            };

            send_and_check(cmd).await?;
            let interval_str = wl_common::duration_parse::format_duration(duration.as_secs());
            eprintln!("Rotation started (interval: {interval_str})");
            Ok(())
        }
        cli::RotateAction::Stop => {
            send_and_check(IpcCommand::RotateStop).await?;
            eprintln!("Rotation stopped");
            Ok(())
        }
        cli::RotateAction::Status => {
            let mut client = connect_or_error().await?;
            let response = client
                .send_command(&IpcCommand::RotateStatus)
                .await
                .map_err(|e| format!("query failed: {e}"))?;

            match response {
                IpcResponse::RotationStatus {
                    active,
                    interval_secs,
                    directories,
                    next_change_secs,
                    images_total,
                    images_remaining,
                } => {
                    if active {
                        let interval_str = interval_secs
                            .map(wl_common::duration_parse::format_duration)
                            .unwrap_or_default();
                        let dirs_str = directories
                            .as_ref()
                            .map(|d| d.join(", "))
                            .unwrap_or_default();
                        let next_str = next_change_secs
                            .map(wl_common::duration_parse::format_duration)
                            .unwrap_or_default();
                        let total = images_total.unwrap_or(0);
                        let remaining = images_remaining.unwrap_or(0);
                        let shown = total - remaining;

                        println!("Rotation: active");
                        println!("Interval: {interval_str}");
                        println!("Directories: {dirs_str}");
                        println!("Next change: {next_str}");
                        println!("Progress: {shown}/{total} images (cycle)");
                    } else {
                        println!("Rotation: inactive");
                    }
                    Ok(())
                }
                IpcResponse::Error { message } => Err(message),
                _ => Err("unexpected response".to_string()),
            }
        }
        cli::RotateAction::Next => {
            send_and_check(IpcCommand::RotateNext).await?;
            eprintln!("Skipped to next wallpaper");
            Ok(())
        }
    }
}

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
        IpcError::DaemonNotRunning => "daemon is not running. Start it with 'wl init'.".to_string(),
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
