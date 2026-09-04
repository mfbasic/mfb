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
    ivec4 arc;     // startAngle / glyph height, endAngle (16.16 rad), edgeBase, capStyle
    ivec4 surface; // width, height, blendMode, gradient kind (0 linear, 1 radial)
    ivec4 clip;    // the clip rectangle x0,y0,x1,y1 (16.16 px); zero-area = unclipped
    ivec4 xform0;  // inverse transform ia,ib,ic,id as float32 BITS
    ivec4 xform1;  // itx, ity (float32 bits), hasTransform (0 or 1), unused
    ivec4 arcCaps; // an arc's two sweep endpoints startX,startY,endX,endY (16.16 px)
    ivec4 ellipse; // an ellipse's rotation cos, sin (16.16); then a gradient's
                   //   stop count and first-stop index
    ivec4 gradient; // a gradient's axis startX,startY,endX,endY (16.16 px)
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

// Where the gradient region starts in that same buffer (plan-116-F), kept in step
// with `VULKAN_GRADIENT_BASE_WORDS` by a unit test rather than by two hand-edited
// numbers — the arrangement `GLYPH_BASE` already has.
const int GRADIENT_BASE = 1114112;

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

// The same segment cut square at its endpoints instead of capped with a disc
// (plan-116-D). A butt stroke is the round BAND intersected with the slab between the
// two end planes, so `half` is taken off before the `max` rather than after it — doing
// it after compares the plane distance against the half-width instead of against zero,
// and the cap then only bites more than `half` past the endpoint.
float segmentDistanceButt(vec2 p, vec2 a, vec2 b, float half_) {
    vec2 v = b - a;
    vec2 w = p - a;
    float len2 = dot(v, v);
    if (len2 <= 0.0) { return 1.0e6; }
    float length_ = sqrt(len2);
    float t = dot(w, v) / len2;
    float d = length(w - v * clamp(t, 0.0, 1.0)) - half_;
    d = max(d, -t * length_);
    return max(d, (t - 1.0) * length_);
}

// Signed distance to a rotated ellipse (plan-116-E) — 24 halvings of a bisection on
// the folded first quadrant, the same count and the same arithmetic the software
// oracle uses. Not Newton: plan-116-E Phase 1 measured a fixed-count Newton at 127.5
// coverage steps wrong at every count from 1 to 8, because outside the evolute of an
// eccentric ellipse the squared distance has three stationary points in the quadrant.
// The bisection's bracket is guaranteed by construction after the |q| fold.
//
// `ca`/`sa` are the CPU's deterministic Taylor pair, carried in the item block, so this
// calls no trigonometry — which is what lets an ellipse agree with the oracle far more
// closely than the arc kind does.
float ellipseDistance(vec2 p, vec2 c, float rx, float ry, float ca, float sa) {
    vec2 d = p - c;
    vec2 q = abs(vec2(d.x * ca + d.y * sa, -d.x * sa + d.y * ca));
    // Equal radii short-circuit to the exact circle distance, so an Ellipse with
    // rx == ry is the Circle of that radius rather than merely close to it.
    if (rx == ry) { return length(q) - rx; }
    vec2 a = vec2(1.0, 0.0);
    vec2 b = vec2(0.0, 1.0);
    vec2 m = a;
    for (int i = 0; i < 24; ++i) {
        m = normalize(a + b);
        float g = (q.x - rx * m.x) * (-rx * m.y) + (q.y - ry * m.y) * (ry * m.x);
        if (g > 0.0) { a = m; } else { b = m; }
    }
    float dist = length(q - vec2(rx * m.x, ry * m.y));
    vec2 u = vec2(q.x / rx, q.y / ry);
    return dot(u, u) < 1.0 ? -dist : dist;
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

// plan-116-C: the inverse transform, decoded from the float32 bits the item block
// carries. `intBitsToFloat` is a reinterpret, not a conversion — the CPU already did
// the narrowing (`__canvas_float32Bits`), because this compiler's assemblers have no
// double→single convert.
bool hasTransform() { return item.xform1.z != 0; }

vec2 inverseMap(vec2 p) {
    return vec2(intBitsToFloat(item.xform0.x) * p.x + intBitsToFloat(item.xform0.z) * p.y
                    + intBitsToFloat(item.xform1.x),
                intBitsToFloat(item.xform0.y) * p.x + intBitsToFloat(item.xform0.w) * p.y
                    + intBitsToFloat(item.xform1.y));
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
        // Round is 1 and is what a Line did before plan-116-D, so it reads as the
        // straight path. The butt arm returns the finished band distance and does not
        // subtract `radius` again.
        if (item.arc.w == 1) {
            return segmentDistance(p, c, vec2(fx(item.shape.z), fx(item.shape.w))) - radius;
        }
        return segmentDistanceButt(p, c, vec2(fx(item.shape.z), fx(item.shape.w)), radius);
    }
    if (item.misc.x == 3) {
        vec2 d = p - c;
        float a0 = fx(item.arc.x);
        float a1 = fx(item.arc.y);
        vec2 s = vec2(cos(a0), sin(a0));
        vec2 e = vec2(cos(a1), sin(a1));
        float band = arcInSweep(d, s, e, (a1 - a0) > PI)
            ? abs(length(d) - fx(item.shape.z)) - radius
            : 1.0e6;
        // Butt is 0 and is what an Arc did before plan-116-D — the sweep test already
        // cuts the band along a radius at each end — so it returns untouched. Round
        // unions a disc of the stroke's half-width at each sweep endpoint, and a union
        // of SDFs is their min. The endpoints are per-shape constants the CPU wrote,
        // so this costs two distances and no trigonometry.
        if (item.arc.w == 0) { return band; }
        vec2 cs = vec2(fx(item.arcCaps.x), fx(item.arcCaps.y));
        vec2 ce = vec2(fx(item.arcCaps.z), fx(item.arcCaps.w));
        band = min(band, length(p - cs) - radius);
        return min(band, length(p - ce) - radius);
    }
    if (item.misc.x == 7) {
        return ellipseDistance(p, c, fx(item.shape.z), fx(item.shape.w),
                               fx(item.ellipse.x), fx(item.ellipse.y)) - radius;
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
    // `floor`, not a cast: a cast truncates toward zero, and a transformed glyph maps
    // to NEGATIVE shape-space coordinates (ink runs up from the pen), where truncation
    // picks the texel on the wrong side. Untransformed, `p` is a surface pixel centre
    // and always positive, so this is the same value the cast gave.
    int ix = int(floor(p.x)) - item.shape.x;
    int iy = int(floor(p.y)) - item.shape.y;
    if (ix < 0 || iy < 0 || ix >= item.misc.w || iy >= item.arc.x) { return 0; }
    return edges.values[GLYPH_BASE + item.arc.z + iy * item.misc.w + ix];
}

float srgbToLinear(float c) {
    c = c / 255.0;
    return c <= 0.04045 ? (c / 12.92) : pow((c + 0.055) / 1.055, 2.4);
}

// The colour a gradient shows at `p`, as an `ivec4` of 0..255 (plan-116-F).
//
// Returned encoded rather than linear so it drops straight into `covered`, which is
// what decodes — the *interpolation* still happens in linear light, which is the whole
// point of §4.3 and the one thing that has to match the oracle. Each channel is
// therefore decoded, mixed and re-encoded here.
//
// The stops are five words each in the buffer's third region: the offset in 16.16,
// then four whole 0..255 channels. Offsets arrive already clamped and monotonic from
// `__canvas_gradientTail`, so this walk needs no defence against a stop that goes
// backwards — and it is a linear walk for the reason `edgeDistance` is: the count is
// small, bounded, and the shape matches what the oracle does.
// One entry of the oracle's 256-entry forward table, recomputed rather than uploaded.
//
// `__color_srgbTable` — the oracle's table, in the `color` package since plan-122-B
// (it was `__canvas_srgbTable`) — holds the curve ROUNDED to integers on a 0..65535
// scale, and the lerp below happens in that quantised space, so the shader has to
// quantise the same way or it lands a step off wherever the true value sits near a
// boundary.
//
// This reproduction is invisible to any grep for the table: it spells neither the
// constant nor the function name, it just recomputes the same expression. If the
// oracle's quantisation ever changes, the test that catches it is
// `the_gpu_draws_the_gradient_scene_the_reference_shows` in `tests/rt_canvas_golden.rs`,
// not the blend tests.
float srgbTable(int i) {
    return floor(srgbToLinear(float(i)) * 65535.0 + 0.5);
}

// One channel of a gradient, reproducing `__canvas_gradientChannel` exactly.
//
// `num` is the position within the stop pair, already quantised to 1/4096 by
// TRUNCATION, because that is what the oracle does — and a continuous `f` here instead
// biases the whole ramp by up to a step, which measured as 2.3% of the gradient scene
// wrong when this was a float `mix`.
//
// The result is the byte whose LINEAR value is nearest, the rule the oracle's binary
// search applies; the analytic encode lands within one byte, so testing the midpoint
// either side settles it in two comparisons rather than eight.
int gradientChannel(int loSrgb, int hiSrgb, int num) {
    float loLin = srgbTable(loSrgb);
    float hiLin = srgbTable(hiSrgb);
    float lin = loLin + trunc((hiLin - loLin) * float(num) / 4096.0);
    float e = lin <= 205.4 ? (lin / 65535.0 * 12.92)
                           : (1.055 * pow(lin / 65535.0, 1.0 / 2.4) - 0.055);
    int c = int(clamp(e * 255.0, 0.0, 255.0));
    if (c > 0 && lin <= floor((srgbTable(c - 1) + srgbTable(c)) * 0.5)) {
        return c - 1;
    }
    if (c < 255 && lin > floor((srgbTable(c) + srgbTable(c + 1)) * 0.5)) {
        return c + 1;
    }
    return c;
}

ivec4 gradientColour(vec2 p) {
    int count = item.ellipse.z;
    int base = GRADIENT_BASE + item.ellipse.w * 5;
    vec2 from = vec2(fx(item.gradient.x), fx(item.gradient.y));
    vec2 to = vec2(fx(item.gradient.z), fx(item.gradient.w));
    vec2 axis = to - from;
    float len2 = dot(axis, axis);
    float t = 0.0;
    if (item.surface.w == 1) {
        // Radial: distance from the centre, over the axis length. A zero-length axis
        // leaves t at 0, so the first stop fills — defined, not a divide by zero.
        float len = sqrt(len2);
        if (len > 0.0) { t = length(p - from) / len; }
    } else {
        if (len2 > 0.0) { t = dot(p - from, axis) / len2; }
    }
    t = clamp(t, 0.0, 1.0);

    // The first stop at or past t; before the first and after the last, the end stop's
    // colour holds rather than extrapolating.
    int idx = count;
    for (int i = 0; i < count; ++i) {
        if (idx == count && fx(edges.values[base + i * 5]) >= t) { idx = i; }
    }
    if (idx >= count) {
        int last = base + (count - 1) * 5;
        return ivec4(edges.values[last + 1], edges.values[last + 2],
                     edges.values[last + 3], edges.values[last + 4]);
    }
    if (idx <= 0) {
        return ivec4(edges.values[base + 1], edges.values[base + 2],
                     edges.values[base + 3], edges.values[base + 4]);
    }
    int lo = base + (idx - 1) * 5;
    int hi = base + idx * 5;
    float loOff = fx(edges.values[lo]);
    float hiOff = fx(edges.values[hi]);
    // Quantised to 1/4096 by truncation, exactly as `__canvas_gradientColor` does — the
    // one number both paths have to agree on before any colour arithmetic starts.
    int num = hiOff > loOff ? int((t - loOff) / (hiOff - loOff) * 4096.0) : 0;
    num = clamp(num, 0, 4096);
    int loA = edges.values[lo + 4];
    int hiA = edges.values[hi + 4];
    return ivec4(
        gradientChannel(edges.values[lo + 1], edges.values[hi + 1], num),
        gradientChannel(edges.values[lo + 2], edges.values[hi + 2], num),
        gradientChannel(edges.values[lo + 3], edges.values[hi + 3], num),
        loA + int(trunc(float(hiA - loA) * float(num) / 4096.0)));
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

// The clip's own antialiased coverage at this pixel, 0..255 (plan-116-B).
//
// The same `rectDistance` and the same `clamp(0.5 - d, 0, 1)` quantization the shape
// edges use, so a fractional clip edge is antialiased identically to a shape edge —
// which is what lets the oracle and this shader agree on it rather than merely come
// close. `__canvas_clipCoverage` in `helper_items.rs` is the same three lines.
//
// A zero-area rectangle means unclipped and returns 255, matching `__canvas_hasClip`:
// testing both extents also rejects a negative one, which `Bounds` cannot forbid.
int clipCoverage(vec2 p) {
    if (item.clip.x >= item.clip.z || item.clip.y >= item.clip.w) { return 255; }
    vec2 lo = vec2(fx(item.clip.x), fx(item.clip.y));
    vec2 hi = vec2(fx(item.clip.z), fx(item.clip.w));
    float d = rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5);
    return int(clamp(0.5 - d, 0.0, 1.0) * 255.0 + 0.5);
}

/// The shape-space distance at `p` and the local scale of the mapping, as `(d, s)`.
///
/// plan-116-C. Untransformed, this is the distance and 1.0 — the same single evaluation
/// the shader always did. Transformed, it is the distance at the inverse-mapped point
/// and `‖∇d‖` by CENTRAL DIFFERENCES at epsilon 0.5, so the `/2ε` divisor is exactly 1.
///
/// The epsilon is part of the specified result and not a tuning knob: the oracle
/// (`__canvas_drawGeometry`) uses the same one, and Phase 1's measurement is against
/// this value. Central differences rather than `fwidth` for the reason
/// `06_canvas.md` gives — a hardware derivative differs between platforms, and this
/// uses only `+ - * /` and `sqrt`.
///
/// Five distance evaluations when transformed, one otherwise. The branch is uniform
/// across an instance, so it costs a predicted branch rather than divergence.
vec2 shapeDistanceAndScale(vec2 p) {
    if (!hasTransform()) { return vec2(geoDistance(p), 1.0); }
    float d = geoDistance(inverseMap(p));
    float gx = geoDistance(inverseMap(p + vec2(0.5, 0.0)))
             - geoDistance(inverseMap(p - vec2(0.5, 0.0)));
    float gy = geoDistance(inverseMap(p + vec2(0.0, 0.5)))
             - geoDistance(inverseMap(p - vec2(0.0, 0.5)));
    float g = sqrt(gx * gx + gy * gy);
    return vec2(d, g > 0.000001 ? g : 1.0);
}

void main() {
    // First, before anything reads it: everything below, and both helpers above, work
    // off this one record.
    item = itemBuf.blocks[vItem];
    // The clip multiplies the shape's own coverage, exactly as it does in the oracle's
    // `(coverage * clipCov) / 255`. Integer, and by 255 rather than a shift, so the two
    // quantize identically — a float multiply here would disagree on the boundary
    // pixels, which are the only ones a clip can affect.
    int clipCov = clipCoverage(gl_FragCoord.xy);
    if (item.misc.x == 6) {
        // A glyph is fill-only: a text item's stroke was turned into an outline
        // polygon by the geometry builder, so there is nothing here to stroke.
        //
        // plan-116-C §4.5: a transformed glyph samples its bitmap at the inverse-mapped
        // point, nearest. `glyphCoverage` already indexes by whole pixels, so mapping
        // the query point is the whole change — the cache stays untransformed and one
        // entry serves every transform.
        vec2 gp = hasTransform() ? inverseMap(gl_FragCoord.xy) : gl_FragCoord.xy;
        fragColor = covered(item.fill, (glyphCoverage(gp) * clipCov) / 255);
        return;
    }
    // `dRaw` is in SHAPE space and `dScale` the local scale; the fill uses the surface
    // distance and the stroke subtracts `half` BEFORE converting, so the outline scales
    // with the shape (§4.3). Untransformed, `dScale` is 1.0 and both collapse to the
    // expressions this shader had.
    vec2 ds = shapeDistanceAndScale(gl_FragCoord.xy);
    float dRaw = ds.x;
    float dScale = ds.y;
    float d = dRaw / dScale;
    // plan-116-F: the gradient replaces the fill COLOUR and nothing else — the shape's
    // distance, its coverage and its stroke are untouched.
    ivec4 fillRgba = item.ellipse.z >= 2 ? gradientColour(gl_FragCoord.xy) : item.fill;
    vec4 colour = covered(fillRgba,
        (int(clamp(0.5 - d, 0.0, 1.0) * 255.0 + 0.5) * clipCov) / 255);
    float halfWidth = fx(item.misc.z);
    if (halfWidth > 0.0) {
        // Stroke over fill, then the hardware puts that over the destination — which
        // is what makes this one fragment equal to the software path's two sequential
        // writes, since `over` is associative.
        //
        // plan-116-B: that identity is `Normal`-ONLY, which is why a non-`Normal`
        // stroked-and-filled item is emitted as two adjacent instances instead (§4.3).
        // By the time such an item reaches here it is either fill-only or stroke-only,
        // so this branch composes two sources only under `Normal`, where it is exact.
        vec4 s = covered(item.stroke,
            (int(clamp(0.5 - (abs(dRaw) - halfWidth) / dScale, 0.0, 1.0) * 255.0 + 0.5) * clipCov) / 255);
        colour = s + colour * (1.0 - s.w);
    }
    fragColor = colour;
}
