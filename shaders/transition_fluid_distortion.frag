#version 450
// Fluid Distortion — viscous liquid melt/pour transition

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

// --- Hash & noise ---
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float hash(float p) {
    return fract(sin(p * 127.1) * 43758.5453);
}

float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Domain-warped fBM for organic shapes
float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    mat2 rot = mat2(0.87, 0.50, -0.50, 0.87);
    for (int i = 0; i < 5; i++) {
        v += a * value_noise(p);
        p = rot * p * 2.03;
        a *= 0.47;
    }
    return v;
}

// Domain warping — makes noise look like swirling liquid
float warped_fbm(vec2 p, float time) {
    vec2 q = vec2(
        fbm(p + vec2(1.7, 9.2) + time * 0.15),
        fbm(p + vec2(8.3, 2.8) + time * 0.12)
    );
    vec2 r = vec2(
        fbm(p + 4.0 * q + vec2(1.2, 3.4) + time * 0.08),
        fbm(p + 4.0 * q + vec2(5.1, 6.3) + time * 0.10)
    );
    return fbm(p + 4.0 * r);
}

// Metaball field — soft blob shapes
float metaball(vec2 p, vec2 center, float radius) {
    float d = length(p - center);
    return radius * radius / (d * d + 0.001);
}

void main() {
    const float PI = 3.14159265;
    float t = pc.progress;

    // Envelope: strong in middle, zero at edges
    float env = sin(t * PI);
    float env2 = env * env;
    float env3 = env2 * env;

    // Time for animation
    float anim = t * 4.0;

    // Aspect-corrected coordinates
    vec2 uv = v_uv;
    vec2 auv = vec2(uv.x * pc.screen_aspect, uv.y);

    // === PHASE 1: Liquid blob mask ===
    // Multiple blobs grow and merge to form the liquid boundary
    float blob_field = 0.0;

    // 8 blobs with pseudo-random positions that drift
    for (int i = 0; i < 8; i++) {
        float fi = float(i);
        vec2 pos = vec2(
            hash(fi * 7.13) * pc.screen_aspect,
            hash(fi * 13.7)
        );
        // Blobs drift slowly
        pos += vec2(
            sin(anim * 0.7 + fi * 2.1) * 0.15,
            cos(anim * 0.5 + fi * 1.7) * 0.15
        );
        // Blob radius grows with progress
        float r = mix(0.0, 0.15 + hash(fi * 3.1) * 0.2, smoothstep(0.0, 0.5 + hash(fi) * 0.3, t));
        blob_field += metaball(auv, pos, r);
    }

    // Threshold for sharp-ish liquid boundary
    float liquid_raw = smoothstep(0.8, 1.8, blob_field);

    // Add noise to boundary for organic edges
    float boundary_noise = warped_fbm(auv * 3.0, anim) * 0.5;
    float liquid_mask = smoothstep(0.3, 0.7, liquid_raw + boundary_noise * env - (1.0 - t) * 0.6);

    // Force full transition at end
    liquid_mask = mix(liquid_mask, 1.0, smoothstep(0.75, 1.0, t));
    liquid_mask = mix(0.0, liquid_mask, smoothstep(0.0, 0.15, t));

    // === PHASE 2: Displacement field ===
    // Strong warped displacement inside liquid zones
    float warp1 = warped_fbm(auv * 2.5 + anim * 0.2, anim);
    float warp2 = warped_fbm(auv * 2.5 + vec2(5.0) + anim * 0.15, anim * 1.1);
    vec2 warp_disp = (vec2(warp1, warp2) - 0.5) * 0.25 * env2;

    // Gravity pull — liquid drips downward
    float gravity = 0.08 * env2 * liquid_mask;
    warp_disp.y += gravity;

    // Ripple waves spreading from blob centers
    float ripple_sum = 0.0;
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        vec2 rp = vec2(hash(fi * 7.13) * pc.screen_aspect, hash(fi * 13.7));
        float d = length(auv - rp);
        float wave_front = t * 2.0 - fi * 0.15;
        float ripple = sin(d * 40.0 - wave_front * 15.0) * exp(-d * 4.0) * exp(-max(0.0, d - wave_front) * 10.0);
        ripple_sum += ripple;
    }
    warp_disp += vec2(ripple_sum) * 0.03 * env;

    // Displacement is stronger inside the liquid zone
    vec2 disp = warp_disp * (0.3 + liquid_mask * 0.7);

    // === PHASE 3: Sample with displacement ===
    vec2 old_uv = uv + disp;
    vec2 new_uv = uv + disp * 0.5; // new image is more stable

    // Chromatic aberration at liquid boundary
    float boundary_dist = abs(liquid_mask - 0.5) * 2.0;
    float at_boundary = 1.0 - boundary_dist;
    float chroma = at_boundary * 0.015 * env;
    vec2 chroma_dir = normalize(vec2(warp1 - 0.5, warp2 - 0.5) + 0.001);

    // Old image with chromatic split
    vec3 old_col;
    old_col.r = sample_old(old_uv + chroma_dir * chroma).r;
    old_col.g = sample_old(old_uv).g;
    old_col.b = sample_old(old_uv - chroma_dir * chroma).b;

    // New image with chromatic split
    vec3 new_col;
    new_col.r = sample_new(new_uv + chroma_dir * chroma * 0.5).r;
    new_col.g = sample_new(new_uv).g;
    new_col.b = sample_new(new_uv - chroma_dir * chroma * 0.5).b;

    // === PHASE 4: Compose ===
    vec3 color = mix(old_col, new_col, liquid_mask);

    // === Liquid surface effects ===

    // Specular highlight on liquid surface (fake normal from noise gradient)
    float eps = 0.005;
    float h0 = warped_fbm(auv * 3.0, anim);
    float hx = warped_fbm(auv * 3.0 + vec2(eps, 0.0), anim);
    float hy = warped_fbm(auv * 3.0 + vec2(0.0, eps), anim);
    vec2 surface_normal = vec2(hx - h0, hy - h0) / eps;
    float specular = pow(max(dot(normalize(vec2(-0.5, -0.7)), normalize(surface_normal)), 0.0), 16.0);
    color += vec3(1.0, 0.95, 0.9) * specular * 0.5 * env2 * liquid_mask;

    // Fresnel-like darkening at liquid edges
    float edge = at_boundary * at_boundary;
    float fresnel_dark = edge * 0.25 * env;
    color *= 1.0 - fresnel_dark;

    // Bright caustic lines at boundary
    float caustic_edge = pow(at_boundary, 8.0);
    color += vec3(0.6, 0.8, 1.0) * caustic_edge * 0.8 * env2;

    // Subtle color tint inside liquid (cool blue-green)
    vec3 liquid_tint = vec3(0.85, 0.92, 1.0);
    color = mix(color, color * liquid_tint, liquid_mask * 0.3 * env);

    f_color = vec4(color, 1.0);
}
