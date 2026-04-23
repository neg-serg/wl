use ash::vk;

use super::{VulkanContext, VulkanError};
use crate::output::GpuTexture;

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
    let mut available = 0u64;
    for i in 0..vk.physical_device_memory_properties.memory_type_count {
        let heap_idx = vk.physical_device_memory_properties.memory_types[i as usize].heap_index;
        let heap = vk.physical_device_memory_properties.memory_heaps[heap_idx as usize];
        if heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL) {
            available = available.max(heap.size);
        }
    }
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

/// Maximum GIF atlas size in bytes (128 MiB).
const MAX_GIF_ATLAS_BYTES: u64 = 128 * 1024 * 1024;

/// Upload multiple GIF frames as a horizontal atlas texture.
///
/// Frames are packed left-to-right in a single row. The atlas width = frame_width * frame_count.
/// Each frame must be `frame_width * frame_height * 4` bytes of RGBA8 data.
///
/// If the atlas would exceed `MAX_GIF_ATLAS_BYTES` or the GPU's maximum texture dimension,
/// frames are evenly sampled down to fit within the limits.
pub fn upload_gif_atlas(
    vk: &VulkanContext,
    frames: &[Vec<u8>],
    frame_width: u32,
    frame_height: u32,
) -> Result<(GpuTexture, Vec<usize>), VulkanError> {
    let max_dim = vk.physical_device_properties.limits.max_image_dimension2_d;
    let frame_bytes = (frame_width as u64) * (frame_height as u64) * 4;

    // Calculate maximum number of frames that fit within limits
    let max_by_dim = if frame_width > 0 {
        (max_dim / frame_width) as usize
    } else {
        frames.len()
    };
    let max_by_mem = if frame_bytes > 0 {
        (MAX_GIF_ATLAS_BYTES / frame_bytes) as usize
    } else {
        frames.len()
    };
    let max_frames = max_by_dim.min(max_by_mem).max(1);

    // Sample frames evenly if we need to drop some
    let (selected_frames, selected_indices): (Vec<&Vec<u8>>, Vec<usize>) =
        if frames.len() > max_frames {
            tracing::info!(
                total_frames = frames.len(),
                max_frames,
                max_by_dim,
                max_by_mem,
                "GIF atlas: sampling frames to fit VRAM/dimension limits"
            );
            let step = frames.len() as f64 / max_frames as f64;
            (0..max_frames)
                .map(|i| {
                    let idx = (i as f64 * step).floor() as usize;
                    (&frames[idx], idx)
                })
                .unzip()
        } else {
            frames.iter().enumerate().map(|(i, f)| (f, i)).unzip()
        };

    let frame_count = selected_frames.len() as u32;
    let atlas_width = frame_width * frame_count;
    let atlas_height = frame_height;

    // Build atlas pixel data: horizontal strip
    let row_bytes = (frame_width * 4) as usize;
    let atlas_row_bytes = (atlas_width * 4) as usize;
    let mut atlas_data = vec![0u8; (atlas_width as usize) * (atlas_height as usize) * 4];

    for (frame_idx, frame_data) in selected_frames.iter().enumerate() {
        let x_offset = frame_idx * row_bytes;
        for y in 0..frame_height as usize {
            let src_start = y * row_bytes;
            let dst_start = y * atlas_row_bytes + x_offset;
            if src_start + row_bytes <= frame_data.len()
                && dst_start + row_bytes <= atlas_data.len()
            {
                atlas_data[dst_start..dst_start + row_bytes]
                    .copy_from_slice(&frame_data[src_start..src_start + row_bytes]);
            }
        }
    }

    let texture = upload_rgba8_texture(vk, &atlas_data, atlas_width, atlas_height)?;
    Ok((texture, selected_indices))
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
