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
float hash(float n) {
    return fract(sin(n) * 43758.5453123);
}

float hash2(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

vec3 hash3(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yxz + 33.33);
    return fract((p3.xxy + p3.yzz) * p3.zyx);
}

// Pseudo-random time seed that changes rapidly
float time_hash(float seed) {
    return hash(floor(pc.progress * 30.0) + seed);
}

// Sample a texture channel with resize
float sample_ch(sampler2D tex, vec2 uv, uint rm, float ia, float sa, int ch) {
    if (apply_resize(uv, rm, ia, sa)) {
        vec4 c = texture(tex, clamp(uv, 0.0, 1.0));
        if (ch == 0) return c.r;
        if (ch == 1) return c.g;
        return c.b;
    }
    return 0.0;
}

void main() {
    float scale = max(pc.wave_x, 4.0);
    float prog = pc.progress;

    // Glitch intensity: aggressive sin^2 curve, peaks at midpoint
    float raw_intensity = sin(prog * 3.14159265);
    float intensity = raw_intensity * raw_intensity;

    // Temporal jitter — different glitch pattern every few frames
    float t = floor(prog * 30.0);

    // === LAYER 1: Horizontal band tearing ===
    // Multiple band scales for layered tears
    float band1_y = floor(v_uv.y * scale * 2.0);
    float band1_noise = hash(band1_y + t * 7.1);
    float band1_active = step(0.6 - intensity * 0.45, band1_noise);
    float tear1 = (band1_noise - 0.5) * 0.2 * intensity * band1_active;

    // Fine sub-bands (higher frequency tears)
    float band2_y = floor(v_uv.y * scale * 8.0);
    float band2_noise = hash(band2_y + t * 13.3);
    float band2_active = step(0.75 - intensity * 0.3, band2_noise);
    float tear2 = (band2_noise - 0.5) * 0.08 * intensity * band2_active;

    // === LAYER 2: Block corruption (multiple scales) ===
    // Large blocks
    vec2 block_l = floor(v_uv * vec2(scale * 0.3, scale * 0.5));
    float bn_l = hash2(block_l + vec2(t * 3.7, t * 2.1));
    float block_l_active = step(0.82 - intensity * 0.35, bn_l);
    float block_l_shift = (bn_l - 0.5) * 0.25 * block_l_active;

    // Medium blocks
    vec2 block_m = floor(v_uv * vec2(scale * 0.8, scale * 1.2));
    float bn_m = hash2(block_m + vec2(t * 5.3, t * 4.7));
    float block_m_active = step(0.78 - intensity * 0.3, bn_m);
    float block_m_shift = (bn_m - 0.5) * 0.12 * block_m_active;

    // Small blocks (pixel-level corruption)
    vec2 block_s = floor(v_uv * vec2(scale * 3.0, scale * 4.0));
    float bn_s = hash2(block_s + vec2(t * 9.1, t * 7.3));
    float block_s_active = step(0.88 - intensity * 0.2, bn_s);
    float block_s_shift = (bn_s - 0.5) * 0.05 * block_s_active;

    // Combined horizontal displacement
    float total_shift = tear1 + tear2 + block_l_shift + block_m_shift + block_s_shift;

    // Vertical jitter on some bands
    float v_jitter = 0.0;
    float vj_band = floor(v_uv.y * scale * 1.5);
    float vj_noise = hash(vj_band + t * 11.7);
    if (vj_noise > 0.9 - intensity * 0.15) {
        v_jitter = (vj_noise - 0.5) * 0.06 * intensity;
    }

    vec2 displaced_uv = v_uv + vec2(total_shift, v_jitter);
    displaced_uv = clamp(displaced_uv, 0.0, 1.0);

    // === LAYER 3: Per-channel RGB split (each channel gets independent offset) ===
    float base_split = intensity * 0.04;

    // Each channel displaced differently per temporal frame
    vec3 rnd = hash3(vec2(t * 1.3, t * 2.7));
    vec2 r_off = vec2(base_split * (rnd.x - 0.3), base_split * (rnd.y - 0.5) * 0.5);
    vec2 g_off = vec2(0.0);  // green stays as anchor
    vec2 b_off = vec2(-base_split * (rnd.z - 0.2), base_split * (rnd.x - 0.5) * 0.5);

    // Extra split in corrupted blocks
    float block_split = (block_l_active + block_m_active) * 0.03 * intensity;
    r_off.x += block_split;
    b_off.x -= block_split;

    // Crossfade
    float blend = smoothstep(0.25, 0.75, prog);

    // Sample R
    float r_old = sample_ch(u_old, displaced_uv + r_off, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect, 0);
    float r_new = sample_ch(u_new, displaced_uv + r_off, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect, 0);
    float r = mix(r_old, r_new, blend);

    // Sample G
    float g_old = sample_ch(u_old, displaced_uv + g_off, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect, 1);
    float g_new = sample_ch(u_new, displaced_uv + g_off, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect, 1);
    float g = mix(g_old, g_new, blend);

    // Sample B
    float b_old = sample_ch(u_old, displaced_uv + b_off, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect, 2);
    float b_new = sample_ch(u_new, displaced_uv + b_off, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect, 2);
    float b = mix(b_old, b_new, blend);

    vec3 color = vec3(r, g, b);

    // === LAYER 4: Color corruption in glitch blocks ===
    // Invert colors in some blocks
    float invert_noise = hash2(block_l + vec2(t * 17.3));
    if (block_l_active > 0.5 && invert_noise > 0.7) {
        color = 1.0 - color;
    }

    // Hue shift in medium blocks
    float hue_noise = hash2(block_m + vec2(t * 23.1));
    if (block_m_active > 0.5 && hue_noise > 0.6) {
        color = color.gbr; // channel rotation
    }

    // Posterize in small blocks (reduce color depth)
    if (block_s_active > 0.5 && bn_s > 0.93) {
        color = floor(color * 4.0) / 4.0;
    }

    // === LAYER 5: Scanline noise ===
    // CRT-style scanlines that intensify with glitch
    float scanline = sin(v_uv.y * scale * 40.0 * 3.14159265) * 0.5 + 0.5;
    scanline = mix(1.0, scanline, intensity * 0.2);
    color *= scanline;

    // Digital static noise
    float static_noise = hash2(v_uv * 800.0 + vec2(t * 100.0));
    float static_mask = step(0.92 - intensity * 0.15, hash(floor(v_uv.y * scale * 3.0) + t * 5.0));
    color = mix(color, vec3(static_noise), static_mask * intensity * 0.6);

    // === LAYER 6: Random full-screen flashes ===
    float flash_rnd = hash(t * 37.7);
    if (flash_rnd > 0.92 && intensity > 0.3) {
        float flash_type = hash(t * 41.3);
        if (flash_type > 0.7) {
            // White flash
            color = mix(color, vec3(1.0), 0.3 * intensity);
        } else if (flash_type > 0.4) {
            // Black flash
            color *= 0.3;
        } else {
            // Color flash
            vec3 flash_color = hash3(vec2(t * 53.0, 0.0));
            color = mix(color, flash_color, 0.25 * intensity);
        }
    }

    // === LAYER 7: Pixel row duplication (freeze glitch) ===
    float dup_band = floor(v_uv.y * scale * 1.0);
    float dup_noise = hash(dup_band + t * 19.7);
    if (dup_noise > 0.93 - intensity * 0.08) {
        // Sample from a shifted row (duplicates a nearby row)
        float src_y = (dup_band + floor(hash(dup_band + t * 3.0) * 3.0) - 1.0) / (scale * 1.0);
        vec2 dup_uv = vec2(displaced_uv.x, clamp(src_y, 0.0, 1.0));

        vec2 dup_old = dup_uv;
        vec2 dup_new = dup_uv;
        vec4 dc_old = vec4(0.0, 0.0, 0.0, 1.0);
        vec4 dc_new = vec4(0.0, 0.0, 0.0, 1.0);
        if (apply_resize(dup_old, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
            dc_old = texture(u_old, dup_old);
        if (apply_resize(dup_new, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
            dc_new = texture(u_new, dup_new);
        color = mix(dc_old.rgb, dc_new.rgb, blend);
    }

    // === LAYER 8: Bit-depth corruption (quantize + offset) ===
    float bit_band = floor(v_uv.y * scale * 5.0);
    float bit_noise = hash(bit_band + t * 29.3);
    if (bit_noise > 0.94 - intensity * 0.06) {
        float levels = mix(2.0, 8.0, hash(bit_band + t * 31.0));
        color = floor(color * levels + 0.5) / levels;
        // Random channel boost
        float boost_ch = hash(bit_band + t * 37.0);
        if (boost_ch > 0.66) color.r = min(color.r * 1.8, 1.0);
        else if (boost_ch > 0.33) color.g = min(color.g * 1.8, 1.0);
        else color.b = min(color.b * 1.8, 1.0);
    }

    f_color = vec4(clamp(color, 0.0, 1.0), 1.0);
}
