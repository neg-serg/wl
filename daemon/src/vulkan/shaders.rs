use ash::vk;
use std::collections::HashMap;

use super::VulkanError;

/// Manages loaded SPIR-V shader modules.
pub struct ShaderModules {
    modules: HashMap<String, vk::ShaderModule>,
}

// SPIR-V bytecode included at compile time from build.rs output.
macro_rules! include_shader {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/shaders/", $name))
    };
}

/// Known shader names and their SPIR-V bytecode.
fn builtin_shaders() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("wallpaper.vert", include_shader!("wallpaper.vert.spv")),
        ("wallpaper.frag", include_shader!("wallpaper.frag.spv")),
        (
            "transition_wipe.frag",
            include_shader!("transition_wipe.frag.spv"),
        ),
        (
            "transition_wave.frag",
            include_shader!("transition_wave.frag.spv"),
        ),
        (
            "transition_outer.frag",
            include_shader!("transition_outer.frag.spv"),
        ),
        (
            "transition_pixelate.frag",
            include_shader!("transition_pixelate.frag.spv"),
        ),
        (
            "transition_burn.frag",
            include_shader!("transition_burn.frag.spv"),
        ),
        (
            "transition_glitch.frag",
            include_shader!("transition_glitch.frag.spv"),
        ),
        (
            "transition_disintegrate.frag",
            include_shader!("transition_disintegrate.frag.spv"),
        ),
        (
            "transition_dreamy.frag",
            include_shader!("transition_dreamy.frag.spv"),
        ),
        (
            "transition_glitch_memories.frag",
            include_shader!("transition_glitch_memories.frag.spv"),
        ),
        (
            "transition_morph.frag",
            include_shader!("transition_morph.frag.spv"),
        ),
        (
            "transition_hexagonalize.frag",
            include_shader!("transition_hexagonalize.frag.spv"),
        ),
        (
            "transition_cross_zoom.frag",
            include_shader!("transition_cross_zoom.frag.spv"),
        ),
        (
            "transition_fluid_distortion.frag",
            include_shader!("transition_fluid_distortion.frag.spv"),
        ),
        (
            "transition_fluid_drain.frag",
            include_shader!("transition_fluid_drain.frag.spv"),
        ),
        (
            "transition_fluid_ripple.frag",
            include_shader!("transition_fluid_ripple.frag.spv"),
        ),
        (
            "transition_fluid_vortex.frag",
            include_shader!("transition_fluid_vortex.frag.spv"),
        ),
        (
            "transition_fluid_wave.frag",
            include_shader!("transition_fluid_wave.frag.spv"),
        ),
        (
            "transition_ink_bleed.frag",
            include_shader!("transition_ink_bleed.frag.spv"),
        ),
        (
            "transition_lava_lamp.frag",
            include_shader!("transition_lava_lamp.frag.spv"),
        ),
        (
            "transition_chromatic_aberration.frag",
            include_shader!("transition_chromatic_aberration.frag.spv"),
        ),
        (
            "transition_lens_distortion.frag",
            include_shader!("transition_lens_distortion.frag.spv"),
        ),
        (
            "transition_crt_shutdown.frag",
            include_shader!("transition_crt_shutdown.frag.spv"),
        ),
        (
            "transition_perlin_wipe.frag",
            include_shader!("transition_perlin_wipe.frag.spv"),
        ),
        (
            "transition_radial_blur.frag",
            include_shader!("transition_radial_blur.frag.spv"),
        ),
    ]
}

impl ShaderModules {
    /// Load all built-in SPIR-V shader modules.
    ///
    /// # Safety
    /// `device` must be a valid Vulkan logical device.
    pub unsafe fn load_builtins(device: &ash::Device) -> Result<Self, VulkanError> {
        let mut modules = HashMap::new();

        for (name, spv_bytes) in builtin_shaders() {
            // SAFETY: device is valid per caller contract.
            let module = unsafe {
                Self::create_module(device, spv_bytes).map_err(|_| {
                    VulkanError::ShaderLoad(format!("failed to create shader module: {name}"))
                })?
            };
            modules.insert(name.to_string(), module);
        }

        Ok(Self { modules })
    }

    /// Load additional transition shaders (called when transition shaders are compiled).
    ///
    /// # Safety
    /// `device` must be a valid Vulkan logical device.
    #[allow(dead_code)]
    pub unsafe fn load_transition_shader(
        &mut self,
        device: &ash::Device,
        name: &str,
        spv_bytes: &[u8],
    ) -> Result<(), VulkanError> {
        // SAFETY: device is valid per caller contract.
        let module = unsafe {
            Self::create_module(device, spv_bytes).map_err(|_| {
                VulkanError::ShaderLoad(format!("failed to create shader module: {name}"))
            })?
        };
        self.modules.insert(name.to_string(), module);
        Ok(())
    }

    /// Get a loaded shader module by name.
    pub fn get(&self, name: &str) -> Option<vk::ShaderModule> {
        self.modules.get(name).copied()
    }

    unsafe fn create_module(
        device: &ash::Device,
        spv_bytes: &[u8],
    ) -> Result<vk::ShaderModule, vk::Result> {
        // SPIR-V must be aligned to 4 bytes. The include_bytes! macro doesn't guarantee
        // alignment, so we copy into a properly aligned buffer.
        assert!(
            spv_bytes.len().is_multiple_of(4),
            "SPIR-V bytecode must be a multiple of 4 bytes"
        );

        let code: Vec<u32> = spv_bytes
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        let create_info = vk::ShaderModuleCreateInfo::default().code(&code);

        // SAFETY: device is valid, code contains valid SPIR-V (compiled by glslc at build time).
        unsafe { device.create_shader_module(&create_info, None) }
    }

    /// Destroy all loaded shader modules.
    ///
    /// # Safety
    /// `device` must be the same device used to create the modules.
    /// Modules must not be in use by any pipeline.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for (_, module) in self.modules.drain() {
            // SAFETY: module was created with this device and is not in use per caller contract.
            unsafe {
                device.destroy_shader_module(module, None);
            }
        }
    }
}
