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

// plan-116-A: the block arrives in a storage buffer indexed by instance rather than in
// a push constant — see `mfb_canvas.vert` for why both of that transport's properties
// had to go.
struct ItemBlock {
    ivec4 quad;
    ivec4 shape;   // p0..p3 (16.16 px); for a glyph, the bitmap origin in WHOLE px
    ivec4 fill;
    ivec4 stroke;
    ivec4 misc;    // kind, radius (16.16), strokeHalf (16.16), edgeCount / glyph width
    ivec4 arc;     // startAngle / glyph height, endAngle (16.16 rad), edgeBase, unused
    ivec4 surface; // width, height, blendMode, unused
    ivec4 clip;    // the clip rectangle x0,y0,x1,y1 (16.16 px); zero-area = unclipped
};

layout(std430, set = 0, binding = 1) readonly buffer Items {
    ItemBlock blocks[];
} itemBuf;

// The instance index the vertex stage read, flat-interpolated. `gl_InstanceIndex` does
// not exist in a fragment shader, so it has to travel as a varying.
layout(location = 0) flat in int vItem;

// A private global rather than a local in `main`, because `shapeDistance` and
// `glyphCoverage` below read `item` directly. Making it a local would mean threading
// twenty-eight ints through both of them for no gain; `main` fills it on its first
// line, before anything can read it.
ItemBlock item;

// The polygon edge buffer: four 16.16 ints per edge, the two endpoints. It is a
// storage buffer rather than more push constants because a polygon carries an
// unbounded number of edges and the guaranteed push-constant range is 128 bytes,
// which the item block already fills. This is the only reason the pipeline needs a
// descriptor set at all.
//
// One buffer serves the whole frame — a command buffer is recorded once and executed
// once, so per-item rebinding would give every polygon the last one's edges. Each
// polygon reads from `item.arc.z` instead. (Metal has no such problem:
// `setFragmentBytes:` copies each item's edges into the command buffer at record
// time, so its edge base is always zero.)
//
// `dx`, `dy` and `invLenSq` are recomputed here rather than carried. The software
// cache stores them to keep a reciprocal off its per-pixel path; a GPU has the divide
// for free, and `invLenSq` is the one header quantity 16.16 represents badly — a
// 100-px edge gives 1e-4, which is 6 in 16.16.
layout(std430, set = 0, binding = 0) readonly buffer Edges {
    int values[];
} edges;

// Where the glyph coverage region starts inside that same buffer. Kept in step with
// `VULKAN_GLYPH_BASE_WORDS` by a unit test rather than by two hand-edited numbers.
const int GLYPH_BASE = 65536;

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

// Nearest edge for the magnitude, a crossing count for the sign — the same shape
// `__canvas_edgeDistance` has, so the polygon shares the fill, the stroke and the
// antialiasing rather than needing its own coverage rule. A scanline filler and an
// SDF filler would disagree about edge pixels, and that is exactly the disagreement
// an oracle cannot have.
float edgeDistance(int base, int count, vec2 p) {
    float best = 1.0e6;
    bool inside = false;
    for (int e = 0; e < count; ++e) {
        int i = (base + e) * 4;
        vec2 a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
        vec2 b = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3]));
        best = min(best, segmentDistance(p, a, b));
        if ((a.y > p.y) != (b.y > p.y)) {
            float u = (p.y - a.y) / (b.y - a.y);
            if (p.x < a.x + u * (b.x - a.x)) { inside = !inside; }
        }
    }
    return inside ? -best : best;
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
    if (item.misc.x == 4) {
        return edgeDistance(item.arc.z, item.misc.w, p);
    }
    // The empty kind (5) — `Picture`, which draws nothing until it has an atlas — and
    // anything unknown. A glyph (6) never reaches here: it has coverage, not a
    // distance, and `main` handles it before asking for one.
    return 1.0e6;
}

// A glyph's coverage, read straight from the cache the software rasteriser filled.
//
// There is no distance field and no sampler. The bitmap already *is* antialiased
// coverage — the CPU evaluated the outline's signed distance once per sample when it
// filled the cache — so the GPU's job is a lookup, and any filtering here would be a
// second antialiasing pass over an already-antialiased image.
//
// The pen is on a whole pixel and the quad is the bitmap's exact box, so the mapping is
// integer and exact. Outside the box is zero rather than clamped: a quad can cover a
// pixel its bitmap does not, and clamping would smear the border row outward.
int glyphCoverage(vec2 p) {
    int ix = int(p.x) - item.shape.x;
    int iy = int(p.y) - item.shape.y;
    if (ix < 0 || iy < 0 || ix >= item.misc.w || iy >= item.arc.x) { return 0; }
    return edges.values[GLYPH_BASE + item.arc.z + iy * item.misc.w + ix];
}

float srgbToLinear(float c) {
    c = c / 255.0;
    return c <= 0.04045 ? (c / 12.92) : pow((c + 0.055) / 1.055, 2.4);
}

vec4 covered(ivec4 rgba, int coverage) {
    float a = float((rgba.w * coverage) / 255) / 255.0;
    return vec4(srgbToLinear(float(rgba.x)) * a,
                srgbToLinear(float(rgba.y)) * a,
                srgbToLinear(float(rgba.z)) * a, a);
}

vec4 premultiplied(ivec4 rgba, float distance) {
    return covered(rgba, int(clamp(0.5 - distance, 0.0, 1.0) * 255.0 + 0.5));
}

void main() {
    // First, before anything reads it: everything below, and both helpers above, work
    // off this one record.
    item = itemBuf.blocks[vItem];
    if (item.misc.x == 6) {
        // A glyph is fill-only: a text item's stroke was turned into an outline
        // polygon by the geometry builder, so there is nothing here to stroke.
        fragColor = covered(item.fill, glyphCoverage(gl_FragCoord.xy));
        return;
    }
    float d = geoDistance(gl_FragCoord.xy);
    vec4 colour = premultiplied(item.fill, d);
    float halfWidth = fx(item.misc.z);
    if (halfWidth > 0.0) {
        // Stroke over fill, then the hardware puts that over the destination — which
        // is what makes this one fragment equal to the software path's two sequential
        // writes, since `over` is associative.
        vec4 s = premultiplied(item.stroke, abs(d) - halfWidth);
        colour = s + colour * (1.0 - s.w);
    }
    fragColor = colour;
}
