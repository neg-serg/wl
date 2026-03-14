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

void main() {
    vec2 center = vec2(pc.pos_x, pc.pos_y);

    // Distortion strength peaks at progress=0.5
    float max_strength = 8.0;
    float strength = max_strength * sin(pc.progress * 3.14159265);

    // Center-relative coordinates
    vec2 centered = v_uv - center;

    // Barrel distortion
    float r = length(centered);
    float distortion = 1.0 + strength * r * r;

    vec2 distorted_uv = center + centered / distortion;

    // Clamp to valid range
    distorted_uv = clamp(distorted_uv, 0.0, 1.0);

    // Choose source based on progress
    bool use_new = pc.progress >= 0.5;

    vec2 sample_uv = distorted_uv;
    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);

    if (use_new) {
        if (apply_resize(sample_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
            color = texture(u_new, sample_uv);
    } else {
        if (apply_resize(sample_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
            color = texture(u_old, sample_uv);
    }

    // Fade to black at extreme distortion for cleaner midpoint
    float darkness = 1.0 - smoothstep(0.4, 0.5, sin(pc.progress * 3.14159265));
    color.rgb *= 1.0 - darkness * 0.5;

    f_color = color;
}
