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

// Hash for noise
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Value noise
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f); // smoothstep

    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));

    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Fractal Brownian Motion — 5 octaves for ink-like detail
float fbm(vec2 p) {
    float value = 0.0;
    float amplitude = 0.5;
    float frequency = 1.0;
    for (int i = 0; i < 5; i++) {
        value += amplitude * vnoise(p * frequency);
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    return value;
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

    float scale = max(pc.wave_x, 6.0);

    // Generate ink bleed pattern
    float noise = fbm(v_uv * scale);

    // Edge softness
    float edge_width = 0.08;

    // Map progress to threshold: ink bleeds from areas with high noise values first
    float threshold = pc.progress;

    // Soft mask with ragged edges
    float mask = smoothstep(threshold - edge_width, threshold + edge_width, noise);

    // Invert: high noise areas reveal new image first
    mask = 1.0 - mask;

    // Slight UV distortion near the bleed edge for organic feel
    float edge_proximity = 1.0 - abs(noise - threshold) / edge_width;
    edge_proximity = max(edge_proximity, 0.0);

    f_color = mix(old_color, new_color, mask);
}
