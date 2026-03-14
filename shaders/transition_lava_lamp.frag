#version 450
// Lava Lamp — large viscous blobs rise and merge, revealing new image

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

float hash(float n) { return fract(sin(n) * 43758.5453); }

// Smooth noise for wobbly blob shapes
float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float n = i.x + i.y * 157.0;
    return mix(
        mix(hash(n), hash(n + 1.0), f.x),
        mix(hash(n + 157.0), hash(n + 158.0), f.x),
        f.y
    );
}

float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 4; i++) {
        v += a * value_noise(p);
        p = rot * p * 2.0;
        a *= 0.5;
    }
    return v;
}

void main() {
    const float PI = 3.14159265;
    float t = pc.progress;
    float anim = t * PI;

    vec2 uv = vec2(v_uv.x * pc.screen_aspect, v_uv.y);

    // === Large metaballs rising from bottom ===
    // 4 big primary blobs + 3 smaller secondary ones
    float field = 0.0;

    // Blob parameters: x_center, x_wobble_amp, x_wobble_freq, radius, rise_speed, start_delay
    const int N_BLOBS = 7;
    float bx[7]     = float[](0.3, 0.65, 0.5,  0.8,  0.15, 0.45, 0.7);
    float bwob[7]   = float[](0.12, 0.15, 0.1, 0.08, 0.1,  0.13, 0.09);
    float bfreq[7]  = float[](1.3, 0.9, 1.1, 1.5, 0.8, 1.2, 1.0);
    float brad[7]   = float[](0.35, 0.30, 0.40, 0.25, 0.28, 0.22, 0.20);
    float bspeed[7] = float[](1.0, 1.15, 0.9, 1.3, 1.1, 1.2, 0.95);
    float bdelay[7] = float[](0.0, 0.05, 0.02, 0.08, 0.12, 0.10, 0.15);

    for (int i = 0; i < N_BLOBS; i++) {
        // Blob activation time
        float bt = max(0.0, t - bdelay[i]) / (1.0 - bdelay[i]);
        bt = clamp(bt, 0.0, 1.0);

        // Rise from below: y goes from 1.5 (below screen) to target position
        float target_y = 0.3 + hash(float(i) * 7.1) * 0.4;
        float y = mix(1.5, target_y, smoothstep(0.0, 0.6, bt));

        // Wobble horizontally
        float x = bx[i] * pc.screen_aspect + bwob[i] * sin(anim * bfreq[i] + float(i) * 2.0);

        // Radius grows as blob activates, then expands to fill at end
        float r = brad[i] * smoothstep(0.0, 0.3, bt);
        // Expand massively at the end to ensure full coverage
        r += brad[i] * 2.0 * smoothstep(0.5, 1.0, bt);

        // Wobbly shape — distort distance with noise
        vec2 diff = uv - vec2(x, y);
        float angle_to_center = atan(diff.y, diff.x);
        float wobble = fbm(vec2(angle_to_center * 2.0 + float(i), anim * 0.5)) * 0.3;
        float dist = length(diff) * (1.0 + wobble);

        // Inverse-square metaball contribution
        field += (r * r) / (dist * dist + 0.0001);
    }

    // Threshold: field > 1.0 = inside blob
    // Smooth boundary with glow zone
    float inside = smoothstep(0.7, 1.3, field);

    // Force complete at end
    inside = mix(inside, 1.0, smoothstep(0.85, 1.0, t));
    // Force zero at start
    inside = mix(0.0, inside, smoothstep(0.0, 0.08, t));

    // === UV displacement inside blobs (viscous warping) ===
    float env = sin(t * PI);
    vec2 warp = vec2(
        fbm(uv * 2.0 + anim * 0.3) - 0.5,
        fbm(uv * 2.0 + vec2(5.0) + anim * 0.25) - 0.5
    ) * 0.06 * env * inside;

    vec3 old_col = sample_old(v_uv + warp * 0.5).rgb;
    vec3 new_col = sample_new(v_uv + warp * 0.3).rgb;

    // === Compose ===
    vec3 color = mix(old_col, new_col, inside);

    // === Blob boundary glow ===
    // Narrow ring at the edge of blobs
    float edge = smoothstep(0.5, 1.0, field) - smoothstep(1.0, 1.8, field);
    edge = max(edge, 0.0);

    // Hot glow color (orange → white at center of edge)
    vec3 glow_color = mix(
        vec3(1.0, 0.3, 0.05),  // deep orange
        vec3(1.0, 0.85, 0.5),  // warm yellow-white
        pow(edge, 2.0)
    );
    color += glow_color * edge * 1.2 * env;

    // === Internal glow — subtle warmth inside blobs ===
    float deep = smoothstep(1.5, 4.0, field);
    vec3 inner_glow = vec3(1.0, 0.5, 0.15) * deep * 0.15 * env;
    color += inner_glow;

    // === Specular highlights on blob surface ===
    float eps = 0.01;
    float f0 = field;
    float fx = 0.0, fy = 0.0;
    // Approximate field gradient for fake specular
    {
        vec2 uv_dx = uv + vec2(eps, 0.0);
        vec2 uv_dy = uv + vec2(0.0, eps);
        float field_dx = 0.0, field_dy = 0.0;
        for (int i = 0; i < N_BLOBS; i++) {
            float bt = clamp((max(0.0, t - bdelay[i])) / (1.0 - bdelay[i]), 0.0, 1.0);
            float target_y = 0.3 + hash(float(i) * 7.1) * 0.4;
            float y = mix(1.5, target_y, smoothstep(0.0, 0.6, bt));
            float x = bx[i] * pc.screen_aspect + bwob[i] * sin(anim * bfreq[i] + float(i) * 2.0);
            float r = brad[i] * smoothstep(0.0, 0.3, bt) + brad[i] * 2.0 * smoothstep(0.5, 1.0, bt);

            vec2 c = vec2(x, y);
            float ddx = dot(uv_dx - c, uv_dx - c) + 0.0001;
            float ddy = dot(uv_dy - c, uv_dy - c) + 0.0001;
            field_dx += (r * r) / ddx;
            field_dy += (r * r) / ddy;
        }
        fx = field_dx - f0;
        fy = field_dy - f0;
    }
    vec2 grad = vec2(fx, fy) / eps;
    float spec = pow(max(dot(normalize(grad + 0.001), normalize(vec2(-0.4, -0.6))), 0.0), 24.0);
    color += vec3(1.0, 0.9, 0.8) * spec * 0.6 * env * smoothstep(0.6, 1.2, field);

    f_color = vec4(color, 1.0);
}
