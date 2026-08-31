#version 450

// plan-98-F: the Vulkan twin of plan-98-E's MSL fragment shader.
//
// Same signed distance fields the software rasteriser evaluates, and the same
// coverage quantization — `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` then the integer
// `(alpha * coverage) / 255`. That quantization is not a detail: near full coverage
// the sRGB encode is steep enough that one coverage step moves a dark channel by up
// to 13 output steps, so blending in float against an integer-coverage oracle cannot
// agree to two steps on an antialiased edge.
//
// `gl_FragCoord.xy` is the pixel centre with a top-left origin, matching the software
// path's `px = x + 0.5, py = y + 0.5` — the same property `[[position]]` has in MSL.

layout(push_constant) uniform Item {
    ivec4 quad;
    ivec4 shape;
    ivec4 fill;
    ivec4 stroke;
    ivec4 misc;
    ivec4 arc;
    ivec4 surface;
} item;

layout(location = 0) out vec4 fragColor;

const float FIXED = 65536.0;
const float PI = 3.141592653589793;

float fx(int v) { return float(v) / FIXED; }

float rectDistance(vec2 p, vec2 c, vec2 h) {
    vec2 d = abs(p - c) - h;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

float segmentDistance(vec2 p, vec2 a, vec2 b) {
    vec2 v = b - a;
    vec2 w = p - a;
    float len2 = dot(v, v);
    float t = len2 > 0.0 ? clamp(dot(w, v) / len2, 0.0, 1.0) : 0.0;
    return length(w - v * t);
}

bool arcInSweep(vec2 d, vec2 s, vec2 e, bool reflex) {
    bool afterStart = s.x * d.y - s.y * d.x >= 0.0;
    bool beforeEnd  = e.x * d.y - e.y * d.x <= 0.0;
    return reflex ? (afterStart || beforeEnd) : (afterStart && beforeEnd);
}

float geoDistance(vec2 p) {
    float radius = fx(item.misc.y);
    vec2 c = vec2(fx(item.shape.x), fx(item.shape.y));
    if (item.misc.x == 0) {
        return rectDistance(p, c, vec2(fx(item.shape.z), fx(item.shape.w))) - radius;
    }
    if (item.misc.x == 1) {
        return length(p - c) - fx(item.shape.z) - radius;
    }
    if (item.misc.x == 2) {
        return segmentDistance(p, c, vec2(fx(item.shape.z), fx(item.shape.w))) - radius;
    }
    if (item.misc.x == 3) {
        vec2 d = p - c;
        float a0 = fx(item.arc.x);
        float a1 = fx(item.arc.y);
        vec2 s = vec2(cos(a0), sin(a0));
        vec2 e = vec2(cos(a1), sin(a1));
        if (!arcInSweep(d, s, e, (a1 - a0) > PI)) { return 1.0e6; }
        return abs(length(d) - fx(item.shape.z)) - radius;
    }
    // Polygon (kind 4) and the empty kind (5) draw nothing until plan-98-F Phase 2
    // adds the edge buffer, which needs a descriptor set — push constants cannot
    // carry 4 KB of edges.
    return 1.0e6;
}

float srgbToLinear(float c) {
    c = c / 255.0;
    return c <= 0.04045 ? (c / 12.92) : pow((c + 0.055) / 1.055, 2.4);
}

vec4 premultiplied(ivec4 rgba, float distance) {
    int coverage = int(clamp(0.5 - distance, 0.0, 1.0) * 255.0 + 0.5);
    float a = float((rgba.w * coverage) / 255) / 255.0;
    return vec4(srgbToLinear(float(rgba.x)) * a,
                srgbToLinear(float(rgba.y)) * a,
                srgbToLinear(float(rgba.z)) * a, a);
}

void main() {
    float d = geoDistance(gl_FragCoord.xy);
    vec4 colour = premultiplied(item.fill, d);
    float halfWidth = fx(item.misc.z);
    if (halfWidth > 0.0) {
        vec4 s = premultiplied(item.stroke, abs(d) - halfWidth);
        colour = s + colour * (1.0 - s.w);
    }
    fragColor = colour;
}
