use std::time::Instant;

use getrandom::fill;
use wl_common::ipc_types::{TransitionParams, TransitionType};

use crate::output::{GpuTexture, TransitionState};
use crate::vulkan::pipeline::TransitionKind;

/// Resolve a TransitionType to a concrete TransitionKind.
/// Random picks uniformly from all types (excludes Wipe).
pub fn resolve_kind(tt: TransitionType) -> Option<TransitionKind> {
    match tt {
        TransitionType::Wipe => Some(TransitionKind::Wipe),
        TransitionType::Wave => Some(TransitionKind::Wave),
        TransitionType::Outer => Some(TransitionKind::Outer),
        TransitionType::Pixelate => Some(TransitionKind::Pixelate),
        TransitionType::Burn => Some(TransitionKind::Burn),
        TransitionType::Glitch => Some(TransitionKind::Glitch),
        TransitionType::Disintegrate => Some(TransitionKind::Disintegrate),
        TransitionType::Dreamy => Some(TransitionKind::Dreamy),
        TransitionType::GlitchMemories => Some(TransitionKind::GlitchMemories),
        TransitionType::Morph => Some(TransitionKind::Morph),
        TransitionType::Hexagonalize => Some(TransitionKind::Hexagonalize),
        TransitionType::CrossZoom => Some(TransitionKind::CrossZoom),
        TransitionType::FluidDistortion => Some(TransitionKind::FluidDistortion),
        TransitionType::FluidDrain => Some(TransitionKind::FluidDrain),
        TransitionType::FluidRipple => Some(TransitionKind::FluidRipple),
        TransitionType::FluidVortex => Some(TransitionKind::FluidVortex),
        TransitionType::FluidWave => Some(TransitionKind::FluidWave),
        TransitionType::InkBleed => Some(TransitionKind::InkBleed),
        TransitionType::LavaLamp => Some(TransitionKind::LavaLamp),
        TransitionType::ChromaticAberration => Some(TransitionKind::ChromaticAberration),
        TransitionType::LensDistortion => Some(TransitionKind::LensDistortion),
        TransitionType::CrtShutdown => Some(TransitionKind::CrtShutdown),
        TransitionType::PerlinWipe => Some(TransitionKind::PerlinWipe),
        TransitionType::RadialBlur => Some(TransitionKind::RadialBlur),
        TransitionType::Random => Some(pick_random()),
        TransitionType::None => None,
    }
}

fn pick_random() -> TransitionKind {
    let choices = [
        TransitionKind::Wave,
        TransitionKind::Outer,
        TransitionKind::Burn,
        TransitionKind::Glitch,
        TransitionKind::Disintegrate,
        TransitionKind::Dreamy,
        TransitionKind::GlitchMemories,
        TransitionKind::Morph,
        TransitionKind::Hexagonalize,
        TransitionKind::CrossZoom,
        TransitionKind::FluidDistortion,
        TransitionKind::FluidDrain,
        TransitionKind::FluidRipple,
        TransitionKind::FluidVortex,
        TransitionKind::FluidWave,
        TransitionKind::InkBleed,
        TransitionKind::LavaLamp,
        TransitionKind::ChromaticAberration,
        TransitionKind::LensDistortion,
        TransitionKind::RadialBlur,
    ];
    let mut buf = [0u8; 8];
    fill(&mut buf).expect("getrandom failed");
    let idx = usize::from_le_bytes(buf) % choices.len();
    choices[idx]
}

/// Create a new TransitionState from IPC params and textures.
pub fn create_transition(
    params: &TransitionParams,
    kind: TransitionKind,
    old_texture: GpuTexture,
    old_resize_mode: wl_common::ipc_types::ResizeMode,
    new_texture: GpuTexture,
    new_resize_mode: wl_common::ipc_types::ResizeMode,
) -> TransitionState {
    TransitionState {
        transition_type: match kind {
            TransitionKind::Wipe => TransitionType::Wipe,
            TransitionKind::Wave => TransitionType::Wave,
            TransitionKind::Outer => TransitionType::Outer,
            TransitionKind::Pixelate => TransitionType::Pixelate,
            TransitionKind::Burn => TransitionType::Burn,
            TransitionKind::Glitch => TransitionType::Glitch,
            TransitionKind::Disintegrate => TransitionType::Disintegrate,
            TransitionKind::Dreamy => TransitionType::Dreamy,
            TransitionKind::GlitchMemories => TransitionType::GlitchMemories,
            TransitionKind::Morph => TransitionType::Morph,
            TransitionKind::Hexagonalize => TransitionType::Hexagonalize,
            TransitionKind::CrossZoom => TransitionType::CrossZoom,
            TransitionKind::FluidDistortion => TransitionType::FluidDistortion,
            TransitionKind::FluidDrain => TransitionType::FluidDrain,
            TransitionKind::FluidRipple => TransitionType::FluidRipple,
            TransitionKind::FluidVortex => TransitionType::FluidVortex,
            TransitionKind::FluidWave => TransitionType::FluidWave,
            TransitionKind::InkBleed => TransitionType::InkBleed,
            TransitionKind::LavaLamp => TransitionType::LavaLamp,
            TransitionKind::ChromaticAberration => TransitionType::ChromaticAberration,
            TransitionKind::LensDistortion => TransitionType::LensDistortion,
            TransitionKind::CrtShutdown => TransitionType::CrtShutdown,
            TransitionKind::PerlinWipe => TransitionType::PerlinWipe,
            TransitionKind::RadialBlur => TransitionType::RadialBlur,
        },
        kind,
        duration_secs: params.duration_secs,
        progress: 0.0,
        start_time: Instant::now(),
        last_present: Instant::now(),
        fps: params.fps,
        angle: params.angle,
        position: params.position,
        bezier: params.bezier,
        wave: params.wave,
        old_texture,
        old_resize_mode,
        new_texture,
        new_resize_mode,
        descriptor_set: None,
    }
}

/// Tick the transition, returning true if complete.
pub fn tick(state: &mut TransitionState) -> bool {
    // A zero, negative, NaN or infinite duration would leave the linear progress
    // below 1.0 forever, so the transition could never finish and the daemon
    // would keep presenting at the loop rate with no way out
    // (`--transition-duration` is an unvalidated f32 straight from the CLI).
    if !state.duration_secs.is_finite() || state.duration_secs <= 0.0 {
        state.progress = 1.0;
        return true;
    }

    let elapsed = state.start_time.elapsed().as_secs_f32();
    let linear = (elapsed / state.duration_secs).clamp(0.0, 1.0);
    state.progress = cubic_bezier(linear, state.bezier);
    linear >= 1.0
}

/// Whether the renderer should draw the next frame of this transition now.
///
/// The main loop runs at a fixed ~62 Hz and used to be the only pacing for
/// transitions, so a 30 fps transition (`--transition-fps`, the default) was
/// drawn at the loop rate and cost roughly twice the GPU time it asked for.
pub fn wants_frame(state: &TransitionState) -> bool {
    // 0 means "no limit": draw on every tick.
    if state.fps == 0 {
        return true;
    }
    let interval = std::time::Duration::from_secs_f32(1.0 / state.fps as f32);
    state.last_present.elapsed() >= interval
}

/// Evaluate cubic bezier easing curve.
/// bezier = [x1, y1, x2, y2] (control points, endpoints are (0,0) and (1,1)).
fn cubic_bezier(t: f32, bezier: [f32; 4]) -> f32 {
    let [x1, y1, x2, y2] = bezier;

    // Newton's method to find the t parameter for the given x
    let mut guess = t;
    for _ in 0..8 {
        let bx = bezier_component(guess, x1, x2);
        let dx = bezier_derivative(guess, x1, x2);
        if dx.abs() < 1e-6 {
            break;
        }
        guess -= (bx - t) / dx;
        guess = guess.clamp(0.0, 1.0);
    }

    bezier_component(guess, y1, y2)
}

fn bezier_component(t: f32, p1: f32, p2: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3
}

fn bezier_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let mt = 1.0 - t;
    3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wl_common::ipc_types::ResizeMode;

    fn texture() -> GpuTexture {
        GpuTexture {
            image: ash::vk::Image::null(),
            view: ash::vk::ImageView::null(),
            memory: ash::vk::DeviceMemory::null(),
            width: 1,
            height: 1,
        }
    }

    fn state(duration_secs: f32, fps: u32) -> TransitionState {
        TransitionState {
            transition_type: TransitionType::None,
            kind: TransitionKind::Wave,
            duration_secs,
            progress: 0.0,
            start_time: Instant::now(),
            last_present: Instant::now(),
            fps,
            angle: 0.0,
            position: (0.5, 0.5),
            bezier: [0.25, 0.1, 0.25, 1.0],
            wave: (20.0, 20.0),
            old_texture: texture(),
            old_resize_mode: ResizeMode::Crop,
            new_texture: texture(),
            new_resize_mode: ResizeMode::Crop,
            descriptor_set: None,
        }
    }

    #[test]
    fn degenerate_duration_completes_immediately() {
        // A duration the progress can never reach used to hang the transition
        // forever, which kept the daemon presenting on every loop tick.
        for duration in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut t = state(duration, 30);
            assert!(
                tick(&mut t),
                "duration {duration} must not leave the transition running"
            );
        }
    }

    #[test]
    fn normal_duration_is_not_complete_immediately() {
        let mut t = state(1.0, 30);
        assert!(!tick(&mut t));
    }

    #[test]
    fn frames_are_paced_by_fps() {
        let mut t = state(1.0, 30);
        t.last_present = Instant::now() - Duration::from_millis(100);
        assert!(wants_frame(&t), "a frame is due after the fps interval");

        t.last_present = Instant::now();
        assert!(!wants_frame(&t), "another frame is not due yet");

        t.fps = 0;
        assert!(wants_frame(&t), "fps 0 means no limit");
    }
}
