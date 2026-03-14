use ash::vk;

use crate::vulkan::pipeline::TransitionKind;
use crate::vulkan::swapchain::Swapchain;

/// Per-output state: tracks the Wayland output, Vulkan surface/swapchain,
/// and current wallpaper/transition/animation.
#[allow(dead_code)]
pub struct Output {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub refresh_rate_mhz: u32,

    // Vulkan resources
    pub swapchain: Option<Swapchain>,

    // Wallpaper state
    pub wallpaper: Option<Wallpaper>,
    pub transition: Option<TransitionState>,
    pub animation: Option<AnimationState>,

    // Pipeline resources
    pub descriptor_set: Option<vk::DescriptorSet>,
    pub framebuffers: Vec<vk::Framebuffer>,
    pub clear_color: [f32; 4],

    // Synchronization
    pub image_available_semaphore: vk::Semaphore,
    pub render_finished_semaphore: vk::Semaphore,
    pub in_flight_fence: vk::Fence,
    pub needs_redraw: bool,

    // Previous frame's command buffer, freed after fence wait
    pub last_command_buffer: Option<vk::CommandBuffer>,
}

/// A wallpaper bound to an output.
#[allow(dead_code)]
pub struct Wallpaper {
    pub source_path: String,
    pub format: wl_common::ipc_types::ImageFormat,
    pub original_dimensions: (u32, u32),
    pub display_dimensions: (u32, u32),
    pub resize_mode: wl_common::ipc_types::ResizeMode,
    pub texture: GpuTexture,
    pub is_animated: bool,
}

/// GPU-resident texture (image + view + memory).
pub struct GpuTexture {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub memory: vk::DeviceMemory,
    pub width: u32,
    pub height: u32,
}

impl GpuTexture {
    /// Destroy all GPU resources.
    ///
    /// # Safety
    /// `device` must be the device that created these resources.
    /// Resources must not be in use by any command buffer.
    pub unsafe fn destroy(&self, device: &ash::Device) {
        // SAFETY: Caller guarantees resources are not in use and device is valid.
        unsafe {
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

/// Active transition between two wallpapers.
#[allow(dead_code)]
pub struct TransitionState {
    pub transition_type: wl_common::ipc_types::TransitionType,
    pub kind: TransitionKind,
    pub duration_secs: f32,
    pub progress: f32,
    pub start_time: std::time::Instant,
    pub fps: u32,
    pub angle: f32,
    pub position: (f32, f32),
    pub bezier: [f32; 4],
    pub wave: (f32, f32),
    pub old_texture: GpuTexture,
    pub old_resize_mode: wl_common::ipc_types::ResizeMode,
    pub new_texture: GpuTexture,
    pub new_resize_mode: wl_common::ipc_types::ResizeMode,
    pub descriptor_set: Option<vk::DescriptorSet>,
}

/// Animation playback state for GIF wallpapers.
pub struct AnimationState {
    pub frame_count: u32,
    pub current_frame: u32,
    pub frame_durations_ms: Vec<u32>,
    pub last_frame_time: std::time::Instant,
    pub paused: bool,
    pub atlas: GpuTexture,
    pub atlas_frame_width: u32,
    pub atlas_frame_height: u32,
}

impl Output {
    /// Create a new Output with sync objects.
    ///
    /// # Safety
    /// `device` must be a valid Vulkan device.
    pub unsafe fn new(
        device: &ash::Device,
        name: String,
        width: u32,
        height: u32,
        scale_factor: f64,
        refresh_rate_mhz: u32,
    ) -> Result<Self, ash::vk::Result> {
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        // SAFETY: device is valid per caller contract.
        let image_available_semaphore = unsafe { device.create_semaphore(&semaphore_info, None)? };
        let render_finished_semaphore = unsafe { device.create_semaphore(&semaphore_info, None)? };
        let in_flight_fence = unsafe { device.create_fence(&fence_info, None)? };

        Ok(Self {
            name,
            width,
            height,
            scale_factor,
            refresh_rate_mhz,
            swapchain: None,
            wallpaper: None,
            transition: None,
            animation: None,
            descriptor_set: None,
            framebuffers: Vec::new(),
            clear_color: [0.0, 0.0, 0.0, 1.0],
            image_available_semaphore,
            render_finished_semaphore,
            in_flight_fence,
            needs_redraw: true,
            last_command_buffer: None,
        })
    }

    /// Effective resolution accounting for fractional scaling.
    #[allow(dead_code)]
    pub fn effective_resolution(&self) -> (u32, u32) {
        (
            (self.width as f64 * self.scale_factor).round() as u32,
            (self.height as f64 * self.scale_factor).round() as u32,
        )
    }

    /// Set a new wallpaper on this output, freeing the old one.
    ///
    /// # Safety
    /// `device` must be valid. Old wallpaper texture must not be in use by GPU.
    pub unsafe fn set_wallpaper(&mut self, device: &ash::Device, wallpaper: Wallpaper) {
        // SAFETY: Caller guarantees old texture is not in use.
        if let Some(old) = self.wallpaper.take() {
            unsafe { old.texture.destroy(device) };
        }
        self.wallpaper = Some(wallpaper);
        self.animation = None;
        self.needs_redraw = true;
    }

    /// Clear wallpaper to nothing (will render solid color).
    ///
    /// # Safety
    /// `device` must be valid. Old wallpaper texture must not be in use by GPU.
    pub unsafe fn clear_wallpaper(&mut self, device: &ash::Device) {
        // SAFETY: Caller guarantees old texture is not in use.
        if let Some(old) = self.wallpaper.take() {
            unsafe { old.texture.destroy(device) };
        }
        self.animation = None;
        self.needs_redraw = true;
    }

    /// Destroy all Vulkan resources owned by this output.
    ///
    /// # Safety
    /// `device` must be the device that created these resources.
    /// All resources must not be in use.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        // SAFETY: Caller guarantees resources not in use and device is valid.
        unsafe {
            if let Some(transition) = self.transition.take() {
                transition.old_texture.destroy(device);
                transition.new_texture.destroy(device);
            }
            if let Some(animation) = self.animation.take() {
                animation.atlas.destroy(device);
            }
            if let Some(wallpaper) = self.wallpaper.take() {
                wallpaper.texture.destroy(device);
            }
            for fb in self.framebuffers.drain(..) {
                device.destroy_framebuffer(fb, None);
            }
            if let Some(mut swapchain) = self.swapchain.take() {
                swapchain.destroy(device);
            }
            device.destroy_semaphore(self.image_available_semaphore, None);
            device.destroy_semaphore(self.render_finished_semaphore, None);
            device.destroy_fence(self.in_flight_fence, None);
        }
    }
}
