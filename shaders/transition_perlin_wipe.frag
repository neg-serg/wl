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

// Hash for value noise
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

float fbm(vec2 p) {
    float value = 0.0;
    float amplitude = 0.5;
    float frequency = 1.0;
    for (int i = 0; i < 4; i++) {
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

    // Wipe direction from angle
    float a = radians(pc.angle);
    vec2 dir = vec2(cos(a), sin(a));

    // Position along wipe direction
    float wipe_pos = dot(v_uv - vec2(0.5), dir) + 0.5;

    // Noise displacement of the boundary
    float noise_scale = 8.0;
    float noise_amplitude = 0.15;
    float noise = fbm(v_uv * noise_scale);

    // Edge width for soft boundary
    float edge_width = 0.04;

    // Threshold moves with progress, displaced by noise
    float threshold = mix(-noise_amplitude - edge_width, 1.0 + noise_amplitude + edge_width, pc.progress);
    float displaced_threshold = threshold + (noise - 0.5) * noise_amplitude * 2.0;

    float mask = smoothstep(displaced_threshold - edge_width, displaced_threshold + edge_width, wipe_pos);

    // Glow at the boundary edge (fire/smoke front)
    float edge_dist = abs(wipe_pos - displaced_threshold);
    float glow_width = 0.06;
    float glow = 1.0 - smoothstep(0.0, glow_width, edge_dist);
    glow *= step(0.01, pc.progress) * step(pc.progress, 0.99); // no glow at start/end

    vec3 glow_color = vec3(1.0, 0.5, 0.1) * glow * 0.6;

    vec4 base = mix(new_color, old_color, mask);
    f_color = vec4(base.rgb + glow_color, 1.0);
}
