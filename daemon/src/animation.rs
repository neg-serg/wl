use std::time::{Duration, Instant};

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
///
/// The clock is carried forward by exactly the durations that were played, not
/// reset to "now": resetting dropped the remainder of every tick, so a frame
/// with a 100 ms delay displayed for ~112 ms at a 16 ms loop tick (12% slow,
/// growing without bound), and a stall only ever advanced playback by one frame
/// per tick afterwards.
pub fn tick(state: &mut AnimationState) -> bool {
    if state.paused || state.frame_count <= 1 {
        return false;
    }

    let mut remaining = state.last_frame_time.elapsed();
    let mut advanced = false;

    // At most one full cycle: durations are clamped to >= 1 ms below, so this
    // always terminates even if the process was suspended for hours.
    for _ in 0..state.frame_count {
        let duration = Duration::from_millis(u64::from(frame_duration_ms(state)));
        if remaining < duration {
            break;
        }
        remaining -= duration;
        state.current_frame = (state.current_frame + 1) % state.frame_count;
        state.last_frame_time += duration;
        advanced = true;
    }

    advanced
}

/// Display duration of the current frame, with GIF's conventions applied.
fn frame_duration_ms(state: &AnimationState) -> u32 {
    match state
        .frame_durations_ms
        .get(state.current_frame as usize)
        .copied()
    {
        // GIF standard: 0 ms (and a missing entry) means "use ~10 fps".
        Some(0) | None => 100,
        Some(ms) => ms,
    }
}

/// Compute the UV offset for the current frame in a horizontal atlas.
/// Returns (u_offset, u_scale) where the frame UV is: u = u_offset + local_u * u_scale
pub fn frame_uv_offset(state: &AnimationState) -> (f32, f32) {
    // `frame_count` is never 0 for a real animation, but a division by zero
    // here would turn into NaN UVs in the shader rather than an obvious bug.
    let u_scale = 1.0 / state.frame_count.max(1) as f32;
    let u_offset = state.current_frame as f32 * u_scale;
    (u_offset, u_scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::GpuTexture;

    fn texture() -> GpuTexture {
        GpuTexture {
            image: ash::vk::Image::null(),
            view: ash::vk::ImageView::null(),
            memory: ash::vk::DeviceMemory::null(),
            width: 1,
            height: 1,
        }
    }

    fn state(durations: &[u32]) -> AnimationState {
        AnimationState {
            frame_count: durations.len() as u32,
            current_frame: 0,
            frame_durations_ms: durations.to_vec(),
            last_frame_time: Instant::now(),
            paused: false,
            atlas: texture(),
            atlas_frame_width: 1,
            atlas_frame_height: 1,
        }
    }

    #[test]
    fn does_not_advance_before_the_frame_elapsed() {
        let mut anim = state(&[200, 200]);
        anim.last_frame_time = Instant::now() - Duration::from_millis(50);
        assert!(!tick(&mut anim));
        assert_eq!(anim.current_frame, 0);
    }

    #[test]
    fn advances_by_whole_durations_and_carries_the_remainder() {
        let mut anim = state(&[200, 200, 200, 200]);
        // 450 ms covers two 200 ms frames and leaves ~50 ms on the clock.
        anim.last_frame_time = Instant::now() - Duration::from_millis(450);
        assert!(tick(&mut anim));
        assert_eq!(anim.current_frame, 2);

        // Another 160 ms pushes the carried 50 ms over the next 200 ms frame;
        // the old implementation would have needed a fresh 200 ms tick.
        anim.last_frame_time -= Duration::from_millis(160);
        assert!(tick(&mut anim));
        assert_eq!(anim.current_frame, 3);
    }

    #[test]
    fn a_long_stall_catches_up_without_spinning_forever() {
        let mut anim = state(&[100, 100, 100]);
        anim.last_frame_time = Instant::now() - Duration::from_millis(10_000);
        assert!(tick(&mut anim));
        // At most one full cycle per tick, and the animation stays consistent.
        assert!(anim.current_frame < anim.frame_count);
    }

    #[test]
    fn zero_duration_frames_are_treated_as_100ms() {
        let mut anim = state(&[0, 0]);
        assert_eq!(frame_duration_ms(&anim), 100);
        anim.current_frame = 1;
        assert_eq!(frame_duration_ms(&anim), 100);
    }

    #[test]
    fn uv_offset_is_finite_for_single_frame_atlases() {
        let mut anim = state(&[100]);
        anim.frame_count = 0;
        let (offset, scale) = frame_uv_offset(&anim);
        assert!(offset.is_finite() && scale.is_finite(), "{offset} {scale}");
    }
}
