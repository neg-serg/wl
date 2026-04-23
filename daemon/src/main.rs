mod animation;
mod ipc;
mod output;
mod render;
mod rotation;
mod state;
mod transition;
mod vulkan;
mod wayland;

use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use nix::sys::socket::{self as nix_socket, AddressFamily, SockFlag, SockType};
use tracing::{debug, error, info, warn};

use wl_common::ipc_types::*;

use crate::ipc::IpcServer;
use crate::output::{Output, Wallpaper};
use crate::state::DaemonState;
use crate::vulkan::VulkanContext;
use crate::vulkan::pipeline::{TransitionKind, TransitionPipeline, WallpaperPipeline};
use crate::vulkan::shaders::ShaderModules;
use crate::vulkan::swapchain::Swapchain;
use crate::vulkan::texture;
use crate::wayland::WaylandState;

/// Check that no other instance of wl-daemon is running using an abstract
/// Unix domain socket lock. This prevents duplicate daemon processes.
fn check_single_instance() -> Result<(), String> {
    let fd = nix_socket::socket(
        AddressFamily::Unix,
        SockType::Datagram,
        SockFlag::empty(),
        None,
    )
    .map_err(|e| format!("failed to create lock socket: {e}"))?;

    let addr =
        nix_socket::UnixAddr::new_abstract(b"wl-daemon").map_err(|e| format!("addr: {e}"))?;

    match nix_socket::bind(fd.as_raw_fd(), &addr) {
        Ok(()) => {
            std::mem::forget(fd);
            Ok(())
        }
        Err(_) => Err("wl-daemon уже запущен".into()),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(e) = check_single_instance() {
        eprintln!("{e}");
        std::process::exit(1);
    }

    if let Err(e) = run().await {
        error!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    info!("wl-daemon starting");

    // 1. Connect to Wayland
    let mut wl = WaylandState::connect().map_err(|e| format!("wayland: {e}"))?;
    info!(outputs = wl.outputs().len(), "connected to wayland");

    // 2. Initialize Vulkan
    let display_ptr = wl.get_display_ptr();
    // SAFETY: display_ptr is a valid wl_display pointer from the active Wayland connection.
    let vk = unsafe { VulkanContext::new(display_ptr).map_err(|e| format!("vulkan: {e}"))? };
    info!("vulkan initialized");

    // 3. Load shader modules
    // SAFETY: vk.device is a valid Vulkan device.
    let shaders =
        unsafe { ShaderModules::load_builtins(&vk.device).map_err(|e| format!("shaders: {e}"))? };
    info!("shaders loaded");

    // 4. Create layer-shell surfaces on all outputs
    wl.create_all_layer_surfaces()
        .map_err(|e| format!("layer surfaces: {e}"))?;
    wl.roundtrip().map_err(|e| format!("roundtrip: {e}"))?;

    // 5. Create per-output state with Vulkan surfaces and swapchains
    let mut outputs = HashMap::new();
    let mut swapchain_format = None;

    for (i, wl_output) in wl.outputs().iter().enumerate() {
        let name = wl_output
            .name
            .clone()
            .unwrap_or_else(|| format!("output-{i}"));

        // SAFETY: vk.device is valid.
        let mut output = unsafe {
            Output::new(
                &vk.device,
                name.clone(),
                wl_output.width,
                wl_output.height,
                wl_output.scale_factor,
                wl_output.refresh_mhz,
            )
            .map_err(|e| format!("output sync objects: {e}"))?
        };

        if let Some(surface_ptr) = wl.get_surface_ptr(i) {
            // SAFETY: display_ptr and surface_ptr are valid Wayland pointers.
            let vk_surface = unsafe {
                Swapchain::create_surface(
                    &vk.entry,
                    &vk.instance,
                    display_ptr,
                    surface_ptr,
                    &vk.wayland_surface_fn,
                )
                .map_err(|e| format!("vk surface: {e}"))?
            };

            // Use effective resolution (logical × scale) for the swapchain so
            // the buffer matches the physical output pixels. wp_viewport maps
            // the physical buffer back to the logical surface area.
            let eff_w = (wl_output.width as f64 * wl_output.scale_factor).round() as u32;
            let eff_h = (wl_output.height as f64 * wl_output.scale_factor).round() as u32;
            let swapchain = Swapchain::new(
                &vk.instance,
                &vk.device,
                vk.physical_device,
                vk_surface,
                eff_w.max(1),
                eff_h.max(1),
            )
            .map_err(|e| format!("swapchain: {e}"))?;

            swapchain_format = Some(swapchain.format.format);
            output.swapchain = Some(swapchain);
            info!(name = %output.name, "output initialized with swapchain");
        }

        outputs.insert(name, output);
    }

    // 6. Create wallpaper pipeline (needs swapchain format)
    let pipeline = if let Some(format) = swapchain_format {
        let vert = shaders
            .get("wallpaper.vert")
            .expect("wallpaper.vert missing");
        let frag = shaders
            .get("wallpaper.frag")
            .expect("wallpaper.frag missing");
        let p = WallpaperPipeline::new(&vk.device, format, vert, frag)
            .map_err(|e| format!("pipeline: {e}"))?;

        // Create framebuffers for each output's swapchain images
        for output in outputs.values_mut() {
            if let Some(ref sc) = output.swapchain {
                for &view in &sc.image_views {
                    let fb = WallpaperPipeline::create_framebuffer(
                        &vk.device,
                        p.render_pass,
                        view,
                        sc.extent.width,
                        sc.extent.height,
                    )
                    .map_err(|e| format!("framebuffer: {e}"))?;
                    output.framebuffers.push(fb);
                }
            }
        }

        info!("wallpaper pipeline created");
        Some(p)
    } else {
        warn!("no swapchain format available, pipeline deferred");
        None
    };

    // 6b. Create transition pipelines
    let transition_pipeline = if let Some(format) = swapchain_format {
        let vert = shaders
            .get("wallpaper.vert")
            .expect("wallpaper.vert missing");
        let frag_modules: Vec<(TransitionKind, _)> = [
            (TransitionKind::Wipe, "transition_wipe.frag"),
            (TransitionKind::Wave, "transition_wave.frag"),
            (TransitionKind::Outer, "transition_outer.frag"),
            (TransitionKind::Pixelate, "transition_pixelate.frag"),
            (TransitionKind::Burn, "transition_burn.frag"),
            (TransitionKind::Glitch, "transition_glitch.frag"),
            (TransitionKind::Disintegrate, "transition_disintegrate.frag"),
            (TransitionKind::Dreamy, "transition_dreamy.frag"),
            (
                TransitionKind::GlitchMemories,
                "transition_glitch_memories.frag",
            ),
            (TransitionKind::Morph, "transition_morph.frag"),
            (TransitionKind::Hexagonalize, "transition_hexagonalize.frag"),
            (TransitionKind::CrossZoom, "transition_cross_zoom.frag"),
            (
                TransitionKind::FluidDistortion,
                "transition_fluid_distortion.frag",
            ),
            (TransitionKind::FluidDrain, "transition_fluid_drain.frag"),
            (TransitionKind::FluidRipple, "transition_fluid_ripple.frag"),
            (TransitionKind::FluidVortex, "transition_fluid_vortex.frag"),
            (TransitionKind::FluidWave, "transition_fluid_wave.frag"),
            (TransitionKind::InkBleed, "transition_ink_bleed.frag"),
            (TransitionKind::LavaLamp, "transition_lava_lamp.frag"),
            (
                TransitionKind::ChromaticAberration,
                "transition_chromatic_aberration.frag",
            ),
            (
                TransitionKind::LensDistortion,
                "transition_lens_distortion.frag",
            ),
            (TransitionKind::CrtShutdown, "transition_crt_shutdown.frag"),
            (TransitionKind::PerlinWipe, "transition_perlin_wipe.frag"),
            (TransitionKind::RadialBlur, "transition_radial_blur.frag"),
        ]
        .into_iter()
        .filter_map(|(kind, name)| shaders.get(name).map(|m| (kind, m)))
        .collect();

        if frag_modules.is_empty() {
            warn!("no transition shaders available");
            None
        } else {
            match TransitionPipeline::new(&vk.device, format, vert, &frag_modules) {
                Ok(tp) => {
                    info!("transition pipelines created");
                    Some(tp)
                }
                Err(e) => {
                    warn!("failed to create transition pipelines: {e}");
                    None
                }
            }
        }
    } else {
        None
    };

    // 7. Bind IPC socket
    let ipc = IpcServer::bind().await.map_err(|e| format!("ipc: {e}"))?;
    info!("IPC server listening");

    // 8. Build daemon state
    let mut daemon = DaemonState {
        vk,
        shaders,
        pipeline,
        transition_pipeline,
        outputs,
        session_cache_path: wl_common::cache::state_dir(),
        image_cache_path: wl_common::cache::cache_dir(),
        running: true,
        rotation: None,
    };

    // 9. Initial render: solid black on all outputs
    for output in daemon.outputs.values_mut() {
        if output.swapchain.is_some() {
            // SAFETY: All Vulkan handles are valid.
            if let Err(e) = unsafe {
                render::render_frame(
                    &daemon.vk,
                    output,
                    daemon.pipeline.as_ref(),
                    daemon.transition_pipeline.as_ref(),
                )
            } {
                warn!(output = %output.name, "initial render failed: {e}");
            }
        }
    }

    // 9b. Restore rotation state from disk if available
    if let Some(persist) = wl_common::cache::load_rotation_state() {
        let mut rot = rotation::RotationState::from_persist(&persist);
        // Set timer: fire immediately if we've been down longer than the interval,
        // otherwise resume with remaining time
        rot.reset_timer();
        info!(
            interval_secs = rot.interval.as_secs(),
            images = rot.candidates.len(),
            index = rot.current_index,
            "rotation restored from disk"
        );
        daemon.rotation = Some(rot);
    }

    info!("daemon ready");

    // 10. Main event loop
    loop {
        if !daemon.running {
            break;
        }

        tokio::select! {
            result = ipc.accept_command() => {
                match result {
                    Ok((cmd, mut stream)) => {
                        let response = handle_command(&mut daemon, cmd);
                        if let Err(e) = ipc::send_response(&mut stream, &response).await {
                            warn!("failed to send IPC response: {e}");
                        }
                    }
                    Err(ipc::IpcError::Io(ref e))
                        if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                    {
                        debug!("IPC probe connection (client disconnected without sending a command)");
                    }
                    Err(e) => {
                        warn!("IPC accept error: {e}");
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(16)) => {}
        }

        // Check if rotation timer has elapsed
        if daemon.rotation.is_some() {
            let elapsed = daemon
                .rotation
                .as_ref()
                .map(|r| r.time_until_next().is_zero())
                .unwrap_or(false);
            if elapsed {
                tick_rotation(&mut daemon);
            }
        }

        if let Err(e) = wl.dispatch_pending() {
            error!("wayland dispatch error: {e}");
            break;
        }
        if let Err(e) = wl.flush() {
            error!("wayland flush error: {e}");
            break;
        }

        // Recover outputs whose layer surface was closed by the compositor.
        // This destroys stale Vulkan resources, recreates the Wayland surface,
        // and rebuilds the swapchain + framebuffers so rendering can resume.
        let lost = wl.lost_surface_indices();
        if !lost.is_empty() {
            // SAFETY: GPU must be idle before destroying swapchain resources.
            unsafe { let _ = daemon.vk.device.device_wait_idle(); }

            for idx in &lost {
                let output_name = wl.outputs()[*idx]
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("output-{idx}"));

                if let Some(output) = daemon.outputs.get_mut(&output_name) {
                    warn!(output = %output_name, "recovering lost surface");

                    // 1. Tear down stale Vulkan resources for this output.
                    // SAFETY: GPU is idle (waited above).
                    unsafe {
                        // Free command buffer if pending
                        if let Some(old_cmd) = output.last_command_buffer.take() {
                            daemon.vk.device.free_command_buffers(daemon.vk.command_pool, &[old_cmd]);
                        }
                        // Destroy framebuffers
                        for fb in output.framebuffers.drain(..) {
                            daemon.vk.device.destroy_framebuffer(fb, None);
                        }
                        // Destroy swapchain (includes VkSurfaceKHR)
                        if let Some(mut sc) = output.swapchain.take() {
                            sc.destroy(&daemon.vk.device);
                        }
                    }
                }

                // 2. Recreate the Wayland layer surface.
                if let Err(e) = wl.create_layer_surface(*idx) {
                    error!(output_index = idx, "failed to recreate layer surface: {e}");
                    continue;
                }
            }

            // Wait for compositor to send Configure events for new surfaces.
            if let Err(e) = wl.roundtrip() {
                error!("roundtrip after surface recreation failed: {e}");
            }

            // 3. Rebuild Vulkan surface + swapchain + framebuffers.
            let display_ptr = wl.get_display_ptr();
            for idx in &lost {
                let wl_output = &wl.outputs()[*idx];
                let output_name = wl_output
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("output-{idx}"));

                if !wl_output.configured {
                    warn!(output = %output_name, "surface not configured after recreation, skipping");
                    continue;
                }

                let Some(surface_ptr) = wl.get_surface_ptr(*idx) else {
                    warn!(output = %output_name, "no surface pointer after recreation");
                    continue;
                };

                let output = match daemon.outputs.get_mut(&output_name) {
                    Some(o) => o,
                    None => continue,
                };

                // Update output dimensions from refreshed Wayland state.
                output.width = wl_output.width;
                output.height = wl_output.height;
                output.scale_factor = wl_output.scale_factor;

                // SAFETY: display_ptr and surface_ptr are valid Wayland pointers.
                let vk_surface = match unsafe {
                    Swapchain::create_surface(
                        &daemon.vk.entry,
                        &daemon.vk.instance,
                        display_ptr,
                        surface_ptr,
                        &daemon.vk.wayland_surface_fn,
                    )
                } {
                    Ok(s) => s,
                    Err(e) => {
                        error!(output = %output_name, "failed to create Vulkan surface: {e}");
                        continue;
                    }
                };

                let (eff_w, eff_h) = output.effective_resolution();
                let swapchain = match Swapchain::new(
                    &daemon.vk.instance,
                    &daemon.vk.device,
                    daemon.vk.physical_device,
                    vk_surface,
                    eff_w.max(1),
                    eff_h.max(1),
                ) {
                    Ok(sc) => sc,
                    Err(e) => {
                        error!(output = %output_name, "failed to create swapchain: {e}");
                        continue;
                    }
                };

                // Rebuild framebuffers for the new swapchain.
                if let Some(ref pipeline) = daemon.pipeline {
                    for &view in &swapchain.image_views {
                        match WallpaperPipeline::create_framebuffer(
                            &daemon.vk.device,
                            pipeline.render_pass,
                            view,
                            swapchain.extent.width,
                            swapchain.extent.height,
                        ) {
                            Ok(fb) => output.framebuffers.push(fb),
                            Err(e) => {
                                error!(output = %output_name, "failed to create framebuffer: {e}");
                            }
                        }
                    }
                }

                output.swapchain = Some(swapchain);
                output.needs_redraw = true;

                // Re-bind wallpaper descriptor set to ensure texture is displayed.
                if let (Some(ds), Some(pipeline), Some(wp)) =
                    (output.descriptor_set, &daemon.pipeline, &output.wallpaper)
                {
                    WallpaperPipeline::update_descriptor_set(
                        &daemon.vk.device,
                        ds,
                        wp.texture.view,
                        pipeline.sampler,
                    );
                }

                info!(output = %output_name, "surface recovered successfully");
            }
        }

        // Tick active transitions and animations
        tick_transitions(&mut daemon);
        tick_animations(&mut daemon);

        let mut device_lost = false;
        for output in daemon.outputs.values_mut() {
            debug!(output = %output.name, needs_redraw = output.needs_redraw, has_swapchain = output.swapchain.is_some(), has_wallpaper = output.wallpaper.is_some(), has_descriptor = output.descriptor_set.is_some(), framebuffers = output.framebuffers.len(), "render loop tick");
            if output.needs_redraw && output.swapchain.is_some() {
                // SAFETY: All Vulkan handles are valid.
                if let Err(e) = unsafe {
                    render::render_frame(
                        &daemon.vk,
                        output,
                        daemon.pipeline.as_ref(),
                        daemon.transition_pipeline.as_ref(),
                    )
                } {
                    if matches!(
                        e,
                        render::RenderError::Vulkan(ash::vk::Result::ERROR_DEVICE_LOST)
                    ) {
                        error!("Vulkan device lost, shutting down");
                        device_lost = true;
                        break;
                    }
                    if matches!(
                        e,
                        render::RenderError::Vulkan(ash::vk::Result::ERROR_SURFACE_LOST_KHR)
                    ) {
                        // Surface was invalidated under us. Drop the swapchain so
                        // the Closed-event recovery path can rebuild it next tick.
                        warn!(output = %output.name, "Vulkan surface lost, dropping swapchain");
                        unsafe {
                            if let Some(old_cmd) = output.last_command_buffer.take() {
                                daemon.vk.device.free_command_buffers(daemon.vk.command_pool, &[old_cmd]);
                            }
                            for fb in output.framebuffers.drain(..) {
                                daemon.vk.device.destroy_framebuffer(fb, None);
                            }
                            if let Some(mut sc) = output.swapchain.take() {
                                sc.destroy(&daemon.vk.device);
                            }
                        }
                        continue;
                    }
                    warn!(output = %output.name, "render error: {e}");
                }
            }
        }
        if device_lost {
            break;
        }
    }

    info!("shutting down");
    // SAFETY: Shutting down, destroy_all waits for GPU idle.
    unsafe {
        daemon.destroy_all();
    }
    info!("daemon stopped");

    Ok(())
}

/// Tick all active transitions, completing them when done.
fn tick_transitions(daemon: &mut DaemonState) {
    let names: Vec<String> = daemon
        .outputs
        .iter()
        .filter(|(_, o)| o.transition.is_some())
        .map(|(n, _)| n.clone())
        .collect();

    for name in names {
        let output = daemon.outputs.get_mut(&name).unwrap();
        let completed = if let Some(ref mut t) = output.transition {
            let done = transition::tick(t);
            output.needs_redraw = true;
            done
        } else {
            false
        };

        if completed {
            complete_transition(daemon, &name);
        }
    }
}

/// Complete a transition: free old texture, wallpaper already has the new texture handles.
fn complete_transition(daemon: &mut DaemonState, output_name: &str) {
    let output = match daemon.outputs.get_mut(output_name) {
        Some(o) => o,
        None => return,
    };

    let t = match output.transition.take() {
        Some(t) => t,
        None => return,
    };

    // SAFETY: GPU idle ensured by fence wait in render loop.
    unsafe {
        let _ = daemon.vk.device.device_wait_idle();
        // Only destroy the old texture. The new texture's handles are shared
        // with output.wallpaper.texture (set during transition start).
        // GpuTexture has no Drop impl, so just not calling destroy() is safe.
        t.old_texture.destroy(&daemon.vk.device);
        // Free the transition descriptor set back to the pool.
        if let Some(ds) = t.descriptor_set
            && let Some(ref tp) = daemon.transition_pipeline
        {
            tp.free_descriptor_set(&daemon.vk.device, ds);
        }
    }

    // Update descriptor set to point to the wallpaper's texture (which has new_texture's handles)
    if let (Some(ds), Some(pipeline), Some(wp)) =
        (output.descriptor_set, &daemon.pipeline, &output.wallpaper)
    {
        WallpaperPipeline::update_descriptor_set(
            &daemon.vk.device,
            ds,
            wp.texture.view,
            pipeline.sampler,
        );
    }

    output.needs_redraw = true;
}

/// Tick all active animations, requesting redraw when frame changes.
fn tick_animations(daemon: &mut DaemonState) {
    for output in daemon.outputs.values_mut() {
        if let Some(ref mut anim) = output.animation
            && animation::tick(anim)
        {
            output.needs_redraw = true;
        }
    }
}

/// Tick rotation: advance to next wallpaper when the timer elapses.
fn tick_rotation(daemon: &mut DaemonState) {
    let (path, resize, transition) = {
        let rot = match daemon.rotation.as_mut() {
            Some(r) => r,
            None => return,
        };

        let path = match rot.next_image() {
            Some(p) => p,
            None => {
                warn!("rotation: no images available");
                rot.reset_timer();
                return;
            }
        };

        let resize = rot.resize;
        let transition = rot.transition;
        rot.reset_timer();
        rot.save();
        (path, resize, transition)
    };

    let path_str = path.to_string_lossy().to_string();
    let result = handle_img(daemon, &path_str, None, resize, &transition);
    if let IpcResponse::Error { ref message } = result {
        warn!("rotation: failed to set wallpaper: {message}");
    } else {
        info!(path = %path_str, "rotation: wallpaper changed");
    }
}

fn handle_command(daemon: &mut DaemonState, cmd: IpcCommand) -> IpcResponse {
    match cmd {
        IpcCommand::Kill => {
            info!("received kill command");
            daemon.running = false;
            IpcResponse::Ok
        }
        IpcCommand::Query => {
            let outputs = daemon
                .outputs
                .values()
                .map(|o| OutputInfo {
                    name: o.name.clone(),
                    wallpaper_path: o.wallpaper.as_ref().map(|w| w.source_path.clone()),
                    dimensions: o.wallpaper.as_ref().map(|w| w.display_dimensions),
                    state: if o.transition.is_some() {
                        OutputState::Transitioning
                    } else if let Some(ref anim) = o.animation {
                        OutputState::Playing {
                            frame: anim.current_frame,
                            total: anim.frame_count,
                        }
                    } else {
                        OutputState::Idle
                    },
                    physical_resolution: Some(o.effective_resolution()),
                })
                .collect();
            IpcResponse::QueryResult { outputs }
        }
        IpcCommand::Img {
            path,
            outputs: target_outputs,
            resize,
            transition,
        } => handle_img(daemon, &path, target_outputs, resize, &transition),
        IpcCommand::Clear {
            outputs: target_outputs,
            color,
        } => handle_clear(daemon, target_outputs, color),
        IpcCommand::Restore => handle_restore(daemon),
        IpcCommand::Pause {
            outputs: target_outputs,
        } => {
            let names = get_target_outputs(daemon, &target_outputs);
            for name in names {
                if let Some(output) = daemon.outputs.get_mut(&name)
                    && let Some(ref mut anim) = output.animation
                {
                    anim.paused = !anim.paused;
                    output.needs_redraw = true;
                }
            }
            IpcResponse::Ok
        }
        IpcCommand::ClearCache => match wl_common::cache::clear_cache() {
            Ok(()) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error {
                message: format!("clear-cache failed: {e}"),
            },
        },
        IpcCommand::RotateStart {
            directories,
            interval_secs,
            resize,
            transition,
            upscale_mode,
            upscale_cmd,
            upscale_scale,
        } => handle_rotate_start(
            daemon,
            rotation::RotateStartParams {
                directories,
                interval_secs,
                resize,
                transition,
                upscale_mode,
                upscale_cmd,
                upscale_scale,
            },
        ),
        IpcCommand::RotateStop => handle_rotate_stop(daemon),
        IpcCommand::RotateNext => handle_rotate_next(daemon),
        IpcCommand::RotateStatus => handle_rotate_status(daemon),
    }
}

fn handle_img(
    daemon: &mut DaemonState,
    path: &str,
    target_outputs: Option<Vec<String>>,
    resize: ResizeMode,
    transition_params: &TransitionParams,
) -> IpcResponse {
    // FR-011: Reset rotation timer when a manual wallpaper is set
    if let Some(ref mut rot) = daemon.rotation {
        rot.reset_timer();
        rot.save();
    }

    let img_path = Path::new(path);

    // Detect GIF for animation
    let is_gif = img_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gif"))
        .unwrap_or(false);

    if daemon.pipeline.is_none() {
        return IpcResponse::Error {
            message: "wallpaper pipeline not initialized".to_string(),
        };
    }

    let names = get_target_outputs(daemon, &target_outputs);
    let transition_kind = transition::resolve_kind(transition_params.transition_type);

    // SAFETY: Wait for GPU idle so old textures can be safely freed.
    unsafe {
        let _ = daemon.vk.device.device_wait_idle();
    }

    if is_gif {
        // GIF animation path
        let gif_frames = match wl_common::image_decode::decode_gif_frames(img_path) {
            Ok(f) => f,
            Err(e) => {
                return IpcResponse::Error {
                    message: format!("failed to decode GIF: {e}"),
                };
            }
        };

        if gif_frames.frames.len() <= 1 {
            // Single-frame GIF: treat as static image
            let data = if gif_frames.frames.is_empty() {
                return IpcResponse::Error {
                    message: "GIF has no frames".to_string(),
                };
            } else {
                &gif_frames.frames[0].data
            };
            // Pre-resize single-frame GIF like static images
            let original_w = gif_frames.width;
            let original_h = gif_frames.height;
            let decoded = wl_common::image_decode::DecodedImage {
                data: data.to_vec(),
                width: gif_frames.width,
                height: gif_frames.height,
            };
            let first_output = names.first().and_then(|n| daemon.outputs.get(n));
            let resized = if let Some(output) = first_output {
                let (eff_w, eff_h) = output.effective_resolution();
                wl_common::image_decode::resize_for_output(decoded, eff_w, eff_h, resize)
            } else {
                decoded
            };

            return set_static_wallpaper(
                daemon,
                StaticWallpaperParams {
                    path,
                    names: &names,
                    resize,
                    transition_params,
                    transition_kind,
                    data: &resized.data,
                    width: resized.width,
                    height: resized.height,
                    original_width: original_w,
                    original_height: original_h,
                    is_gif: true,
                },
            );
        }

        // Multi-frame GIF: pre-resize each frame, then create atlas
        let durations: Vec<u32> = gif_frames.frames.iter().map(|f| f.duration_ms).collect();

        // Pre-resize frames to target output resolution for pixel-perfect rendering
        let first_output = names.first().and_then(|n| daemon.outputs.get(n));
        let (resized_frames, frame_w, frame_h) = if let Some(output) = first_output {
            let (eff_w, eff_h) = output.effective_resolution();
            let mut resized = Vec::with_capacity(gif_frames.frames.len());
            let mut rw = gif_frames.width;
            let mut rh = gif_frames.height;
            for frame in &gif_frames.frames {
                let decoded = wl_common::image_decode::DecodedImage {
                    data: frame.data.clone(),
                    width: gif_frames.width,
                    height: gif_frames.height,
                };
                let r = wl_common::image_decode::resize_for_output(decoded, eff_w, eff_h, resize);
                rw = r.width;
                rh = r.height;
                resized.push(r.data);
            }
            (resized, rw, rh)
        } else {
            let frames: Vec<Vec<u8>> = gif_frames.frames.iter().map(|f| f.data.clone()).collect();
            (frames, gif_frames.width, gif_frames.height)
        };

        for name in &names {
            let (atlas_tex, _frame_offsets) =
                match texture::upload_gif_atlas(&daemon.vk, &resized_frames, frame_w, frame_h) {
                    Ok(result) => result,
                    Err(e) => {
                        warn!(output = %name, "failed to upload GIF atlas: {e}");
                        continue;
                    }
                };

            let pipeline = daemon.pipeline.as_ref().unwrap();

            if let Some(output) = daemon.outputs.get_mut(name) {
                // Allocate descriptor set for atlas (reuse existing if available)
                let ds = match output.descriptor_set {
                    Some(ds) => ds,
                    None => match pipeline.allocate_descriptor_set(&daemon.vk.device) {
                        Ok(ds) => ds,
                        Err(e) => {
                            warn!(output = %name, "failed to allocate descriptor set: {e}");
                            continue;
                        }
                    },
                };

                WallpaperPipeline::update_descriptor_set(
                    &daemon.vk.device,
                    ds,
                    atlas_tex.view,
                    pipeline.sampler,
                );

                output.descriptor_set = Some(ds);

                let anim_state = animation::create_animation(
                    gif_frames.frames.len() as u32,
                    durations.clone(),
                    atlas_tex,
                    frame_w,
                    frame_h,
                );

                // Create a wallpaper entry pointing to the atlas
                let wallpaper = Wallpaper {
                    source_path: path.to_string(),
                    format: ImageFormat::Gif,
                    original_dimensions: (gif_frames.width, gif_frames.height),
                    display_dimensions: (frame_w, frame_h),
                    resize_mode: resize,
                    // The atlas texture is owned by the animation state.
                    // Use a copy of the handles for the wallpaper.
                    texture: output::GpuTexture {
                        image: anim_state.atlas.image,
                        view: anim_state.atlas.view,
                        memory: anim_state.atlas.memory,
                        width: anim_state.atlas.width,
                        height: anim_state.atlas.height,
                    },
                    is_animated: true,
                };

                // SAFETY: GPU is idle.
                unsafe {
                    // Clear old wallpaper texture (if not animated - if animated, animation owns it)
                    if let Some(old_wp) = output.wallpaper.take()
                        && output.animation.is_none()
                    {
                        old_wp.texture.destroy(&daemon.vk.device);
                    }
                    if let Some(old_anim) = output.animation.take() {
                        old_anim.atlas.destroy(&daemon.vk.device);
                    }
                }

                output.wallpaper = Some(wallpaper);
                output.animation = Some(anim_state);
                output.needs_redraw = true;
            }
        }
    } else {
        // Static image path
        let decoded = match wl_common::image_decode::decode_to_rgba8(img_path) {
            Ok(img) => img,
            Err(e) => {
                return IpcResponse::Error {
                    message: format!("failed to decode image: {e}"),
                };
            }
        };

        // Pre-resize image to match each output's effective resolution for
        // pixel-perfect rendering. We resize per-output since monitors may
        // differ in resolution/scale. For multiple outputs we decode once
        // and resize per target.
        let original_w = decoded.width;
        let original_h = decoded.height;

        // For simplicity, resize to the first target output's effective resolution.
        // (Multi-output with different resolutions: we use the first target's dims,
        // which is correct for the common single-monitor case.)
        let first_output = names.first().and_then(|n| daemon.outputs.get(n));
        let resized = if let Some(output) = first_output {
            let (eff_w, eff_h) = output.effective_resolution();
            info!(
                img_w = original_w,
                img_h = original_h,
                eff_w = eff_w,
                eff_h = eff_h,
                scale = output.scale_factor,
                logical_w = output.width,
                logical_h = output.height,
                "pre-resize: image vs effective resolution"
            );
            wl_common::image_decode::resize_for_output(decoded, eff_w, eff_h, resize)
        } else {
            decoded
        };

        return set_static_wallpaper(
            daemon,
            StaticWallpaperParams {
                path,
                names: &names,
                resize,
                transition_params,
                transition_kind,
                data: &resized.data,
                width: resized.width,
                height: resized.height,
                original_width: original_w,
                original_height: original_h,
                is_gif: false,
            },
        );
    }

    if let Err(e) = daemon.save_session() {
        warn!("failed to save session state: {e}");
    }

    IpcResponse::Ok
}

struct StaticWallpaperParams<'a> {
    path: &'a str,
    names: &'a [String],
    resize: ResizeMode,
    transition_params: &'a TransitionParams,
    transition_kind: Option<TransitionKind>,
    data: &'a [u8],
    width: u32,
    height: u32,
    original_width: u32,
    original_height: u32,
    is_gif: bool,
}

fn set_static_wallpaper(
    daemon: &mut DaemonState,
    params: StaticWallpaperParams<'_>,
) -> IpcResponse {
    let StaticWallpaperParams {
        path,
        names,
        resize,
        transition_params,
        transition_kind,
        data,
        width,
        height,
        original_width,
        original_height,
        is_gif,
    } = params;
    for name in names {
        let gpu_tex = match texture::upload_rgba8_texture(&daemon.vk, data, width, height) {
            Ok(tex) => tex,
            Err(e) => {
                warn!(output = %name, "failed to upload texture: {e}");
                continue;
            }
        };

        let pipeline = daemon.pipeline.as_ref().unwrap();

        if let Some(output) = daemon.outputs.get_mut(name) {
            let should_transition = transition_kind.is_some()
                && output.wallpaper.is_some()
                && daemon.transition_pipeline.is_some();

            if should_transition {
                let kind = transition_kind.unwrap();

                // Cancel any existing transition: free its old texture and descriptor set.
                // The existing transition's new_texture handles are shared with
                // output.wallpaper.texture, so we must NOT destroy new_texture here.
                if let Some(old_transition) = output.transition.take() {
                    // SAFETY: GPU is idle (device_wait_idle called above).
                    unsafe {
                        old_transition.old_texture.destroy(&daemon.vk.device);
                        if let Some(ds) = old_transition.descriptor_set
                            && let Some(ref tp) = daemon.transition_pipeline
                        {
                            tp.free_descriptor_set(&daemon.vk.device, ds);
                        }
                    }
                }

                // Steal old wallpaper texture handles for the transition
                let old_wp = output.wallpaper.as_ref().unwrap();
                let old_resize_mode = old_wp.resize_mode;
                let old_texture = output::GpuTexture {
                    image: old_wp.texture.image,
                    view: old_wp.texture.view,
                    memory: old_wp.texture.memory,
                    width: old_wp.texture.width,
                    height: old_wp.texture.height,
                };

                let mut t = transition::create_transition(
                    transition_params,
                    kind,
                    old_texture,
                    old_resize_mode,
                    gpu_tex,
                    resize,
                );

                if let Some(ref tp) = daemon.transition_pipeline {
                    match tp.allocate_descriptor_set(&daemon.vk.device) {
                        Ok(ds) => {
                            TransitionPipeline::update_descriptor_set(
                                &daemon.vk.device,
                                ds,
                                t.old_texture.view,
                                t.new_texture.view,
                                tp.sampler,
                            );
                            t.descriptor_set = Some(ds);
                        }
                        Err(e) => {
                            warn!(output = %name, "failed to allocate transition descriptor: {e}");
                        }
                    }
                }

                let new_wallpaper = Wallpaper {
                    source_path: path.to_string(),
                    format: if is_gif {
                        ImageFormat::Gif
                    } else {
                        ImageFormat::Jpeg
                    },
                    original_dimensions: (original_width, original_height),
                    display_dimensions: (t.new_texture.width, t.new_texture.height),
                    resize_mode: resize,
                    texture: output::GpuTexture {
                        image: t.new_texture.image,
                        view: t.new_texture.view,
                        memory: t.new_texture.memory,
                        width: t.new_texture.width,
                        height: t.new_texture.height,
                    },
                    is_animated: false,
                };

                // Clear animation state. Do NOT destroy the atlas here —
                // its handles are now owned by transition.old_texture and
                // will be freed in complete_transition().
                output.animation = None;

                output.wallpaper = Some(new_wallpaper);
                output.transition = Some(t);
                output.needs_redraw = true;
            } else {
                // No transition: set wallpaper directly (reuse descriptor set if available)
                let ds = match output.descriptor_set {
                    Some(ds) => ds,
                    None => match pipeline.allocate_descriptor_set(&daemon.vk.device) {
                        Ok(ds) => ds,
                        Err(e) => {
                            warn!(output = %name, "failed to allocate descriptor set: {e}");
                            continue;
                        }
                    },
                };

                WallpaperPipeline::update_descriptor_set(
                    &daemon.vk.device,
                    ds,
                    gpu_tex.view,
                    pipeline.sampler,
                );

                output.descriptor_set = Some(ds);

                let wallpaper = Wallpaper {
                    source_path: path.to_string(),
                    format: if is_gif {
                        ImageFormat::Gif
                    } else {
                        ImageFormat::Jpeg
                    },
                    original_dimensions: (original_width, original_height),
                    display_dimensions: (gpu_tex.width, gpu_tex.height),
                    resize_mode: resize,
                    texture: gpu_tex,
                    is_animated: false,
                };

                // SAFETY: GPU is idle (waited above), old texture safe to free.
                unsafe {
                    output.set_wallpaper(&daemon.vk.device, wallpaper);
                }
                debug!(output = %name, "set_static_wallpaper: non-transition path complete");
            }
        }
    }

    if let Err(e) = daemon.save_session() {
        warn!("failed to save session state: {e}");
    }

    IpcResponse::Ok
}

fn handle_clear(
    daemon: &mut DaemonState,
    target_outputs: Option<Vec<String>>,
    color: [u8; 3],
) -> IpcResponse {
    let names = get_target_outputs(daemon, &target_outputs);
    let clear_color = [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        1.0,
    ];

    for name in names {
        if let Some(output) = daemon.outputs.get_mut(&name) {
            // SAFETY: device_wait_idle ensures no GPU work references the old texture.
            unsafe {
                let _ = daemon.vk.device.device_wait_idle();
                // Cancel any active transition.
                // Only destroy old_texture here — new_texture shares handles with
                // output.wallpaper.texture, which clear_wallpaper will destroy.
                if let Some(t) = output.transition.take() {
                    t.old_texture.destroy(&daemon.vk.device);
                    if let Some(ds) = t.descriptor_set
                        && let Some(ref tp) = daemon.transition_pipeline
                    {
                        tp.free_descriptor_set(&daemon.vk.device, ds);
                    }
                }
                output.clear_wallpaper(&daemon.vk.device);
            }
            output.clear_color = clear_color;
            output.needs_redraw = true;
        }
    }

    IpcResponse::Ok
}

fn handle_restore(daemon: &mut DaemonState) -> IpcResponse {
    let state = match wl_common::cache::load_session_state() {
        Ok(s) => s,
        Err(e) => {
            return IpcResponse::Error {
                message: format!("failed to load session state: {e}"),
            };
        }
    };

    for (output_name, saved) in &state.outputs {
        if daemon.outputs.contains_key(output_name) {
            let resize = match saved.resize_mode.as_str() {
                "crop" => ResizeMode::Crop,
                "fit" => ResizeMode::Fit,
                "no" => ResizeMode::No,
                "center" => ResizeMode::Center,
                _ => ResizeMode::Crop,
            };

            // Restore without transition
            let no_transition = TransitionParams::default();
            let result = handle_img(
                daemon,
                &saved.wallpaper_path,
                Some(vec![output_name.clone()]),
                resize,
                &no_transition,
            );

            if let IpcResponse::Error { message } = &result {
                warn!(output = %output_name, "restore failed: {message}");
            }
        }
    }

    IpcResponse::Ok
}

fn handle_rotate_start(
    daemon: &mut DaemonState,
    params: rotation::RotateStartParams,
) -> IpcResponse {
    let rotation::RotateStartParams {
        directories,
        interval_secs,
        resize,
        transition,
        upscale_mode,
        upscale_cmd,
        upscale_scale,
    } = params;

    if interval_secs == 0 {
        return IpcResponse::Error {
            message: "interval must be greater than 0".to_string(),
        };
    }

    let candidates = rotation::RotationState::new_cycle(&directories);
    if candidates.is_empty() {
        return IpcResponse::Error {
            message: "no image files found in specified directories".to_string(),
        };
    }

    let interval = std::time::Duration::from_secs(interval_secs);
    let mut rot = rotation::RotationState {
        directories,
        interval,
        candidates,
        current_index: 0,
        next_rotation: std::time::Instant::now() + interval,
        resize,
        transition,
        upscale_mode,
        upscale_cmd,
        upscale_scale,
    };

    // Show first image immediately
    if let Some(path) = rot.next_image() {
        let path_str = path.to_string_lossy().to_string();
        let result = handle_img(daemon, &path_str, None, rot.resize, &rot.transition);
        if let IpcResponse::Error { ref message } = result {
            warn!("rotation: failed to set first wallpaper: {message}");
        }
    }

    rot.save();
    info!(
        interval_secs = interval_secs,
        images = rot.candidates.len(),
        "rotation started"
    );
    daemon.rotation = Some(rot);

    IpcResponse::Ok
}

fn handle_rotate_stop(daemon: &mut DaemonState) -> IpcResponse {
    daemon.rotation = None;
    wl_common::cache::delete_rotation_state();
    info!("rotation stopped");
    IpcResponse::Ok
}

fn handle_rotate_next(daemon: &mut DaemonState) -> IpcResponse {
    let rot = match daemon.rotation.as_mut() {
        Some(r) => r,
        None => {
            return IpcResponse::Error {
                message: "rotation is not active".to_string(),
            };
        }
    };

    let resize = rot.resize;
    let transition = rot.transition;

    if let Some(path) = rot.next_image() {
        rot.reset_timer();
        rot.save();
        let path_str = path.to_string_lossy().to_string();
        handle_img(daemon, &path_str, None, resize, &transition);
    } else {
        warn!("rotation: no images available after reshuffle");
    }

    IpcResponse::Ok
}

fn handle_rotate_status(daemon: &DaemonState) -> IpcResponse {
    match &daemon.rotation {
        Some(rot) => {
            let remaining = rot.candidates.len().saturating_sub(rot.current_index);
            let next_secs = rot.time_until_next().as_secs();
            IpcResponse::RotationStatus {
                active: true,
                interval_secs: Some(rot.interval.as_secs()),
                directories: Some(
                    rot.directories
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                ),
                next_change_secs: Some(next_secs),
                images_total: Some(rot.candidates.len()),
                images_remaining: Some(remaining),
            }
        }
        None => IpcResponse::RotationStatus {
            active: false,
            interval_secs: None,
            directories: None,
            next_change_secs: None,
            images_total: None,
            images_remaining: None,
        },
    }
}

fn get_target_outputs(daemon: &DaemonState, targets: &Option<Vec<String>>) -> Vec<String> {
    match targets {
        Some(names) => names.clone(),
        None => daemon.outputs.keys().cloned().collect(),
    }
}
