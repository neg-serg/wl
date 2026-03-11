use std::collections::HashMap;

use swww_vulkan_common::cache::{self, OutputSessionState, SessionState};
use swww_vulkan_common::ipc_types::ResizeMode;

use crate::output::Output;
use crate::vulkan::VulkanContext;
use crate::vulkan::pipeline::{TransitionPipeline, WallpaperPipeline};
use crate::vulkan::shaders::ShaderModules;

/// Global daemon state.
#[allow(dead_code)]
pub struct DaemonState {
    pub vk: VulkanContext,
    pub shaders: ShaderModules,
    pub pipeline: Option<WallpaperPipeline>,
    pub transition_pipeline: Option<TransitionPipeline>,
    pub outputs: HashMap<String, Output>,
    pub session_cache_path: std::path::PathBuf,
    pub image_cache_path: std::path::PathBuf,
    pub running: bool,
}

impl DaemonState {
    /// Persist current wallpaper state for all outputs to state.json.
    pub fn save_session(&self) -> Result<(), std::io::Error> {
        let mut state = SessionState::default();

        for (name, output) in &self.outputs {
            if let Some(ref wp) = output.wallpaper {
                state.outputs.insert(
                    name.clone(),
                    OutputSessionState {
                        wallpaper_path: wp.source_path.clone(),
                        resize_mode: match wp.resize_mode {
                            ResizeMode::Crop => "crop".to_string(),
                            ResizeMode::Fit => "fit".to_string(),
                            ResizeMode::No => "no".to_string(),
                        },
                    },
                );
            }
        }

        cache::save_session_state(&state)
    }

    /// Destroy all Vulkan resources.
    ///
    /// # Safety
    /// Must be called before dropping VulkanContext.
    /// All GPU work must be complete.
    pub unsafe fn destroy_all(&mut self) {
        // SAFETY: Caller guarantees GPU is idle.
        unsafe {
            let _ = self.vk.device.device_wait_idle();
            for (_, mut output) in self.outputs.drain() {
                output.destroy(&self.vk.device);
            }
            if let Some(mut pipeline) = self.pipeline.take() {
                pipeline.destroy(&self.vk.device);
            }
            if let Some(mut tp) = self.transition_pipeline.take() {
                tp.destroy(&self.vk.device);
            }
            self.shaders.destroy(&self.vk.device);
        }
    }
}
