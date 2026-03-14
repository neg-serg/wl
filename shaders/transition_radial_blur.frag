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

vec4 sample_blurred(sampler2D tex, vec2 uv, vec2 dir, float blur_amount, uint resize_mode, float img_aspect, float scr_aspect) {
    const int NUM_SAMPLES = 10;
    vec4 color = vec4(0.0);
    float total = 0.0;

    for (int i = 0; i < NUM_SAMPLES; i++) {
        float t = float(i) / float(NUM_SAMPLES - 1);
        vec2 sample_uv = uv + dir * blur_amount * t;
        if (apply_resize(sample_uv, resize_mode, img_aspect, scr_aspect)) {
            color += texture(tex, sample_uv);
            total += 1.0;
        }
    }

    return total > 0.0 ? color / total : vec4(0.0, 0.0, 0.0, 1.0);
}

void main() {
    vec2 center = vec2(pc.pos_x, pc.pos_y);

    // Radial direction from center
    vec2 dir = v_uv - center;

    // Blur amount peaks at progress=0.5
    float max_blur = 0.15;
    float blur_amount = max_blur * sin(pc.progress * 3.14159265);

    // Crossfade
    float fade = smoothstep(0.35, 0.65, pc.progress);

    vec4 old_blurred = sample_blurred(u_old, v_uv, dir, blur_amount, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect);
    vec4 new_blurred = sample_blurred(u_new, v_uv, dir, blur_amount, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect);

    f_color = mix(old_blurred, new_blurred, fade);
}
