#version 450
// GL Transitions — hexagonalize by Fernando Kuteken
// License: MIT
// Source: https://github.com/gl-transitions/gl-transitions

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

vec4 getFromColor(vec2 uv) {
    vec2 ruv = uv;
    if (!apply_resize(ruv, pc.old_resize_mode, pc.old_img_aspect, pc.screen_aspect))
        return vec4(0.0, 0.0, 0.0, 1.0);
    return texture(u_old, ruv);
}

vec4 getToColor(vec2 uv) {
    vec2 ruv = uv;
    if (!apply_resize(ruv, pc.new_resize_mode, pc.new_img_aspect, pc.screen_aspect))
        return vec4(0.0, 0.0, 0.0, 1.0);
    return texture(u_new, ruv);
}

const int steps = 50;
const float horizontalHexagons = 50.0;
const float edgeWidth = 0.15;
const vec3 edgeColor = vec3(1.0, 1.0, 1.0);

struct Hexagon {
    float q;
    float r;
    float s;
};

Hexagon createHexagon(float q, float r) {
    Hexagon hex;
    hex.q = q;
    hex.r = r;
    hex.s = -q - r;
    return hex;
}

Hexagon roundHexagon(Hexagon hex) {
    float q = floor(hex.q + 0.5);
    float r = floor(hex.r + 0.5);
    float s = floor(hex.s + 0.5);

    float deltaQ = abs(q - hex.q);
    float deltaR = abs(r - hex.r);
    float deltaS = abs(s - hex.s);

    if (deltaQ > deltaR && deltaQ > deltaS)
        q = -r - s;
    else if (deltaR > deltaS)
        r = -q - s;
    else
        s = -q - r;

    return createHexagon(q, r);
}

Hexagon hexagonFromPoint(vec2 point, float size) {
    point.y /= pc.screen_aspect;
    point = (point - 0.5) / size;

    float q = (sqrt(3.0) / 3.0) * point.x + (-1.0 / 3.0) * point.y;
    float r = 0.0 * point.x + 2.0 / 3.0 * point.y;

    Hexagon hex = createHexagon(q, r);
    return roundHexagon(hex);
}

vec2 pointFromHexagon(Hexagon hex, float size) {
    float x = (sqrt(3.0) * hex.q + (sqrt(3.0) / 2.0) * hex.r) * size + 0.5;
    float y = (0.0 * hex.q + (3.0 / 2.0) * hex.r) * size + 0.5;

    return vec2(x, y * pc.screen_aspect);
}

vec4 transition(vec2 uv) {
    float dist = 2.0 * min(pc.progress, 1.0 - pc.progress);
    dist = steps > 0 ? ceil(dist * float(steps)) / float(steps) : dist;

    float size = (sqrt(3.0) / 3.0) * dist / horizontalHexagons;

    if (dist <= 0.0) {
        return mix(getFromColor(uv), getToColor(uv), pc.progress);
    }

    Hexagon hex = hexagonFromPoint(uv, size);
    vec2 center = pointFromHexagon(hex, size);

    // Distance from pixel to hex center (normalized by size)
    vec2 delta = uv - center;
    delta.y /= pc.screen_aspect;
    float d = length(delta) / size;

    // Edge glow: bright outline at hex boundaries, fades with transition progress
    float edge = smoothstep(1.0 - edgeWidth, 1.0, d);
    float edgeIntensity = dist * edge;

    vec4 color = mix(getFromColor(center), getToColor(center), pc.progress);
    color.rgb = mix(color.rgb, edgeColor, edgeIntensity);

    return color;
}

void main() {
    f_color = transition(v_uv);
}
