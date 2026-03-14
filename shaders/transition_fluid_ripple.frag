#version 450
// Fluid Ripple — stone dropped in water: concentric waves refract image,
// as ripples pass they leave behind the new image. Water settles at end.

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

float hash(float n) { return fract(sin(n) * 43758.5453); }

void main() {
    const float PI = 3.14159265;
    float t = pc.progress;

    // Impact point
    vec2 center = vec2(pc.pos_x, pc.pos_y);
    vec2 auv = vec2(v_uv.x * pc.screen_aspect, v_uv.y);
    vec2 acenter = vec2(center.x * pc.screen_aspect, center.y);

    float dist = length(auv - acenter);
    vec2 dir = normalize(auv - acenter + 0.0001);

    // === Multiple ripple rings expanding outward ===
    float max_radius = 2.0; // enough to cover the whole screen
    float wave_speed = max_radius / 0.7; // reach edges by t=0.7

    // Total displacement from all ripple layers
    vec2 total_disp = vec2(0.0);
    float reveal_mask = 0.0;

    // 3 splash impacts at staggered times
    for (int s = 0; s < 3; s++) {
        float delay = float(s) * 0.08;
        float st = max(0.0, t - delay);
        float splash_radius = st * wave_speed;

        // Damping increases with time
        float damping = exp(-st * 3.0);

        // Multiple concentric rings per splash
        for (int w = 0; w < 4; w++) {
            float freq = 18.0 + float(w) * 8.0;
            float phase = float(w) * 0.5 + float(s) * 1.7;
            float amp = (0.04 - float(w) * 0.008) * damping;

            // Ring travels outward
            float wave_val = sin(dist * freq - splash_radius * freq + phase) * amp;

            // Only active behind the wave front
            float behind_front = smoothstep(splash_radius, splash_radius - 0.1, dist);
            wave_val *= behind_front;

            // Displacement along radial direction
            total_disp += dir * wave_val;
        }

        // Reveal mask: everything inside the outermost ring shows new image
        // The ring sweeps outward, leaving new image behind it
        float ring_pos = splash_radius;
        float ring_reveal = smoothstep(ring_pos + 0.05, ring_pos - 0.05, dist);
        reveal_mask = max(reveal_mask, ring_reveal * smoothstep(0.0, 0.15, st));
    }

    // Dampen displacement as transition completes (water settling)
    float settle = smoothstep(0.6, 1.0, t);
    total_disp *= 1.0 - settle * 0.9;

    // Force complete reveal
    reveal_mask = mix(reveal_mask, 1.0, smoothstep(0.8, 1.0, t));

    // === Sample with refraction displacement ===
    vec2 old_uv = v_uv + total_disp;
    vec2 new_uv = v_uv + total_disp * 0.3; // new image has less distortion (underwater)

    // Chromatic aberration through water
    float chroma = length(total_disp) * 3.0;
    vec3 old_col;
    old_col.r = sample_old(old_uv + dir * chroma * 0.003).r;
    old_col.g = sample_old(old_uv).g;
    old_col.b = sample_old(old_uv - dir * chroma * 0.003).b;

    vec3 new_col = sample_new(new_uv).rgb;

    vec3 color = mix(old_col, new_col, reveal_mask);

    // === Water surface caustics ===
    float caustic_env = sin(t * PI) * (1.0 - settle);
    float c1 = sin(dist * 50.0 - t * 20.0) * sin(auv.x * 30.0 + t * 15.0);
    float c2 = sin(dist * 35.0 + t * 12.0) * sin(auv.y * 25.0 - t * 18.0);
    float caustics = pow(abs(c1 + c2) * 0.5, 3.0) * 2.0;
    color += vec3(0.7, 0.85, 1.0) * caustics * 0.2 * caustic_env;

    // === Bright ring at the wave front ===
    for (int s = 0; s < 3; s++) {
        float delay = float(s) * 0.08;
        float st = max(0.0, t - delay);
        float ring_r = st * wave_speed;
        float ring_bright = exp(-abs(dist - ring_r) * 40.0) * exp(-st * 2.5);
        color += vec3(1.0, 0.95, 0.9) * ring_bright * 0.5;
    }

    // === Depth darkening — underwater areas slightly darker/bluer ===
    float underwater = reveal_mask * (1.0 - settle);
    color = mix(color, color * vec3(0.9, 0.93, 1.0), underwater * 0.2);

    f_color = vec4(color, 1.0);
}
