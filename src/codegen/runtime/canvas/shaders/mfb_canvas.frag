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
    ivec4 arcCaps; // an arc's two sweep endpoints startX,startY,endX,endY (16.16 px);
                   //   for a polygon, its band header (bug-688, `polygonBand`)
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

// plan-116-H: the group translation for THIS draw, in 16.16 — the same push constant
// the vertex stage declares. Both stages consume it, which is what makes the range
// valid (H4).
layout(push_constant) uniform Draw {
    ivec2 offset;
} draw;

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
// One vertex coordinate as the CPU had it: the 16.16 word plus its residual.
vec2 edgeCoord(int i, int r) {
    return dfAdd(dfWord(edges.values[i]), vec2(residualPx(edges.values[r]), 0.0));
}
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
bool edgeToggles32(vec2 p, vec2 a, vec2 b) {
    if ((a.y > p.y) == (b.y > p.y)) { return false; }
    float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
    return b.y > a.y ? cross < 0.0 : cross > 0.0;
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
    vec2 wx = dfSub(x, g.ax);
    vec2 wy = dfSub(y, g.ay);
    vec2 len2 = dfAdd(dfMul(g.vx, g.vx), dfMul(g.vy, g.vy));
    vec2 t = vec2(0.0, 0.0);
    if (len2.x > 0.0) {
        t = dfDiv(dfAdd(dfMul(wx, g.vx), dfMul(wy, g.vy)), len2);
        if (dfLess(t, vec2(0.0, 0.0))) { t = vec2(0.0, 0.0); }
        if (dfLess(vec2(1.0, 0.0), t)) { t = vec2(1.0, 0.0); }
    }
    vec2 qx = dfSub(wx, dfMul(g.vx, t));
    vec2 qy = dfSub(wy, dfMul(g.vy, t));
    return dfSqrt(dfAdd(dfMul(qx, qx), dfMul(qy, qy)));
}
bool dfToggles(DfEdge g, vec2 x, vec2 y) {
    if (dfLess(y, g.ay) == dfLess(y, dfAdd(g.ay, g.vy))) { return false; }
    vec2 lhs = dfMul(dfSub(x, g.ax), g.vy);
    vec2 rhs = dfMul(dfSub(y, g.ay), g.vx);
    return g.vy.x > 0.0 ? dfLess(lhs, rhs) : dfLess(rhs, lhs);
}
vec2 edgeDistance(int base, int count, int list, vec2 x, vec2 y, float reach) {
    vec2 p = vec2(x.x, y.x);
    float bound = 1.0e6;
    float low = 1.0e6;
    bool unsure = false;
    bool inside = false;
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
        } else if (edgeToggles32(p, a, b)) {
            inside = !inside;
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
        if (unsureEdge && dfToggles(g, x, y)) { inside = !inside; }
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
        float d = length(w - v * c) - halfW;
        if (butt) {
            float len_ = sqrt(len2);
            if (e == first) { d = max(d, -t * len_); }
            if (e == last) { d = max(d, (t - 1.0) * len_); }
        }
        if (d < best) { best = d; param = (float(e) + c) / float(item.misc.w); }
    }
    return vec2(best, param);
}

float geoDistance(ivec2 band, vec2 p) {
    float radius = fx(item.misc.y);
    vec2 c = vec2(fx(item.shape.x), fx(item.shape.y));
    // bug-484: a Picture (9) is a rectangle to the distance field — its header carries
    // the rectangle's centre, half-extent and zero radius — so its coverage, edge, clip
    // and stroke are a rectangle's. Only its fill colour differs (`pictureColour`).
    if (item.misc.x == 0 || item.misc.x == 9) {
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
    if (item.misc.x == 10) {
        return polylineScan(band, p).x;
    }
    // A polygon (4) never reaches here: `polygonCoverage` evaluates it in double-float
    // (bug-687). The empty kind (5) and anything unknown. (`Picture` was once kind 5 and drew
    // nothing; since bug-484 it is kind 9 and takes the rectangle arm above.) A glyph (6) never reaches here: it has coverage, not a
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
///
/// bug-688: all five evaluations of a transformed polygon use the band of the CENTRE
/// point `q`. The emitter widened that band's reach by the transform's scale for
/// exactly this, so it holds every edge any of the four neighbours needs; a band looked
/// up per neighbour could differ between them, and the difference quotient of two
/// different edge sets is not the gradient of anything.
vec2 shapeDistanceAndScale(vec2 p) {
    if (!hasTransform()) { return vec2(geoDistance(polygonBand(p.y), p), 1.0); }
    vec2 q = inverseMap(p);
    ivec2 band = polygonBand(q.y);
    float d = geoDistance(band, q);
    float gx = geoDistance(band, inverseMap(p + vec2(0.5, 0.0)))
             - geoDistance(band, inverseMap(p - vec2(0.5, 0.0)));
    float gy = geoDistance(band, inverseMap(p + vec2(0.0, 0.5)))
             - geoDistance(band, inverseMap(p - vec2(0.0, 0.5)));
    float g = sqrt(gx * gx + gy * gy);
    return vec2(d, g > 0.000001 ? g : 1.0);
}

// bug-687: a polygon's fill and stroke coverage, 0..255 each, from its distance in
// double-float at the double-float shape-space point (`x`, `y`). `shapeDistanceAndScale`
// and `main`'s two quantisations, operation for operation: the inverse map with the
// float32 terms, the central differences at 0.5 (all five on the centre's band),
// `d / ||grad d||`, and the stroke's half-width taken off in shape space before the
// scale. The five samples are one loop so `edgeDistance` is inlined once. Metal's
// `polygonCoverage`, line for line.
vec2 dfMapX(vec2 x, vec2 y) {
    return dfAdd(dfAdd(dfMulF(x, intBitsToFloat(item.xform0.x)), dfMulF(y, intBitsToFloat(item.xform0.z))),
                 vec2(intBitsToFloat(item.xform1.x), 0.0));
}
vec2 dfMapY(vec2 x, vec2 y) {
    return dfAdd(dfAdd(dfMulF(x, intBitsToFloat(item.xform0.y)), dfMulF(y, intBitsToFloat(item.xform0.w))),
                 vec2(intBitsToFloat(item.xform1.y), 0.0));
}
ivec2 polygonCoverage(vec2 x, vec2 y) {
    bool mapped = hasTransform();
    ivec2 band = polygonBand(mapped ? dfMapY(x, y).x : y.x);
    float reach = mapped ? 1.0e30 : 0.5 + max(fx(item.misc.z), 0.0);
    vec2 d[5];
    for (int k = 0; k < (mapped ? 5 : 1); ++k) {
        vec2 sx = dfAdd(x, vec2(k == 1 ? 0.5 : (k == 2 ? -0.5 : 0.0), 0.0));
        vec2 sy = dfAdd(y, vec2(k == 3 ? 0.5 : (k == 4 ? -0.5 : 0.0), 0.0));
        vec2 qx = mapped ? dfMapX(sx, sy) : sx;
        vec2 qy = mapped ? dfMapY(sx, sy) : sy;
        d[k] = edgeDistance(item.arc.z, band.y, band.x, qx, qy, reach);
    }
    vec2 dRaw = d[0];
    vec2 g = vec2(1.0, 0.0);
    if (mapped) {
        vec2 gx = dfSub(d[1], d[2]);
        vec2 gy = dfSub(d[3], d[4]);
        g = dfSqrt(dfAdd(dfMul(gx, gx), dfMul(gy, gy)));
        if (!(g.x > 0.000001)) { g = vec2(1.0, 0.0); }
    }
    int fill = dfCoverage(mapped ? dfDiv(dRaw, g) : dRaw);
    int stroke = 0;
    float halfWidth = fx(item.misc.z);
    if (halfWidth > 0.0) {
        vec2 s = dfSub(dRaw.x < 0.0 ? -dRaw : dRaw, vec2(halfWidth, 0.0));
        stroke = dfCoverage(mapped ? dfDiv(s, g) : s);
    }
    return ivec2(fill, stroke);
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
    // `dRaw` is in SHAPE space and `dScale` the local scale; the fill uses the surface
    // distance and the stroke subtracts `half` BEFORE converting, so the outline scales
    // with the shape (§4.3). Untransformed, `dScale` is 1.0 and both collapse to the
    // expressions this shader had.
    //
    // bug-687: a polygon's two coverages come from `polygonCoverage` in double-float, at
    // the double-float twin of `p` — the surface point with the group offset taken off,
    // exactly. Every other kind keeps its fp32 distance and the same quantisation.
    int fillCov;
    int strokeCov;
    if (item.misc.x == 4) {
        ivec2 cov = polygonCoverage(dfSub(vec2(gl_FragCoord.x, 0.0), dfWord(draw.offset.x)),
                                    dfSub(vec2(gl_FragCoord.y, 0.0), dfWord(draw.offset.y)));
        fillCov = cov.x;
        strokeCov = cov.y;
    } else {
        vec2 ds = shapeDistanceAndScale(p);
        float dRaw = ds.x;
        float dScale = ds.y;
        fillCov = int(clamp(0.5 - dRaw / dScale, 0.0, 1.0) * 255.0 + 0.5);
        strokeCov = int(clamp(0.5 - (abs(dRaw) - fx(item.misc.z)) / dScale, 0.0, 1.0) * 255.0 + 0.5);
    }
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
    vec4 colour = covered(fillRgba, (fillCov * clipCov) / 255);
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
        vec4 s = covered(item.stroke, (strokeCov * clipCov) / 255);
        colour = s + colour * (1.0 - s.w);
    }
    fragColor = colour;
}
