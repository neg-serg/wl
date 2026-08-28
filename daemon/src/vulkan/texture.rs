use ash::vk;

use super::{VulkanContext, VulkanError};
use crate::output::GpuTexture;

/// Size in bytes of the largest device-local memory heap.
///
/// This is the practical "how much VRAM does the GPU have" number used to
/// budget GPU allocations dynamically. Returns 0 when the driver reports no
/// device-local heap (rare; some software renderers).
fn device_local_heap_size(vk: &VulkanContext) -> u64 {
    let mut available = 0u64;
    for i in 0..vk.physical_device_memory_properties.memory_type_count {
        let heap_idx = vk.physical_device_memory_properties.memory_types[i as usize].heap_index;
        let heap = vk.physical_device_memory_properties.memory_heaps[heap_idx as usize];
        if heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL) {
            available = available.max(heap.size);
        }
    }
    available
}

/// Upload RGBA8 pixel data to a GPU-local texture, returning a ready-to-sample `GpuTexture`.
///
/// This creates a staging buffer, copies the data into it, then uses a one-shot command buffer
/// to transfer from the staging buffer into a device-local `VkImage` with the appropriate
/// layout transitions.
pub fn upload_rgba8_texture(
    vk: &VulkanContext,
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<GpuTexture, VulkanError> {
    let expected_size = (width as usize) * (height as usize) * 4;
    if data.len() < expected_size {
        return Err(VulkanError::TextureUpload(format!(
            "data too small: expected at least {} bytes for {}x{} RGBA8, got {}",
            expected_size,
            width,
            height,
            data.len()
        )));
    }

    // Check against GPU image dimension limits
    let max_dim = vk.physical_device_properties.limits.max_image_dimension2_d;
    if width > max_dim || height > max_dim {
        return Err(VulkanError::TextureUpload(format!(
            "image {}x{} exceeds GPU max dimension {max_dim}",
            width, height,
        )));
    }

    // Check against available GPU memory (rough heuristic)
    let buffer_bytes = expected_size as u64;
    let image_bytes = buffer_bytes; // device-local copy
    let total_needed = buffer_bytes + image_bytes;
    let available = device_local_heap_size(vk);
    // Tight VRAM budget: reject textures exceeding ~6.25% of GPU memory to keep
    // headroom for swapchain images, transitions, and driver overhead.
    // If the tight limit is exceeded, fall back to a hard limit of 25%.
    let vram_limit = available / 16;
    let hard_limit = available / 4;
    if available > 0 && total_needed > vram_limit {
        if total_needed > hard_limit {
            return Err(VulkanError::TextureUpload(format!(
                "image requires {total_needed} bytes but VRAM hard limit is ~{hard_limit} bytes \
                 (25% of {available} total)",
            )));
        }
        tracing::warn!(
            "image requires {total_needed} bytes, exceeds tight VRAM budget of {vram_limit} bytes \
             (6.25% of {available}), but within hard limit of {hard_limit}"
        );
    }

    let device = &vk.device;
    let buffer_size = expected_size as vk::DeviceSize;

    // --- Create staging buffer ---
    let staging_buffer_info = vk::BufferCreateInfo::default()
        .size(buffer_size)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    // SAFETY: device is a valid Vulkan device; staging_buffer_info is properly initialized.
    let staging_buffer = unsafe {
        device
            .create_buffer(&staging_buffer_info, None)
            .map_err(|e| VulkanError::TextureUpload(format!("staging buffer creation: {e}")))?
    };

    // SAFETY: staging_buffer is a valid buffer just created above.
    let staging_mem_reqs = unsafe { device.get_buffer_memory_requirements(staging_buffer) };

    let staging_mem_type = vk
        .find_memory_type(
            staging_mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| {
            // SAFETY: staging_buffer is valid and not bound to memory yet.
            unsafe { device.destroy_buffer(staging_buffer, None) };
            VulkanError::TextureUpload(
                "no HOST_VISIBLE | HOST_COHERENT memory type found".to_string(),
            )
        })?;

    let staging_alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(staging_mem_reqs.size)
        .memory_type_index(staging_mem_type);

    // SAFETY: device is valid; alloc info references a valid memory type.
    let staging_memory = unsafe {
        device
            .allocate_memory(&staging_alloc_info, None)
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                VulkanError::TextureUpload(format!("staging memory allocation: {e}"))
            })?
    };

    // SAFETY: staging_buffer and staging_memory are valid; offset 0 is within the allocation.
    unsafe {
        device
            .bind_buffer_memory(staging_buffer, staging_memory, 0)
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                VulkanError::TextureUpload(format!("bind staging buffer memory: {e}"))
            })?;
    }

    // Map, copy, unmap
    // SAFETY: staging_memory is HOST_VISIBLE, bound, and not currently mapped.
    unsafe {
        let ptr = device
            .map_memory(staging_memory, 0, buffer_size, vk::MemoryMapFlags::empty())
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                VulkanError::TextureUpload(format!("map staging memory: {e}"))
            })?;

        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, expected_size);

        device.unmap_memory(staging_memory);
    }

    // --- Create the device-local image ---
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    // SAFETY: device is valid; image_info is fully initialized.
    let image = unsafe {
        device.create_image(&image_info, None).map_err(|e| {
            device.destroy_buffer(staging_buffer, None);
            device.free_memory(staging_memory, None);
            VulkanError::TextureUpload(format!("image creation: {e}"))
        })?
    };

    // SAFETY: image is a valid image handle.
    let image_mem_reqs = unsafe { device.get_image_memory_requirements(image) };

    let image_mem_type = vk
        .find_memory_type(
            image_mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| {
            // SAFETY: All handles below are valid.
            unsafe {
                device.destroy_image(image, None);
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
            }
            VulkanError::TextureUpload("no DEVICE_LOCAL memory type found".to_string())
        })?;

    let image_alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(image_mem_reqs.size)
        .memory_type_index(image_mem_type);

    // SAFETY: device is valid; alloc info references a valid memory type.
    let image_memory = unsafe {
        device
            .allocate_memory(&image_alloc_info, None)
            .map_err(|e| {
                device.destroy_image(image, None);
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                VulkanError::TextureUpload(format!("image memory allocation: {e}"))
            })?
    };

    // SAFETY: image and image_memory are valid; offset 0 satisfies alignment requirements
    // because image_alloc_info.allocation_size comes from get_image_memory_requirements.
    unsafe {
        device
            .bind_image_memory(image, image_memory, 0)
            .map_err(|e| {
                device.free_memory(image_memory, None);
                device.destroy_image(image, None);
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                VulkanError::TextureUpload(format!("bind image memory: {e}"))
            })?;
    }

    // --- Record and submit transfer commands ---
    // SAFETY: VulkanContext guarantees a valid command pool and device.
    let cmd = unsafe {
        vk.begin_single_time_commands().map_err(|e| {
            device.free_memory(image_memory, None);
            device.destroy_image(image, None);
            device.destroy_buffer(staging_buffer, None);
            device.free_memory(staging_memory, None);
            VulkanError::TextureUpload(format!("begin command buffer: {e}"))
        })?
    };

    // Transition UNDEFINED -> TRANSFER_DST_OPTIMAL
    let barrier_to_transfer = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // SAFETY: cmd is a valid recording command buffer; image is a valid image.
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_transfer],
        );
    }

    // Copy staging buffer -> image
    let region = vk::BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
        image_extent: vk::Extent3D {
            width,
            height,
            depth: 1,
        },
    };

    // SAFETY: cmd is recording; staging_buffer contains the pixel data; image is in
    // TRANSFER_DST_OPTIMAL layout.
    unsafe {
        device.cmd_copy_buffer_to_image(
            cmd,
            staging_buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }

    // Transition TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL
    let barrier_to_shader = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // SAFETY: cmd is a valid recording command buffer; image is in TRANSFER_DST_OPTIMAL layout.
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_shader],
        );
    }

    // Submit and wait
    // SAFETY: cmd is a recording command buffer from begin_single_time_commands.
    let submit_result = unsafe { vk.end_single_time_commands(cmd) };

    // Clean up staging resources regardless of submit outcome
    // SAFETY: staging_buffer and staging_memory are valid and no longer in use
    // (end_single_time_commands waits for queue idle).
    unsafe {
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_memory, None);
    }

    if let Err(e) = submit_result {
        // SAFETY: image and image_memory are valid.
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(image_memory, None);
        }
        if e == vk::Result::ERROR_DEVICE_LOST {
            return Err(VulkanError::DeviceLost);
        }
        return Err(VulkanError::TextureUpload(format!(
            "command submission: {e}"
        )));
    }

    // --- Create image view ---
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .components(vk::ComponentMapping {
            r: vk::ComponentSwizzle::IDENTITY,
            g: vk::ComponentSwizzle::IDENTITY,
            b: vk::ComponentSwizzle::IDENTITY,
            a: vk::ComponentSwizzle::IDENTITY,
        })
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // SAFETY: device and image are valid; view_info is fully initialized.
    let view = unsafe {
        device.create_image_view(&view_info, None).map_err(|e| {
            device.destroy_image(image, None);
            device.free_memory(image_memory, None);
            VulkanError::TextureUpload(format!("image view creation: {e}"))
        })?
    };

    Ok(GpuTexture {
        image,
        view,
        memory: image_memory,
        width,
        height,
    })
}

/// Upper safety bound for a GIF frame atlas in bytes (256 MiB).
///
/// The actual budget is computed *dynamically* from the GPU's device-local
/// heap in [`plan_gif_atlas`]; this constant only prevents an extremely large
/// VRAM budget from producing an absurdly huge wallpaper atlas.
const MAX_GIF_ATLAS_BYTES: u64 = 256 * 1024 * 1024;

/// How many of a GIF's frames fit in memory, and where they go in the atlas.
///
/// Produced by [`plan_gif_atlas`] *before* any pixel data is decoded, so the
/// caller can decide up front how much memory the animation is allowed to use.
pub struct GifAtlasPlan {
    /// Indices (into the GIF's frame sequence) of the frames to keep, in
    /// ascending order. Frames are sampled evenly when the full animation
    /// would not fit the budget.
    pub kept_indices: Vec<usize>,
    /// Width of the final atlas texture (`frame_width * kept_indices.len()`).
    pub atlas_width: u32,
    /// Height of the final atlas texture (equals the frame height).
    pub atlas_height: u32,
}

/// Evenly sample `frame_count` frames down to at most `max_frames`.
///
/// Keeps the first frame and spreads the rest uniformly across the animation
/// so its timing is preserved as well as possible. Returns kept indices in
/// ascending order. If everything fits, all indices are kept.
fn select_atlas_frames(frame_count: usize, max_frames: usize) -> Vec<usize> {
    if frame_count <= max_frames {
        return (0..frame_count).collect();
    }
    // Even sampling: keep `max_frames` frames spread uniformly across the
    // animation (same selection as before, just decided before decoding).
    let step = frame_count as f64 / max_frames as f64;
    (0..max_frames)
        .map(|i| (i as f64 * step).floor() as usize)
        .collect()
}

/// Decide which frames of a GIF fit a dynamically-determined memory budget.
///
/// The budget is the smaller of:
/// - 25% of the GPU's device-local VRAM (the same heuristic the static-image
///   path uses), so a GPU with little VRAM gets a smaller atlas and a GPU with
///   plenty gets a larger one; and
/// - [`MAX_GIF_ATLAS_BYTES`], a hard safety bound.
///
/// The GPU's maximum texture dimension also caps the atlas width (frames are
/// laid out left-to-right in a single row). If the whole animation does not
/// fit, frames are sampled evenly across the animation so its timing is
/// preserved as well as possible.
pub fn plan_gif_atlas(
    vk: &VulkanContext,
    frame_count: usize,
    frame_width: u32,
    frame_height: u32,
) -> Result<GifAtlasPlan, VulkanError> {
    if frame_count == 0 {
        return Err(VulkanError::TextureUpload(
            "GIF atlas: no frames to plan".to_string(),
        ));
    }
    if frame_width == 0 || frame_height == 0 {
        return Err(VulkanError::TextureUpload(format!(
            "GIF atlas: invalid frame size {frame_width}x{frame_height}"
        )));
    }

    let max_dim = vk.physical_device_properties.limits.max_image_dimension2_d;
    let frame_bytes = (frame_width as u64) * (frame_height as u64) * 4;

    // Dynamic memory budget: a sixteenth of the VRAM the GPU actually reports
    // (the same tight tier the static-image path uses; if the atlas would
    // exceed it we simply keep fewer frames instead of failing), capped by a
    // hard safety bound.
    let heap = device_local_heap_size(vk);
    let vram_budget = heap / 16;
    let mem_budget = vram_budget.min(MAX_GIF_ATLAS_BYTES);

    let max_by_dim = (max_dim / frame_width) as usize;
    let max_by_mem = (mem_budget / frame_bytes) as usize;
    let max_frames = max_by_dim.min(max_by_mem).max(1).min(frame_count);

    let kept_indices = select_atlas_frames(frame_count, max_frames);

    let atlas_width = frame_width * kept_indices.len() as u32;
    tracing::debug!(
        total_frames = frame_count,
        kept_frames = kept_indices.len(),
        vram_bytes = heap,
        budget_bytes = mem_budget,
        frame_bytes,
        max_frames,
        sampled = frame_count > max_frames,
        atlas_w = atlas_width,
        atlas_h = frame_height,
        "GIF atlas plan (dynamic VRAM budget)"
    );
    Ok(GifAtlasPlan {
        kept_indices,
        atlas_width,
        atlas_height: frame_height,
    })
}

/// Upload a GIF frame atlas to a GPU-local texture, decoding one frame at a time.
///
/// `get_frame` is called once per kept frame, in ascending index order, and
/// must return that frame's RGBA8 pixel data (`frame_width * frame_height * 4`
/// bytes). Frames are copied into the atlas through a single small staging
/// buffer (one frame's worth), so at no point is the whole animation — or even
/// the whole atlas — held in host memory: peak RAM is one decoded frame plus
/// one resized frame in the caller.
pub fn upload_gif_atlas(
    vk: &VulkanContext,
    plan: &GifAtlasPlan,
    frame_width: u32,
    frame_height: u32,
    mut get_frame: impl FnMut(usize) -> Result<Vec<u8>, String>,
) -> Result<GpuTexture, VulkanError> {
    let device = &vk.device;
    let frame_bytes = (frame_width as usize) * (frame_height as usize) * 4;
    let atlas_width = plan.atlas_width;
    let atlas_height = plan.atlas_height;

    tracing::debug!(
        frames = plan.kept_indices.len(),
        atlas_w = atlas_width,
        atlas_h = atlas_height,
        atlas_mib = (atlas_width as u64 * atlas_height as u64 * 4) / (1024 * 1024),
        staging_bytes = frame_bytes,
        "GIF atlas upload: streaming frame-by-frame"
    );

    // --- Create the device-local atlas image ---
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width: atlas_width,
            height: atlas_height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    // SAFETY: device is valid; image_info is fully initialized.
    let image = unsafe {
        device.create_image(&image_info, None).map_err(|e| {
            VulkanError::TextureUpload(format!("atlas image creation: {e}"))
        })?
    };

    // SAFETY: image is a valid image handle.
    let image_mem_reqs = unsafe { device.get_image_memory_requirements(image) };

    let image_mem_type = vk
        .find_memory_type(
            image_mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| {
            // SAFETY: image is valid and not bound to memory yet.
            unsafe { device.destroy_image(image, None) };
            VulkanError::TextureUpload("no DEVICE_LOCAL memory type found".to_string())
        })?;

    let image_alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(image_mem_reqs.size)
        .memory_type_index(image_mem_type);

    // SAFETY: device is valid; alloc info references a valid memory type.
    let image_memory = unsafe {
        device
            .allocate_memory(&image_alloc_info, None)
            .map_err(|e| {
                device.destroy_image(image, None);
                VulkanError::TextureUpload(format!("atlas image memory allocation: {e}"))
            })?
    };

    // SAFETY: image and image_memory are valid; offset 0 satisfies alignment requirements.
    unsafe {
        device
            .bind_image_memory(image, image_memory, 0)
            .map_err(|e| {
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                VulkanError::TextureUpload(format!("bind atlas image memory: {e}"))
            })?;
    }

    // --- Record and submit transfer commands ---
    // SAFETY: VulkanContext guarantees a valid command pool and device.
    let cmd = unsafe {
        vk.begin_single_time_commands().map_err(|e| {
            device.destroy_image(image, None);
            device.free_memory(image_memory, None);
            VulkanError::TextureUpload(format!("begin command buffer: {e}"))
        })?
    };

    // Transition UNDEFINED -> TRANSFER_DST_OPTIMAL
    let barrier_to_transfer = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // SAFETY: cmd is a valid recording command buffer; image is a valid image.
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_transfer],
        );
    }

    // --- Small staging buffer: one frame's worth, reused for every frame ---
    let staging_buffer_info = vk::BufferCreateInfo::default()
        .size(frame_bytes as vk::DeviceSize)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    // SAFETY: device is valid; staging_buffer_info is properly initialized.
    let staging_buffer = unsafe {
        device
            .create_buffer(&staging_buffer_info, None)
            .map_err(|e| {
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                device.free_command_buffers(vk.command_pool, &[cmd]);
                VulkanError::TextureUpload(format!("atlas staging buffer creation: {e}"))
            })?
    };

    // SAFETY: staging_buffer is a valid buffer just created above.
    let staging_mem_reqs = unsafe { device.get_buffer_memory_requirements(staging_buffer) };

    let staging_mem_type = vk
        .find_memory_type(
            staging_mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or_else(|| {
            // SAFETY: All handles below are valid.
            unsafe {
                device.destroy_buffer(staging_buffer, None);
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                device.free_command_buffers(vk.command_pool, &[cmd]);
            }
            VulkanError::TextureUpload(
                "no HOST_VISIBLE | HOST_COHERENT memory type found".to_string(),
            )
        })?;

    let staging_alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(staging_mem_reqs.size)
        .memory_type_index(staging_mem_type);

    // SAFETY: device is valid; alloc info references a valid memory type.
    let staging_memory = unsafe {
        device
            .allocate_memory(&staging_alloc_info, None)
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                device.free_command_buffers(vk.command_pool, &[cmd]);
                VulkanError::TextureUpload(format!("atlas staging memory allocation: {e}"))
            })?
    };

    // SAFETY: staging_buffer and staging_memory are valid; offset 0 is within the allocation.
    unsafe {
        device
            .bind_buffer_memory(staging_buffer, staging_memory, 0)
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                device.free_command_buffers(vk.command_pool, &[cmd]);
                VulkanError::TextureUpload(format!("bind atlas staging buffer memory: {e}"))
            })?;
    }

    // SAFETY: staging_memory is HOST_VISIBLE, bound, and not currently mapped.
    let mapped = unsafe {
        device
            .map_memory(
                staging_memory,
                0,
                frame_bytes as vk::DeviceSize,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                device.free_command_buffers(vk.command_pool, &[cmd]);
                VulkanError::TextureUpload(format!("map atlas staging memory: {e}"))
            })?
    };

    // Copy each kept frame into its atlas slot, one at a time.
    for (slot, &frame_idx) in plan.kept_indices.iter().enumerate() {
        let data = match get_frame(frame_idx) {
            Ok(d) => d,
            Err(e) => {
                // SAFETY: All handles valid; nothing has been submitted yet.
                unsafe {
                    device.unmap_memory(staging_memory);
                    device.destroy_buffer(staging_buffer, None);
                    device.free_memory(staging_memory, None);
                    device.destroy_image(image, None);
                    device.free_memory(image_memory, None);
                    device.free_command_buffers(vk.command_pool, &[cmd]);
                }
                return Err(VulkanError::TextureUpload(format!(
                    "failed to produce GIF frame {frame_idx}: {e}"
                )));
            }
        };
        if data.len() < frame_bytes {
            // SAFETY: All handles valid; nothing has been submitted yet.
            unsafe {
                device.unmap_memory(staging_memory);
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                device.free_command_buffers(vk.command_pool, &[cmd]);
            }
            return Err(VulkanError::TextureUpload(format!(
                "GIF frame {frame_idx} is {} bytes, expected at least {frame_bytes}",
                data.len()
            )));
        }

        // SAFETY: mapped is valid for frame_bytes bytes; data is at least that long.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), mapped as *mut u8, frame_bytes);
        }

        // Copy staging buffer -> atlas image at this frame's horizontal slot.
        let region = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D {
                x: ((slot as u32) * frame_width) as i32,
                y: 0,
                z: 0,
            },
            image_extent: vk::Extent3D {
                width: frame_width,
                height: frame_height,
                depth: 1,
            },
        };

        // SAFETY: cmd is recording; staging_buffer contains the frame pixels;
        // the atlas image is in TRANSFER_DST_OPTIMAL layout.
        unsafe {
            device.cmd_copy_buffer_to_image(
                cmd,
                staging_buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
    }

    // SAFETY: staging_memory is mapped and HOST_COHERENT; no further writes happen.
    unsafe {
        device.unmap_memory(staging_memory);
    }

    // Transition TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL
    let barrier_to_shader = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // SAFETY: cmd is a valid recording command buffer; image is in TRANSFER_DST_OPTIMAL layout.
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_shader],
        );
    }

    // Submit and wait.
    // SAFETY: cmd is a recording command buffer from begin_single_time_commands.
    let submit_result = unsafe { vk.end_single_time_commands(cmd) };

    // Clean up staging resources regardless of submit outcome.
    // SAFETY: staging_buffer and staging_memory are valid and no longer in use
    // (end_single_time_commands waits for queue idle).
    unsafe {
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_memory, None);
    }

    if let Err(e) = submit_result {
        // SAFETY: image and image_memory are valid.
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(image_memory, None);
        }
        if e == vk::Result::ERROR_DEVICE_LOST {
            return Err(VulkanError::DeviceLost);
        }
        return Err(VulkanError::TextureUpload(format!(
            "atlas command submission: {e}"
        )));
    }

    // --- Create image view ---
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .components(vk::ComponentMapping {
            r: vk::ComponentSwizzle::IDENTITY,
            g: vk::ComponentSwizzle::IDENTITY,
            b: vk::ComponentSwizzle::IDENTITY,
            a: vk::ComponentSwizzle::IDENTITY,
        })
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    // SAFETY: device and image are valid; view_info is fully initialized.
    let view = unsafe {
        device.create_image_view(&view_info, None).map_err(|e| {
            device.destroy_image(image, None);
            device.free_memory(image_memory, None);
            VulkanError::TextureUpload(format!("atlas image view creation: {e}"))
        })?
    };

    Ok(GpuTexture {
        image,
        view,
        memory: image_memory,
        width: atlas_width,
        height: atlas_height,
    })
}

/// Create a texture sampler with linear filtering and clamp-to-edge addressing.
#[allow(dead_code)]
pub fn create_sampler(vk: &VulkanContext) -> Result<vk::Sampler, VulkanError> {
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .anisotropy_enable(false)
        .max_anisotropy(1.0)
        .compare_enable(false)
        .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
        .unnormalized_coordinates(false)
        .mip_lod_bias(0.0)
        .min_lod(0.0)
        .max_lod(0.0);

    // SAFETY: device is valid; sampler_info is fully initialized.
    let sampler = unsafe {
        vk.device
            .create_sampler(&sampler_info, None)
            .map_err(|e| VulkanError::TextureUpload(format!("sampler creation: {e}")))?
    };

    Ok(sampler)
}

#[cfg(test)]
mod tests {
    use super::select_atlas_frames;

    #[test]
    fn keeps_all_frames_when_they_fit() {
        assert_eq!(select_atlas_frames(10, 10), (0..10).collect::<Vec<_>>());
        assert_eq!(select_atlas_frames(3, 100), vec![0, 1, 2]);
    }

    #[test]
    fn samples_evenly_when_over_budget() {
        // step = 10 / 4 = 2.5 -> indices 0, 2, 5, 7
        assert_eq!(select_atlas_frames(10, 4), vec![0, 2, 5, 7]);
        // step = 5 / 2 = 2.5 -> indices 0, 2
        assert_eq!(select_atlas_frames(5, 2), vec![0, 2]);
        // Always keeps at least the first frame.
        assert_eq!(select_atlas_frames(5, 1), vec![0]);
        assert_eq!(select_atlas_frames(1, 1), vec![0]);
    }

    #[test]
    fn sampled_indices_are_valid_ascending_and_bounded() {
        for count in [7usize, 10, 33, 100, 1000] {
            for max in [1usize, 2, 3, 7, 10, 31, 100, 500] {
                let kept = select_atlas_frames(count, max);
                let expected_len = count.min(max);
                assert_eq!(kept.len(), expected_len, "count={count} max={max}");
                assert_eq!(kept.first(), Some(&0), "count={count} max={max}");
                assert!(
                    kept.windows(2).all(|w| w[0] < w[1]),
                    "not strictly ascending: {kept:?} (count={count} max={max})"
                );
                assert!(
                    kept.iter().all(|&i| i < count),
                    "index out of range: {kept:?} (count={count} max={max})"
                );
            }
        }
    }
}
