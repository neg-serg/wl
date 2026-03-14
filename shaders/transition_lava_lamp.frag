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
    vec2 old_uv = v_uv;
    vec2 new_uv = v_uv;

    vec4 old_color = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_color = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_color = texture(u_old, old_uv);
    if (apply_resize(new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_color = texture(u_new, new_uv);

    // Aspect-corrected UV
    vec2 uv = v_uv;
    uv.x *= pc.screen_aspect;

    // Use pos as seed for variation
    float seed = pc.pos_x * 7.3 + pc.pos_y * 13.1;

    // 6 metaballs with sinusoidal motion
    float field = 0.0;
    float t = pc.progress * 3.14159265;

    // Metaball centers animated along smooth paths
    vec2 centers[6];
    centers[0] = vec2(0.5 + 0.3 * sin(t * 1.1 + seed),        0.5 + 0.3 * cos(t * 0.9 + 1.0));
    centers[1] = vec2(0.5 + 0.35 * cos(t * 0.8 + seed + 2.0), 0.5 + 0.25 * sin(t * 1.2 + 3.0));
    centers[2] = vec2(0.5 + 0.25 * sin(t * 1.3 + seed + 4.0), 0.5 + 0.35 * cos(t * 0.7 + 5.0));
    centers[3] = vec2(0.5 + 0.3 * cos(t * 0.6 + seed + 6.0),  0.5 + 0.3 * sin(t * 1.0 + 7.0));
    centers[4] = vec2(0.5 + 0.2 * sin(t * 1.4 + seed + 8.0),  0.5 + 0.2 * cos(t * 1.1 + 9.0));
    centers[5] = vec2(0.5 + 0.28 * cos(t * 0.9 + seed + 10.0),0.5 + 0.28 * sin(t * 0.8 + 11.0));

    float radii[6] = float[](0.12, 0.10, 0.11, 0.09, 0.08, 0.10);

    for (int i = 0; i < 6; i++) {
        vec2 c = centers[i];
        c.x *= pc.screen_aspect;
        float r = radii[i];
        vec2 diff = uv - c;
        float dist_sq = dot(diff, diff);
        field += (r * r) / (dist_sq + 0.001);
    }

    // Threshold grows with progress — more area covered by metaballs
    float threshold = mix(0.5, 8.0, pc.progress);

    // Smooth blob edges
    float mask = smoothstep(threshold - 0.5, threshold + 0.5, field);

    f_color = mix(old_color, new_color, mask);
}
