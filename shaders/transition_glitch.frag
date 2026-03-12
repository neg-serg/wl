#version 450

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

float hash(float n) {
    return fract(sin(n) * 43758.5453123);
}

float hash2(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void main() {
    // Glitch intensity peaks at midpoint
    float intensity = sin(pc.progress * 3.14159);
    float blend = smoothstep(0.3, 0.7, pc.progress);

    // Horizontal band displacement
    float band_y = floor(v_uv.y * 40.0);
    float band_noise = hash(band_y + floor(pc.progress * 20.0));
    float band_active = step(0.7 - intensity * 0.4, band_noise);
    float displacement = (band_noise - 0.5) * 0.15 * intensity * band_active;

    // Block corruption: large rectangular glitch blocks
    vec2 block = floor(v_uv * vec2(8.0, 12.0));
    float block_noise = hash2(block + vec2(floor(pc.progress * 15.0)));
    float block_active = step(0.85 - intensity * 0.3, block_noise);
    float block_shift = (block_noise - 0.5) * 0.2 * block_active;

    // RGB channel separation
    float rgb_split = intensity * 0.03;

    // Displaced UV for sampling
    vec2 displaced_uv = v_uv + vec2(displacement + block_shift, 0.0);
    displaced_uv = clamp(displaced_uv, 0.0, 1.0);

    // Sample with RGB split
    vec2 uv_r = displaced_uv + vec2(rgb_split, 0.0);
    vec2 uv_g = displaced_uv;
    vec2 uv_b = displaced_uv - vec2(rgb_split, 0.0);

    uv_r = clamp(uv_r, 0.0, 1.0);
    uv_b = clamp(uv_b, 0.0, 1.0);

    // Sample old image channels
    vec2 old_r = uv_r, old_g = uv_g, old_b = uv_b;
    vec4 old_cr = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 old_cg = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 old_cb = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(old_r, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_cr = texture(u_old, old_r);
    if (apply_resize(old_g, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_cg = texture(u_old, old_g);
    if (apply_resize(old_b, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_cb = texture(u_old, old_b);

    vec3 old_color = vec3(old_cr.r, old_cg.g, old_cb.b);

    // Sample new image channels
    vec2 new_r = uv_r, new_g = uv_g, new_b = uv_b;
    vec4 new_cr = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_cg = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_cb = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(new_r, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_cr = texture(u_new, new_r);
    if (apply_resize(new_g, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_cg = texture(u_new, new_g);
    if (apply_resize(new_b, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_cb = texture(u_new, new_b);

    vec3 new_color = vec3(new_cr.r, new_cg.g, new_cb.b);

    // Mix with glitchy crossfade
    vec3 base = mix(old_color, new_color, blend);

    // Static noise scanlines
    float scanline = hash2(v_uv * 500.0 + vec2(pc.progress * 100.0));
    float noise_overlay = scanline * intensity * 0.15;

    // Occasional white flash in glitch blocks
    float flash = block_active * step(0.95, block_noise) * intensity * 0.5;

    f_color = vec4(base + noise_overlay + flash, 1.0);
}
