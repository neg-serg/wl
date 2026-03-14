#version 450
// Fluid Wave — a massive ocean wave rolls across the screen from one side,
// the crest curls over carrying the old image, leaving new image in its wake.

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

float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2(1, 0)), f.x),
        mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x),
        f.y
    );
}

float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    mat2 rot = mat2(0.87, 0.50, -0.50, 0.87);
    for (int i = 0; i < 5; i++) { v += a * value_noise(p); p = rot * p * 2.03; a *= 0.47; }
    return v;
}

void main() {
    const float PI = 3.14159265;
    float t = pc.progress;

    // Wave direction from angle
    float rad = pc.angle * PI / 180.0;
    vec2 wave_dir = vec2(cos(rad), sin(rad));

    vec2 uv = v_uv;

    // Project UV along wave direction
    float projected = dot(uv - 0.5, wave_dir) + 0.5;

    // === Wave front position ===
    float front = mix(-0.4, 1.4, t); // sweeps across screen

    // Distance from wave front
    float d = projected - front;

    // === Wave shape ===
    // Main wave crest — tall, curling shape
    float crest_height = 0.20;
    float crest_width = 0.12;

    // Noise on the wave front for organic shape
    vec2 perp = vec2(-wave_dir.y, wave_dir.x);
    float along_wave = dot(uv - 0.5, perp);
    float front_noise = fbm(vec2(along_wave * 6.0, t * 3.0)) * 0.08;
    float front_noise2 = fbm(vec2(along_wave * 12.0, t * 5.0 + 3.0)) * 0.03;
    float noisy_d = d + front_noise + front_noise2;

    // Wave profile: crest shape
    // Rising front → curl → trailing foam
    float wave_profile = 0.0;
    // Main crest
    wave_profile += crest_height * exp(-noisy_d * noisy_d / (crest_width * crest_width));
    // Curl (asymmetric — steeper in front, gentle behind)
    float curl = 0.0;
    if (noisy_d > 0.0) {
        curl = 0.15 * exp(-noisy_d * noisy_d / 0.005); // sharp front face
    } else {
        curl = 0.08 * exp(-noisy_d * noisy_d / 0.02); // gentle back slope
    }
    wave_profile += curl;

    // === Reveal mask: behind the wave = new image ===
    float reveal = smoothstep(0.03, -0.03, noisy_d);
    reveal = mix(reveal, 1.0, smoothstep(0.9, 1.0, t));
    reveal = mix(0.0, reveal, smoothstep(0.0, 0.05, t));

    // === UV displacement — water lens effect ===
    // Displacement strongest at the crest
    vec2 disp = wave_dir * wave_profile * 0.5;
    // Add vertical lift at crest
    disp.y -= wave_profile * 0.3;

    // Turbulence behind the wave (foam zone)
    float foam_zone = smoothstep(0.0, -0.15, noisy_d) * smoothstep(-0.4, -0.15, noisy_d);
    vec2 foam_disp = vec2(
        fbm(uv * 8.0 + t * 2.0) - 0.5,
        fbm(uv * 8.0 + vec2(5.0) + t * 1.5) - 0.5
    ) * 0.04 * foam_zone;

    vec2 total_disp = disp + foam_disp;

    // Scale displacement with overall transition envelope
    float env = sin(t * PI);
    total_disp *= env;

    vec3 old_col = sample_old(uv + total_disp).rgb;
    vec3 new_col = sample_new(uv + foam_disp * 0.5).rgb;

    vec3 color = mix(old_col, new_col, reveal);

    // === Wave crest highlight (white foam/spray) ===
    float crest_intensity = exp(-noisy_d * noisy_d / 0.003);
    float spray_noise = fbm(vec2(along_wave * 15.0 + t * 8.0, noisy_d * 20.0));
    float spray = crest_intensity * (0.5 + spray_noise * 0.5);
    color = mix(color, vec3(0.9, 0.95, 1.0), spray * 0.7 * env);

    // === Foam trails behind wave ===
    float foam_noise = fbm(vec2(along_wave * 10.0, noisy_d * 15.0 + t * 3.0));
    float foam = foam_zone * smoothstep(0.4, 0.7, foam_noise);
    color = mix(color, vec3(0.85, 0.9, 0.95), foam * 0.5 * env);

    // === Water tint in displaced area ===
    float in_wave = smoothstep(0.15, -0.05, noisy_d) * smoothstep(-0.3, -0.05, noisy_d);
    color = mix(color, color * vec3(0.8, 0.9, 1.05), in_wave * 0.3 * env);

    // === Bright edge at wave leading face ===
    float leading_edge = exp(-max(noisy_d, 0.0) * 60.0) * env;
    color += vec3(0.7, 0.85, 1.0) * leading_edge * 0.4;

    // === Subtle caustics ahead of wave ===
    float ahead = smoothstep(0.0, 0.2, noisy_d) * smoothstep(0.4, 0.2, noisy_d);
    float caustic = sin(projected * 40.0 - t * 15.0) * sin(along_wave * 30.0 + t * 10.0);
    caustic = pow(abs(caustic), 3.0) * 2.0;
    color += vec3(0.5, 0.7, 1.0) * caustic * ahead * 0.1 * env;

    f_color = vec4(color, 1.0);
}
