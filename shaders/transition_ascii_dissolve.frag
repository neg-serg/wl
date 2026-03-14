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

// Character density patterns (4x6 grid encoded as fill ratios)
// Maps luminance to character density: space . : - = + * # @ █
float char_density(float lum) {
    // 10 density levels
    if (lum < 0.1) return 0.0;       // space
    if (lum < 0.2) return 0.15;      // .
    if (lum < 0.3) return 0.25;      // :
    if (lum < 0.4) return 0.35;      // -
    if (lum < 0.5) return 0.45;      // =
    if (lum < 0.6) return 0.55;      // +
    if (lum < 0.7) return 0.65;      // *
    if (lum < 0.8) return 0.75;      // #
    if (lum < 0.9) return 0.85;      // @
    return 0.95;                      // █
}

// Simple hash for dithering
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

void main() {
    // Grid cell count
    float cell_count = max(pc.wave_x, 80.0);
    float cell_h = cell_count / pc.screen_aspect;

    // Cell coordinates
    vec2 cell = floor(v_uv * vec2(cell_count, cell_h));
    vec2 cell_uv = fract(v_uv * vec2(cell_count, cell_h));

    // Center UV of this cell (for color sampling)
    vec2 cell_center = (cell + 0.5) / vec2(cell_count, cell_h);

    // ASCII effect intensity: peaks at progress=0.5
    float ascii_intensity = sin(pc.progress * 3.14159265);
    ascii_intensity = pow(ascii_intensity, 0.8); // broaden the peak slightly

    // Sample source image at cell center
    vec2 old_uv = cell_center;
    vec2 new_uv = cell_center;
    vec4 old_color = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 new_color = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        old_color = texture(u_old, old_uv);
    if (apply_resize(new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        new_color = texture(u_new, new_uv);

    // Crossfade between old and new
    float fade = smoothstep(0.3, 0.7, pc.progress);
    vec4 source_color = mix(old_color, new_color, fade);

    // Luminance
    float lum = dot(source_color.rgb, vec3(0.299, 0.587, 0.114));

    // Get character density for this luminance
    float density = char_density(lum);

    // Create character-like pattern within the cell
    // Use a simple threshold pattern based on cell position
    float pattern = hash(cell + vec2(0.5));
    float threshold = density;

    // Create blocky character pattern
    float char_pixel = step(1.0 - threshold, hash(cell * 17.31 + cell_uv * 3.7));

    // Mix between normal image and ASCII representation
    vec4 ascii_color = source_color * char_pixel;

    // Also sample the full-resolution image for blending
    vec2 full_old_uv = v_uv;
    vec2 full_new_uv = v_uv;
    vec4 full_old = vec4(0.0, 0.0, 0.0, 1.0);
    vec4 full_new = vec4(0.0, 0.0, 0.0, 1.0);

    if (apply_resize(full_old_uv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        full_old = texture(u_old, full_old_uv);
    if (apply_resize(full_new_uv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        full_new = texture(u_new, full_new_uv);

    vec4 full_color = mix(full_old, full_new, fade);

    // Blend: normal → ASCII → normal
    f_color = mix(full_color, ascii_color, ascii_intensity);
}
