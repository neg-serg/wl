use std::time::Instant;

use swww_vulkan_common::ipc_types::{TransitionParams, TransitionType};

use crate::output::{GpuTexture, TransitionState};
use crate::vulkan::pipeline::TransitionKind;

/// Resolve a TransitionType to a concrete TransitionKind.
/// Random picks uniformly from the 5 types.
pub fn resolve_kind(tt: TransitionType) -> Option<TransitionKind> {
    match tt {
        TransitionType::Fade => Some(TransitionKind::Fade),
        TransitionType::Wipe => Some(TransitionKind::Wipe),
        TransitionType::Wave => Some(TransitionKind::Wave),
        TransitionType::Outer => Some(TransitionKind::Outer),
        TransitionType::Pixelate => Some(TransitionKind::Pixelate),
        TransitionType::Burn => Some(TransitionKind::Burn),
        TransitionType::Glitch => Some(TransitionKind::Glitch),
        TransitionType::Disintegrate => Some(TransitionKind::Disintegrate),
        TransitionType::Random => Some(pick_random()),
        TransitionType::None => None,
    }
}

fn pick_random() -> TransitionKind {
    let choices = [
        TransitionKind::Fade,
        TransitionKind::Wipe,
        TransitionKind::Wave,
        TransitionKind::Outer,
        TransitionKind::Pixelate,
        TransitionKind::Burn,
        TransitionKind::Glitch,
        TransitionKind::Disintegrate,
    ];
    // Simple pseudo-random using time nanos
    let nanos = Instant::now().elapsed().subsec_nanos() as usize;
    // Mix with system time for better distribution
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize;
    choices[(nanos ^ now) % choices.len()]
}

/// Create a new TransitionState from IPC params and textures.
pub fn create_transition(
    params: &TransitionParams,
    kind: TransitionKind,
    old_texture: GpuTexture,
    old_resize_mode: swww_vulkan_common::ipc_types::ResizeMode,
    new_texture: GpuTexture,
    new_resize_mode: swww_vulkan_common::ipc_types::ResizeMode,
) -> TransitionState {
    TransitionState {
        transition_type: match kind {
            TransitionKind::Fade => TransitionType::Fade,
            TransitionKind::Wipe => TransitionType::Wipe,
            TransitionKind::Wave => TransitionType::Wave,
            TransitionKind::Outer => TransitionType::Outer,
            TransitionKind::Pixelate => TransitionType::Pixelate,
            TransitionKind::Burn => TransitionType::Burn,
            TransitionKind::Glitch => TransitionType::Glitch,
            TransitionKind::Disintegrate => TransitionType::Disintegrate,
        },
        kind,
        duration_secs: params.duration_secs,
        progress: 0.0,
        start_time: Instant::now(),
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
    let elapsed = state.start_time.elapsed().as_secs_f32();
    let linear = (elapsed / state.duration_secs).clamp(0.0, 1.0);
    state.progress = cubic_bezier(linear, state.bezier);
    linear >= 1.0
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
