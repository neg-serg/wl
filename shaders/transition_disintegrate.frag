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

// Hash for Voronoi cell centers
vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)),
             dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Voronoi: returns (distance_to_center, cell_noise, distance_to_edge)
vec3 voronoi(vec2 p, float scale) {
    vec2 sp = p * scale;
    vec2 i = floor(sp);
    vec2 f = fract(sp);

    float min_dist = 1.0;
    float second_dist = 1.0;
    float cell_val = 0.0;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 neighbor = vec2(float(x), float(y));
            vec2 point = hash2(i + neighbor);
            vec2 diff = neighbor + point - f;
            float d = length(diff);
            if (d < min_dist) {
                second_dist = min_dist;
                min_dist = d;
                cell_val = fract(sin(dot(i + neighbor, vec2(12.9898, 78.233))) * 43758.5453);
            } else if (d < second_dist) {
                second_dist = d;
            }
        }
    }

    // Edge distance: difference between closest and second closest
    float edge = second_dist - min_dist;
    return vec3(min_dist, cell_val, edge);
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

    // Voronoi bubble cells — small soap-bubble sized
    vec3 vor = voronoi(v_uv, 25.0);
    float cell_dist = vor.x;    // distance to cell center
    float cell_noise = vor.y;   // per-cell random
    float cell_edge = vor.z;    // distance to cell boundary

    // Bottom-to-top sweep with cell noise
    float sweep = mix(cell_noise, v_uv.y, 0.5);

    // Cell disintegrates when sweep < progress
    float threshold = pc.progress * 1.2;
    float dissolved = step(sweep, threshold);

    // Near-edge detection for glow
    float edge_dist = sweep - threshold;
    float near_edge = smoothstep(0.12, 0.0, edge_dist) * step(0.0, edge_dist);

    // Bubble membrane: thin iridescent edge on each cell
    float membrane = smoothstep(0.08, 0.03, cell_edge);

    // Iridescent rainbow color based on cell angle and distance
    float angle = atan(v_uv.y - 0.5, v_uv.x - 0.5);
    float iridescence = sin(cell_dist * 30.0 + angle * 3.0 + pc.progress * 5.0) * 0.5 + 0.5;
    vec3 bubble_color = mix(
        mix(vec3(0.8, 0.2, 1.0), vec3(0.2, 0.8, 1.0), iridescence),      // purple to cyan
        mix(vec3(1.0, 0.5, 0.8), vec3(0.3, 1.0, 0.5), iridescence),      // pink to green
        sin(cell_noise * 6.28) * 0.5 + 0.5
    );

    // Cells near edge float upward before popping
    float lift = near_edge * 0.03;
    vec2 lifted_uv = v_uv + vec2((cell_noise - 0.5) * near_edge * 0.02, -lift);
    lifted_uv = clamp(lifted_uv, 0.0, 1.0);

    vec4 lifted_old = vec4(0.0, 0.0, 0.0, 1.0);
    vec2 lo_uv = lifted_uv;
    if (apply_resize(lo_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        lifted_old = texture(u_old, lo_uv);

    // Fade out cells as they lift
    float fade = smoothstep(0.0, 0.08, edge_dist);
    vec4 old_lifted = mix(old_color, lifted_old, near_edge);
    old_lifted.rgb *= fade;

    // Base compositing
    vec4 base = mix(new_color, old_lifted, 1.0 - dissolved);

    // Bubble membrane glow on surviving cells
    float membrane_glow = membrane * (1.0 - dissolved) * 0.4;
    base.rgb += bubble_color * membrane_glow;

    // Iridescent pop flash at disintegration edge
    float pop_flash = near_edge * membrane * 1.5;
    base.rgb += bubble_color * pop_flash;

    // Faint sparkle particles at edge
    float sparkle = hash(v_uv * 300.0 + vec2(pc.progress * 50.0));
    float sparkle_vis = step(0.96, sparkle) * near_edge * 1.5;
    base.rgb += vec3(1.0, 0.95, 0.9) * sparkle_vis;

    f_color = vec4(clamp(base.rgb, 0.0, 1.0), 1.0);
}
