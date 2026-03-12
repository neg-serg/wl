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

// Hash for cell center positions
vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)),
             dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

// Voronoi: returns distance to nearest cell center and cell noise value
vec2 voronoi(vec2 p, float scale) {
    vec2 sp = p * scale;
    vec2 i = floor(sp);
    vec2 f = fract(sp);

    float min_dist = 1.0;
    float cell_val = 0.0;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 neighbor = vec2(float(x), float(y));
            vec2 point = hash2(i + neighbor);
            vec2 diff = neighbor + point - f;
            float d = length(diff);
            if (d < min_dist) {
                min_dist = d;
                // Cell noise value for burn ordering
                cell_val = fract(sin(dot(i + neighbor, vec2(12.9898, 78.233))) * 43758.5453);
            }
        }
    }

    return vec2(min_dist, cell_val);
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

    // Voronoi cells — large round blobs
    vec2 vor = voronoi(v_uv, 12.0);
    float cell_dist = vor.x;   // distance to cell center
    float cell_noise = vor.y;  // per-cell random value

    // Bottom-to-top bias: fire rises
    float biased = mix(cell_noise, 1.0 - v_uv.y, 0.4);

    // A cell burns when its biased value < progress
    float threshold = pc.progress;
    float burned = step(biased, threshold);

    // Base: new where burned, old where not
    vec4 base = mix(old_color, new_color, burned);

    // Ember glow at burn edge — follows cell boundaries
    float edge_dist = biased - threshold;
    float glow_width = 0.08;

    float inner = 1.0 - smoothstep(0.0, glow_width * 0.3, edge_dist);
    float mid   = 1.0 - smoothstep(0.0, glow_width * 0.6, edge_dist);
    float outer = 1.0 - smoothstep(0.0, glow_width,       edge_dist);

    float on_edge = step(0.0, edge_dist) * step(edge_dist, glow_width);

    // Voronoi cell edges get extra glow (fire follows cracks)
    float cell_edge = smoothstep(0.05, 0.15, cell_dist);
    float edge_boost = 1.0 + (1.0 - cell_edge) * 0.5;

    vec3 glow_color = vec3(0.0);
    glow_color += inner * vec3(1.0, 0.7, 0.1);         // bright yellow-orange
    glow_color += (mid - inner) * vec3(1.0, 0.3, 0.0);  // orange-red
    glow_color += (outer - mid) * vec3(0.3, 0.0, 0.0);  // dark red

    f_color = vec4(base.rgb + glow_color * on_edge * edge_boost, 1.0);
}
