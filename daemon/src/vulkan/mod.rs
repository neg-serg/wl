use ash::vk;
use std::ffi::{CStr, CString};

pub mod pipeline;
pub mod shaders;
pub mod swapchain;
pub mod texture;

/// Core Vulkan state shared across the daemon.
#[allow(dead_code)]
pub struct VulkanContext {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub physical_device_properties: vk::PhysicalDeviceProperties,
    pub physical_device_memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub device: ash::Device,
    pub graphics_queue: vk::Queue,
    pub graphics_queue_family: u32,
    pub transfer_queue: Option<vk::Queue>,
    pub transfer_queue_family: Option<u32>,
    pub command_pool: vk::CommandPool,
    pub wayland_surface_fn: ash::khr::wayland_surface::Instance,
}

impl VulkanContext {
    /// Initialize Vulkan: create instance with wayland_surface extension,
    /// select physical device, create logical device with graphics queue,
    /// and create command pool.
    ///
    /// # Safety
    /// `wl_display` must be a valid pointer to a connected wl_display.
    pub unsafe fn new(wl_display: *mut std::ffi::c_void) -> Result<Self, VulkanError> {
        // SAFETY: Loading the Vulkan library. This is safe if a Vulkan ICD is installed.
        let entry = unsafe { ash::Entry::load().map_err(|_| VulkanError::NoVulkan)? };

        let instance = Self::create_instance(&entry)?;
        let wayland_surface_fn = ash::khr::wayland_surface::Instance::new(&entry, &instance);

        let (physical_device, graphics_queue_family, transfer_queue_family) =
            Self::select_physical_device(&instance, wl_display, &wayland_surface_fn)?;

        let physical_device_properties =
            // SAFETY: instance and physical_device are valid handles created above.
            unsafe { instance.get_physical_device_properties(physical_device) };
        let physical_device_memory_properties =
            // SAFETY: instance and physical_device are valid handles created above.
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let (device, graphics_queue, transfer_queue) = Self::create_device(
            &instance,
            physical_device,
            graphics_queue_family,
            transfer_queue_family,
        )?;

        let command_pool = Self::create_command_pool(&device, graphics_queue_family)?;

        Ok(Self {
            entry,
            instance,
            physical_device,
            physical_device_properties,
            physical_device_memory_properties,
            device,
            graphics_queue,
            graphics_queue_family,
            transfer_queue,
            transfer_queue_family,
            command_pool,
            wayland_surface_fn,
        })
    }

    fn create_instance(entry: &ash::Entry) -> Result<ash::Instance, VulkanError> {
        let app_name = CString::new("swww-vulkan-daemon").unwrap();
        let engine_name = CString::new("swww-vulkan").unwrap();

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_1);

        let extensions = [
            ash::khr::surface::NAME.as_ptr(),
            ash::khr::wayland_surface::NAME.as_ptr(),
        ];

        let mut layer_names: Vec<*const i8> = Vec::new();

        // Enable validation layers if VK_INSTANCE_LAYERS is set
        let validation_layer = CString::new("VK_LAYER_KHRONOS_validation").unwrap();
        let enable_validation = std::env::var("VK_INSTANCE_LAYERS")
            .map(|v| v.contains("VK_LAYER_KHRONOS_validation"))
            .unwrap_or(false);
        if enable_validation {
            layer_names.push(validation_layer.as_ptr());
        }

        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extensions)
            .enabled_layer_names(&layer_names);

        // SAFETY: All pointers in create_info reference valid CStrings that outlive this call.
        let instance = unsafe {
            entry
                .create_instance(&create_info, None)
                .map_err(VulkanError::InstanceCreation)?
        };

        Ok(instance)
    }

    fn select_physical_device(
        instance: &ash::Instance,
        wl_display: *mut std::ffi::c_void,
        wayland_surface_fn: &ash::khr::wayland_surface::Instance,
    ) -> Result<(vk::PhysicalDevice, u32, Option<u32>), VulkanError> {
        // SAFETY: instance is a valid handle.
        let physical_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(VulkanError::DeviceEnumeration)?
        };

        if physical_devices.is_empty() {
            return Err(VulkanError::NoSuitableDevice);
        }

        for &pdev in &physical_devices {
            // SAFETY: instance and pdev are valid handles.
            let queue_families =
                unsafe { instance.get_physical_device_queue_family_properties(pdev) };

            let mut graphics_family = None;
            let mut transfer_family = None;

            for (i, qf) in queue_families.iter().enumerate() {
                let i = i as u32;

                // Check Wayland presentation support
                // SAFETY: pdev is valid, wl_display is a valid wl_display pointer per caller contract.
                // SAFETY: pdev is valid, wl_display is a valid wl_display pointer per caller contract.
                let wayland_support = unsafe {
                    wayland_surface_fn.get_physical_device_wayland_presentation_support(
                        pdev,
                        i,
                        &mut *wl_display,
                    )
                };

                if qf.queue_flags.contains(vk::QueueFlags::GRAPHICS) && wayland_support {
                    graphics_family = Some(i);
                } else if qf.queue_flags.contains(vk::QueueFlags::TRANSFER)
                    && !qf.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                {
                    transfer_family = Some(i);
                }
            }

            if let Some(gf) = graphics_family {
                // Check swapchain extension support
                // SAFETY: pdev is a valid physical device handle.
                let extensions = unsafe {
                    instance
                        .enumerate_device_extension_properties(pdev)
                        .unwrap_or_default()
                };

                let has_swapchain = extensions.iter().any(|ext| {
                    // SAFETY: Extension name is a valid C string from the driver.
                    let name = unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) };
                    name == ash::khr::swapchain::NAME
                });

                if has_swapchain {
                    return Ok((pdev, gf, transfer_family));
                }
            }
        }

        Err(VulkanError::NoSuitableDevice)
    }

    fn create_device(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        graphics_queue_family: u32,
        transfer_queue_family: Option<u32>,
    ) -> Result<(ash::Device, vk::Queue, Option<vk::Queue>), VulkanError> {
        let queue_priority = [1.0_f32];

        let mut queue_create_infos = vec![
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(graphics_queue_family)
                .queue_priorities(&queue_priority),
        ];

        if let Some(tf) = transfer_queue_family
            && tf != graphics_queue_family
        {
            queue_create_infos.push(
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(tf)
                    .queue_priorities(&queue_priority),
            );
        }

        let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions);

        // SAFETY: instance, physical_device are valid. queue_create_infos reference valid data.
        let device = unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(VulkanError::DeviceCreation)?
        };

        // SAFETY: device is valid, queue family index and queue index 0 are valid.
        let graphics_queue = unsafe { device.get_device_queue(graphics_queue_family, 0) };

        let transfer_queue = transfer_queue_family.map(|tf| {
            // SAFETY: device is valid, transfer queue family was requested in device creation.
            unsafe { device.get_device_queue(tf, 0) }
        });

        Ok((device, graphics_queue, transfer_queue))
    }

    fn create_command_pool(
        device: &ash::Device,
        queue_family: u32,
    ) -> Result<vk::CommandPool, VulkanError> {
        let pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(queue_family);

        // SAFETY: device is valid, queue_family is a valid queue family index.
        let pool = unsafe {
            device
                .create_command_pool(&pool_info, None)
                .map_err(VulkanError::CommandPoolCreation)?
        };

        Ok(pool)
    }

    /// Find a memory type index matching the requirements.
    pub fn find_memory_type(
        &self,
        type_filter: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        (0..self.physical_device_memory_properties.memory_type_count).find(|&i| {
            (type_filter & (1 << i)) != 0
                && self.physical_device_memory_properties.memory_types[i as usize]
                    .property_flags
                    .contains(properties)
        })
    }

    /// Allocate and begin a single-use command buffer.
    ///
    /// # Safety
    /// Caller must end and submit the returned command buffer.
    pub unsafe fn begin_single_time_commands(&self) -> Result<vk::CommandBuffer, vk::Result> {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        // SAFETY: device and command_pool are valid.
        let cmd = unsafe { self.device.allocate_command_buffers(&alloc_info)?[0] };

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        // SAFETY: cmd is a valid, newly allocated command buffer.
        unsafe {
            self.device.begin_command_buffer(cmd, &begin_info)?;
        }

        Ok(cmd)
    }

    /// End, submit, and wait for a single-use command buffer.
    ///
    /// # Safety
    /// `cmd` must be a recording command buffer from `begin_single_time_commands`.
    pub unsafe fn end_single_time_commands(
        &self,
        cmd: vk::CommandBuffer,
    ) -> Result<(), vk::Result> {
        // SAFETY: cmd is a valid recording command buffer per caller contract.
        unsafe {
            self.device.end_command_buffer(cmd)?;
        }

        let submit_info = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd));

        // SAFETY: graphics_queue and cmd are valid. We wait immediately after submit.
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], vk::Fence::null())?;
            self.device.queue_wait_idle(self.graphics_queue)?;
            self.device.free_command_buffers(self.command_pool, &[cmd]);
        }

        Ok(())
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        // SAFETY: All Vulkan handles are valid and owned by this struct.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum VulkanError {
    NoVulkan,
    InstanceCreation(vk::Result),
    DeviceEnumeration(vk::Result),
    NoSuitableDevice,
    DeviceCreation(vk::Result),
    CommandPoolCreation(vk::Result),
    SurfaceCreation(vk::Result),
    SwapchainCreation(vk::Result),
    ShaderLoad(String),
    PipelineCreation(String),
    TextureUpload(String),
    DeviceLost,
    Other(String),
}

impl std::fmt::Display for VulkanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoVulkan => write!(f, "failed to load Vulkan library"),
            Self::InstanceCreation(e) => write!(f, "Vulkan instance creation failed: {e}"),
            Self::DeviceEnumeration(e) => write!(f, "failed to enumerate devices: {e}"),
            Self::NoSuitableDevice => {
                write!(f, "no Vulkan device with Wayland + swapchain support found")
            }
            Self::DeviceCreation(e) => write!(f, "Vulkan device creation failed: {e}"),
            Self::CommandPoolCreation(e) => write!(f, "command pool creation failed: {e}"),
            Self::SurfaceCreation(e) => write!(f, "surface creation failed: {e}"),
            Self::SwapchainCreation(e) => write!(f, "swapchain creation failed: {e}"),
            Self::ShaderLoad(msg) => write!(f, "shader load failed: {msg}"),
            Self::PipelineCreation(msg) => write!(f, "pipeline creation failed: {msg}"),
            Self::TextureUpload(msg) => write!(f, "texture upload failed: {msg}"),
            Self::DeviceLost => write!(f, "Vulkan device lost"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for VulkanError {}
