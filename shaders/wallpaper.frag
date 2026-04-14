#version 450

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 f_color;

layout(set = 0, binding = 0) uniform sampler2D u_wallpaper;

layout(push_constant) uniform PushConstants {
    // 0 = crop, 1 = fit, 2 = no resize
    uint resize_mode;
    float img_aspect;
    float screen_aspect;
    // Atlas UV offset and scale (for GIF animation frames)
    // For static images: uv_offset=0, uv_scale=1
    float uv_offset;
    float uv_scale;
} pc;

void main() {
    vec2 uv = v_uv;

    if (pc.resize_mode == 0u) {
        // Crop: scale to fill, center-clip overflow
        if (pc.img_aspect > pc.screen_aspect) {
            // Image is wider relative to height → zoom in vertically, crop top/bottom
            float scale = pc.img_aspect / pc.screen_aspect;
            uv.y = uv.y * scale + (1.0 - scale) * 0.5;
        } else {
            // Image is narrower → zoom in horizontally, crop left/right
            float scale = pc.screen_aspect / pc.img_aspect;
            uv.x = uv.x * scale + (1.0 - scale) * 0.5;
        }
    } else if (pc.resize_mode == 1u) {
        // Fit: scale to fit, letterbox with black
        if (pc.img_aspect > pc.screen_aspect) {
            // Image is wider → fit width, letterbox top/bottom
            float scale = pc.screen_aspect / pc.img_aspect;
            float offset = (1.0 - scale) * 0.5;
            if (uv.y < offset || uv.y > 1.0 - offset) {
                f_color = vec4(0.0, 0.0, 0.0, 1.0);
                return;
            }
            uv.y = (uv.y - offset) / scale;
        } else {
            // Image is narrower → fit height, letterbox left/right
            float scale = pc.img_aspect / pc.screen_aspect;
            float offset = (1.0 - scale) * 0.5;
            if (uv.x < offset || uv.x > 1.0 - offset) {
                f_color = vec4(0.0, 0.0, 0.0, 1.0);
                return;
            }
            uv.x = (uv.x - offset) / scale;
        }
    }
    // resize_mode == 2: no resize, UV passthrough (image centered at native size via CPU)

    // Apply atlas UV transformation for animated wallpapers
    uv.x = pc.uv_offset + uv.x * pc.uv_scale;

    f_color = texture(u_wallpaper, uv);
}
