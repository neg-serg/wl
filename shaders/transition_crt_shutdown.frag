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
    // Phase 1 (0.0 → 0.5): compress old image
    // Phase 2 (0.5 → 1.0): expand new image
    bool is_phase2 = pc.progress >= 0.5;
    float phase_progress = is_phase2 ? (pc.progress - 0.5) * 2.0 : pc.progress * 2.0;

    // In phase 1: compress (phase_progress goes 0→1 = full→collapsed)
    // In phase 2: expand (phase_progress goes 0→1 = collapsed→full)
    float compress = is_phase2 ? (1.0 - phase_progress) : phase_progress;

    // Vertical compression first, then horizontal
    // Use pow for non-linear feel (CRTs collapse quickly then slow)
    float v_compress = pow(compress, 0.7);
    float h_compress = pow(max(compress - 0.3, 0.0) / 0.7, 0.5);

    // Scale factors (1.0 = full size, 0.001 = collapsed to point)
    float v_scale = max(1.0 - v_compress * 0.999, 0.001);
    float h_scale = max(1.0 - h_compress * 0.999, 0.001);

    // Check if pixel is within the compressed area
    float v_dist = abs(v_uv.y - 0.5);
    float h_dist = abs(v_uv.x - 0.5);

    bool in_rect = (v_dist < v_scale * 0.5) && (h_dist < h_scale * 0.5);

    if (!in_rect) {
        // Outside the compressed rect — black
        f_color = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    // Map pixel back to original UV space
    vec2 uv;
    uv.x = (v_uv.x - 0.5) / h_scale + 0.5;
    uv.y = (v_uv.y - 0.5) / v_scale + 0.5;

    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);

    if (is_phase2) {
        vec2 new_uv = uv;
        if (apply_resize(new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
            color = texture(u_new, new_uv);
    } else {
        vec2 old_uv = uv;
        if (apply_resize(old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
            color = texture(u_old, old_uv);
    }

    // Scanline effect
    float scanline = 0.92 + 0.08 * sin(v_uv.y * 800.0 * 3.14159265);
    color.rgb *= scanline;

    // Brightness boost when compressed (phosphor glow)
    float glow = 1.0 + compress * 2.0;
    color.rgb *= min(glow, 3.0);
    color.rgb = min(color.rgb, vec3(1.0));

    f_color = color;
}
