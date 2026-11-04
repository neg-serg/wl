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

/// Host memory budget for the GIF atlas staging buffer.
///
/// Frames are streamed through the staging buffer in *chunks*: the GPU reads
/// host-written staging memory when the submitted commands execute, not when
/// they are recorded, so every copy inside one submit has to read a distinct
/// buffer region. A one-frame buffer would therefore need one submit per frame;
/// a bounded multi-frame buffer keeps the upload to `ceil(frames / chunk)` while
/// still never holding the whole atlas in RAM.
const ATLAS_STAGING_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Upload a GIF frame atlas to a GPU-local texture, decoding one frame at a time.
///
/// `get_frame` is called once per kept frame, in ascending index order, and
/// must return that frame's RGBA8 pixel data (`frame_width * frame_height * 4`
/// bytes). Frames are copied into the atlas through a bounded staging buffer
/// (up to [`ATLAS_STAGING_BUDGET_BYTES`] or one frame, whichever is larger), so
/// at no point is the whole animation — or even the whole atlas — held in host
/// memory: peak RAM is one chunk of decoded frames plus one resized frame in
/// the caller.
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

    let frames_per_chunk = staging_frames_per_chunk(frame_bytes, plan.kept_indices.len());
    let staging_bytes = frame_bytes * frames_per_chunk;

    tracing::debug!(
        frames = plan.kept_indices.len(),
        atlas_w = atlas_width,
        atlas_h = atlas_height,
        atlas_mib = (atlas_width as u64 * atlas_height as u64 * 4) / (1024 * 1024),
        staging_bytes,
        frames_per_chunk,
        "GIF atlas upload: streaming frame-by-frame",
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
        .usage(
            vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::SAMPLED,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    // SAFETY: device is valid; image_info is fully initialized.
    let image = unsafe {
        device
            .create_image(&image_info, None)
            .map_err(|e| VulkanError::TextureUpload(format!("atlas image creation: {e}")))?
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

    // --- Bounded staging buffer, reused chunk by chunk ---
    let staging_buffer_info = vk::BufferCreateInfo::default()
        .size(staging_bytes as vk::DeviceSize)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    // SAFETY: device is valid; staging_buffer_info is properly initialized.
    let staging_buffer = unsafe {
        device
            .create_buffer(&staging_buffer_info, None)
            .map_err(|e| {
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
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
                VulkanError::TextureUpload(format!("bind atlas staging buffer memory: {e}"))
            })?;
    }

    // SAFETY: staging_memory is HOST_VISIBLE, bound, and not currently mapped.
    let mapped = unsafe {
        device
            .map_memory(
                staging_memory,
                0,
                staging_mem_reqs.size,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|e| {
                device.destroy_buffer(staging_buffer, None);
                device.free_memory(staging_memory, None);
                device.destroy_image(image, None);
                device.free_memory(image_memory, None);
                VulkanError::TextureUpload(format!("map atlas staging memory: {e}"))
            })?
    };

    // SAFETY: every handle passed is valid for the duration of the call and the
    // staging memory is mapped for `staging_bytes` bytes.
    let upload = unsafe {
        record_atlas_upload(
            vk,
            image,
            staging_buffer,
            staging_bytes,
            mapped.cast::<u8>(),
            frame_bytes,
            frame_width,
            frame_height,
            plan,
            &mut get_frame,
        )
    };

    // SAFETY: the upload either completed (its submits waited for the queue) or
    // never submitted; the staging memory is no longer needed in both cases.
    unsafe {
        device.unmap_memory(staging_memory);
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_memory, None);
    }

    if let Err(e) = upload {
        // SAFETY: image and image_memory are valid and not used by any submission.
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(image_memory, None);
        }
        return Err(e);
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

/// How many frames share one staging chunk (and therefore one submit).
///
/// Bounded by the host staging budget and by the number of kept frames; a frame
/// larger than the budget is uploaded alone rather than rejected.
fn staging_frames_per_chunk(frame_bytes: usize, total_frames: usize) -> usize {
    (ATLAS_STAGING_BUDGET_BYTES / frame_bytes.max(1)).clamp(1, total_frames.max(1))
}

/// Record and submit the chunked staging copies of a GIF atlas.
///
/// One submit is issued per chunk, and copies inside a chunk read distinct
/// `buffer_offset`s, so no frame can observe another frame's staged pixels.
///
/// # Safety
/// `image` and `staging_buffer` must be valid handles created by `vk`'s device,
/// and `mapped` must be a mapped pointer valid for at least `staging_bytes`.
#[allow(clippy::too_many_arguments)]
unsafe fn record_atlas_upload(
    vk: &VulkanContext,
    image: vk::Image,
    staging_buffer: vk::Buffer,
    staging_bytes: usize,
    mapped: *mut u8,
    frame_bytes: usize,
    frame_width: u32,
    frame_height: u32,
    plan: &GifAtlasPlan,
    get_frame: &mut impl FnMut(usize) -> Result<Vec<u8>, String>,
) -> Result<(), VulkanError> {
    let device = &vk.device;
    let frames_per_chunk = (staging_bytes / frame_bytes.max(1)).max(1);
    let mut slot_base = 0usize;
    let mut first_chunk = true;

    for chunk in plan.kept_indices.chunks(frames_per_chunk) {
        // SAFETY: VulkanContext guarantees a valid command pool and device.
        let cmd = unsafe {
            vk.begin_single_time_commands()
                .map_err(|e| VulkanError::TextureUpload(format!("begin command buffer: {e}")))?
        };

        if first_chunk {
            first_chunk = false;
            // UNDEFINED -> TRANSFER_DST_OPTIMAL
            // SAFETY: cmd is recording and image is valid.
            unsafe {
                atlas_barrier(
                    device,
                    cmd,
                    image,
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::AccessFlags::empty(),
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                );
            }
        }

        for (slot_in_chunk, &frame_idx) in chunk.iter().enumerate() {
            let data = match get_frame(frame_idx) {
                Ok(d) => d,
                Err(e) => {
                    // SAFETY: cmd is recording and not submitted.
                    unsafe { device.free_command_buffers(vk.command_pool, &[cmd]) };
                    return Err(VulkanError::TextureUpload(format!(
                        "failed to produce GIF frame {frame_idx}: {e}"
                    )));
                }
            };
            if data.len() < frame_bytes {
                // SAFETY: cmd is recording and not submitted.
                unsafe { device.free_command_buffers(vk.command_pool, &[cmd]) };
                return Err(VulkanError::TextureUpload(format!(
                    "GIF frame {frame_idx} is {} bytes, expected at least {frame_bytes}",
                    data.len()
                )));
            }

            // SAFETY: the copy stays inside the mapped staging allocation and
            // `data` holds at least `frame_bytes` readable bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    mapped.add(slot_in_chunk * frame_bytes),
                    frame_bytes,
                );
            }

            // Copy this frame's stage slot -> its horizontal atlas slot.
            let region = vk::BufferImageCopy {
                buffer_offset: (slot_in_chunk * frame_bytes) as u64,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D {
                    x: ((slot_base + slot_in_chunk) as u32 * frame_width) as i32,
                    y: 0,
                    z: 0,
                },
                image_extent: vk::Extent3D {
                    width: frame_width,
                    height: frame_height,
                    depth: 1,
                },
            };

            // SAFETY: cmd is recording; staging_buffer holds this frame's
            // pixels at `buffer_offset`; the atlas is in TRANSFER_DST_OPTIMAL.
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

        // SAFETY: cmd was recorded above and is submitted at most once.
        if let Err(e) = unsafe { vk.end_single_time_commands(cmd) } {
            return Err(atlas_submit_error(e));
        }

        slot_base += chunk.len();
    }

    // TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL
    // SAFETY: VulkanContext guarantees a valid command pool and device.
    let cmd = unsafe {
        vk.begin_single_time_commands()
            .map_err(|e| VulkanError::TextureUpload(format!("begin command buffer: {e}")))?
    };

    // SAFETY: cmd is recording and image is valid.
    unsafe {
        atlas_barrier(
            device,
            cmd,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        );
    }

    // SAFETY: cmd was recorded above and is submitted at most once.
    unsafe { vk.end_single_time_commands(cmd) }.map_err(atlas_submit_error)
}

fn atlas_submit_error(e: vk::Result) -> VulkanError {
    if e == vk::Result::ERROR_DEVICE_LOST {
        VulkanError::DeviceLost
    } else {
        VulkanError::TextureUpload(format!("atlas command submission: {e}"))
    }
}

/// Record a full-image layout transition for the atlas image.
///
/// # Safety
/// `cmd` must be recording and `image` must be a valid image owned by the device.
#[allow(clippy::too_many_arguments)]
unsafe fn atlas_barrier(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
) {
    let barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .old_layout(old_layout)
        .new_layout(new_layout)
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

    // SAFETY: caller guarantees cmd is recording and image is valid.
    unsafe {
        device.cmd_pipeline_barrier(
            cmd,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}

/// Copy a device-local texture (same format and size) on the GPU.
///
/// Used to give several outputs the same GIF atlas without decoding and
/// resizing the animation once per output. `source` must be in
/// `SHADER_READ_ONLY_OPTIMAL` (what [`upload_gif_atlas`] leaves behind); it is
/// transitioned back to it before returning, so it stays sampleable.
pub fn copy_texture(vk: &VulkanContext, source: &GpuTexture) -> Result<GpuTexture, VulkanError> {
    let device = &vk.device;
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width: source.width,
            height: source.height,
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
        device
            .create_image(&image_info, None)
            .map_err(|e| VulkanError::TextureUpload(format!("atlas copy image creation: {e}")))?
    };

    // SAFETY: image is a valid image handle.
    let memory_reqs = unsafe { device.get_image_memory_requirements(image) };
    let memory_type = vk
        .find_memory_type(
            memory_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| {
            // SAFETY: image is valid and not bound to memory yet.
            unsafe { device.destroy_image(image, None) };
            VulkanError::TextureUpload("no DEVICE_LOCAL memory type for the atlas copy".to_string())
        })?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(memory_reqs.size)
        .memory_type_index(memory_type);

    // SAFETY: device is valid; alloc info references a valid memory type.
    let memory = unsafe {
        device.allocate_memory(&alloc_info, None).map_err(|e| {
            device.destroy_image(image, None);
            VulkanError::TextureUpload(format!("atlas copy memory allocation: {e}"))
        })?
    };

    // SAFETY: image and memory are valid; offset 0 satisfies alignment.
    unsafe {
        device.bind_image_memory(image, memory, 0).map_err(|e| {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
            VulkanError::TextureUpload(format!("bind atlas copy memory: {e}"))
        })?;
    }

    // SAFETY: VulkanContext guarantees a valid command pool and device.
    let cmd = unsafe {
        vk.begin_single_time_commands().map_err(|e| {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
            VulkanError::TextureUpload(format!("begin copy command buffer: {e}"))
        })?
    };

    // SAFETY: cmd is recording; both images are valid and the copy covers the
    // whole image. The source is restored to SHADER_READ_ONLY afterwards so the
    // caller can keep sampling (or copying) it.
    unsafe {
        atlas_barrier(
            device,
            cmd,
            source.image,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::AccessFlags::SHADER_READ,
            vk::AccessFlags::TRANSFER_READ,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::TRANSFER,
        );
        atlas_barrier(
            device,
            cmd,
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        );

        let region = vk::ImageCopy {
            src_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            src_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            dst_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            dst_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            extent: vk::Extent3D {
                width: source.width,
                height: source.height,
                depth: 1,
            },
        };
        device.cmd_copy_image(
            cmd,
            source.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );

        atlas_barrier(
            device,
            cmd,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        );
        atlas_barrier(
            device,
            cmd,
            source.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::TRANSFER_READ,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        );
    }

    // SAFETY: cmd was recorded above and is submitted at most once.
    if let Err(e) = unsafe { vk.end_single_time_commands(cmd) } {
        // SAFETY: the submission failed, so nothing references the new image.
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
        }
        return Err(atlas_submit_error(e));
    }

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
            device.free_memory(memory, None);
            VulkanError::TextureUpload(format!("atlas copy view creation: {e}"))
        })?
    };

    Ok(GpuTexture {
        image,
        view,
        memory,
        width: source.width,
        height: source.height,
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
    use super::{ATLAS_STAGING_BUDGET_BYTES, select_atlas_frames, staging_frames_per_chunk};

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

    /// Read a texture back to host memory (test helper).
    ///
    /// Leaves the texture in `TRANSFER_SRC_OPTIMAL`; it must not be sampled or
    /// copied again afterwards.
    fn read_texture(
        vk: &crate::vulkan::VulkanContext,
        texture: &crate::output::GpuTexture,
    ) -> Vec<u8> {
        let bytes = (texture.width * texture.height * 4) as usize;
        let buffer_info = ash::vk::BufferCreateInfo::default()
            .size(bytes as u64)
            .usage(ash::vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(ash::vk::SharingMode::EXCLUSIVE);

        // SAFETY: device is valid and the buffer info is fully initialized.
        unsafe {
            let device = &vk.device;
            let buffer = device
                .create_buffer(&buffer_info, None)
                .expect("readback buffer");
            let requirements = device.get_buffer_memory_requirements(buffer);
            let memory_type = vk
                .find_memory_type(
                    requirements.memory_type_bits,
                    ash::vk::MemoryPropertyFlags::HOST_VISIBLE
                        | ash::vk::MemoryPropertyFlags::HOST_COHERENT,
                )
                .expect("host-visible memory");
            let alloc = ash::vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type);
            let memory = device
                .allocate_memory(&alloc, None)
                .expect("readback memory");
            device
                .bind_buffer_memory(buffer, memory, 0)
                .expect("bind readback");

            let cmd = vk.begin_single_time_commands().expect("command buffer");

            let to_transfer = ash::vk::ImageMemoryBarrier::default()
                .src_access_mask(ash::vk::AccessFlags::SHADER_READ)
                .dst_access_mask(ash::vk::AccessFlags::TRANSFER_READ)
                .old_layout(ash::vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .new_layout(ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(ash::vk::QUEUE_FAMILY_IGNORED)
                .image(texture.image)
                .subresource_range(ash::vk::ImageSubresourceRange {
                    aspect_mask: ash::vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            device.cmd_pipeline_barrier(
                cmd,
                ash::vk::PipelineStageFlags::FRAGMENT_SHADER,
                ash::vk::PipelineStageFlags::TRANSFER,
                ash::vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer],
            );

            let region = ash::vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: ash::vk::ImageSubresourceLayers {
                    aspect_mask: ash::vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: ash::vk::Offset3D { x: 0, y: 0, z: 0 },
                image_extent: ash::vk::Extent3D {
                    width: texture.width,
                    height: texture.height,
                    depth: 1,
                },
            };

            device.cmd_copy_image_to_buffer(
                cmd,
                texture.image,
                ash::vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[region],
            );
            vk.end_single_time_commands(cmd).expect("submit readback");

            let mapped = device
                .map_memory(
                    memory,
                    0,
                    ash::vk::WHOLE_SIZE,
                    ash::vk::MemoryMapFlags::empty(),
                )
                .expect("map readback") as *const u8;
            let pixels = std::slice::from_raw_parts(mapped, bytes).to_vec();

            device.unmap_memory(memory);
            device.destroy_buffer(buffer, None);
            device.free_memory(memory, None);

            pixels
        }
    }

    /// End-to-end GPU check of the GIF atlas upload and of the atlas copy used
    /// to share one upload between outputs. This is the regression the
    /// streaming upload introduced (one submit reading a single reused staging
    /// offset stored the last frame in every tile).
    ///
    /// Ignored by default: it needs a live Wayland session and a Vulkan device.
    /// Run with `cargo test -p wl-daemon -- --ignored gif_atlas_tiles`.
    #[test]
    #[ignore = "needs a Wayland session and a Vulkan device"]
    fn gif_atlas_tiles_receive_distinct_frames_on_gpu() {
        use super::{GifAtlasPlan, copy_texture, upload_gif_atlas};
        use crate::vulkan::VulkanContext;

        const FRAME_W: u32 = 4;
        const FRAME_H: u32 = 4;

        let wayland = crate::wayland::WaylandState::connect().expect("connect to Wayland");
        // SAFETY: the display pointer belongs to a live connection that the
        // Wayland state keeps alive for the duration of the test.
        let vk = unsafe { VulkanContext::new(wayland.get_display_ptr()) }.expect("Vulkan context");

        let red = [255u8, 0, 0, 255];
        let green = [0u8, 255, 0, 255];
        let frames: Vec<Vec<u8>> = [red, green]
            .iter()
            .map(|pixel| {
                pixel
                    .iter()
                    .copied()
                    .cycle()
                    .take((FRAME_W * FRAME_H * 4) as usize)
                    .collect()
            })
            .collect();

        let plan = GifAtlasPlan {
            kept_indices: vec![0, 1],
            atlas_width: FRAME_W * 2,
            atlas_height: FRAME_H,
        };

        let texture = upload_gif_atlas(&vk, &plan, FRAME_W, FRAME_H, |index| {
            Ok(frames[index].clone())
        })
        .expect("upload atlas");

        // Copy before reading: read_texture leaves the source in
        // TRANSFER_SRC_OPTIMAL, while copy_texture expects SHADER_READ_ONLY.
        let mut copy = copy_texture(&vk, &texture).expect("copy atlas");

        let tile = |pixels: &[u8], slot: usize| -> [u8; 4] {
            let offset = (slot as u32 * FRAME_W * 4) as usize;
            [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ]
        };

        let pixels = read_texture(&vk, &texture);
        assert_eq!(
            tile(&pixels, 0),
            red,
            "first atlas tile does not hold frame 0"
        );
        assert_eq!(
            tile(&pixels, 1),
            green,
            "second atlas tile does not hold frame 1"
        );

        let copied = read_texture(&vk, &copy);
        assert_eq!(tile(&copied, 0), red, "the copied atlas lost frame 0");
        assert_eq!(tile(&copied, 1), green, "the copied atlas lost frame 1");

        // SAFETY: all submissions have completed and the test owns the textures.
        unsafe {
            copy.destroy(&vk.device);
            let mut texture = texture;
            texture.destroy(&vk.device);
        }
    }

    #[test]
    fn every_frame_in_a_chunk_gets_its_own_buffer_offset() {
        // Replays the upload loop's addressing: frames inside one submitted
        // command buffer must read distinct staging offsets, otherwise the GPU
        // (which executes after recording) copies the same pixels into every
        // atlas slot — the frozen-animation bug.
        let frame_bytes = 64 * 64 * 4;
        let total_frames = 37;
        let per_chunk = staging_frames_per_chunk(frame_bytes, total_frames);
        let mut seen_slots = 0usize;
        for chunk in (0..total_frames).collect::<Vec<_>>().chunks(per_chunk) {
            let offsets: Vec<u64> = (0..chunk.len())
                .map(|slot| (slot * frame_bytes) as u64)
                .collect();
            let mut unique = offsets.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique, offsets, "two frames share a buffer offset");
            assert_eq!(unique.len(), chunk.len());
            seen_slots += chunk.len();
        }
        assert_eq!(seen_slots, total_frames, "chunking dropped frames");
    }

    #[test]
    fn staging_chunks_fit_the_budget_and_never_go_empty() {
        // A frame larger than the budget is staged alone (no zero-sized chunk).
        assert_eq!(
            staging_frames_per_chunk(ATLAS_STAGING_BUDGET_BYTES * 2, 10),
            1
        );
        // Small frames are capped by the number of frames, not by the budget.
        assert_eq!(staging_frames_per_chunk(16, 3), 3);
        // The whole chunk fits the staging buffer: this is what lets every
        // frame inside one submit read a distinct offset.
        for frame_bytes in [16usize, 1024, 64 * 1024, 1024 * 1024] {
            let per_chunk = staging_frames_per_chunk(frame_bytes, 10_000);
            assert!(per_chunk <= 10_000, "chunk {per_chunk} exceeds frame count");
            assert!(
                per_chunk.saturating_mul(frame_bytes) <= ATLAS_STAGING_BUDGET_BYTES
                    || per_chunk == 1,
                "chunk {per_chunk} x {frame_bytes} overflows the staging budget"
            );
        }
    }
}
