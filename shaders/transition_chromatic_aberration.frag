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

float sample_channel(sampler2D tex, vec2 base_uv, vec2 offset, uint resize_mode, float img_aspect, float scr_aspect, int channel) {
    vec2 uv = base_uv + offset;
    if (apply_resize(uv, resize_mode, img_aspect, scr_aspect)) {
        vec4 c = texture(tex, uv);
        if (channel == 0) return c.r;
        if (channel == 1) return c.g;
        return c.b;
    }
    return 0.0;
}

void main() {
    // Separation direction from angle
    float a = radians(pc.angle);
    vec2 dir = vec2(cos(a), sin(a));

    // Max offset peaks at progress=0.5
    float max_offset = 0.06;
    float offset_amount = max_offset * sin(pc.progress * 3.14159265);

    // Channel offsets: R goes one way, B goes the other, G stays
    vec2 r_offset = dir * offset_amount;
    vec2 b_offset = -dir * offset_amount;

    // Crossfade between old and new
    float fade = smoothstep(0.35, 0.65, pc.progress);

    // Sample R channel
    float r_old = sample_channel(u_old, v_uv, r_offset, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect, 0);
    float r_new = sample_channel(u_new, v_uv, r_offset, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect, 0);
    float r = mix(r_old, r_new, fade);

    // Sample G channel (no offset)
    float g_old = sample_channel(u_old, v_uv, vec2(0.0), pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect, 1);
    float g_new = sample_channel(u_new, v_uv, vec2(0.0), pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect, 1);
    float g = mix(g_old, g_new, fade);

    // Sample B channel
    float b_old = sample_channel(u_old, v_uv, b_offset, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect, 2);
    float b_new = sample_channel(u_new, v_uv, b_offset, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect, 2);
    float b = mix(b_old, b_new, fade);

    f_color = vec4(r, g, b, 1.0);
}
