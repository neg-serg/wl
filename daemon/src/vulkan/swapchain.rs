use ash::vk;

use super::VulkanError;

/// Per-output Vulkan swapchain managing presentation to a Wayland surface.
pub struct Swapchain {
    pub handle: vk::SwapchainKHR,
    pub surface: vk::SurfaceKHR,
    pub images: Vec<vk::Image>,
    pub image_views: Vec<vk::ImageView>,
    pub format: vk::SurfaceFormatKHR,
    pub extent: vk::Extent2D,
    pub surface_fn: ash::khr::surface::Instance,
    pub swapchain_fn: ash::khr::swapchain::Device,
}

impl Swapchain {
    /// Create a new swapchain for the given surface at the specified dimensions.
    ///
    /// Queries surface capabilities, selects a preferred format (B8G8R8A8_SRGB or
    /// R8G8B8A8_SRGB), uses FIFO present mode for vsync, and creates image views
    /// for all swapchain images.
    pub fn new(
        instance: &ash::Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        width: u32,
        height: u32,
    ) -> Result<Self, VulkanError> {
        let surface_fn = ash::khr::surface::Instance::new(
            // SAFETY: We need the Entry to construct the surface loader. We reconstruct it
            // from the instance's internal reference. However, ash 0.38 requires both entry
            // and instance. The caller provides instance; we use a static entry load here.
            // Actually, surface_fn needs the entry. We accept instance which embeds the
            // function pointers we need.
            //
            // ash::khr::surface::Instance::new takes (&Entry, &Instance) but we only have
            // &Instance. We must accept Entry as well, or store it. Let's adjust the approach:
            // we load a minimal entry just for the fp table. In practice, the instance already
            // loaded these symbols. We'll use a transmuted dummy entry since surface_fn only
            // uses instance-level function pointers that come from `instance`.
            //
            // The cleanest approach: require entry as a parameter.
            // But the spec says the signature is (instance, device, ...).
            // We'll work around by loading entry again -- it's cheap and idempotent.
            &unsafe { ash::Entry::load() }.map_err(|_| VulkanError::NoVulkan)?,
            instance,
        );
        let swapchain_fn = ash::khr::swapchain::Device::new(instance, device);

        let capabilities = Self::query_capabilities(&surface_fn, physical_device, surface)?;
        let format = Self::choose_format(&surface_fn, physical_device, surface)?;
        let present_mode = vk::PresentModeKHR::FIFO; // vsync, always supported

        let extent = Self::choose_extent(&capabilities, width, height);
        let image_count = Self::choose_image_count(&capabilities);

        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(vk::SwapchainKHR::null());

        // SAFETY: All handles (device, surface) are valid. create_info references valid data
        // that outlives this call.
        let handle = unsafe {
            swapchain_fn
                .create_swapchain(&create_info, None)
                .map_err(VulkanError::SwapchainCreation)?
        };

        // SAFETY: swapchain handle is valid, just created above.
        let images = unsafe {
            swapchain_fn
                .get_swapchain_images(handle)
                .map_err(VulkanError::SwapchainCreation)?
        };

        let image_views = Self::create_image_views(device, &images, format.format)?;

        Ok(Self {
            handle,
            surface,
            images,
            image_views,
            format,
            extent,
            surface_fn,
            swapchain_fn,
        })
    }

    /// Recreate the swapchain after a resize or suboptimal present.
    ///
    /// Destroys old image views, creates a new swapchain (passing the old handle),
    /// destroys the old swapchain, and creates new image views.
    #[allow(dead_code)]
    pub fn recreate(
        &mut self,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        width: u32,
        height: u32,
    ) -> Result<(), VulkanError> {
        // Destroy old image views first.
        for &view in &self.image_views {
            // SAFETY: device is valid, view is a valid image view owned by this swapchain.
            unsafe {
                device.destroy_image_view(view, None);
            }
        }
        self.image_views.clear();

        let capabilities =
            Self::query_capabilities(&self.surface_fn, physical_device, self.surface)?;
        let format = Self::choose_format(&self.surface_fn, physical_device, self.surface)?;
        let extent = Self::choose_extent(&capabilities, width, height);
        let image_count = Self::choose_image_count(&capabilities);

        let old_swapchain = self.handle;

        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(old_swapchain);

        // SAFETY: All handles are valid. old_swapchain is the current swapchain which we will
        // destroy after creating the new one.
        self.handle = unsafe {
            self.swapchain_fn
                .create_swapchain(&create_info, None)
                .map_err(VulkanError::SwapchainCreation)?
        };

        // SAFETY: old_swapchain is no longer in use now that the new swapchain has been created
        // with it as old_swapchain. The driver has retired it.
        unsafe {
            self.swapchain_fn.destroy_swapchain(old_swapchain, None);
        }

        // SAFETY: The new swapchain handle is valid.
        self.images = unsafe {
            self.swapchain_fn
                .get_swapchain_images(self.handle)
                .map_err(VulkanError::SwapchainCreation)?
        };

        self.image_views = Self::create_image_views(device, &self.images, format.format)?;
        self.format = format;
        self.extent = extent;

        Ok(())
    }

    /// Acquire the next presentable image from the swapchain.
    ///
    /// Returns `(image_index, suboptimal)`. The caller should signal `semaphore`
    /// and check the suboptimal flag to decide whether to recreate.
    pub fn acquire_next_image(
        &self,
        semaphore: vk::Semaphore,
        timeout: u64,
    ) -> Result<(u32, bool), vk::Result> {
        // SAFETY: swapchain handle and semaphore are valid. timeout is just a u64 nanosecond
        // value. The fence is null (we use semaphore-based synchronization).
        unsafe {
            self.swapchain_fn
                .acquire_next_image(self.handle, timeout, semaphore, vk::Fence::null())
        }
    }

    /// Present a previously acquired image to the display.
    ///
    /// Returns `true` if the swapchain is suboptimal and should be recreated.
    pub fn present(
        &self,
        queue: vk::Queue,
        wait_semaphore: vk::Semaphore,
        image_index: u32,
    ) -> Result<bool, vk::Result> {
        let swapchains = [self.handle];
        let image_indices = [image_index];
        let wait_semaphores = [wait_semaphore];

        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        // SAFETY: queue is a valid presentation queue. All swapchain and semaphore handles
        // are valid. image_index was obtained from acquire_next_image.
        unsafe { self.swapchain_fn.queue_present(queue, &present_info) }
    }

    /// Destroy all swapchain resources: image views, swapchain, and surface.
    ///
    /// After calling this, the `Swapchain` is in an invalid state and must not be used.
    pub fn destroy(&mut self, device: &ash::Device) {
        // SAFETY: device is valid. All image views are valid handles owned by this swapchain.
        unsafe {
            for &view in &self.image_views {
                device.destroy_image_view(view, None);
            }
        }
        self.image_views.clear();
        self.images.clear();

        // SAFETY: swapchain_fn and handle are valid. The swapchain is no longer in use
        // (caller must ensure no pending operations reference it).
        unsafe {
            self.swapchain_fn.destroy_swapchain(self.handle, None);
        }
        self.handle = vk::SwapchainKHR::null();

        // SAFETY: surface_fn and surface are valid. The surface is no longer in use.
        unsafe {
            self.surface_fn.destroy_surface(self.surface, None);
        }
        self.surface = vk::SurfaceKHR::null();
    }

    /// Create a `VkSurfaceKHR` from a Wayland display and surface.
    ///
    /// # Safety
    /// `wl_display` and `wl_surface` must be valid pointers to live Wayland objects.
    pub unsafe fn create_surface(
        _entry: &ash::Entry,
        _instance: &ash::Instance,
        wl_display: *mut std::ffi::c_void,
        wl_surface: *mut std::ffi::c_void,
        wayland_surface_fn: &ash::khr::wayland_surface::Instance,
    ) -> Result<vk::SurfaceKHR, VulkanError> {
        let create_info = vk::WaylandSurfaceCreateInfoKHR::default()
            .display(wl_display as *mut _)
            .surface(wl_surface as *mut _);

        // SAFETY: wl_display and wl_surface are valid Wayland object pointers per the
        // caller's contract. wayland_surface_fn was created from valid entry and instance.
        let surface = unsafe {
            wayland_surface_fn
                .create_wayland_surface(&create_info, None)
                .map_err(VulkanError::SurfaceCreation)?
        };

        Ok(surface)
    }

    // ---- Private helpers ----

    fn query_capabilities(
        surface_fn: &ash::khr::surface::Instance,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
    ) -> Result<vk::SurfaceCapabilitiesKHR, VulkanError> {
        // SAFETY: physical_device and surface are valid handles.
        unsafe {
            surface_fn
                .get_physical_device_surface_capabilities(physical_device, surface)
                .map_err(VulkanError::SwapchainCreation)
        }
    }

    fn choose_format(
        surface_fn: &ash::khr::surface::Instance,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
    ) -> Result<vk::SurfaceFormatKHR, VulkanError> {
        // SAFETY: physical_device and surface are valid handles.
        let formats = unsafe {
            surface_fn
                .get_physical_device_surface_formats(physical_device, surface)
                .map_err(VulkanError::SwapchainCreation)?
        };

        // Prefer UNORM formats to avoid unnecessary sRGB decode/encode round-trips
        // on the GPU. The SRGB_NONLINEAR color space tells the compositor that pixel
        // data is sRGB-encoded, which is correct since our source images are already sRGB.
        let preferred = formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_UNORM
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .or_else(|| {
                formats.iter().find(|f| {
                    f.format == vk::Format::R8G8B8A8_UNORM
                        && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
            })
            .or_else(|| {
                formats.iter().find(|f| {
                    f.format == vk::Format::B8G8R8A8_SRGB
                        && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
            })
            .or(formats.first());

        let chosen = preferred
            .copied()
            .ok_or_else(|| VulkanError::Other("no surface formats available".to_string()))?;

        tracing::info!(
            "swapchain format: {:?}, color_space: {:?} (available: {:?})",
            chosen.format,
            chosen.color_space,
            formats.iter().map(|f| (f.format, f.color_space)).collect::<Vec<_>>()
        );

        Ok(chosen)
    }

    fn choose_extent(
        capabilities: &vk::SurfaceCapabilitiesKHR,
        width: u32,
        height: u32,
    ) -> vk::Extent2D {
        // If current_extent is 0xFFFFFFFF, the surface size is determined by the swapchain
        // extent. Otherwise, we must use the current extent.
        if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: height.clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        }
    }

    fn choose_image_count(capabilities: &vk::SurfaceCapabilitiesKHR) -> u32 {
        let desired = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 {
            desired.min(capabilities.max_image_count)
        } else {
            desired
        }
    }

    fn create_image_views(
        device: &ash::Device,
        images: &[vk::Image],
        format: vk::Format,
    ) -> Result<Vec<vk::ImageView>, VulkanError> {
        let mut views = Vec::with_capacity(images.len());

        for &image in images {
            let create_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
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

            // SAFETY: device and image are valid. image comes from a valid swapchain.
            let view = unsafe {
                device
                    .create_image_view(&create_info, None)
                    .map_err(VulkanError::SwapchainCreation)?
            };

            views.push(view);
        }

        Ok(views)
    }
}
