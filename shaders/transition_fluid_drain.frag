#version 450
// Fluid Drain — old image drains down like thick paint sliding off a wall,
// pooling at the bottom, revealing new image underneath.

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
    for (int i = 0; i < 5; i++) { v += a * value_noise(p); p = rot * p * 2.03; a *= 0.47; }
    return v;
}

void main() {
    const float PI = 3.14159265;
    float t = pc.progress;

    vec2 uv = v_uv;
    vec2 auv = vec2(uv.x * pc.screen_aspect, uv.y);

    // === Drip columns ===
    // Each vertical strip has a slightly different drip speed
    float col_noise = fbm(vec2(auv.x * 8.0, 0.5));
    float col_noise2 = fbm(vec2(auv.x * 15.0, 1.7));

    // Drip front moves from top to bottom with noise variation
    // Fast columns drip first, slow ones lag behind
    float drip_speed = 0.6 + col_noise * 0.8; // per-column speed variation
    float drip_front = t * drip_speed * 2.5 - 0.3;

    // Add wavy drip edge
    float wave = sin(auv.x * 25.0 + t * 5.0) * 0.03;
    wave += sin(auv.x * 40.0 - t * 8.0) * 0.015;
    float drip_edge = drip_front + wave + col_noise2 * 0.15;

    // Thick drip fingers hanging down at intervals
    float finger_pattern = fbm(vec2(auv.x * 6.0, 2.3));
    float is_finger = smoothstep(0.45, 0.55, finger_pattern);
    float finger_length = is_finger * 0.25 * smoothstep(0.1, 0.5, t);
    float finger_width_mod = 1.0 + is_finger * 0.3;

    drip_edge += finger_length;

    // Drip mask: 1 = new image visible (paint has drained away)
    float drip_mask = smoothstep(drip_edge - 0.02, drip_edge + 0.02, uv.y);
    drip_mask = 1.0 - drip_mask; // invert: top drains first

    // Force completion
    drip_mask = mix(drip_mask, 1.0, smoothstep(0.85, 1.0, t));
    drip_mask = mix(0.0, drip_mask, smoothstep(0.0, 0.05, t));

    // === UV displacement — old image stretches downward as it drains ===
    float stretch = smoothstep(drip_edge - 0.15, drip_edge, uv.y) * (1.0 - drip_mask);
    vec2 old_disp = vec2(0.0, stretch * 0.15 * t);

    // Slight horizontal wobble near drip edge
    float near_edge = 1.0 - abs(uv.y - drip_edge) * 10.0;
    near_edge = max(near_edge, 0.0);
    old_disp.x += sin(auv.x * 30.0 + t * 10.0) * 0.01 * near_edge;

    vec3 old_col = sample_old(uv + old_disp).rgb;
    vec3 new_col = sample_new(uv).rgb;

    vec3 color = mix(old_col, new_col, drip_mask);

    // === Wet sheen on the drip edge ===
    float edge_glow = exp(-abs(uv.y - drip_edge) * 80.0);
    // Specular on wet surface
    float spec_noise = fbm(auv * 10.0 + t);
    float specular = pow(spec_noise, 4.0) * edge_glow;
    color += vec3(1.0, 0.98, 0.95) * specular * 0.6;

    // Slight darkening in the wet zone just above the drip edge (thick paint)
    float wet_zone = smoothstep(drip_edge, drip_edge - 0.08, uv.y) * (1.0 - drip_mask);
    color *= 1.0 - wet_zone * 0.15;

    // === Paint pooling at bottom ===
    float pool_height = 0.05 * t * t;
    float in_pool = smoothstep(1.0 - pool_height, 1.0, uv.y);
    // Pool has a glossy surface
    float pool_surface = smoothstep(1.0 - pool_height - 0.01, 1.0 - pool_height, uv.y);
    color += vec3(0.8, 0.85, 1.0) * pool_surface * 0.3 * t;
    // Darken inside pool slightly
    color = mix(color, color * 0.7, in_pool * 0.5);

    f_color = vec4(color, 1.0);
}
