use std::time::Instant;

use crate::output::AnimationState;

/// Create an AnimationState from decoded GIF frame info.
pub fn create_animation(
    frame_count: u32,
    frame_durations_ms: Vec<u32>,
    atlas: crate::output::GpuTexture,
    atlas_frame_width: u32,
    atlas_frame_height: u32,
) -> AnimationState {
    AnimationState {
        frame_count,
        current_frame: 0,
        frame_durations_ms,
        last_frame_time: Instant::now(),
        paused: false,
        atlas,
        atlas_frame_width,
        atlas_frame_height,
    }
}

/// Advance the animation, returning true if the frame changed.
pub fn tick(state: &mut AnimationState) -> bool {
    if state.paused || state.frame_count <= 1 {
        return false;
    }

    let elapsed_ms = state.last_frame_time.elapsed().as_millis() as u32;
    let current_duration = state
        .frame_durations_ms
        .get(state.current_frame as usize)
        .copied()
        .unwrap_or(100);

    // GIF standard: 0ms delay typically means 100ms
    let current_duration = if current_duration == 0 {
        100
    } else {
        current_duration
    };

    if elapsed_ms >= current_duration {
        state.current_frame = (state.current_frame + 1) % state.frame_count;
        state.last_frame_time = Instant::now();
        true
    } else {
        false
    }
}

/// Compute the UV offset for the current frame in a horizontal atlas.
/// Returns (u_offset, u_scale) where the frame UV is: u = u_offset + local_u * u_scale
pub fn frame_uv_offset(state: &AnimationState) -> (f32, f32) {
    let u_scale = 1.0 / state.frame_count as f32;
    let u_offset = state.current_frame as f32 * u_scale;
    (u_offset, u_scale)
}
