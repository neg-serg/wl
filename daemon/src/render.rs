use ash::vk;

use crate::animation;
use crate::output::Output;
use crate::vulkan::VulkanContext;
use crate::vulkan::pipeline::{
    TransitionPipeline, TransitionPushConstants, WallpaperPipeline, WallpaperPushConstants,
};

/// Render a single frame for one output.
///
/// Renders transition if active, wallpaper if set, or solid color fallback.
///
/// # Safety
/// All Vulkan handles in `output` and `vk` must be valid.
pub unsafe fn render_frame(
    vk: &VulkanContext,
    output: &mut Output,
    pipeline: Option<&WallpaperPipeline>,
    transition_pipeline: Option<&TransitionPipeline>,
) -> Result<(), RenderError> {
    let swapchain = output.swapchain.as_ref().ok_or(RenderError::NoSwapchain)?;

    // SAFETY: fence is valid, created in Output::new.
    // Wait for the previous frame's fence but do NOT reset it yet.
    // The fence stays signaled until just before queue_submit so that
    // any early return leaves it in a signaled state for the next call.
    unsafe {
        vk.device
            .wait_for_fences(&[output.in_flight_fence], true, u64::MAX)
            .map_err(RenderError::Vulkan)?;

        // Free the previous frame's command buffer now that the fence has signaled.
        if let Some(old_cmd) = output.last_command_buffer.take() {
            vk.device.free_command_buffers(vk.command_pool, &[old_cmd]);
        }
    }

    let (image_index, suboptimal) =
        match swapchain.acquire_next_image(output.image_available_semaphore, u64::MAX) {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                return Err(RenderError::NeedsRecreate);
            }
            Err(e) => return Err(RenderError::Vulkan(e)),
        };

    if suboptimal {
        // Continue rendering this frame but signal recreation afterwards.
        output.needs_recreate = true;
    }

    // Allocate command buffer
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(vk.command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);

    // SAFETY: device and command_pool are valid.
    let cmd = unsafe {
        vk.device
            .allocate_command_buffers(&alloc_info)
            .map_err(RenderError::Vulkan)?[0]
    };

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

    // SAFETY: cmd is freshly allocated.
    unsafe {
        if let Err(e) = vk.device.begin_command_buffer(cmd, &begin_info) {
            vk.device.free_command_buffers(vk.command_pool, &[cmd]);
            return Err(RenderError::Vulkan(e));
        }
    }

    // Re-borrow swapchain (first borrow was consumed by acquire_next_image match).
    let swapchain = output.swapchain.as_ref().ok_or(RenderError::NoSwapchain)?;
    let extent = swapchain.extent;
    let fb_index = image_index as usize;

    let resize_to_u32 = |m: wl_common::ipc_types::ResizeMode| -> u32 {
        match m {
            wl_common::ipc_types::ResizeMode::Crop
            | wl_common::ipc_types::ResizeMode::Center => 0,
            wl_common::ipc_types::ResizeMode::Fit => 1,
            wl_common::ipc_types::ResizeMode::No => 2,
        }
    };

    // Try transition → wallpaper → solid color fallback.
    if let (Some(transition), Some(tp), Some(&framebuffer)) = (
        output.transition.as_ref(),
        transition_pipeline,
        output.framebuffers.get(fb_index),
    ) {
        tracing::debug!("render: transition path");
        if let (Some(descriptor_set), Some(vk_pipeline)) =
            (transition.descriptor_set, tp.get(transition.kind))
        {
            let push_constants = TransitionPushConstants {
                progress: transition.progress,
                angle: transition.angle,
                pos_x: transition.position.0,
                pos_y: transition.position.1,
                wave_x: transition.wave.0,
                wave_y: transition.wave.1,
                old_resize_mode: resize_to_u32(transition.old_resize_mode),
                old_img_aspect: transition.old_texture.width as f32
                    / transition.old_texture.height as f32,
                new_resize_mode: resize_to_u32(transition.new_resize_mode),
                new_img_aspect: transition.new_texture.width as f32
                    / transition.new_texture.height as f32,
                screen_aspect: extent.width as f32 / extent.height as f32,
            };

            // SAFETY: All handles valid, transition pipeline and descriptor set are set up.
            unsafe {
                record_pipeline_draw(
                    &vk.device,
                    cmd,
                    &DrawParams {
                        render_pass: tp.render_pass,
                        framebuffer,
                        extent,
                        clear_color: output.clear_color,
                        pipeline: vk_pipeline,
                        pipeline_layout: tp.pipeline_layout,
                        descriptor_set,
                        push_constants_ptr: &push_constants as *const TransitionPushConstants
                            as *const u8,
                        push_constants_size: std::mem::size_of::<TransitionPushConstants>(),
                    },
                );
            }
        }
    } else if let (Some(wallpaper), Some(pipeline), Some(descriptor_set), Some(&framebuffer)) = (
        output.wallpaper.as_ref(),
        pipeline,
        output.descriptor_set,
        output.framebuffers.get(fb_index),
    ) {
        tracing::debug!("render: wallpaper path");
        let resize_mode = resize_to_u32(wallpaper.resize_mode);

        // Compute animation UV offset if animating
        let (uv_offset, uv_scale) = if let Some(ref anim) = output.animation {
            animation::frame_uv_offset(anim)
        } else {
            (0.0, 1.0)
        };

        // For animations, img_aspect is per-frame (not atlas-wide)
        let img_aspect = if let Some(ref anim) = output.animation {
            anim.atlas_frame_width as f32 / anim.atlas_frame_height as f32
        } else {
            wallpaper.texture.width as f32 / wallpaper.texture.height as f32
        };

        let push_constants = WallpaperPushConstants {
            resize_mode,
            img_aspect,
            screen_aspect: extent.width as f32 / extent.height as f32,
            uv_offset,
            uv_scale,
        };

        // SAFETY: All handles valid, wallpaper pipeline and descriptor set are set up.
        unsafe {
            record_pipeline_draw(
                &vk.device,
                cmd,
                &DrawParams {
                    render_pass: pipeline.render_pass,
                    framebuffer,
                    extent,
                    clear_color: output.clear_color,
                    pipeline: pipeline.pipeline,
                    pipeline_layout: pipeline.pipeline_layout,
                    descriptor_set,
                    push_constants_ptr: &push_constants as *const WallpaperPushConstants
                        as *const u8,
                    push_constants_size: std::mem::size_of::<WallpaperPushConstants>(),
                },
            );
        }
    } else if let Some(image) = swapchain.images.get(image_index as usize).copied() {
        // No wallpaper: clear to solid color
        tracing::debug!("render: solid color fallback (no wallpaper)");
        // SAFETY: cmd and image are valid.
        unsafe {
            record_clear_image(&vk.device, cmd, image, output.clear_color);
        }
    }

    // SAFETY: cmd is recording.
    unsafe {
        if let Err(e) = vk.device.end_command_buffer(cmd) {
            vk.device.free_command_buffers(vk.command_pool, &[cmd]);
            return Err(RenderError::Vulkan(e));
        }
    }

    // Submit — reset fence immediately before submit so it stays signaled on any
    // earlier failure path, preventing a deadlock on the next wait_for_fences call.
    let wait_semaphores = [output.image_available_semaphore];
    let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
    let signal_semaphores = [output.render_finished_semaphore];
    let command_buffers = [cmd];

    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(&wait_semaphores)
        .wait_dst_stage_mask(&wait_stages)
        .command_buffers(&command_buffers)
        .signal_semaphores(&signal_semaphores);

    // SAFETY: All handles valid. Reset fence just before submit so that any failure
    // before this point leaves the fence signaled (preventing deadlock on next frame).
    unsafe {
        vk.device
            .reset_fences(&[output.in_flight_fence])
            .map_err(|e| {
                vk.device.free_command_buffers(vk.command_pool, &[cmd]);
                RenderError::Vulkan(e)
            })?;
        if let Err(e) =
            vk.device
                .queue_submit(vk.graphics_queue, &[submit_info], output.in_flight_fence)
        {
            vk.device.free_command_buffers(vk.command_pool, &[cmd]);
            return Err(RenderError::Vulkan(e));
        }
    }

    // Present
    let swapchain = output.swapchain.as_ref().ok_or(RenderError::NoSwapchain)?;
    match swapchain.present(
        vk.graphics_queue,
        output.render_finished_semaphore,
        image_index,
    ) {
        Ok(suboptimal) if suboptimal => {
            output.needs_recreate = true;
        }
        Ok(_) => {}
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) | Err(vk::Result::SUBOPTIMAL_KHR) => {
            output.needs_recreate = true;
        }
        Err(e) => return Err(RenderError::Vulkan(e)),
    }

    // Store the command buffer so it can be freed after the fence signals next frame.
    output.last_command_buffer = Some(cmd);
    output.needs_redraw = output.needs_recreate;
    Ok(())
}

/// Parameters for recording a pipeline draw command.
struct DrawParams {
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    extent: vk::Extent2D,
    clear_color: [f32; 4],
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    descriptor_set: vk::DescriptorSet,
    push_constants_ptr: *const u8,
    push_constants_size: usize,
}

/// Record a pipeline draw command (shared between wallpaper and transition).
///
/// # Safety
/// All Vulkan handles must be valid.
unsafe fn record_pipeline_draw(device: &ash::Device, cmd: vk::CommandBuffer, params: &DrawParams) {
    let DrawParams {
        render_pass,
        framebuffer,
        extent,
        clear_color,
        pipeline,
        pipeline_layout,
        descriptor_set,
        push_constants_ptr,
        push_constants_size,
    } = *params;
    let clear_value = vk::ClearValue {
        color: vk::ClearColorValue {
            float32: clear_color,
        },
    };

    let render_pass_info = vk::RenderPassBeginInfo::default()
        .render_pass(render_pass)
        .framebuffer(framebuffer)
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        })
        .clear_values(std::slice::from_ref(&clear_value));

    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };

    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    };

    // SAFETY: All handles are valid. Caller guarantees validity.
    unsafe {
        device.cmd_begin_render_pass(cmd, &render_pass_info, vk::SubpassContents::INLINE);
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
        device.cmd_set_viewport(cmd, 0, &[viewport]);
        device.cmd_set_scissor(cmd, 0, &[scissor]);
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            pipeline_layout,
            0,
            &[descriptor_set],
            &[],
        );

        let pc_bytes = std::slice::from_raw_parts(push_constants_ptr, push_constants_size);
        device.cmd_push_constants(
            cmd,
            pipeline_layout,
            vk::ShaderStageFlags::FRAGMENT,
            0,
            pc_bytes,
        );

        device.cmd_draw(cmd, 3, 1, 0, 0);
        device.cmd_end_render_pass(cmd);
    }
}

/// Record clear-to-color commands for an image.
///
/// # Safety
/// `device`, `cmd`, and `image` must be valid.
unsafe fn record_clear_image(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    clear_color: [f32; 4],
) {
    let subresource_range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    let to_clear = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .image(image)
        .subresource_range(subresource_range);

    let vk_clear_color = vk::ClearColorValue {
        float32: clear_color,
    };

    let to_present = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::empty())
        .image(image)
        .subresource_range(subresource_range);

    // SAFETY: Caller guarantees all handles are valid.
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_clear],
        );

        device.cmd_clear_color_image(
            cmd,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &vk_clear_color,
            &[subresource_range],
        );

        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
    }
}

#[derive(Debug)]
pub enum RenderError {
    NoSwapchain,
    /// The swapchain is out-of-date or suboptimal and must be recreated.
    NeedsRecreate,
    Vulkan(vk::Result),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSwapchain => write!(f, "no swapchain for output"),
            Self::NeedsRecreate => write!(f, "swapchain needs recreation"),
            Self::Vulkan(e) => write!(f, "Vulkan error: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}
