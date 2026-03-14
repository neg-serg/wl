#version 450
// Fluid Vortex — old image swirls into a vortex/whirlpool at center,
// gets sucked in, new image spirals out from behind.

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 f_color;

layout(set = 0, binding = 0) uniform sampler2D u_old;
layout(set = 0, binding = 1) uniform sampler2D u_new;

layout(push_constant) uniform PushConstants {
    float progress;
    float angle;
    float pos_x;
    float pos_y;
    float wave_x;
    float wave_y;
    uint old_resize_mode;
    float old_img_aspect;
    uint new_resize_mode;
    float new_img_aspect;
    float screen_aspect;
} pc;

bool apply_resize(inout vec2 uv, uint resize_mode, float img_aspect, float scr_aspect) {
    if (resize_mode == 0u) {
        if (img_aspect > scr_aspect) {
            float scale = scr_aspect / img_aspect;
            uv.x = uv.x * scale + (1.0 - scale) * 0.5;
        } else {
            float scale = img_aspect / scr_aspect;
            uv.y = uv.y * scale + (1.0 - scale) * 0.5;
        }
    } else if (resize_mode == 1u) {
        if (img_aspect > scr_aspect) {
            float scale = scr_aspect / img_aspect;
            float offset = (1.0 - scale) * 0.5;
            if (uv.y < offset || uv.y > 1.0 - offset) return false;
            uv.y = (uv.y - offset) / scale;
        } else {
            float scale = img_aspect / scr_aspect;
            float offset = (1.0 - scale) * 0.5;
            if (uv.x < offset || uv.x > 1.0 - offset) return false;
            uv.x = (uv.x - offset) / scale;
        }
    }
    return true;
}

vec4 sample_old(vec2 uv) {
    vec2 r = uv;
    if (!apply_resize(r, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        return vec4(0.0, 0.0, 0.0, 1.0);
    return texture(u_old, r);
}

vec4 sample_new(vec2 uv) {
    vec2 r = uv;
    if (!apply_resize(r, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        return vec4(0.0, 0.0, 0.0, 1.0);
    return texture(u_new, r);
}

float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2(1, 0)), f.x),
        mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x),
        f.y
    );
}

float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    mat2 rot = mat2(0.87, 0.50, -0.50, 0.87);
    for (int i = 0; i < 4; i++) { v += a * value_noise(p); p = rot * p * 2.03; a *= 0.5; }
    return v;
}

void main() {
    const float PI = 3.14159265;
    float t = pc.progress;

    vec2 center = vec2(pc.pos_x, pc.pos_y);
    vec2 uv = v_uv;

    // Aspect-corrected distance
    vec2 delta = uv - center;
    delta.x *= pc.screen_aspect;
    float dist = length(delta);
    float ang = atan(delta.y, delta.x);

    // === Phase 1 (0-0.5): Old image swirls INTO vortex ===
    // === Phase 2 (0.5-1): New image swirls OUT from vortex ===

    float phase1 = smoothstep(0.0, 0.5, t); // 0→1 during first half
    float phase2 = smoothstep(0.5, 1.0, t); // 0→1 during second half

    // Vortex rotation — accelerates then decelerates
    float rotation_env = sin(t * PI); // peaks at 0.5
    float rotation = rotation_env * rotation_env * 8.0; // max ~8 radians

    // Rotation is stronger near center (like real vortex)
    float vortex_strength = exp(-dist * 1.5);
    float swirl_angle = rotation * vortex_strength;

    // Radial pull toward center (suction effect in phase 1)
    float suction = phase1 * (1.0 - phase2) * 0.3 * vortex_strength;

    // Apply swirl to UV
    float cos_a = cos(swirl_angle);
    float sin_a = sin(swirl_angle);
    vec2 swirled_delta = vec2(
        delta.x * cos_a - delta.y * sin_a,
        delta.x * sin_a + delta.y * cos_a
    );
    // Pull toward center
    swirled_delta *= 1.0 - suction;

    // Un-aspect-correct
    swirled_delta.x /= pc.screen_aspect;
    vec2 swirled_uv = center + swirled_delta;

    // === Reveal mask: spiral arms reveal new image ===
    // Spiral boundary rotates and expands
    float spiral_angle = ang - t * PI * 4.0; // 2 full rotations
    float spiral = fract(spiral_angle / (2.0 * PI) + dist * 2.0);

    // Number of spiral arms
    float arms = 3.0;
    float spiral_mask = 0.0;
    for (int i = 0; i < 3; i++) {
        float arm_angle = ang - t * PI * 4.0 + float(i) * 2.0 * PI / arms;
        float arm = fract(arm_angle / (2.0 * PI) + dist * 1.5);
        // Arm width grows with progress
        float width = 0.15 + phase1 * 0.35;
        float arm_val = smoothstep(width, width - 0.08, arm) * smoothstep(0.0, 0.05, arm);
        // Arms only reveal up to current radius
        float radius_limit = t * 2.5;
        arm_val *= smoothstep(radius_limit, radius_limit - 0.2, dist);
        spiral_mask = max(spiral_mask, arm_val);
    }

    // After midpoint, rapidly fill remaining areas
    spiral_mask = mix(spiral_mask, 1.0, phase2 * phase2);
    // Force edges
    spiral_mask = mix(0.0, spiral_mask, smoothstep(0.0, 0.08, t));

    // === Add noise to spiral edges for organic feel ===
    float edge_noise = fbm(vec2(ang * 3.0, dist * 8.0) + t * 2.0);
    spiral_mask = smoothstep(0.0, 1.0, spiral_mask + (edge_noise - 0.5) * 0.3);
    spiral_mask = clamp(spiral_mask, 0.0, 1.0);

    // === Sample ===
    vec3 old_col = sample_old(swirled_uv).rgb;
    vec3 new_col = sample_new(swirled_uv).rgb;

    vec3 color = mix(old_col, new_col, spiral_mask);

    // === Vortex center glow ===
    float center_glow = exp(-dist * 6.0) * rotation_env;
    color += vec3(0.6, 0.75, 1.0) * center_glow * 0.5;

    // === Spiral arm edge glow ===
    // Bright line at boundary between old and new
    float boundary = abs(spiral_mask - 0.5) * 2.0;
    float at_edge = 1.0 - boundary;
    float edge_bright = pow(at_edge, 6.0) * rotation_env;
    color += vec3(0.8, 0.9, 1.0) * edge_bright * 0.6;

    // === Motion blur streaks ===
    float streak_angle = fract(ang / (2.0 * PI) * 12.0 + t * 3.0);
    float streaks = pow(abs(sin(streak_angle * PI)), 20.0) * vortex_strength * rotation_env;
    color += vec3(0.5, 0.6, 0.8) * streaks * 0.2;

    // === Slight chromatic shift from swirl ===
    float chroma = swirl_angle * 0.003;
    vec2 chroma_dir = vec2(cos(ang), sin(ang));
    chroma_dir.x /= pc.screen_aspect;
    color.r = mix(color.r, sample_old(swirled_uv + chroma_dir * chroma).r, 1.0 - spiral_mask);
    color.b = mix(color.b, sample_old(swirled_uv - chroma_dir * chroma).b, 1.0 - spiral_mask);

    f_color = vec4(color, 1.0);
}
