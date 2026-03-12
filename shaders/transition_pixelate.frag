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

float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void main() {
    // Per-region noise: different areas pixelate at different times
    // Use coarse grid (8x6 regions) for the "torn" look
    vec2 region = floor(v_uv * vec2(8.0, 6.0));
    float region_noise = hash(region);

    // Each region has its own progress offset — creates jagged/torn timing
    float region_progress = clamp(pc.progress * 1.6 - region_noise * 0.6, 0.0, 1.0);

    // Pixelation intensity: peaks at region midpoint
    float intensity = 1.0 - abs(2.0 * region_progress - 1.0);

    // Sharper pixelation steps: fewer levels, bigger jumps
    float level = round(intensity * 4.0);
    float grid_size = 128.0 / pow(2.0, level);

    // Quantize UVs to cell centers
    vec2 pixelated_uv = (floor(v_uv * grid_size) + 0.5) / grid_size;

    // Per-block swap decision: each block flips independently based on noise
    vec2 block_id = floor(v_uv * grid_size);
    float block_noise = hash(block_id + vec2(37.0, 91.0));

    // Hard per-block switch — no smooth blend, abrupt "torn" flips
    float block_threshold = pc.progress * 1.3 - block_noise * 0.3;
    float show_new = step(0.5, block_threshold);

    // When fully pixelated, swap happens hidden by the mosaic
    // When region is mostly done, force new image
    if (region_progress > 0.85) show_new = 1.0;

    vec2 old_uv = pixelated_uv;
    vec2 new_uv = pixelated_uv;

    vec4 old_color = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_color = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_color = texture(u_old, old_uv);
    if (apply_resize(new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_color = texture(u_new, new_uv);

    f_color = mix(old_color, new_color, show_new);
}
