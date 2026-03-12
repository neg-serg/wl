//! Vulkan graphics pipeline for fullscreen wallpaper rendering.
//!
//! Renders a fullscreen quad using vertex indices (no vertex buffer) with a
//! combined image sampler for the wallpaper texture. Push constants carry
//! resize-mode parameters so the fragment shader can crop/fit on the GPU.

use ash::vk;

use super::VulkanError;

/// Maximum number of outputs (monitors) we support simultaneously.
const MAX_OUTPUTS: u32 = 8;

/// Push constants passed to the fragment shader for resize calculations.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WallpaperPushConstants {
    /// Resize strategy: 0 = crop, 1 = fit, 2 = no resize.
    pub resize_mode: u32,
    /// Image aspect ratio (width / height).
    pub img_aspect: f32,
    /// Screen aspect ratio (width / height).
    pub screen_aspect: f32,
    /// Atlas UV offset (0.0 for static images).
    pub uv_offset: f32,
    /// Atlas UV scale (1.0 for static images).
    pub uv_scale: f32,
}

/// Owns the entire Vulkan pipeline state needed to draw wallpapers.
pub struct WallpaperPipeline {
    pub render_pass: vk::RenderPass,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub pipeline_layout: vk::PipelineLayout,
    pub pipeline: vk::Pipeline,
    pub descriptor_pool: vk::DescriptorPool,
    pub sampler: vk::Sampler,
}

impl WallpaperPipeline {
    /// Create a complete graphics pipeline for wallpaper rendering.
    ///
    /// `format` must match the swapchain image format.  `vert_module` and
    /// `frag_module` are pre-loaded SPIR-V shader modules.
    pub fn new(
        device: &ash::Device,
        format: vk::Format,
        vert_module: vk::ShaderModule,
        frag_module: vk::ShaderModule,
    ) -> Result<Self, VulkanError> {
        let render_pass = Self::create_render_pass(device, format)?;
        let descriptor_set_layout = Self::create_descriptor_set_layout(device)?;
        let pipeline_layout = Self::create_pipeline_layout(device, descriptor_set_layout)?;
        let pipeline = Self::create_graphics_pipeline(
            device,
            render_pass,
            pipeline_layout,
            vert_module,
            frag_module,
        )?;
        let descriptor_pool = Self::create_descriptor_pool(device)?;
        let sampler = Self::create_sampler(device)?;

        Ok(Self {
            render_pass,
            descriptor_set_layout,
            pipeline_layout,
            pipeline,
            descriptor_pool,
            sampler,
        })
    }

    // ------------------------------------------------------------------
    // Render pass
    // ------------------------------------------------------------------

    fn create_render_pass(
        device: &ash::Device,
        format: vk::Format,
    ) -> Result<vk::RenderPass, VulkanError> {
        let color_attachment = vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_attachment_ref));

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

        let create_info = vk::RenderPassCreateInfo::default()
            .attachments(std::slice::from_ref(&color_attachment))
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency));

        // SAFETY: device is a valid logical device handle and all referenced
        // structs live on the stack for the duration of this call.
        let render_pass = unsafe {
            device
                .create_render_pass(&create_info, None)
                .map_err(|e| VulkanError::PipelineCreation(format!("render pass: {e}")))?
        };

        Ok(render_pass)
    }

    // ------------------------------------------------------------------
    // Descriptor set layout
    // ------------------------------------------------------------------

    fn create_descriptor_set_layout(
        device: &ash::Device,
    ) -> Result<vk::DescriptorSetLayout, VulkanError> {
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);

        let layout_info =
            vk::DescriptorSetLayoutCreateInfo::default().bindings(std::slice::from_ref(&binding));

        // SAFETY: device is valid; binding data lives on the stack.
        let layout = unsafe {
            device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| VulkanError::PipelineCreation(format!("descriptor set layout: {e}")))?
        };

        Ok(layout)
    }

    // ------------------------------------------------------------------
    // Pipeline layout
    // ------------------------------------------------------------------

    fn create_pipeline_layout(
        device: &ash::Device,
        descriptor_set_layout: vk::DescriptorSetLayout,
    ) -> Result<vk::PipelineLayout, VulkanError> {
        let push_constant_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<WallpaperPushConstants>() as u32);

        let layouts = [descriptor_set_layout];

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&layouts)
            .push_constant_ranges(std::slice::from_ref(&push_constant_range));

        // SAFETY: device is valid; all referenced data lives on the stack.
        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout(&layout_info, None)
                .map_err(|e| VulkanError::PipelineCreation(format!("pipeline layout: {e}")))?
        };

        Ok(pipeline_layout)
    }

    // ------------------------------------------------------------------
    // Graphics pipeline
    // ------------------------------------------------------------------

    fn create_graphics_pipeline(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        pipeline_layout: vk::PipelineLayout,
        vert_module: vk::ShaderModule,
        frag_module: vk::ShaderModule,
    ) -> Result<vk::Pipeline, VulkanError> {
        let entry_point = c"main";

        let shader_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vert_module)
                .name(entry_point),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(frag_module)
                .name(entry_point),
        ];

        // No vertex input — the fullscreen quad is generated from gl_VertexIndex.
        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default();

        let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        // Viewport and scissor are dynamic, but we must declare the count.
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .depth_bias_enable(false)
            .line_width(1.0);

        let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1)
            .sample_shading_enable(false);

        // Opaque — no blending.
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA);

        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(std::slice::from_ref(&color_blend_attachment));

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];

        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly_state)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization_state)
            .multisample_state(&multisample_state)
            .color_blend_state(&color_blend_state)
            .dynamic_state(&dynamic_state)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);

        // SAFETY: device, render_pass, pipeline_layout, and shader modules are
        // all valid Vulkan handles.  Every referenced struct lives on the stack
        // for the duration of the call.
        let pipelines = unsafe {
            device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    std::slice::from_ref(&pipeline_info),
                    None,
                )
                .map_err(|(_, e)| {
                    VulkanError::PipelineCreation(format!("graphics pipeline: {e}"))
                })?
        };

        Ok(pipelines[0])
    }

    // ------------------------------------------------------------------
    // Descriptor pool
    // ------------------------------------------------------------------

    fn create_descriptor_pool(device: &ash::Device) -> Result<vk::DescriptorPool, VulkanError> {
        let pool_size = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(MAX_OUTPUTS);

        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(MAX_OUTPUTS)
            .pool_sizes(std::slice::from_ref(&pool_size));

        // SAFETY: device is valid; pool_size lives on the stack.
        let pool = unsafe {
            device
                .create_descriptor_pool(&pool_info, None)
                .map_err(|e| VulkanError::PipelineCreation(format!("descriptor pool: {e}")))?
        };

        Ok(pool)
    }

    // ------------------------------------------------------------------
    // Sampler
    // ------------------------------------------------------------------

    fn create_sampler(device: &ash::Device) -> Result<vk::Sampler, VulkanError> {
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false)
            .max_anisotropy(1.0)
            .compare_enable(false)
            .min_lod(0.0)
            .max_lod(0.0)
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false);

        // SAFETY: device is a valid logical device handle.
        let sampler = unsafe {
            device
                .create_sampler(&sampler_info, None)
                .map_err(|e| VulkanError::PipelineCreation(format!("sampler: {e}")))?
        };

        Ok(sampler)
    }

    // ------------------------------------------------------------------
    // Public helpers
    // ------------------------------------------------------------------

    /// Allocate a single descriptor set from the internal pool.
    pub fn allocate_descriptor_set(
        &self,
        device: &ash::Device,
    ) -> Result<vk::DescriptorSet, VulkanError> {
        let layouts = [self.descriptor_set_layout];

        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);

        // SAFETY: device, descriptor_pool, and descriptor_set_layout are valid
        // handles owned by this struct.
        let sets = unsafe {
            device
                .allocate_descriptor_sets(&alloc_info)
                .map_err(|e| VulkanError::PipelineCreation(format!("descriptor set alloc: {e}")))?
        };

        Ok(sets[0])
    }

    /// Update a descriptor set to reference the given wallpaper texture.
    pub fn update_descriptor_set(
        device: &ash::Device,
        descriptor_set: vk::DescriptorSet,
        image_view: vk::ImageView,
        sampler: vk::Sampler,
    ) {
        let image_info = vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(image_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(std::slice::from_ref(&image_info));

        // SAFETY: device and descriptor_set are valid handles; image_info
        // references a valid image_view and sampler.
        unsafe {
            device.update_descriptor_sets(std::slice::from_ref(&write), &[]);
        }
    }

    /// Create a framebuffer for one swapchain image view.
    pub fn create_framebuffer(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        image_view: vk::ImageView,
        width: u32,
        height: u32,
    ) -> Result<vk::Framebuffer, VulkanError> {
        let attachments = [image_view];

        let fb_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&attachments)
            .width(width)
            .height(height)
            .layers(1);

        // SAFETY: device, render_pass, and image_view are valid Vulkan handles.
        let framebuffer = unsafe {
            device
                .create_framebuffer(&fb_info, None)
                .map_err(|e| VulkanError::PipelineCreation(format!("framebuffer: {e}")))?
        };

        Ok(framebuffer)
    }

    /// Destroy all Vulkan objects owned by this pipeline.
    pub fn destroy(&mut self, device: &ash::Device) {
        // SAFETY: All handles were created by this struct and have not been
        // destroyed yet.  The device must not be in use (caller must ensure
        // device idle before calling destroy).
        unsafe {
            device.destroy_sampler(self.sampler, None);
            device.destroy_descriptor_pool(self.descriptor_pool, None);
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            device.destroy_render_pass(self.render_pass, None);
        }
    }
}

// ======================================================================
// Transition pipeline
// ======================================================================

/// Push constants for transition fragment shaders.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TransitionPushConstants {
    pub progress: f32,
    pub angle: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub wave_x: f32,
    pub wave_y: f32,
    // Resize params so transitions match the final wallpaper scaling
    pub old_resize_mode: u32,
    pub old_img_aspect: f32,
    pub new_resize_mode: u32,
    pub new_img_aspect: f32,
    pub screen_aspect: f32,
}

/// Owns Vulkan pipeline state for transition rendering.
/// One pipeline per transition type, all sharing the same layout.
pub struct TransitionPipeline {
    pub render_pass: vk::RenderPass,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub pipeline_layout: vk::PipelineLayout,
    pub pipelines: std::collections::HashMap<TransitionKind, vk::Pipeline>,
    pub descriptor_pool: vk::DescriptorPool,
    pub sampler: vk::Sampler,
}

/// Internal transition types (excludes None and Random).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionKind {
    Fade,
    Wipe,
    Wave,
    Outer,
    Pixelate,
    Burn,
    Glitch,
    Disintegrate,
    Dreamy,
    GlitchMemories,
    Morph,
    Hexagonalize,
    Kaleidoscope,
    CrossZoom,
    FilmBurn,
    CircleCrop,
}

impl TransitionPipeline {
    /// Create transition pipelines for all transition types.
    pub fn new(
        device: &ash::Device,
        format: vk::Format,
        vert_module: vk::ShaderModule,
        frag_modules: &[(TransitionKind, vk::ShaderModule)],
    ) -> Result<Self, VulkanError> {
        let render_pass = WallpaperPipeline::create_render_pass(device, format)?;
        let descriptor_set_layout = Self::create_descriptor_set_layout(device)?;
        let pipeline_layout = Self::create_pipeline_layout(device, descriptor_set_layout)?;

        let mut pipelines = std::collections::HashMap::new();
        for &(kind, frag_module) in frag_modules {
            let pipeline = WallpaperPipeline::create_graphics_pipeline(
                device,
                render_pass,
                pipeline_layout,
                vert_module,
                frag_module,
            )?;
            pipelines.insert(kind, pipeline);
        }

        let descriptor_pool = Self::create_descriptor_pool(device)?;
        let sampler = WallpaperPipeline::create_sampler(device)?;

        Ok(Self {
            render_pass,
            descriptor_set_layout,
            pipeline_layout,
            pipelines,
            descriptor_pool,
            sampler,
        })
    }

    /// Descriptor set layout with two combined image samplers (old + new).
    fn create_descriptor_set_layout(
        device: &ash::Device,
    ) -> Result<vk::DescriptorSetLayout, VulkanError> {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);

        // SAFETY: device is valid; bindings live on the stack.
        let layout = unsafe {
            device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| {
                    VulkanError::PipelineCreation(format!("transition descriptor set layout: {e}"))
                })?
        };

        Ok(layout)
    }

    fn create_pipeline_layout(
        device: &ash::Device,
        descriptor_set_layout: vk::DescriptorSetLayout,
    ) -> Result<vk::PipelineLayout, VulkanError> {
        let push_constant_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<TransitionPushConstants>() as u32);

        let layouts = [descriptor_set_layout];

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&layouts)
            .push_constant_ranges(std::slice::from_ref(&push_constant_range));

        // SAFETY: device is valid; all referenced data lives on the stack.
        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout(&layout_info, None)
                .map_err(|e| {
                    VulkanError::PipelineCreation(format!("transition pipeline layout: {e}"))
                })?
        };

        Ok(pipeline_layout)
    }

    fn create_descriptor_pool(device: &ash::Device) -> Result<vk::DescriptorPool, VulkanError> {
        // 2 samplers per set, MAX_OUTPUTS sets
        let pool_size = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(MAX_OUTPUTS * 2);

        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .max_sets(MAX_OUTPUTS)
            .pool_sizes(std::slice::from_ref(&pool_size));

        // SAFETY: device is valid.
        let pool = unsafe {
            device
                .create_descriptor_pool(&pool_info, None)
                .map_err(|e| {
                    VulkanError::PipelineCreation(format!("transition descriptor pool: {e}"))
                })?
        };

        Ok(pool)
    }

    /// Allocate a descriptor set for a transition (dual-texture).
    pub fn allocate_descriptor_set(
        &self,
        device: &ash::Device,
    ) -> Result<vk::DescriptorSet, VulkanError> {
        let layouts = [self.descriptor_set_layout];

        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);

        // SAFETY: device, descriptor_pool, and layout are valid.
        let sets = unsafe {
            device.allocate_descriptor_sets(&alloc_info).map_err(|e| {
                VulkanError::PipelineCreation(format!("transition descriptor set alloc: {e}"))
            })?
        };

        Ok(sets[0])
    }

    /// Update a transition descriptor set with old and new texture views.
    pub fn update_descriptor_set(
        device: &ash::Device,
        descriptor_set: vk::DescriptorSet,
        old_view: vk::ImageView,
        new_view: vk::ImageView,
        sampler: vk::Sampler,
    ) {
        let old_info = vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(old_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

        let new_info = vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(new_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&old_info)),
            vk::WriteDescriptorSet::default()
                .dst_set(descriptor_set)
                .dst_binding(1)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&new_info)),
        ];

        // SAFETY: device, descriptor_set, views, and sampler are all valid.
        unsafe {
            device.update_descriptor_sets(&writes, &[]);
        }
    }

    /// Free a previously allocated descriptor set back to the pool.
    ///
    /// # Safety
    /// The descriptor set must not be in use by any command buffer.
    pub unsafe fn free_descriptor_set(
        &self,
        device: &ash::Device,
        descriptor_set: vk::DescriptorSet,
    ) {
        unsafe {
            let _ = device.free_descriptor_sets(self.descriptor_pool, &[descriptor_set]);
        }
    }

    /// Get the pipeline for a specific transition kind.
    pub fn get(&self, kind: TransitionKind) -> Option<vk::Pipeline> {
        self.pipelines.get(&kind).copied()
    }

    /// Destroy all Vulkan objects owned by this pipeline.
    pub fn destroy(&mut self, device: &ash::Device) {
        // SAFETY: All handles were created by this struct and are not in use.
        unsafe {
            device.destroy_sampler(self.sampler, None);
            device.destroy_descriptor_pool(self.descriptor_pool, None);
            for (_, pipeline) in self.pipelines.drain() {
                device.destroy_pipeline(pipeline, None);
            }
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            device.destroy_render_pass(self.render_pass, None);
        }
    }
}
