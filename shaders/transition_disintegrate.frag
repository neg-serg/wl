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

float hash1f(float n) {
    return fract(sin(n) * 43758.5453123);
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

// --- fBM: 5 octaves with rotation ---
float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 5; i++) {
        v += a * vnoise(p);
        p = rot * p * 2.0;
        a *= 0.5;
    }
    return v;
}

// --- Voronoi with 2nd-nearest (5x5 search for quality) ---
vec4 voronoi2(vec2 p, float scale) {
    vec2 sp = p * scale;
    vec2 i = floor(sp);
    vec2 f = fract(sp);

    float d1 = 1.0, d2 = 1.0;
    float cell_val = 0.0;
    vec2 cell_center = vec2(0.0);

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
                cell_center = (i + neighbor + point) / scale;
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    // x=dist, y=cell_noise, z=crack_width, w unused
    return vec4(d1, cell_val, d2 - d1, 0.0);
}

// --- Energy color ramp ---
vec3 energy_color(float t) {
    t = clamp(t, 0.0, 1.0);
    vec3 c1 = vec3(0.1, 0.05, 0.2);    // deep purple
    vec3 c2 = vec3(0.4, 0.1, 0.8);     // violet
    vec3 c3 = vec3(0.2, 0.5, 1.0);     // electric blue
    vec3 c4 = vec3(0.6, 0.9, 1.0);     // cyan-white
    vec3 c5 = vec3(1.0, 1.0, 1.0);     // white-hot

    if (t < 0.25) return mix(c1, c2, t * 4.0);
    if (t < 0.5)  return mix(c2, c3, (t - 0.25) * 4.0);
    if (t < 0.75) return mix(c3, c4, (t - 0.5) * 4.0);
    return mix(c4, c5, (t - 0.75) * 4.0);
}

void main() {
    float prog = pc.progress;
    float base_scale = max(pc.wave_x * 0.67, 5.0);

    // --- Multi-scale Voronoi particles ---
    vec4 vor_coarse = voronoi2(v_uv, base_scale);        // large fragments
    vec4 vor_fine   = voronoi2(v_uv, base_scale * 3.0);  // fine dust

    float cell_noise_c = vor_coarse.y;
    float cell_noise_f = vor_fine.y;
    float crack_c = vor_coarse.z;
    float crack_f = vor_fine.z;

    // Blended disintegration value
    float cell_noise = mix(cell_noise_c, cell_noise_f, 0.35);

    // --- Disintegration front with fBM turbulence ---
    float turb = fbm(v_uv * 6.0);
    float directional = v_uv.y; // bottom-to-top sweep
    float disint_value = mix(cell_noise, directional, 0.4) + (turb - 0.5) * 0.18;

    float threshold = prog * 1.15;
    float edge_dist = disint_value - threshold;

    // Smooth disintegration mask
    float dissolved = 1.0 - smoothstep(-0.01, 0.01, edge_dist);

    // --- Particle drift: fragments fly away before dissolving ---
    // Time since this particle started dissolving
    float particle_age = max(0.0, threshold - disint_value);
    particle_age = clamp(particle_age, 0.0, 0.3);

    // Wind direction (from angle parameter)
    float wind_angle = radians(pc.angle);
    vec2 wind_dir = vec2(cos(wind_angle), sin(wind_angle));

    // Each particle gets a unique drift velocity
    vec2 particle_id = floor(v_uv * base_scale);
    float drift_speed = hash1(particle_id * 7.3) * 0.5 + 0.5;
    float drift_angle = (hash1(particle_id * 13.7) - 0.5) * 1.5;
    vec2 drift_dir = wind_dir + vec2(cos(drift_angle), sin(drift_angle)) * 0.4;

    // Accelerating drift
    vec2 drift = drift_dir * particle_age * particle_age * drift_speed * 2.0;

    // Particles also shrink and fade as they drift
    float particle_fade = 1.0 - smoothstep(0.0, 0.25, particle_age);
    float particle_scale = 1.0 - particle_age * 2.0;

    // Drifted UV for sampling the old image fragment
    vec2 drifted_uv = v_uv - drift;

    // --- Sample textures ---
    vec2 old_uv = drifted_uv;
    vec2 new_uv = v_uv;
    vec4 old_color = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_color = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_color = texture(u_old, old_uv);
    if (apply_resize(new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_color = texture(u_new, new_uv);

    // Drifting particles desaturate and warm up (turning to ash)
    float desat = particle_age * 3.0;
    float lum = dot(old_color.rgb, vec3(0.299, 0.587, 0.114));
    vec3 ash_color = mix(old_color.rgb, vec3(lum) * vec3(1.1, 0.9, 0.7), clamp(desat, 0.0, 1.0));
    old_color.rgb = ash_color * particle_fade;

    // --- Compositing ---
    // In the dissolved zone: show new image underneath
    // In the edge zone: old particles drifting away over new image
    float in_edge_zone = smoothstep(0.0, 0.15, -edge_dist) * smoothstep(0.3, 0.0, -edge_dist);
    float show_old = max(1.0 - dissolved, in_edge_zone * particle_fade);

    vec3 base = mix(new_color.rgb, old_color.rgb, show_old);

    // --- Energy field at disintegration front ---
    float edge_proximity = smoothstep(0.12, 0.0, abs(edge_dist));

    // Crackling energy along Voronoi edges
    float crack_energy = (1.0 - smoothstep(0.0, 0.06, crack_c)) * edge_proximity;
    float fine_crack = (1.0 - smoothstep(0.0, 0.04, crack_f)) * edge_proximity;

    // Turbulent energy field
    float energy_turb = fbm(v_uv * 20.0 + prog * 3.0);
    float energy_intensity = edge_proximity * (0.6 + 0.4 * energy_turb);
    energy_intensity += crack_energy * 1.5 + fine_crack * 0.8;

    // HDR energy glow
    vec3 energy = energy_color(energy_intensity) * energy_intensity * 2.5;

    // Secondary outer glow (softer, wider)
    float outer_glow = smoothstep(0.2, 0.0, abs(edge_dist)) * 0.3;
    energy += energy_color(0.3) * outer_glow;

    // --- Electric arcs along cracks ---
    float arc_noise = vnoise(v_uv * 80.0 + vec2(prog * 10.0, 0.0));
    float arc = step(0.85, arc_noise) * edge_proximity * 2.0;
    energy += vec3(0.7, 0.8, 1.0) * arc;

    // --- Ember/spark particles flying from the edge ---
    float spark_noise = hash1(floor(v_uv * 200.0) + vec2(floor(prog * 20.0)));
    float spark_vis = step(0.97, spark_noise) * edge_proximity;

    // Sparks have random warm colors
    vec3 spark_color = mix(vec3(1.0, 0.8, 0.3), vec3(0.5, 0.7, 1.0), hash1(floor(v_uv * 200.0)));
    energy += spark_color * spark_vis * 3.0;

    // --- Fine dust particles in the wake ---
    float dust_zone = smoothstep(0.0, 0.25, -edge_dist) * (1.0 - smoothstep(0.25, 0.5, -edge_dist));
    float dust = hash1(floor(v_uv * base_scale * 6.0) + vec2(floor(prog * 8.0)));
    float dust_vis = step(0.9, dust) * dust_zone * 0.4;
    float dust_lum = hash1(floor(v_uv * base_scale * 6.0) * 3.7);
    base += mix(vec3(0.8, 0.7, 0.6), vec3(0.3, 0.4, 0.6), dust_lum) * dust_vis;

    // --- Compose final ---
    vec3 final_color = base + energy;

    // Subtle vignette darkening on fully dissolved areas (enhances depth)
    float depth_dark = dissolved * 0.05;
    final_color -= depth_dark;

    f_color = vec4(clamp(final_color, 0.0, 1.0), 1.0);
}
