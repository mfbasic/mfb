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
    ivec4 arc;     // glyph/picture height, arc reflex flag (bug-735) / polygon fill rule
                   //   (plan-163-A, 0 EvenOdd, 1 NonZero), edgeBase, capStyle
    ivec4 surface; // width, height, blendMode, gradient kind (0 linear, 1 radial)
    ivec4 clip;    // the clip rectangle x0,y0,x1,y1 (16.16 px); zero-area = unclipped
    ivec4 xform0;  // inverse transform ia,ib,ic,id as float32 BITS
    ivec4 xform1;  // itx, ity (float32 bits), hasTransform (0 or 1), unused
    ivec4 arcCaps; // an arc's two sweep endpoints startX,startY,endX,endY (16.16 px);
                   //   for a polygon, its band header (bug-688, `polygonBand`)
    ivec4 ellipse; // an ellipse's rotation cos, sin (16.16); then a gradient's
                   //   stop count and first-stop index
    ivec4 gradient; // a gradient's axis startX,startY,endX,endY (16.16 px)
    // bug-735: the residual words (units of 2^-47 px) of every field a distance reads.
    ivec4 shapeLo;  // of shape.xyzw
    ivec4 capsLo;   // of arcCaps.xyzw
    ivec4 clipLo;   // of clip.xyzw
    ivec4 miscLo;   // of misc.y (radius), misc.z (strokeHalf), ellipse.x, ellipse.y
    // plan-163-B: the stroke's join records -- first edge slot, record count -- then
    // its dash pattern's first entry and entry count.
    ivec4 strokeStyle;
};

layout(std430, set = 0, binding = 1) readonly buffer Items {
    ItemBlock blocks[];
} itemBuf;

// The instance index the vertex stage read, flat-interpolated. `gl_InstanceIndex` does
// not exist in a fragment shader, so it has to travel as a varying.
layout(location = 0) flat in int vItem;

// plan-116-H: the group translation for THIS draw, in 16.16 — the same push constant
// the vertex stage declares. Both stages consume it, which is what makes the range
// valid (H4). bug-735: `zw` are the residual words of `xy` (units of 2^-47 px).
layout(push_constant) uniform Draw {
    ivec4 offset;
} draw;

// A private global rather than a local in `main`, because the distance and coverage
// helpers below read `item` directly. Making it a local would mean threading
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

// Where the other regions start inside that same buffer, in words: glyph coverage and
// picture texels, gradient stops (plan-116-F), and the polygon band index (bug-688).
//
// SPECIALIZATION CONSTANTS, not literals (bug-688). The pipeline feeds each one from
// the Rust layout constant of the same region (`VULKAN_SPEC_CONSTANTS` in
// `runtime/canvas/mod.rs`, through `VkSpecializationInfo`), so the layout is defined
// once and this blob never has to be rebuilt because a cap before a region moved. They
// were literals until then, and every cap was frozen by it. The default 0 is never
// used: a pipeline built without the specialization data would read every region from
// the edges, which `the_pipeline_specializes_every_region_base` rules out.
layout(constant_id = 0) const int GLYPH_BASE = 0;
layout(constant_id = 1) const int GRADIENT_BASE = 0;
layout(constant_id = 2) const int BAND_BASE = 0;
// bug-687: each polygon edge's four residual words, parallel to its 16.16 words.
layout(constant_id = 3) const int EDGE_RESIDUAL_BASE = 0;
// plan-163-B: the dash words (`dashWord`).
layout(constant_id = 4) const int DASH_BASE = 0;

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

// plan-163-B: the same segment with SQUARE ends -- `__canvas_segmentDistanceSquare`: the
// rectangle `half` wider than the segment on every side, `max(perp - half, -t|v| - half,
// (t-1)|v| - half)`; a zero-length one is the axis-aligned square of side 2*half.
float segmentDistanceSquare(vec2 p, vec2 a, vec2 b, float half_) {
    vec2 v = b - a;
    vec2 w = p - a;
    float len2 = dot(v, v);
    if (len2 <= 0.0) { return max(abs(w.x), abs(w.y)) - half_; }
    float length_ = sqrt(len2);
    float t = dot(w, v) / len2;
    float d = abs(w.x * v.y - w.y * v.x) / length_ - half_;
    d = max(d, -t * length_ - half_);
    return max(d, (t - 1.0) * length_ - half_);
}

// bug-687: double-float arithmetic — a value as an unevaluated sum `hi + lo` of two
// floats, about 48 bits. A polygon's distance is evaluated in it because fp32 is not the
// oracle's f64: where the exact distance lies within ~1e-5 px of a coverage quantisation
// boundary the two round to neighbouring steps, and near black one step is 3-4 sRGB
// levels, past GPU_DEFAULT. Metal's shader has the same helpers.
//
// The error terms only exist if the compiler keeps every operation, in the order
// written, rounded once. `precise` is what says so: without it a driver may contract a
// multiply and an add, or treat `(a - (s - v)) + (b - v)` as the 0 it is algebraically.
// TwoProd is Dekker's split rather than `fma`, which GLSL does not promise to fuse.
vec2 dfTwoSum(float a, float b) {
    precise float s = a + b;
    precise float v = s - a;
    precise float e = (a - (s - v)) + (b - v);
    return vec2(s, e);
}
vec2 dfQuickSum(float a, float b) {
    precise float s = a + b;
    precise float e = b - (s - a);
    return vec2(s, e);
}
vec2 dfSplit(float a) {
    precise float t = 4097.0 * a;
    precise float hi = t - (t - a);
    precise float lo = a - hi;
    return vec2(hi, lo);
}
vec2 dfTwoProd(float a, float b) {
    precise float p = a * b;
    vec2 x = dfSplit(a);
    vec2 y = dfSplit(b);
    precise float e = ((x.x * y.x - p) + x.x * y.y + x.y * y.x) + x.y * y.y;
    return vec2(p, e);
}
vec2 dfAdd(vec2 a, vec2 b) {
    vec2 s = dfTwoSum(a.x, b.x);
    vec2 t = dfTwoSum(a.y, b.y);
    precise float lo = s.y + t.x;
    s = dfQuickSum(s.x, lo);
    precise float lo2 = s.y + t.y;
    return dfQuickSum(s.x, lo2);
}
vec2 dfSub(vec2 a, vec2 b) { return dfAdd(a, -b); }
vec2 dfMul(vec2 a, vec2 b) {
    vec2 p = dfTwoProd(a.x, b.x);
    precise float lo = p.y + (a.x * b.y + a.y * b.x);
    return dfQuickSum(p.x, lo);
}
vec2 dfMulF(vec2 a, float b) {
    vec2 p = dfTwoProd(a.x, b);
    precise float lo = p.y + a.y * b;
    return dfQuickSum(p.x, lo);
}
vec2 dfDiv(vec2 a, vec2 b) {
    precise float q1 = a.x / b.x;
    vec2 r = dfSub(a, dfMulF(b, q1));
    precise float q2 = r.x / b.x;
    r = dfSub(r, dfMulF(b, q2));
    precise float q3 = r.x / b.x;
    return dfAdd(dfQuickSum(q1, q2), vec2(q3, 0.0));
}
vec2 dfSqrt(vec2 a) {
    if (a.x <= 0.0) { return vec2(0.0); }
    precise float x = inversesqrt(a.x);
    precise float y = a.x * x;
    vec2 d = dfSub(a, dfTwoProd(y, y));
    precise float c = d.x * (x * 0.5);
    return dfTwoSum(y, c);
}
bool dfLess(vec2 a, vec2 b) { return a.x < b.x || (a.x == b.x && a.y < b.y); }
// A 16.16 word as a double-float, exactly: the two halves are exact floats (at most 23
// and 8 significant bits) and the scale is a power of two.
vec2 dfWord(int v) { return dfTwoSum(float(v & ~255) / FIXED, float(v & 255) / FIXED); }
// A residual word (bug-687, `EDGE_RESIDUAL_SCALE`) in px: units of 2^-47.
float residualPx(int r) { return float(r) * (1.0 / 140737488355328.0); }
// bug-735: the residual encoding for every item-block field a distance reads -- the 16.16
// word and its residual word in the block's `*Lo` ivec4s -- as a double-float, and the few
// double-float helpers every shape kind shares. Metal's, line for line.
vec2 dfFixed(int word, int residual) { return dfAdd(dfWord(word), vec2(residualPx(residual), 0.0)); }
vec2 dfAbs(vec2 a) { return a.x < 0.0 ? -a : a; }
vec2 dfMax(vec2 a, vec2 b) { return dfLess(a, b) ? b : a; }
vec2 dfMin(vec2 a, vec2 b) { return dfLess(b, a) ? b : a; }
vec2 dfLength(vec2 x, vec2 y) { return dfSqrt(dfAdd(dfMul(x, x), dfMul(y, y))); }
// `a < b` for two (word, residual) pairs, exactly: the pair is order-preserving, since a
// residual is at most half a 16.16 step.
bool fixedLess(int wa, int ra, int wb, int rb) { return wa < wb || (wa == wb && ra < rb); }
// One vertex coordinate as the CPU had it: the 16.16 word plus its residual.
vec2 edgeCoord(int i, int r) { return dfFixed(edges.values[i], edges.values[r]); }
// `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` — the quantisation every shape uses — for a
// double-float `d`, rounded exactly.
int dfCoverage(vec2 d) {
    vec2 c = dfSub(vec2(0.5, 0.0), d);
    if (!dfLess(vec2(0.0), c)) { return 0; }
    if (!dfLess(c, vec2(1.0, 0.0))) { return 255; }
    vec2 x = dfAdd(dfMulF(c, 255.0), vec2(0.5, 0.0));
    float k = floor(x.x);
    if (x.x == k && x.y < 0.0) { k -= 1.0; }
    return int(k);
}

// Nearest edge for the magnitude, a crossing count for the sign — the same shape
// `__canvas_edgeDistance` has, so the polygon shares the fill, the stroke and the
// antialiasing rather than needing its own coverage rule. A scanline filler and an
// SDF filler would disagree about edge pixels, and that is exactly the disagreement
// an oracle cannot have.
//
// bug-688 (Metal's bug-686 band index): `list` is where this pixel's band's edge
// numbers start in `edges`, and `count` how many there are -- or `list` is -1 and
// `count` the polygon's edge count, the loop over every edge (`polygonBand`). An edge
// left out of a band is one whose y-span misses the pixel's row (so it cannot toggle
// `inside`) and that is further than the band's reach from it (so it cannot change the
// clamped coverage `best` feeds). Order does not matter: `min` and a parity toggle are
// both order-independent.
//
// bug-687: in double-float, at the shape-space point (`x`, `y`). Each endpoint is its
// 16.16 word plus its residual, so the polygon is the oracle's to ~2^-41 px; the edge
// vector is the integer difference of the words plus the residuals' difference. The
// crossing test is the oracle's `px < ax + (py - ay) / dy * dx`, multiplied out by `dy`
// so it needs no division.
//
// Double-float is spent only where fp32 cannot decide, and that is NOT a tolerance: the
// value returned is the double-float value, pixel for pixel. `edgeSlack` is an ERROR
// BOUND on evaluating one edge in fp32 (the words' conversion, the dropped residual, the
// arithmetic): 2^-18 of the six coordinates' magnitudes plus 2^-12 px, so each edge's
// double-float distance lies in [d32 - slack, d32 + slack]. Pass 1 (fp32) finds `bound`
// (least `d32 + slack`), `low` (least `d32 - slack`) and the parity of every edge whose
// crossing fp32 decides; pass 2 evaluates in double-float only edges that can be the
// nearest (`d32 - slack <= bound`) and the `edgeUnsure` crossings, and is skipped when
// `low` is past `reach`, where coverage is 0 or 255 whatever the distance. Each
// double-float helper has one call site, because lavapipe compiles the whole shader at
// pipeline creation. Metal's `edgeDistance`, line for line, where the argument is set
// out in full.
float edgeSlack(vec2 p, vec2 a, vec2 b) {
    return (abs(p.x) + abs(p.y) + abs(a.x) + abs(a.y) + abs(b.x) + abs(b.y)) * (1.0 / 262144.0) + (1.0 / 4096.0);
}
bool edgeUnsure(vec2 p, vec2 a, vec2 b, float slack) {
    if (abs(a.y - p.y) <= slack || abs(b.y - p.y) <= slack) { return true; }
    if ((a.y > p.y) == (b.y > p.y)) { return false; }
    float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
    return abs(cross) <= slack * (abs(b.x - a.x) + abs(b.y - a.y) + 2.0 * (abs(p.x - a.x) + abs(p.y - a.y)));
}
// plan-163-A: an edge's signed crossing -- 0 when it does not cross the pixel's row left
// of the pixel, else +1 for an edge running down the screen (`b.y > a.y`) and -1 for one
// running up: `__canvas_edgeDistance`'s winding convention. Metal's, line for line.
int edgeCrossing32(vec2 p, vec2 a, vec2 b) {
    if ((a.y > p.y) == (b.y > p.y)) { return 0; }
    float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
    if (b.y > a.y) { return cross < 0.0 ? 1 : 0; }
    return cross > 0.0 ? -1 : 0;
}
// bug-735: `__canvas_rectDistance` in double-float, from the point's offset to the centre
// and the half-extents: a shape's rectangle (less its corner radius) and the clip's.
vec2 dfRectDistance(vec2 dx, vec2 dy, vec2 hx, vec2 hy) {
    vec2 ox = dfSub(dfAbs(dx), hx);
    vec2 oy = dfSub(dfAbs(dy), hy);
    vec2 outside = dfLength(dfMax(ox, vec2(0.0)), dfMax(oy, vec2(0.0)));
    return dfAdd(outside, dfMin(dfMax(ox, oy), vec2(0.0)));
}
// bug-735: a segment's distance, its UNCLAMPED parameter `t` and `len2`, from `w = p - a`
// and `v = b - a` -- `__canvas_segmentDistance`'s arithmetic (and the butt twin's `t`), so
// a polygon edge, a line and a polyline segment share it. `t` stays 0 for a zero-length
// segment, whose distance is to its point.
struct DfSegment { vec2 d; vec2 t; vec2 len2; };
DfSegment dfSegment(vec2 wx, vec2 wy, vec2 vx, vec2 vy) {
    DfSegment s;
    s.len2 = dfAdd(dfMul(vx, vx), dfMul(vy, vy));
    s.t = vec2(0.0);
    vec2 c = vec2(0.0);
    if (s.len2.x > 0.0) {
        s.t = dfDiv(dfAdd(dfMul(wx, vx), dfMul(wy, vy)), s.len2);
        c = s.t;
        if (dfLess(c, vec2(0.0))) { c = vec2(0.0); }
        if (dfLess(vec2(1.0, 0.0), c)) { c = vec2(1.0, 0.0); }
    }
    s.d = dfLength(dfSub(wx, dfMul(vx, c)), dfSub(wy, dfMul(vy, c)));
    return s;
}
struct DfEdge { vec2 ax; vec2 ay; vec2 vx; vec2 vy; };
DfEdge dfEdge(int base, int e) {
    int i = (base + e) * 4;
    int r = (base + e) * 4 + EDGE_RESIDUAL_BASE;
    DfEdge g;
    g.ax = edgeCoord(i, r);
    g.ay = edgeCoord(i + 1, r + 1);
    g.vx = dfAdd(dfWord(edges.values[i + 2] - edges.values[i]),
                 dfTwoSum(residualPx(edges.values[r + 2]), -residualPx(edges.values[r])));
    g.vy = dfAdd(dfWord(edges.values[i + 3] - edges.values[i + 1]),
                 dfTwoSum(residualPx(edges.values[r + 3]), -residualPx(edges.values[r + 1])));
    return g;
}
vec2 dfSegmentDistance(DfEdge g, vec2 x, vec2 y) {
    return dfSegment(dfSub(x, g.ax), dfSub(y, g.ay), g.vx, g.vy).d;
}
int dfCrossing(DfEdge g, vec2 x, vec2 y) {
    if (dfLess(y, g.ay) == dfLess(y, dfAdd(g.ay, g.vy))) { return 0; }
    vec2 lhs = dfMul(dfSub(x, g.ax), g.vy);
    vec2 rhs = dfMul(dfSub(y, g.ay), g.vx);
    if (g.vy.x > 0.0) { return dfLess(lhs, rhs) ? 1 : 0; }
    return dfLess(rhs, lhs) ? -1 : 0;
}
// plan-163-A: `nonZero` is the polygon's fill rule (`arc.y`). The crossings are summed as a
// winding number; NonZero is inside when it is not zero, EvenOdd when it is odd, which is
// the parity the toggles it replaced computed.
vec2 edgeDistance(int base, int count, int list, vec2 x, vec2 y, float reach, int nonZero) {
    vec2 p = vec2(x.x, y.x);
    float bound = 1.0e6;
    float low = 1.0e6;
    bool unsure = false;
    int winding = 0;
    for (int j = 0; j < count; ++j) {
        int i = (base + (list < 0 ? j : edges.values[list + j])) * 4;
        vec2 a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
        vec2 b = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3]));
        float slack = edgeSlack(p, a, b);
        float d32 = segmentDistance(p, a, b);
        bound = min(bound, d32 + slack);
        low = min(low, d32 - slack);
        if (edgeUnsure(p, a, b, slack)) {
            unsure = true;
        } else {
            winding += edgeCrossing32(p, a, b);
        }
    }
    bool distant = low > reach;
    vec2 best = distant ? vec2(low, 0.0) : vec2(1.0e6, 0.0);
    for (int j = 0; j < count && (!distant || unsure); ++j) {
        int e = list < 0 ? j : edges.values[list + j];
        int i = (base + e) * 4;
        vec2 a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
        vec2 b = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3]));
        float slack = edgeSlack(p, a, b);
        vec2 gap = max(max(min(a, b) - p, p - max(a, b)), vec2(0.0, 0.0));
        float within = bound + slack;
        bool candidate = !distant && dot(gap, gap) <= within * within && segmentDistance(p, a, b) - slack <= bound;
        bool unsureEdge = unsure && edgeUnsure(p, a, b, slack);
        if (!candidate && !unsureEdge) { continue; }
        DfEdge g = dfEdge(base, e);
        if (candidate) {
            vec2 d = dfSegmentDistance(g, x, y);
            if (dfLess(d, best)) { best = d; }
        }
        if (unsureEdge) { winding += dfCrossing(g, x, y); }
    }
    bool inside = nonZero != 0 ? winding != 0 : (winding & 1) != 0;
    return inside ? -best : best;
}

// plan-163-B: join records (`helper_joins.rs`). A Miter or Bevel stroke's corners ride the
// edge region after the item's own edges, one record per outline edge -- the corner at
// that edge's START -- as two edge slots: its four points V, A, P, B as eight 16.16 words,
// with their residuals at the residual region's same offsets (`strokeStyle.x` is the
// first slot, `strokeStyle.y` the record count). Its corner PIECE, read from exact
// equalities as the oracle reads it: none when A = V, the disc of radius |A - V| about V
// when A = B, that disc cut by the chord A-B on V's side when P = V (a bevel), and the
// disc with the polygon V, A, P, B otherwise (a miter, P its point). The pieces overlap
// the butt bands they join, so their `min` has no seam. Metal's twins, line for line.
int joinWord(int r, int w) { return (item.strokeStyle.x + r * 2) * 4 + w; }
bool joinSame(int r, int a, int b) {
    int i = joinWord(r, a * 2);
    int j = joinWord(r, b * 2);
    return edges.values[i] == edges.values[j] && edges.values[i + 1] == edges.values[j + 1]
        && edges.values[i + EDGE_RESIDUAL_BASE] == edges.values[j + EDGE_RESIDUAL_BASE]
        && edges.values[i + 1 + EDGE_RESIDUAL_BASE] == edges.values[j + 1 + EDGE_RESIDUAL_BASE];
}
// 0 no piece, 1 the disc, 2 a bevel, 3 a miter.
int joinKind(int r) {
    if (joinSame(r, 1, 0)) { return 0; }
    if (joinSame(r, 1, 3)) { return 1; }
    return joinSame(r, 2, 0) ? 2 : 3;
}
vec2 joinPoint32(int r, int k) {
    int i = joinWord(r, k * 2);
    return vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
}
vec2 joinCoord(int r, int w) {
    int i = joinWord(r, w);
    return edgeCoord(i, i + EDGE_RESIDUAL_BASE);
}
// A record's piece distance in fp32 as the pair (least, most) the double-float one can
// be, widened by `edgeSlack`'s bound: the disc, cut by the chord, or with the polygon --
// its nearest edge, signed by the crossing parity, both signs when a crossing is too
// close to call.
vec2 joinBounds32(int r, int kind, vec2 p) {
    vec2 v = joinPoint32(r, 0);
    vec2 a = joinPoint32(r, 1);
    float ds = 2.0 * edgeSlack(p, v, a);
    float disc = length(p - v) - length(a - v);
    vec2 piece = vec2(disc - ds, disc + ds);
    if (kind == 1) { return piece; }
    vec2 b = joinPoint32(r, 3);
    if (kind == 2) {
        vec2 c = b - a;
        float cs = 2.0 * edgeSlack(p, a, b);
        float o = (c.x * (p.y - a.y) - c.y * (p.x - a.x)) / length(c);
        if (c.x * (v.y - a.y) - c.y * (v.x - a.x) > 0.0) { o = -o; }
        return vec2(max(piece.x, o - cs), max(piece.y, o + cs));
    }
    float d = 1.0e6;
    float slack = 0.0;
    bool unsure = false;
    int winding = 0;
    for (int k = 0; k < 4; ++k) {
        vec2 s0 = joinPoint32(r, k);
        vec2 s1 = joinPoint32(r, (k + 1) & 3);
        float sl = edgeSlack(p, s0, s1);
        slack = max(slack, sl);
        d = min(d, segmentDistance(p, s0, s1));
        if (edgeUnsure(p, s0, s1, sl)) { unsure = true; } else { winding += edgeCrossing32(p, s0, s1); }
    }
    vec2 quad = unsure ? vec2(-d - slack, d + slack)
        : ((winding & 1) != 0 ? vec2(-d - slack, -d + slack) : vec2(d - slack, d + slack));
    return vec2(min(piece.x, quad.x), min(piece.y, quad.y));
}
// The piece in double-float, for the disc and the chord; the polygon's distance `quad`
// (negative inside) is measured by the caller's shared `dfSegment` loop.
vec2 dfJoinPiece(int r, int kind, vec2 x, vec2 y, vec2 quad) {
    vec2 vx = joinCoord(r, 0);
    vec2 vy = joinCoord(r, 1);
    vec2 ax = joinCoord(r, 2);
    vec2 ay = joinCoord(r, 3);
    vec2 disc = dfSub(dfLength(dfSub(x, vx), dfSub(y, vy)), dfLength(dfSub(ax, vx), dfSub(ay, vy)));
    if (kind == 1) { return disc; }
    if (kind == 3) { return dfMin(disc, quad); }
    vec2 cx = dfSub(joinCoord(r, 6), ax);
    vec2 cy = dfSub(joinCoord(r, 7), ay);
    vec2 o = dfDiv(dfSub(dfMul(cx, dfSub(y, ay)), dfMul(cy, dfSub(x, ax))), dfLength(cx, cy));
    vec2 ov = dfSub(dfMul(cx, dfSub(vy, ay)), dfMul(cy, dfSub(vx, ax)));
    if (dfLess(vec2(0.0), ov)) { o = -o; }
    return dfMax(disc, o);
}
// One outline edge of a joined stroke, in fp32: a polygon's band edge `e`, or a
// rectangle's side `e` from its centre and half-extents (`__canvas_rectSides`).
void joinedEdge32(bool poly, int e, out vec2 a, out vec2 b) {
    if (poly) {
        int i = (item.arc.z + e) * 4;
        a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
        b = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3]));
        return;
    }
    vec2 c = vec2(fx(item.shape.x), fx(item.shape.y));
    vec2 h = vec2(fx(item.shape.z), fx(item.shape.w));
    vec2 corner[4] = vec2[4](c - h, vec2(c.x + h.x, c.y - h.y), c + h, vec2(c.x - h.x, c.y + h.y));
    a = corner[e];
    b = corner[(e + 1) & 3];
}
// The stroke distance of a polygon (its band's edges) or a rectangle or picture (its four
// sides) with join records -- `__canvas_joinedStroke`: the min of each outline edge's BUTT
// band and the record at its start. bug-687's two passes: fp32 bounds over every edge and
// record, then double-float for the ones that can be the nearest. `reach` is where the
// coverage is decided by the sign. Every double-float segment goes through ONE `dfSegment`
// call, `k` 0 the outline edge and 1..4 the record's edges, for lavapipe's compile time.
vec2 joinedStroke(ivec2 band, vec2 x, vec2 y, vec2 half2, float reach) {
    bool poly = item.misc.x == 4;
    int count = poly ? band.y : 4;
    vec2 p = vec2(x.x, y.x);
    float halfW = half2.x;
    float bound = 1.0e6;
    float low = 1.0e6;
    for (int j = 0; j < count; ++j) {
        int e = poly ? (band.x < 0 ? j : edges.values[band.x + j]) : j;
        vec2 a;
        vec2 b;
        joinedEdge32(poly, e, a, b);
        float slack = edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0);
        float d32 = segmentDistanceButt(p, a, b, halfW);
        bound = min(bound, d32 + slack);
        low = min(low, d32 - slack);
        int kind = joinKind(e);
        if (kind != 0) {
            vec2 jb = joinBounds32(e, kind, p);
            bound = min(bound, jb.y);
            low = min(low, jb.x);
        }
    }
    if (low > reach) { return vec2(low, 0.0); }
    vec2 cx = dfFixed(item.shape.x, item.shapeLo.x);
    vec2 cy = dfFixed(item.shape.y, item.shapeLo.y);
    vec2 hx = dfFixed(item.shape.z, item.shapeLo.z);
    vec2 hy = dfFixed(item.shape.w, item.shapeLo.w);
    vec2 best = vec2(1.0e6, 0.0);
    for (int j = 0; j < count; ++j) {
        int e = poly ? (band.x < 0 ? j : edges.values[band.x + j]) : j;
        vec2 a;
        vec2 b;
        joinedEdge32(poly, e, a, b);
        float slack = edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0);
        bool edgeNear = segmentDistanceButt(p, a, b, halfW) - slack <= bound;
        int kind = joinKind(e);
        bool joinNear = kind != 0 && joinBounds32(e, kind, p).x <= bound;
        if (!edgeNear && !joinNear) { continue; }
        vec2 jd = vec2(1.0e6, 0.0);
        int winding = 0;
        for (int k = edgeNear ? 0 : 1; k < (joinNear && kind == 3 ? 5 : 1); ++k) {
            DfEdge g;
            if (k == 0 && poly) {
                g = dfEdge(item.arc.z, e);
            } else if (k == 0) {
                int s0 = e;
                int s1 = (e + 1) & 3;
                vec2 ax = (s0 == 0 || s0 == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
                vec2 ay = (s0 < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
                vec2 bx = (s1 == 0 || s1 == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
                vec2 by = (s1 < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
                g.ax = ax;
                g.ay = ay;
                g.vx = dfSub(bx, ax);
                g.vy = dfSub(by, ay);
            } else {
                g.ax = joinCoord(e, (k - 1) * 2);
                g.ay = joinCoord(e, (k - 1) * 2 + 1);
                g.vx = dfSub(joinCoord(e, (k & 3) * 2), g.ax);
                g.vy = dfSub(joinCoord(e, (k & 3) * 2 + 1), g.ay);
            }
            DfSegment s = dfSegment(dfSub(x, g.ax), dfSub(y, g.ay), g.vx, g.vy);
            if (k == 0) {
                if (s.len2.x > 0.0) {
                    vec2 len_ = dfSqrt(s.len2);
                    vec2 d = dfMax(dfSub(s.d, half2), -dfMul(s.t, len_));
                    d = dfMax(d, dfMul(dfSub(s.t, vec2(1.0, 0.0)), len_));
                    if (dfLess(d, best)) { best = d; }
                }
            } else {
                if (dfLess(s.d, jd)) { jd = s.d; }
                winding += dfCrossing(g, x, y);
            }
        }
        if (joinNear) {
            if ((winding & 1) != 0) { jd = -jd; }
            vec2 piece = dfJoinPiece(e, kind, x, y, jd);
            if (dfLess(piece, best)) { best = piece; }
        }
    }
    return best;
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

// bug-688: the edge list a polygon's shape-space point at height `y` visits, as (first
// word in `edges`, length), or (-1, edgeCount) for "every edge" -- any other kind, or a
// polygon the emitter gave no band index (`arcCaps.z` = 0). The band header is
// `arcCaps` = (top, height in 16.16, band count, table offset), written by
// `emit_band_index` in `vulkan.rs`; the table is (first, count) per band and the list
// follows it. A point above the first band or below the last uses the end band, which
// the emitter built to hold every edge that can matter there. Metal's `polygonBand`,
// line for line.
ivec2 polygonBand(float y) {
    int nb = item.arcCaps.z;
    if ((item.misc.x != 4 && item.misc.x != 10) || nb <= 0) { return ivec2(-1, item.misc.w); }
    float k = floor((y - fx(item.arcCaps.x)) / fx(item.arcCaps.y));
    int band = int(clamp(k, 0.0, float(nb - 1)));
    int t = BAND_BASE + item.arcCaps.w;
    return ivec2(t + nb * 2 + edges.values[t + band * 2], edges.values[t + band * 2 + 1]);
}

// plan-154-C: a PolyLine's distance and path parameter, as (d, t) -- the twin of
// `__canvas_polylineScan` in the oracle and of `polylineScan` in Metal's shader. The
// minimum over the segments' round bands is the union that makes every corner round;
// only the first and last LIVE segments (`gradient.x`/`.y`, whole numbers) take a butt
// plane, each on its outer end. A zero-length segment is skipped unless it is the whole
// path (first == last), where it is the round dot. `t` is (segment + clamped t) /
// segments, continuous at a corner, and one walk sets both, so the colour's segment is
// the distance's.
vec2 polylineScan(ivec2 band, vec2 p) {
    int first = item.gradient.x;
    int last = item.gradient.y;
    float halfW = fx(item.misc.y);
    bool butt = item.arc.w == 0;
    bool square = item.arc.w == 2;
    float best = 1.0e6;
    float param = 0.0;
    for (int j = 0; j < band.y; ++j) {
        int e = band.x < 0 ? j : edges.values[band.x + j];
        if (e < first || e > last) { continue; }
        int i = (item.arc.z + e) * 4;
        vec2 a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
        vec2 v = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3])) - a;
        float len2 = dot(v, v);
        if (!(len2 > 0.0) && first != last) { continue; }
        vec2 w = p - a;
        float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;
        float c = clamp(t, 0.0, 1.0);
        // plan-163-B: a square OUTER end measures to the segment's line past its endpoint.
        float at = c;
        if (square && ((e == first && t < 0.0) || (e == last && t > 1.0))) { at = t; }
        float d = length(w - v * at) - halfW;
        if (butt) {
            float len_ = sqrt(len2);
            if (e == first) { d = max(d, -t * len_); }
            if (e == last) { d = max(d, (t - 1.0) * len_); }
        }
        if (square) {
            if (len2 > 0.0) {
                float len_ = sqrt(len2);
                if (e == first) { d = max(d, -t * len_ - halfW); }
                if (e == last) { d = max(d, (t - 1.0) * len_ - halfW); }
            } else {
                d = max(abs(w.x), abs(w.y)) - halfW;
            }
        }
        if (d < best) { best = d; param = (float(e) + c) / float(item.misc.w); }
    }
    return vec2(best, param);
}

// bug-735: every shape kind but the polygon and the glyph, in double-float -- the twin of
// `__canvas_geoDistance` in the oracle, operation for operation, on inputs carried as their
// 16.16 words plus residuals, so each input is the oracle's to ~2^-41 px. fp32 from 16.16
// rounded a pixel near a coverage step to the neighbouring step, 3-12 sRGB levels near
// black. The arc's sweep test is the oracle's two cross products against `arcCaps -
// centre` (a positive multiple of its Taylor `(cos, sin)`) and `arc.y` is its `reflex`,
// decided in f64 by the emitter; the ellipse is the oracle's 24-step bisection with every
// step in double-float. Metal's `dfShapeDistance`, line for line, where the cases are set
// out in full.
vec2 dfShapeDistance(vec2 x, vec2 y) {
    int kind = item.misc.x;
    vec2 cx = dfFixed(item.shape.x, item.shapeLo.x);
    vec2 cy = dfFixed(item.shape.y, item.shapeLo.y);
    vec2 ex = dfFixed(item.shape.z, item.shapeLo.z);
    vec2 ey = dfFixed(item.shape.w, item.shapeLo.w);
    vec2 radius = dfFixed(item.misc.y, item.miscLo.x);
    vec2 dx = dfSub(x, cx);
    vec2 dy = dfSub(y, cy);
    if (kind == 0 || kind == 9) {
        return dfSub(dfRectDistance(dx, dy, ex, ey), radius);
    }
    if (kind == 1) { return dfSub(dfSub(dfLength(dx, dy), ex), radius); }
    if (kind == 2) {
        DfSegment s = dfSegment(dx, dy, dfSub(ex, cx), dfSub(ey, cy));
        if (item.arc.w == 1) { return dfSub(s.d, radius); }
        // plan-163-B: Square -- `__canvas_segmentDistanceSquare`.
        if (item.arc.w == 2) {
            if (!(s.len2.x > 0.0)) { return dfSub(dfMax(dfAbs(dx), dfAbs(dy)), radius); }
            vec2 sl = dfSqrt(s.len2);
            vec2 perp = dfDiv(dfAbs(dfSub(dfMul(dx, dfSub(ey, cy)), dfMul(dy, dfSub(ex, cx)))), sl);
            vec2 sd = dfMax(dfSub(perp, radius), dfSub(-dfMul(s.t, sl), radius));
            return dfMax(sd, dfSub(dfMul(dfSub(s.t, vec2(1.0, 0.0)), sl), radius));
        }
        if (!(s.len2.x > 0.0)) { return vec2(1.0e6, 0.0); }
        vec2 len_ = dfSqrt(s.len2);
        vec2 d = dfMax(dfSub(s.d, radius), -dfMul(s.t, len_));
        return dfMax(d, dfMul(dfSub(s.t, vec2(1.0, 0.0)), len_));
    }
    if (kind == 3) {
        vec2 sx = dfSub(dfFixed(item.arcCaps.x, item.capsLo.x), cx);
        vec2 sy = dfSub(dfFixed(item.arcCaps.y, item.capsLo.y), cy);
        vec2 tx = dfSub(dfFixed(item.arcCaps.z, item.capsLo.z), cx);
        vec2 ty = dfSub(dfFixed(item.arcCaps.w, item.capsLo.w), cy);
        bool afterStart = !dfLess(dfMul(sx, dy), dfMul(sy, dx));
        bool beforeEnd = !dfLess(dfMul(ty, dx), dfMul(tx, dy));
        bool inSweep = item.arc.y != 0 ? (afterStart || beforeEnd) : (afterStart && beforeEnd);
        vec2 band = inSweep ? dfSub(dfAbs(dfSub(dfLength(dx, dy), ex)), radius) : vec2(1.0e6, 0.0);
        if (item.arc.w == 0) { return band; }
        for (int k = 0; k < 2; ++k) {
            vec2 rx = k == 0 ? sx : tx;
            vec2 ry = k == 0 ? sy : ty;
            vec2 ox = dfSub(dx, rx);
            vec2 oy = dfSub(dy, ry);
            if (item.arc.w == 2) {
                // plan-163-B: `__canvas_arcSquareCap` -- the half-width box past the end,
                // along the tangent; `along` is measured outward from the end.
                vec2 rl = dfLength(rx, ry);
                if (!(rl.x > 0.0)) { continue; }
                vec2 ux = dfDiv(rx, rl);
                vec2 uy = dfDiv(ry, rl);
                vec2 along = dfSub(dfMul(ox, uy), dfMul(oy, ux));
                if (k == 1) { along = -along; }
                vec2 across = dfAdd(dfMul(ox, ux), dfMul(oy, uy));
                vec2 box = dfMax(dfMax(dfSub(dfAbs(across), radius), dfSub(along, radius)), -along);
                band = dfMin(band, box);
            } else {
                band = dfMin(band, dfSub(dfLength(ox, oy), radius));
            }
        }
        return band;
    }
    if (kind == 7) {
        vec2 ca = dfFixed(item.ellipse.x, item.miscLo.z);
        vec2 sa = dfFixed(item.ellipse.y, item.miscLo.w);
        vec2 qx = dfAbs(dfAdd(dfMul(dx, ca), dfMul(dy, sa)));
        vec2 qy = dfAbs(dfSub(dfMul(dy, ca), dfMul(dx, sa)));
        if (item.shape.z == item.shape.w && item.shapeLo.z == item.shapeLo.w) {
            return dfSub(dfSub(dfLength(qx, qy), ex), radius);
        }
        vec2 c0 = vec2(1.0, 0.0);
        vec2 s0 = vec2(0.0);
        vec2 c1 = vec2(0.0);
        vec2 s1 = vec2(1.0, 0.0);
        vec2 cm = c0;
        vec2 sm = s0;
        for (int k = 0; k < 24; ++k) {
            vec2 cs = dfAdd(c0, c1);
            vec2 ss = dfAdd(s0, s1);
            vec2 nn = dfLength(cs, ss);
            cm = dfDiv(cs, nn);
            sm = dfDiv(ss, nn);
            vec2 g = dfAdd(dfMul(dfSub(qx, dfMul(ex, cm)), -dfMul(ex, sm)),
                           dfMul(dfSub(qy, dfMul(ey, sm)), dfMul(ey, cm)));
            if (dfLess(vec2(0.0), g)) { c0 = cm; s0 = sm; } else { c1 = cm; s1 = sm; }
        }
        vec2 d = dfLength(dfSub(qx, dfMul(ex, cm)), dfSub(qy, dfMul(ey, sm)));
        vec2 ux = dfDiv(qx, ex);
        vec2 uy = dfDiv(qy, ey);
        if (dfLess(dfAdd(dfMul(ux, ux), dfMul(uy, uy)), vec2(1.0, 0.0))) { d = -d; }
        return dfSub(d, radius);
    }
    return vec2(1.0e6, 0.0);
}

// bug-735: the fp32 pre-pass for the same kinds -- a value `v` such that, whenever
// `|v| > reach`, the distance has `v`'s sign and at least its magnitude, so its coverage is
// `v`'s and double-float is not needed. An ERROR BOUND, like `edgeSlack`, not a tolerance.
// Metal's `farBound`, line for line, where each kind's bound is argued.
float farBound(vec2 p) {
    int kind = item.misc.x;
    vec2 c = vec2(fx(item.shape.x), fx(item.shape.y));
    vec2 e = vec2(fx(item.shape.z), fx(item.shape.w));
    float r = fx(item.misc.y);
    float slack = (abs(p.x) + abs(p.y) + abs(c.x) + abs(c.y) + abs(e.x) + abs(e.y) + abs(r)) * (1.0 / 262144.0) + (1.0 / 4096.0);
    if (kind == 7) {
        vec2 d = p - c;
        float ca = fx(item.ellipse.x);
        float sa = fx(item.ellipse.y);
        float s = length(abs(vec2(d.x * ca + d.y * sa, -d.x * sa + d.y * ca)) / e);
        float rmin = min(e.x, e.y);
        float sl = (abs(d.x) + abs(d.y) + e.x + e.y + 1.0) * (1.0 / 16384.0) + (1.0 / 4096.0);
        return s >= 1.0 ? rmin * (s - 1.0) - r - sl : rmin * (s - 1.0) - r + sl;
    }
    if (kind == 3) {
        vec2 cs = vec2(fx(item.arcCaps.x), fx(item.arcCaps.y));
        vec2 ce = vec2(fx(item.arcCaps.z), fx(item.arcCaps.w));
        float m = abs(length(p - c) - e.x) - r;
        // plan-163-B: a Square end lies within `r * sqrt(2)` of its endpoint (1.4143 > sqrt 2,
        // so this stays a lower bound).
        float capR = item.arc.w == 2 ? r * 1.4143 : r;
        if (item.arc.w != 0) { m = min(m, min(length(p - cs), length(p - ce)) - capR); }
        slack += (abs(cs.x) + abs(cs.y) + abs(ce.x) + abs(ce.y)) * (1.0 / 262144.0);
        return max(m - slack, 0.0);
    }
    float d;
    if (kind == 0 || kind == 9) {
        d = rectDistance(p, c, e) - r;
    } else if (kind == 1) {
        d = length(p - c) - e.x - r;
    } else if (kind == 2) {
        if (item.arc.w != 1 && !(dot(e - c, e - c) > 0.0)) { return 0.0; }
        d = item.arc.w == 1 ? segmentDistance(p, c, e) - r
            : (item.arc.w == 2 ? segmentDistanceSquare(p, c, e, r) : segmentDistanceButt(p, c, e, r));
    } else {
        return 1.0e6;
    }
    return d > 0.0 ? max(d - slack, 0.0) : min(d + slack, 0.0);
}

// bug-735: a PolyLine's distance in double-float -- `__canvas_polylineScan`'s, with bug-687's
// two passes over the band: fp32 `bound` and `low`, then double-float for the segments that
// can be the nearest. Metal's `polylineDistance`, line for line.
float polylineSegment32(int e, vec2 p, float halfW, out float slack) {
    int first = item.gradient.x;
    int last = item.gradient.y;
    int i = (item.arc.z + e) * 4;
    int ri = (item.arc.z + e) * 4 + EDGE_RESIDUAL_BASE;
    bool live = first == last || edges.values[i] != edges.values[i + 2]
        || edges.values[i + 1] != edges.values[i + 3]
        || edges.values[ri] != edges.values[ri + 2]
        || edges.values[ri + 1] != edges.values[ri + 3];
    if (!live) { slack = -1.0; return 1.0e6; }
    vec2 a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
    vec2 b = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3]));
    vec2 v = b - a;
    float len2 = dot(v, v);
    vec2 w = p - a;
    float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;
    float at = clamp(t, 0.0, 1.0);
    bool square = item.arc.w == 2;
    if (square && ((e == first && t < 0.0) || (e == last && t > 1.0))) { at = t; }
    float d = length(w - v * at) - halfW;
    if (item.arc.w == 0 && len2 > 0.0) {
        float len_ = sqrt(len2);
        if (e == first) { d = max(d, -t * len_); }
        if (e == last) { d = max(d, (t - 1.0) * len_); }
    }
    if (square) {
        if (len2 > 0.0) {
            float len_ = sqrt(len2);
            if (e == first) { d = max(d, -t * len_ - halfW); }
            if (e == last) { d = max(d, (t - 1.0) * len_ - halfW); }
        } else {
            d = max(abs(w.x), abs(w.y)) - halfW;
        }
    }
    // plan-163-B: with join records the inner ends are cut butt; the corners are the
    // records.
    if (item.strokeStyle.y > 0 && len2 > 0.0) {
        float len_ = sqrt(len2);
        if (e != first) { d = max(d, -t * len_); }
        if (e != last) { d = max(d, (t - 1.0) * len_); }
    }
    slack = len2 > 0.0 ? edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0) : 1.0e30;
    return d;
}
vec2 polylineDistance(ivec2 band, vec2 x, vec2 y, float reach) {
    int first = item.gradient.x;
    int last = item.gradient.y;
    bool joined = item.strokeStyle.y > 0;
    float halfW = fx(item.misc.y);
    vec2 p = vec2(x.x, y.x);
    float bound = 1.0e6;
    float low = 1.0e6;
    for (int j = 0; j < band.y; ++j) {
        int e = band.x < 0 ? j : edges.values[band.x + j];
        if (e < first || e > last) { continue; }
        float slack;
        float d32 = polylineSegment32(e, p, halfW, slack);
        int kind = joined ? joinKind(e) : 0;
        if (kind != 0) {
            vec2 jb = joinBounds32(e, kind, p);
            bound = min(bound, jb.y);
            low = min(low, jb.x);
        }
        if (slack < 0.0) { continue; }
        bound = min(bound, d32 + slack);
        low = min(low, d32 - slack);
    }
    if (low > reach) { return vec2(low, 0.0); }
    vec2 half2 = dfFixed(item.misc.y, item.miscLo.x);
    vec2 best = vec2(1.0e6, 0.0);
    for (int j = 0; j < band.y; ++j) {
        int e = band.x < 0 ? j : edges.values[band.x + j];
        if (e < first || e > last) { continue; }
        float slack;
        float d32 = polylineSegment32(e, p, halfW, slack);
        bool segNear = slack >= 0.0 && d32 - slack <= bound;
        int kind = joined ? joinKind(e) : 0;
        bool joinNear = kind != 0 && joinBounds32(e, kind, p).x <= bound;
        if (!segNear && !joinNear) { continue; }
        // plan-163-B: `k` 0 the segment, 1..4 the edges of the join record at its start,
        // through one `dfSegment` call.
        vec2 jd = vec2(1.0e6, 0.0);
        int winding = 0;
        for (int k = segNear ? 0 : 1; k < (joinNear && kind == 3 ? 5 : 1); ++k) {
        DfEdge g;
        if (k == 0) {
            g = dfEdge(item.arc.z, e);
        } else {
            g.ax = joinCoord(e, (k - 1) * 2);
            g.ay = joinCoord(e, (k - 1) * 2 + 1);
            g.vx = dfSub(joinCoord(e, (k & 3) * 2), g.ax);
            g.vy = dfSub(joinCoord(e, (k & 3) * 2 + 1), g.ay);
        }
        vec2 wx = dfSub(x, g.ax);
        vec2 wy = dfSub(y, g.ay);
        DfSegment s = dfSegment(wx, wy, g.vx, g.vy);
        if (k > 0) {
            if (dfLess(s.d, jd)) { jd = s.d; }
            winding += dfCrossing(g, x, y);
            continue;
        }
        vec2 d = dfSub(s.d, half2);
        if (item.arc.w == 0 && s.len2.x > 0.0) {
            vec2 len_ = dfSqrt(s.len2);
            if (e == first) { d = dfMax(d, -dfMul(s.t, len_)); }
            if (e == last) { d = dfMax(d, dfMul(dfSub(s.t, vec2(1.0, 0.0)), len_)); }
        }
        // plan-163-B: a square OUTER end -- to the line past the endpoint, then the end
        // plane moved out by the half-width; a lone zero-length segment is the square.
        if (item.arc.w == 2) {
            if (s.len2.x > 0.0) {
                vec2 len_ = dfSqrt(s.len2);
                bool before = e == first && dfLess(s.t, vec2(0.0));
                bool after = e == last && dfLess(vec2(1.0, 0.0), s.t);
                if (before || after) {
                    d = dfSub(dfDiv(dfAbs(dfSub(dfMul(wx, g.vy), dfMul(wy, g.vx))), len_), half2);
                }
                if (e == first) { d = dfMax(d, dfSub(-dfMul(s.t, len_), half2)); }
                if (e == last) { d = dfMax(d, dfSub(dfMul(dfSub(s.t, vec2(1.0, 0.0)), len_), half2)); }
            } else {
                d = dfSub(dfMax(dfAbs(wx), dfAbs(wy)), half2);
            }
        }
        if (joined && s.len2.x > 0.0) {
            vec2 len_ = dfSqrt(s.len2);
            if (e != first) { d = dfMax(d, -dfMul(s.t, len_)); }
            if (e != last) { d = dfMax(d, dfMul(dfSub(s.t, vec2(1.0, 0.0)), len_)); }
        }
        if (dfLess(d, best)) { best = d; }
        }
        if (joinNear) {
            if ((winding & 1) != 0) { jd = -jd; }
            vec2 piece = dfJoinPiece(e, kind, x, y, jd);
            if (dfLess(piece, best)) { best = piece; }
        }
    }
    return best;
}

// plan-163-B: dashes (`helper_dashes.rs`). An item's dash words ride the dash region from
// `strokeStyle.z`, `strokeStyle.w` of them (0 for a solid stroke), as float32 bit
// patterns: the phase, the entry count, the period, each entry's END along the period,
// then the kind's perimeter data -- three words per outline edge (its start and end
// position and its ring's length, negative for an open path), an ellipse's arc-length
// table, an arc's start angle and sweep. Each function is the oracle's in fp32, at
// offsets from the item's own outline taken in double-float, so the precision is the
// outline's, not the coordinates'. Metal's twins, line for line.
float dashWord(int k) { return intBitsToFloat(edges.values[DASH_BASE + item.strokeStyle.z + k]); }
float dfF(vec2 v) { return v.x + v.y; }
float dashWrap(float x) {
    float period = dashWord(2);
    return x - floor(x / period) * period;
}
bool dashOnAt(float x) {
    float w = dashWrap(x);
    int n = int(dashWord(1));
    for (int j = 0; j < n; ++j) {
        if (w < dashWord(3 + j)) { return (j & 1) == 0; }
    }
    return true;
}
bool dashOnBefore(float x) {
    float w = dashWrap(x);
    if (w <= 0.0) { w = dashWord(2); }
    int n = int(dashWord(1));
    for (int j = 0; j < n; ++j) {
        if (w <= dashWord(3 + j)) { return (j & 1) == 0; }
    }
    return false;
}
// `__canvas_dashLocate` then `__canvas_dashAlongPiece`: the signed distance along the
// outline from `u` to the dashes on the piece [s0, s1] (negative inside one), a dash
// extended by `ext` past a TRUE end; those ends near `u` into `ends`, flagged by `has`.
float dashAlong(float u, float s0, float s1, float len_, bool closed, float ext, out vec2 ends, out bvec2 has) {
    float at = min(max(u, s0), s1);
    float phase = dashWord(0);
    float period = dashWord(2);
    int n = int(dashWord(1));
    float w = dashWrap(at + phase);
    float lowW = 0.0;
    float highW = period;
    int j = 0;
    bool found = false;
    for (; j < n; ++j) {
        float c = dashWord(3 + j);
        if (w < c) { highW = c; found = true; break; }
        lowW = c;
    }
    bool on = (j & 1) == 0;
    if (!found) { on = true; lowW = period; highW = period + dashWord(3); }
    float lo = at - (w - lowW);
    float hi = at + (highW - w);
    if (closed) {
        bool onStart = dashOnAt(phase);
        bool onEnd = dashOnBefore(len_ + phase);
        if (lo < 0.0) { lo = on == onEnd ? -1.0e9 : 0.0; }
        if (hi > len_) { hi = on == onStart ? 1.0e9 : len_; }
    } else {
        if (lo < 0.0) { lo = on ? 0.0 : -1.0e9; }
        if (hi > len_) { hi = on ? len_ : 1.0e9; }
    }
    ends = vec2(0.0);
    has = bvec2(false);
    if (on) {
        float left = s0;
        float right = s1;
        if (lo >= s0) { left = lo - ext; has.x = true; ends.x = lo; }
        if (hi <= s1) { right = hi + ext; has.y = true; ends.y = hi; }
        return max(left - u, u - right);
    }
    float d = 1.0e6;
    if (lo > s0) { d = u - lo - ext; has.x = true; ends.x = lo; }
    if (hi < s1) { d = min(d, hi - ext - u); has.y = true; ends.y = hi; }
    return d;
}
// The oracle's trigonometry (`__canvas_atan2`, `__canvas_sin`, `__canvas_cos`) as series in
// `+ - * / sqrt`, not the driver's `atan`/`sin`/`cos`: Vulkan leaves `atan` at 4096 ULP and
// `sin`/`cos` at 2^-11 absolute, which on a real driver moves a dash end on a 60 px circle
// by a coverage step (measured on box 206's Intel UHD 630). Each is folded first, so the
// series runs where it converges: the arctangent's ratio into [0, 1] and halved twice, the
// sine's and cosine's angle into [-pi/2, pi/2].
float dashAtan(float y, float x) {
    float ax = abs(x);
    float ay = abs(y);
    if (ax == 0.0 && ay == 0.0) { return 0.0; }
    bool swapped = ay > ax;
    float z = swapped ? ax / ay : ay / ax;
    z = z / (1.0 + sqrt(1.0 + z * z));
    z = z / (1.0 + sqrt(1.0 + z * z));
    float w = z * z;
    float p = 1.0 / 21.0;
    p = w * p - 1.0 / 19.0;
    p = w * p + 1.0 / 17.0;
    p = w * p - 1.0 / 15.0;
    p = w * p + 1.0 / 13.0;
    p = w * p - 1.0 / 11.0;
    p = w * p + 1.0 / 9.0;
    p = w * p - 1.0 / 7.0;
    p = w * p + 1.0 / 5.0;
    p = w * p - 1.0 / 3.0;
    p = w * p + 1.0;
    float a = 4.0 * z * p;
    if (swapped) { a = PI / 2.0 - a; }
    if (x < 0.0) { a = PI - a; }
    if (y < 0.0) { a = 2.0 * PI - a; }
    return a >= 2.0 * PI ? 0.0 : a;
}
float dashWrapAngle(float a) { return a - 2.0 * PI * floor((a + PI) / (2.0 * PI)); }
float dashFold(float a) {
    float x = dashWrapAngle(a);
    if (x > PI / 2.0) { return PI - x; }
    if (x < -PI / 2.0) { return -PI - x; }
    return x;
}
float dashSin(float a) {
    float x = dashFold(a);
    float x2 = x * x;
    return x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))));
}
float dashCos(float a) {
    float x = dashFold(a);
    float x2 = x * x;
    float m = 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0 * (1.0 - x2 / 90.0))));
    return abs(dashWrapAngle(a)) > PI / 2.0 ? -m : m;
}
// The direction of `q` in [0, 2pi), from +X towards +Y.
float dashAngle(vec2 q) { return dashAtan(q.y, q.x); }
// A join record's corner piece at `w`, the pixel less its vertex V, from its points
// relative to V (`__canvas_joinPieceDistance`).
vec2 joinRel(int r, int k) {
    return vec2(dfF(dfSub(joinCoord(r, k * 2), joinCoord(r, 0))),
                dfF(dfSub(joinCoord(r, k * 2 + 1), joinCoord(r, 1))));
}
float joinPieceAt(int r, int kind, vec2 w) {
    vec2 a = joinRel(r, 1);
    float disc = length(w) - length(a);
    if (kind == 1) { return disc; }
    vec2 b = joinRel(r, 3);
    if (kind == 2) {
        vec2 c = b - a;
        float o = (c.x * (w.y - a.y) - c.y * (w.x - a.x)) / length(c);
        if (c.x * (-a.y) - c.y * (-a.x) > 0.0) { o = -o; }
        return max(disc, o);
    }
    vec2 pts[4] = vec2[4](vec2(0.0), a, joinRel(r, 2), b);
    float d = 1.0e6;
    bool inside = false;
    for (int k = 0; k < 4; ++k) {
        vec2 s0 = pts[k];
        vec2 s1 = pts[(k + 1) & 3];
        d = min(d, segmentDistance(w, s0, s1));
        if ((s0.y > w.y) != (s1.y > w.y) && w.x < s0.x + (w.y - s0.y) / (s1.y - s0.y) * (s1.x - s0.x)) {
            inside = !inside;
        }
    }
    return min(disc, inside ? -d : d);
}
// `__canvas_dashedEdges`: a polygon's or polyline's band edges, a line's one edge, or a
// rectangle's or picture's four sides, each band cut by the dashes on its own [s0, s1],
// with the corner piece wherever a dash runs through a vertex. An open path with no
// length is the dot its cap draws (`__canvas_dashDot`).
float dashedEdges(ivec2 band, vec2 x, vec2 y, float halfW, int cap) {
    int kind = item.misc.x;
    bool poly = kind == 4 || kind == 10;
    int count = poly ? band.y : (kind == 2 ? 1 : 4);
    int data = 3 + int(dashWord(1));
    float phase = dashWord(0);
    float ext = cap == 2 ? halfW : 0.0;
    vec2 cx = dfFixed(item.shape.x, item.shapeLo.x);
    vec2 cy = dfFixed(item.shape.y, item.shapeLo.y);
    vec2 hx = dfFixed(item.shape.z, item.shapeLo.z);
    vec2 hy = dfFixed(item.shape.w, item.shapeLo.w);
    if ((kind == 2 || kind == 10) && dashWord(data + 2) == 0.0) {
        vec2 ox = cx;
        vec2 oy = cy;
        if (kind == 10) {
            DfEdge g0 = dfEdge(item.arc.z, 0);
            ox = g0.ax;
            oy = g0.ay;
        }
        vec2 w0 = vec2(dfF(dfSub(x, ox)), dfF(dfSub(y, oy)));
        if (cap == 0 || !dashOnAt(phase)) { return 1.0e6; }
        if (cap == 2) { return max(abs(w0.x), abs(w0.y)) - halfW; }
        return length(w0) - halfW;
    }
    float best = 1.0e6;
    for (int j = 0; j < count; ++j) {
        int e = poly ? (band.x < 0 ? j : edges.values[band.x + j]) : j;
        vec2 ax;
        vec2 ay;
        vec2 v;
        if (poly) {
            DfEdge g = dfEdge(item.arc.z, e);
            ax = g.ax;
            ay = g.ay;
            v = vec2(dfF(g.vx), dfF(g.vy));
        } else if (kind == 2) {
            ax = cx;
            ay = cy;
            v = vec2(dfF(dfSub(hx, cx)), dfF(dfSub(hy, cy)));
        } else {
            int f = (e + 1) & 3;
            ax = (e == 0 || e == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
            ay = (e < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
            vec2 bx = (f == 0 || f == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
            vec2 by = (f < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
            v = vec2(dfF(dfSub(bx, ax)), dfF(dfSub(by, ay)));
        }
        float len2 = dot(v, v);
        if (!(len2 > 0.0)) { continue; }
        vec2 w = vec2(dfF(dfSub(x, ax)), dfF(dfSub(y, ay)));
        float s0 = dashWord(data + e * 3);
        float s1 = dashWord(data + e * 3 + 1);
        float ring = dashWord(data + e * 3 + 2);
        float el = sqrt(len2);
        float u = s0 + dot(w, v) / el;
        float bandD = abs(w.x * v.y - w.y * v.x) / el - halfW;
        if (max(bandD, max(s0 - u, u - s1) - halfW) < best) {
            vec2 ends;
            bvec2 has;
            float piece = max(bandD, dashAlong(u, s0, s1, abs(ring), ring > 0.0, ext, ends, has));
            if (cap == 1) {
                if (has.x) { piece = min(piece, length(w - v * ((ends.x - s0) / el)) - halfW); }
                if (has.y) { piece = min(piece, length(w - v * ((ends.y - s0) / el)) - halfW); }
            }
            best = min(best, piece);
        }
        if (item.strokeStyle.y > 0) {
            int jk = joinKind(e);
            if (jk != 0) {
                float before = (ring > 0.0 && s0 == 0.0) ? abs(ring) : s0;
                if (dashOnBefore(before + phase) && dashOnAt(s0 + phase)) {
                    vec2 wv = vec2(dfF(dfSub(x, joinCoord(e, 0))), dfF(dfSub(y, joinCoord(e, 1))));
                    best = min(best, joinPieceAt(e, jk, wv));
                }
            }
        }
    }
    return best;
}
// `__canvas_rrectPosition`: a rounded rectangle's perimeter position of the outline point
// nearest `q` (from its centre), from the top side's left end, clockwise.
float rrectPosition(vec2 q, float iw, float ih, float r) {
    float qr = r * (PI / 2.0);
    float ax = abs(q.x) - iw;
    float ay = abs(q.y) - ih;
    if (ax > 0.0 && ay > 0.0) {
        if (q.x > 0.0 && q.y < 0.0) {
            return 2.0 * iw + r * min(max(dashAngle(vec2(q.x - iw, q.y + ih)) - 1.5 * PI, 0.0), PI / 2.0);
        }
        if (q.x > 0.0) {
            return 2.0 * iw + qr + 2.0 * ih + r * min(dashAngle(vec2(q.x - iw, q.y - ih)), PI / 2.0);
        }
        if (q.y > 0.0) {
            return 4.0 * iw + 2.0 * qr + 2.0 * ih + r * min(max(dashAngle(vec2(q.x + iw, q.y - ih)) - PI / 2.0, 0.0), PI / 2.0);
        }
        return 4.0 * iw + 3.0 * qr + 4.0 * ih + r * min(max(dashAngle(vec2(q.x + iw, q.y + ih)) - PI, 0.0), PI / 2.0);
    }
    if (ay >= ax) {
        if (q.y < 0.0) { return q.x + iw; }
        return 2.0 * iw + 2.0 * qr + 2.0 * ih + (iw - q.x);
    }
    if (q.x > 0.0) { return 2.0 * iw + qr + (q.y + ih); }
    return 4.0 * iw + 3.0 * qr + 2.0 * ih + (ih - q.y);
}
// The outline point at perimeter position `s` of a curve, from its centre
// (`__canvas_dashCurveX`).
vec2 dashCurvePoint(int kind, int data, float s) {
    if (kind == 1 || kind == 3) {
        float r = dfF(dfFixed(item.shape.z, item.shapeLo.z));
        float a = s / r + (kind == 3 ? dashWord(data) : 0.0);
        return r * vec2(dashCos(a), dashSin(a));
    }
    if (kind == 7) {
        float rx = dfF(dfFixed(item.shape.z, item.shapeLo.z));
        float ry = dfF(dfFixed(item.shape.w, item.shapeLo.w));
        float ca = dfF(dfFixed(item.ellipse.x, item.miscLo.z));
        float sa = dfF(dfFixed(item.ellipse.y, item.miscLo.w));
        int steps = item.strokeStyle.w - data - 1;
        int lo = 0;
        int hi = steps;
        while (hi - lo > 1) {
            int mid = (lo + hi) / 2;
            if (dashWord(data + mid) <= s) { lo = mid; } else { hi = mid; }
        }
        float t0 = dashWord(data + lo);
        float t1 = dashWord(data + lo + 1);
        float t = (float(lo) + (s - t0) / (t1 - t0)) * (2.0 * PI / float(steps));
        float ox = rx * dashCos(t);
        float oy = ry * dashSin(t);
        return vec2(ox * ca - oy * sa, ox * sa + oy * ca);
    }
    float iw = dfF(dfFixed(item.shape.z, item.shapeLo.z));
    float ih = dfF(dfFixed(item.shape.w, item.shapeLo.w));
    float r = dfF(dfFixed(item.misc.y, item.miscLo.x));
    float qr = r * (PI / 2.0);
    float right = 2.0 * iw + qr;
    float bottomRight = right + 2.0 * ih;
    float bottom = bottomRight + qr;
    float bottomLeft = bottom + 2.0 * iw;
    float left = bottomLeft + qr;
    float topLeft = left + 2.0 * ih;
    if (s < 2.0 * iw) { return vec2(-iw + s, -ih - r); }
    if (s < right) {
        float a = 1.5 * PI + (s - 2.0 * iw) / r;
        return vec2(iw, -ih) + r * vec2(dashCos(a), dashSin(a));
    }
    if (s < bottomRight) { return vec2(iw + r, -ih + (s - right)); }
    if (s < bottom) {
        float a = (s - bottomRight) / r;
        return vec2(iw, ih) + r * vec2(dashCos(a), dashSin(a));
    }
    if (s < bottomLeft) { return vec2(iw - (s - bottom), ih + r); }
    if (s < left) {
        float a = PI / 2.0 + (s - bottomLeft) / r;
        return vec2(-iw, ih) + r * vec2(dashCos(a), dashSin(a));
    }
    if (s < topLeft) { return vec2(-iw - r, ih - (s - left)); }
    float a = PI + (s - topLeft) / r;
    return vec2(-iw, -ih) + r * vec2(dashCos(a), dashSin(a));
}
// `__canvas_dashedCurve`: a circle, arc, ellipse or rounded rectangle, cut at the
// perimeter position of the pixel's nearest outline point.
float dashedCurve(vec2 x, vec2 y, float halfW, int cap) {
    int kind = item.misc.x;
    vec2 q = vec2(dfF(dfSub(x, dfFixed(item.shape.x, item.shapeLo.x))),
                  dfF(dfSub(y, dfFixed(item.shape.y, item.shapeLo.y))));
    int data = 3 + int(dashWord(1));
    float bandD;
    float u;
    float len_;
    bool closed = true;
    if (kind == 1 || kind == 3) {
        float r = dfF(dfFixed(item.shape.z, item.shapeLo.z));
        bandD = abs(length(q) - r) - halfW;
        float a = dashAngle(q);
        if (kind == 1) {
            u = r * a;
            len_ = 2.0 * PI * r;
        } else {
            float start = dashWord(data);
            float sweep = dashWord(data + 1);
            float rel = a - start;
            rel -= floor(rel / (2.0 * PI)) * (2.0 * PI);
            if (rel > sweep && rel - sweep >= 2.0 * PI - rel) { rel -= 2.0 * PI; }
            u = r * rel;
            len_ = r * sweep;
            closed = false;
        }
    } else if (kind == 7) {
        float rx = dfF(dfFixed(item.shape.z, item.shapeLo.z));
        float ry = dfF(dfFixed(item.shape.w, item.shapeLo.w));
        float ca = dfF(dfFixed(item.ellipse.x, item.miscLo.z));
        float sa = dfF(dfFixed(item.ellipse.y, item.miscLo.w));
        vec2 o = vec2(q.x * ca + q.y * sa, q.y * ca - q.x * sa);
        float t;
        if (rx == ry) {
            t = dashAngle(o);
            bandD = abs(length(o) - rx) - halfW;
        } else {
            vec2 a = abs(o);
            vec2 c0 = vec2(1.0, 0.0);
            vec2 c1 = vec2(0.0, 1.0);
            vec2 m = c0;
            for (int k = 0; k < 24; ++k) {
                m = normalize(c0 + c1);
                float g = (a.x - rx * m.x) * (-rx * m.y) + (a.y - ry * m.y) * (ry * m.x);
                if (g > 0.0) { c0 = m; } else { c1 = m; }
            }
            float dist = length(a - vec2(rx, ry) * m);
            if (dot(a / vec2(rx, ry), a / vec2(rx, ry)) < 1.0) { dist = -dist; }
            bandD = abs(dist) - halfW;
            t = dashAngle(vec2(o.x < 0.0 ? -m.x : m.x, o.y < 0.0 ? -m.y : m.y));
        }
        int steps = item.strokeStyle.w - data - 1;
        float f = t / (2.0 * PI / float(steps));
        int k = clamp(int(f), 0, steps - 1);
        float t0 = dashWord(data + k);
        float t1 = dashWord(data + k + 1);
        u = t0 + (t1 - t0) * (f - float(k));
        len_ = dashWord(data + steps);
    } else {
        float iw = dfF(dfFixed(item.shape.z, item.shapeLo.z));
        float ih = dfF(dfFixed(item.shape.w, item.shapeLo.w));
        float r = dfF(dfFixed(item.misc.y, item.miscLo.x));
        bandD = abs(rectDistance(q, vec2(0.0), vec2(iw, ih)) - r) - halfW;
        u = rrectPosition(q, iw, ih, r);
        len_ = 4.0 * iw + 4.0 * ih + 4.0 * (r * (PI / 2.0));
    }
    vec2 ends;
    bvec2 has;
    float d = max(bandD, dashAlong(u, 0.0, len_, len_, closed, cap == 2 ? halfW : 0.0, ends, has));
    if (cap == 1) {
        if (has.x) { d = min(d, length(q - dashCurvePoint(kind, data, ends.x)) - halfW); }
        if (has.y) { d = min(d, length(q - dashCurvePoint(kind, data, ends.y)) - halfW); }
    }
    return d;
}
// `__canvas_dashedStroke`: a dashed item's stroke distance at the shape-space point --
// for a Line, Arc or PolyLine its whole distance, for every other kind its band's.
vec2 dashedStroke(ivec2 band, vec2 x, vec2 y, float halfW) {
    int kind = item.misc.x;
    bool rounded = kind == 0 && (item.misc.y != 0 || item.miscLo.x != 0);
    bool curve = kind == 1 || kind == 3 || kind == 7 || rounded;
    return vec2(curve ? dashedCurve(x, y, halfW, item.arc.w) : dashedEdges(band, x, y, halfW, item.arc.w), 0.0);
}

// bug-735: one sample's distance at the double-float shape-space point, for every kind a
// distance draws: the polygon's edge walk (bug-687), the polyline's, or the pre-pass and
// then `dfShapeDistance`. One call site each.
vec2 sampleDistance(ivec2 band, vec2 x, vec2 y, float reach) {
    int kind = item.misc.x;
    // plan-163-B: a dashed Line, Arc or PolyLine is its dashed stroke.
    if (item.strokeStyle.w > 0 && (kind == 2 || kind == 3 || kind == 10)) {
        return dashedStroke(band, x, y, dfF(dfFixed(item.misc.y, item.miscLo.x)));
    }
    if (kind == 4) { return edgeDistance(item.arc.z, band.y, band.x, x, y, reach, item.arc.y); }
    if (kind == 10) { return polylineDistance(band, x, y, reach); }
    float v = farBound(vec2(x.x, y.x));
    if (abs(v) > reach) { return vec2(v, 0.0); }
    return dfShapeDistance(x, y);
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

// A picture's fill colour at the SHAPE-space point `q`, as an `ivec4` of 0..255
// (bug-484) — the oracle's picture arm in `helper_items.rs`, operation for operation.
//
// Sampled NEAREST, the plan-116-C §4.5 rule glyphs follow: `floor` and `+ - * /` only,
// in the oracle's order `(q - x0) * iw / dw`, so every renderer picks the same texel.
// The index is clamped rather than rejected, because the antialiased edge pixels just
// outside the destination still take coverage and belong to the border texel.
//
// The texels ride the glyph region, one packed `r | g<<8 | b<<16 | a<<24` word each,
// at `arc.z`; the width is `misc.w` and the height `arc.x`, as a glyph names its
// bitmap. Each channel is masked AFTER the shift, because an `int` shift is
// arithmetic and the alpha byte's top bit would otherwise smear down. The texel is
// then tinted by `fill` integer-wise, alpha included — the fill's alpha is the
// picture's opacity. A zero width means the emitter had no room: draw nothing.
ivec4 pictureColour(vec2 q) {
    int iw = item.misc.w;
    int ih = item.arc.x;
    if (iw <= 0 || ih <= 0) { return ivec4(0); }
    float x0 = fx(item.shape.x) - fx(item.shape.z);
    float y0 = fx(item.shape.y) - fx(item.shape.w);
    float dw = fx(item.shape.z) * 2.0;
    float dh = fx(item.shape.w) * 2.0;
    int u = clamp(int(floor((q.x - x0) * float(iw) / dw)), 0, iw - 1);
    int v = clamp(int(floor((q.y - y0) * float(ih) / dh)), 0, ih - 1);
    int word = edges.values[GLYPH_BASE + item.arc.z + v * iw + u];
    ivec4 tex = ivec4(word & 255, (word >> 8) & 255, (word >> 16) & 255, (word >> 24) & 255);
    return (tex * item.fill) / 255;
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

// plan-154-C: the stop lookup at a ramp parameter `t`, split from the axis that computes
// `t` so a PolyLine can ask it at its path parameter instead.
ivec4 gradientAt(float t) {
    int count = item.ellipse.z;
    int base = GRADIENT_BASE + item.ellipse.w * 5;
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

ivec4 gradientColour(vec2 p) {
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
    return gradientAt(t);
}

vec4 covered(ivec4 rgba, int coverage) {
    float a = float((rgba.w * coverage) / 255) / 255.0;
    return vec4(srgbToLinear(float(rgba.x)) * a,
                srgbToLinear(float(rgba.y)) * a,
                srgbToLinear(float(rgba.z)) * a, a);
}

// The clip's own antialiased coverage at this pixel, 0..255 (plan-116-B).
//
// The same rectangle distance and the same `clamp(0.5 - d, 0, 1)` quantization the shape
// edges use, so a fractional clip edge is antialiased identically to a shape edge — which
// is what lets the oracle and this shader agree on it rather than merely come close.
// `__canvas_clipCoverage` in `helper_items.rs` is the same three lines.
//
// An empty rectangle means unclipped and returns 255, matching `__canvas_hasClip`:
// testing both extents also rejects a negative one, which `Bounds` cannot forbid.
//
// bug-735: in double-float, from the clip's words plus residuals, wherever the fp32
// rectangle distance (with `edgeSlack`'s bound) cannot say the pixel is wholly in or out.
// Metal's `clipCoverage`, line for line.
int clipCoverage(vec2 p) {
    if (!fixedLess(item.clip.x, item.clipLo.x, item.clip.z, item.clipLo.z)
        || !fixedLess(item.clip.y, item.clipLo.y, item.clip.w, item.clipLo.w)) { return 255; }
    vec2 lo = vec2(fx(item.clip.x), fx(item.clip.y));
    vec2 hi = vec2(fx(item.clip.z), fx(item.clip.w));
    float d = rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5);
    float slack = edgeSlack(p, lo, hi);
    if (d - slack > 0.5) { return 0; }
    if (d + slack < -0.5) { return 255; }
    vec2 x0 = dfFixed(item.clip.x, item.clipLo.x);
    vec2 y0 = dfFixed(item.clip.y, item.clipLo.y);
    vec2 x1 = dfFixed(item.clip.z, item.clipLo.z);
    vec2 y1 = dfFixed(item.clip.w, item.clipLo.w);
    vec2 dx = dfSub(vec2(p.x, 0.0), dfMulF(dfAdd(x0, x1), 0.5));
    vec2 dy = dfSub(vec2(p.y, 0.0), dfMulF(dfAdd(y0, y1), 0.5));
    return dfCoverage(dfRectDistance(dx, dy, dfMulF(dfSub(x1, x0), 0.5), dfMulF(dfSub(y1, y0), 0.5)));
}

// The inverse map in double-float, with the float32 terms the oracle maps with.
vec2 dfMapX(vec2 x, vec2 y) {
    return dfAdd(dfAdd(dfMulF(x, intBitsToFloat(item.xform0.x)), dfMulF(y, intBitsToFloat(item.xform0.z))),
                 vec2(intBitsToFloat(item.xform1.x), 0.0));
}
vec2 dfMapY(vec2 x, vec2 y) {
    return dfAdd(dfAdd(dfMulF(x, intBitsToFloat(item.xform0.y)), dfMulF(y, intBitsToFloat(item.xform0.w))),
                 vec2(intBitsToFloat(item.xform1.y), 0.0));
}
// bug-687, extended to every kind by bug-735: an item's fill and stroke coverage, 0..255
// each, and whether it strokes, from its distance in double-float at the double-float
// shape-space point (`x`, `y`). The oracle's `__canvas_drawGeometry`, operation for
// operation: the inverse map with the float32 terms, the central differences at 0.5 (all
// five on the centre's band), `d / ||grad d||`, and the stroke's half-width -- its word
// plus its residual, stroking exactly when the oracle's `half > 0` -- taken off in shape
// space before the scale. The five samples are one loop so each distance helper is inlined
// once. A transformed CENTRE sample past `half + 0.5 * max(F, 1)` (F the inverse matrix's
// Frobenius norm, which bounds the gradient) decides the coverage by its sign and the four
// neighbours are skipped; never for an arc, whose distance jumps. Metal's `shapeCoverage`,
// line for line, where the bound is argued.
ivec3 shapeCoverage(vec2 x, vec2 y) {
    bool mapped = hasTransform();
    ivec2 band = polygonBand(mapped ? dfMapY(x, y).x : y.x);
    vec2 half2 = dfFixed(item.misc.z, item.miscLo.y);
    bool stroked = dfLess(vec2(0.0), half2);
    float reach = 0.5 + max(half2.x, 0.0);
    if (mapped) {
        vec2 c0 = vec2(intBitsToFloat(item.xform0.x), intBitsToFloat(item.xform0.y));
        vec2 c1 = vec2(intBitsToFloat(item.xform0.z), intBitsToFloat(item.xform0.w));
        float frob = sqrt(dot(c0, c0) + dot(c1, c1)) * (1.0 + 1.0 / 256.0) + 1.0 / 256.0;
        reach = item.misc.x == 3 ? 1.0e30 : max(half2.x, 0.0) + 0.5 * max(frob, 1.0);
    }
    // plan-163-B: a polygon, rectangle or picture with join records strokes from its own
    // distance (`joinedStroke`), which a miter can reach past `half` from the outline --
    // so the centre's shape distance cannot decide the neighbours there.
    // plan-163-B: so can a dashed one, which strokes from `dashedStroke`.
    bool dashed = item.strokeStyle.w > 0;
    bool joined = (item.strokeStyle.y > 0 || dashed) && item.misc.x != 10;
    float strokeReach = mapped ? reach - max(half2.x, 0.0) : 0.5;
    vec2 d[5];
    vec2 cqx = x;
    vec2 cqy = y;
    for (int k = 0; k < (mapped ? 5 : 1); ++k) {
        vec2 sx = dfAdd(x, vec2(k == 1 ? 0.5 : (k == 2 ? -0.5 : 0.0), 0.0));
        vec2 sy = dfAdd(y, vec2(k == 3 ? 0.5 : (k == 4 ? -0.5 : 0.0), 0.0));
        vec2 qx = mapped ? dfMapX(sx, sy) : sx;
        vec2 qy = mapped ? dfMapY(sx, sy) : sy;
        if (k == 0) { cqx = qx; cqy = qy; }
        d[k] = sampleDistance(band, qx, qy, k == 0 ? reach : 1.0e30);
        if (k == 0 && !joined && abs(d[0].x) > reach) { mapped = false; }
    }
    vec2 dRaw = d[0];
    vec2 g = vec2(1.0, 0.0);
    if (mapped) {
        vec2 gx = dfSub(d[1], d[2]);
        vec2 gy = dfSub(d[3], d[4]);
        g = dfLength(gx, gy);
        if (!(g.x > 0.000001)) { g = vec2(1.0, 0.0); }
    }
    int fill = dfCoverage(mapped ? dfDiv(dRaw, g) : dRaw);
    int stroke = 0;
    if (stroked) {
        vec2 s = dashed ? dashedStroke(band, cqx, cqy, dfF(half2))
            : (joined ? joinedStroke(band, cqx, cqy, half2, strokeReach) : dfSub(dfAbs(dRaw), half2));
        stroke = dfCoverage(mapped ? dfDiv(s, g) : s);
    }
    return ivec3(fill, stroke, stroked ? 1 : 0);
}

void main() {
    // First, before anything reads it: everything below, and both helpers above, work
    // off this one record.
    item = itemBuf.blocks[vItem];
    // The clip multiplies the shape's own coverage, exactly as it does in the oracle's
    // `(coverage * clipCov) / 255`. Integer, and by 255 rather than a shift, so the two
    // quantize identically — a float multiply here would disagree on the boundary
    // pixels, which are the only ones a clip can affect.
    // plan-116-H: the shape-space point. The vertex stage moved the quad by +offset, so
    // the distance field is evaluated at p - offset — the same translation-as-inverse-map
    // rule plan-116-C established for `Paint.transform` and plan-116-G §4.5 restated for
    // groups. One rule, three renderers.
    //
    // The CLIP deliberately keeps `gl_FragCoord.xy`. `Paint.clip` is a surface rectangle
    // by definition (plan-116-B), so a group translates the shape THROUGH it — G5's
    // first exception, and the reason this is two variables rather than one substitution.
    vec2 p = gl_FragCoord.xy - vec2(fx(draw.offset.x), fx(draw.offset.y));
    int clipCov = clipCoverage(gl_FragCoord.xy);
    if (item.misc.x == 6) {
        // A glyph is fill-only: a text item's stroke was turned into an outline
        // polygon by the geometry builder, so there is nothing here to stroke.
        //
        // plan-116-C §4.5: a transformed glyph samples its bitmap at the inverse-mapped
        // point, nearest. `glyphCoverage` already indexes by whole pixels, so mapping
        // the query point is the whole change — the cache stays untransformed and one
        // entry serves every transform.
        vec2 gp = hasTransform() ? inverseMap(p) : p;
        fragColor = covered(item.fill, (glyphCoverage(gp) * clipCov) / 255);
        return;
    }
    // Both coverages come from `shapeCoverage` in double-float at the double-float twin of
    // `p` — the surface point with the group offset (word and residual) taken off,
    // exactly: a polygon since bug-687, every other kind since bug-735. The fill uses the
    // surface distance and the stroke subtracts `half` BEFORE converting, so the outline
    // scales with the shape (§4.3).
    ivec3 cov = shapeCoverage(dfSub(vec2(gl_FragCoord.x, 0.0), dfFixed(draw.offset.x, draw.offset.z)),
                              dfSub(vec2(gl_FragCoord.y, 0.0), dfFixed(draw.offset.y, draw.offset.w)));
    // plan-116-F: the gradient replaces the fill COLOUR and nothing else — the shape's
    // distance, its coverage and its stroke are untouched.
    ivec4 fillRgba = item.ellipse.z >= 2 ? gradientColour(p) : item.fill;
    // plan-154-C: a PolyLine's stops are its per-point colours, and its ramp runs along
    // the path -- the parameter of the nearest point, in shape space, from the same band
    // the distance used.
    if (item.misc.x == 10 && item.ellipse.z >= 2) {
        vec2 q = hasTransform() ? inverseMap(p) : p;
        fillRgba = gradientAt(polylineScan(polygonBand(q.y), q).y);
    }
    // bug-484: a picture's image replaces the fill colour the same way — sampled at the
    // shape-space point, the inverse-mapped one under a transform.
    if (item.misc.x == 9) {
        fillRgba = pictureColour(hasTransform() ? inverseMap(p) : p);
    }
    vec4 colour = covered(fillRgba, (cov.x * clipCov) / 255);
    if (cov.z != 0) {
        // Stroke over fill, then the hardware puts that over the destination — which
        // is what makes this one fragment equal to the software path's two sequential
        // writes, since `over` is associative.
        //
        // plan-116-B: that identity is `Normal`-ONLY, which is why a non-`Normal`
        // stroked-and-filled item is emitted as two adjacent instances instead (§4.3).
        // By the time such an item reaches here it is either fill-only or stroke-only,
        // so this branch composes two sources only under `Normal`, where it is exact.
        vec4 s = covered(item.stroke, (cov.y * clipCov) / 255);
        colour = s + colour * (1.0 - s.w);
    }
    fragColor = colour;
}
