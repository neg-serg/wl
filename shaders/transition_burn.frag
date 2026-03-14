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

// --- Hashing ---
vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)),
             dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

float hash1(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

// --- Voronoi with 2nd-nearest for crack detection ---
// Returns: x = dist to nearest, y = cell noise, z = dist to 2nd nearest
vec3 voronoi2(vec2 p, float scale) {
    vec2 sp = p * scale;
    vec2 i = floor(sp);
    vec2 f = fract(sp);

    float d1 = 1.0, d2 = 1.0;
    float cell_val = 0.0;

    for (int y = -2; y <= 2; y++) {
        for (int x = -2; x <= 2; x++) {
            vec2 neighbor = vec2(float(x), float(y));
            vec2 point = hash2(i + neighbor);
            vec2 diff = neighbor + point - f;
            float d = length(diff);
            if (d < d1) {
                d2 = d1;
                d1 = d;
                cell_val = hash1(i + neighbor);
            } else if (d < d2) {
                d2 = d;
            }
        }
    }

    return vec3(d1, cell_val, d2);
}

// --- Value noise ---
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash1(i);
    float b = hash1(i + vec2(1.0, 0.0));
    float c = hash1(i + vec2(0.0, 1.0));
    float d = hash1(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// --- fBM turbulence: 6 octaves ---
float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8); // domain rotation reduces axis-aligned artifacts
    for (int i = 0; i < 6; i++) {
        v += a * vnoise(p);
        p = rot * p * 2.0;
        a *= 0.5;
    }
    return v;
}

// --- Fire color ramp: 5-band physically-inspired gradient ---
vec3 fire_color(float t) {
    // t=0: dark red → t=0.25: red-orange → t=0.5: orange → t=0.75: yellow → t=1: white-yellow
    vec3 c1 = vec3(0.15, 0.0, 0.0);   // dark ember
    vec3 c2 = vec3(0.7, 0.1, 0.0);    // deep red
    vec3 c3 = vec3(1.0, 0.35, 0.0);   // orange
    vec3 c4 = vec3(1.0, 0.7, 0.1);    // yellow-orange
    vec3 c5 = vec3(1.0, 0.95, 0.7);   // white-hot

    t = clamp(t, 0.0, 1.0);
    if (t < 0.25) return mix(c1, c2, t * 4.0);
    if (t < 0.5)  return mix(c2, c3, (t - 0.25) * 4.0);
    if (t < 0.75) return mix(c3, c4, (t - 0.5) * 4.0);
    return mix(c4, c5, (t - 0.75) * 4.0);
}

void main() {
    float prog = pc.progress;

    // --- Multi-scale Voronoi ---
    // wave_x controls cell scale: default 20 = medium, higher = finer, lower = coarser
    float base_scale = max(pc.wave_x, 4.0);
    vec3 vor_large = voronoi2(v_uv, base_scale);        // primary burn chunks
    vec3 vor_small = voronoi2(v_uv, base_scale * 2.5);  // fine detail layer

    float cell_dist1   = vor_large.x;
    float cell_noise1  = vor_large.y;
    float cell_crack1  = vor_large.z - vor_large.x; // crack width (2nd - 1st)

    float cell_noise2  = vor_small.y;

    // Blend two scales for richer burn pattern
    float cell_noise = mix(cell_noise1, cell_noise2, 0.3);

    // --- fBM turbulence for irregular burn front ---
    float turb = fbm(v_uv * 8.0);

    // --- Burn direction: bottom-to-top with turbulence ---
    float directional = 1.0 - v_uv.y; // fire rises
    float burn_value = mix(cell_noise, directional, 0.35) + (turb - 0.5) * 0.2;

    // --- Burn threshold ---
    float threshold = prog;
    float edge_dist = burn_value - threshold;

    // --- Burned mask (smooth for anti-aliasing) ---
    float burned = 1.0 - smoothstep(-0.005, 0.005, edge_dist);

    // --- Heat distortion: warp UV near the burn edge ---
    float edge_proximity = 1.0 - smoothstep(0.0, 0.12, abs(edge_dist));
    float heat_strength = edge_proximity * 0.015;
    vec2 heat_offset = vec2(
        fbm(v_uv * 30.0 + vec2(prog * 3.0, 0.0)) - 0.5,
        fbm(v_uv * 30.0 + vec2(0.0, prog * 3.0)) - 0.5
    ) * heat_strength;

    vec2 distorted_uv = v_uv + heat_offset;

    // --- Sample textures with heat distortion ---
    vec2 old_uv = distorted_uv;
    vec2 new_uv = distorted_uv;
    vec4 old_color = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_color = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_color = texture(u_old, old_uv);
    if (apply_resize(new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_color = texture(u_new, new_uv);

    // --- Char darkening: image darkens just before burning ---
    float char_zone = smoothstep(0.15, 0.0, edge_dist) * (1.0 - burned);
    old_color.rgb *= 1.0 - char_zone * 0.6;

    // --- Base color: burned areas show new image ---
    vec3 base = mix(old_color.rgb, new_color.rgb, burned);

    // --- Fire glow along the burn edge ---
    float glow_width = 0.1;
    float glow_raw = smoothstep(glow_width, 0.0, edge_dist) * smoothstep(-0.01, 0.0, edge_dist);

    // Voronoi cracks glow brighter (fire spreads along fractures)
    float crack_glow = 1.0 - smoothstep(0.0, 0.08, cell_crack1);
    float crack_boost = 1.0 + crack_glow * 1.5 * edge_proximity;

    // Fire intensity varies with turbulence
    float fire_intensity = glow_raw * crack_boost;
    fire_intensity *= 0.7 + 0.3 * fbm(v_uv * 15.0 + prog * 2.0); // flickering

    vec3 fire = fire_color(fire_intensity) * fire_intensity * 1.8;

    // --- Ember particles: tiny bright sparks scattered near edge ---
    float ember_noise = vnoise(v_uv * 120.0 + vec2(prog * 5.0, prog * 3.0));
    float ember_mask = step(0.92, ember_noise) * edge_proximity;
    vec3 embers = vec3(1.0, 0.6, 0.1) * ember_mask * 2.0;

    // --- Smoke/ash: slight darkening and desaturation above burned area ---
    float smoke_zone = burned * smoothstep(0.0, 0.3, prog - burn_value);
    float smoke_noise = fbm(v_uv * 6.0 + vec2(0.0, prog * 1.5));
    float smoke = smoke_zone * smoke_noise * 0.25;
    base *= 1.0 - smoke;
    // Desaturate in smoke
    float lum = dot(base, vec3(0.299, 0.587, 0.114));
    base = mix(base, vec3(lum), smoke * 0.5);

    // --- Compose ---
    vec3 final_color = base + fire + embers;

    // Clamp to prevent oversaturation but allow HDR-like bloom feel
    final_color = min(final_color, vec3(1.5));
    final_color = clamp(final_color, 0.0, 1.0);

    f_color = vec4(final_color, 1.0);
}
