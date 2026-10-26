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
    ivec4 shape;   // p0..p3 (float32 bits); for a glyph, the bitmap origin in WHOLE px
    ivec4 fill;
    ivec4 stroke;
    ivec4 misc;    // kind, radius, strokeHalf (float32 bits), edgeCount / glyph width
    ivec4 arc;     // glyph/picture height, arc sweep turn (bug-735, bug-1088) / polygon fill rule
                   //   (plan-163-A, 0 EvenOdd, 1 NonZero), edgeBase, capStyle
    ivec4 surface; // width, height, blendMode, gradient kind (0 linear, 1 radial)
    ivec4 clip;    // the clip rectangle x0,y0,x1,y1 (float32 bits); zero-area = unclipped
    ivec4 xform0;  // inverse transform ia,ib,ic,id as float32 BITS
    ivec4 xform1;  // itx, ity (float32 bits), hasTransform (0 or 1); `.w` is the vertex
                   // stage's (bug-1083: the rim of the quad's x sides)
    ivec4 arcCaps; // an arc's two sweep endpoints startX,startY,endX,endY (float32 bits);
                   //   for a polygon, its band header (bug-688, `polygonBand`)
    ivec4 ellipse; // an ellipse's rotation cos, sin (float32 bits; for a banded polygon `.x`
                   //   is its band norm); then a gradient's stop count and first-stop index
    ivec4 gradient; // a gradient's axis startX,startY,endX,endY (float32 bits)
    // plan-163-B: the stroke's join records -- first edge slot, record count -- then
    // its dash pattern's first entry and entry count.
    ivec4 strokeStyle;
    // plan-163-C: a path clip's band header (`arcCaps`'s shape, for the clip's edges),
    // then its first edge, its edge count, its mode | rule << 2, and a picture's
    // transparency step T.
    ivec4 clipBand;
    ivec4 clipPath;
    // plan-163-D: a radial gradient's own inverse transform (float32 bits) and its flag,
    // then its focal offset and focal radius (float32 bits).
    ivec4 gradInv0;
    ivec4 gradInv1;
    ivec4 gradFocal;
};

layout(std430, set = 0, binding = 1) readonly buffer Items {
    ItemBlock blocks[];
} itemBuf;

// The instance index the vertex stage read, flat-interpolated. `gl_InstanceIndex` does
// not exist in a fragment shader, so it has to travel as a varying.
layout(location = 0) flat in int vItem;
// plan-175-B: the NODE this fragment's instance is drawn under (the vertex stage's
// `vNode`). Its eight words, in the node region of `edges` below, are the group
// translation as float32 bits and two zero words, then (plan-172-H) the linear part of
// the INVERSE of the enclosing groups' composed affine as four float32 bit patterns
// (`ia`, `ib`, `ic`, `id`), all-zero bits when the groups only translate. See the vertex
// stage. They were a push constant both stages declared until plan-175-B.
layout(location = 1) flat in int vNode;

struct Node {
    ivec4 offset;
    ivec4 inverse;
};
// A private global `main` fills on its first lines, as `item` is.
Node draw;

// Whether this draw has a group matrix, and it. Private globals `main` fills on its
// first lines, as `item` is, because `shapeCoverage` reads them.
bool grouped;
vec4 gi;

// A private global rather than a local in `main`, because the distance and coverage
// helpers below read `item` directly. Making it a local would mean threading
// twenty-eight ints through both of them for no gain; `main` fills it on its first
// line, before anything can read it.
ItemBlock item;

// The polygon edge buffer: four float32 words per edge, the two endpoints. It is a
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
// for free.
layout(std430, set = 0, binding = 0) readonly buffer Edges {
    int values[];
} edges;

// Where the other regions start inside that same buffer, in words: glyph coverage and
// picture texels, gradient stops (plan-116-F), and the polygon band index (bug-688).
//
// SPECIALIZATION CONSTANTS, not literals (bug-688). The pipeline feeds each one from
// the Rust layout constant of the same region (`VULKAN_SPEC_CONSTANTS` in
// `runtime/canvas/scene_layout.rs`, through `VkSpecializationInfo`), so the layout is defined
// once and this blob never has to be rebuilt because a cap before a region moved. They
// were literals until then, and every cap was frozen by it. The default 0 is never
// used: a pipeline built without the specialization data would read every region from
// the edges, which `the_pipeline_specializes_every_region_base` rules out.
layout(constant_id = 0) const int GLYPH_BASE = 0;
layout(constant_id = 1) const int GRADIENT_BASE = 0;
layout(constant_id = 2) const int BAND_BASE = 0;
// plan-163-B: the dash words (`dashWord`).
layout(constant_id = 4) const int DASH_BASE = 0;
// plan-175-B: the node table, eight words a node, read here and by the vertex stage.
layout(constant_id = 5) const int NODE_BASE = 0;

layout(location = 0) out vec4 fragColor;

const float PI = 3.141592653589793;

// A word of the item block or the edge region as the float32 it holds -- every position
// reaches the GPU as float32 bits (`emit_f32_bits`). Metal's, line for line.
float fx(int v) { return intBitsToFloat(v); }

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

// The oracle's coverage quantisation, `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)`, which every
// shape kind's fill and stroke and every clip share. Metal's, line for line.
int coverage(float d) { return int(clamp(0.5 - d, 0.0, 1.0) * 255.0 + 0.5); }

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
// clamped coverage `best` feeds). Order does not matter: `min` and a winding sum are
// both order-independent.
//
// In fp32, at the shape-space point `p`, from each endpoint's float32 words. Two edges
// that share a vertex read the same words for it, so a pixel row through that vertex
// counts the corner once. Metal's `edgeDistance`, line for line.
//
// plan-163-A: an edge's signed crossing -- 0 when it does not cross the pixel's row left
// of the pixel, else +1 for an edge running down the screen (`b.y > a.y`) and -1 for one
// running up: `__canvas_edgeDistance`'s winding convention, its `px < ax + (py - ay) / dy
// * dx` multiplied out by `dy` so it needs no division. Metal's, line for line.
int edgeCrossing(vec2 p, vec2 a, vec2 b) {
    if ((a.y > p.y) == (b.y > p.y)) { return 0; }
    float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
    if (b.y > a.y) { return cross < 0.0 ? 1 : 0; }
    return cross > 0.0 ? -1 : 0;
}
// Edge `e` of the run starting at slot `base`, as its two endpoints.
void edgeEnds(int base, int e, out vec2 a, out vec2 b) {
    int i = (base + e) * 4;
    a = vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
    b = vec2(fx(edges.values[i + 2]), fx(edges.values[i + 3]));
}
// plan-163-A: `nonZero` is the polygon's fill rule (`arc.y`). The crossings are summed as a
// winding number; NonZero is inside when it is not zero, EvenOdd when it is odd.
float edgeDistance(int base, int count, int list, vec2 p, int nonZero) {
    float best = 1.0e6;
    int winding = 0;
    for (int j = 0; j < count; ++j) {
        vec2 a;
        vec2 b;
        edgeEnds(base, list < 0 ? j : edges.values[list + j], a, b);
        best = min(best, segmentDistance(p, a, b));
        winding += edgeCrossing(p, a, b);
    }
    bool inside = nonZero != 0 ? winding != 0 : (winding & 1) != 0;
    return inside ? -best : best;
}

// plan-163-B: join records (`helper_joins.rs`). A Miter or Bevel stroke's corners ride the
// edge region after the item's own edges, one record per outline edge -- the corner at
// that edge's START -- as two edge slots: its four points V, A, P, B as eight float32 words
// (`strokeStyle.x` is the first slot, `strokeStyle.y` the record count). Its corner PIECE,
// read from exact equalities as the oracle reads it: none when A = V, the disc of radius
// |A - V| about V when A = B, that disc cut by the chord A-B on V's side when P = V (a
// bevel), and the disc with the polygon V, A, P, B otherwise (a miter, P its point). The
// pieces overlap the butt bands they join, so their `min` has no seam. Metal's twins,
// line for line.
int joinWord(int r, int w) { return (item.strokeStyle.x + r * 2) * 4 + w; }
bool joinSame(int r, int a, int b) {
    int i = joinWord(r, a * 2);
    int j = joinWord(r, b * 2);
    return edges.values[i] == edges.values[j] && edges.values[i + 1] == edges.values[j + 1];
}
// 0 no piece, 1 the disc, 2 a bevel, 3 a miter.
int joinKind(int r) {
    if (joinSame(r, 1, 0)) { return 0; }
    if (joinSame(r, 1, 3)) { return 1; }
    return joinSame(r, 2, 0) ? 2 : 3;
}
// Point `k` of record `r`: 0 V, 1 A, 2 P, 3 B.
vec2 joinPoint(int r, int k) {
    int i = joinWord(r, k * 2);
    return vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
}
// A join record's corner piece at `w`, the pixel less its vertex V, from its points
// relative to V (`__canvas_joinPieceDistance`).
vec2 joinRel(int r, int k) { return joinPoint(r, k) - joinPoint(r, 0); }
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
// One outline edge of a joined stroke: a polygon's band edge `e`, or a rectangle's side
// `e` from its centre and half-extents (`__canvas_rectSides`).
void joinedEdge(bool poly, int e, out vec2 a, out vec2 b) {
    if (poly) {
        edgeEnds(item.arc.z, e, a, b);
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
// band and the record at its start.
float joinedStroke(ivec2 band, vec2 p, float halfW) {
    bool poly = item.misc.x == 4;
    int count = poly ? band.y : 4;
    float best = 1.0e6;
    for (int j = 0; j < count; ++j) {
        int e = poly ? (band.x < 0 ? j : edges.values[band.x + j]) : j;
        vec2 a;
        vec2 b;
        joinedEdge(poly, e, a, b);
        best = min(best, segmentDistanceButt(p, a, b, halfW));
        int kind = joinKind(e);
        if (kind != 0) { best = min(best, joinPieceAt(e, kind, p - joinPoint(e, 0))); }
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
// `arcCaps` = (top, height as float32 bits, band count, table offset), written by
// `emit_band_index` in `vulkan/bands.rs`; the table is (first, count) per band and the list
// follows it. A point above the first band or below the last uses the end band, which
// the emitter built to hold every edge that can matter there. Metal's `polygonBand`,
// line for line.
//
// bug-1085: `norm` is the pixel's composed inverse norm (0 for a pixel that takes its
// centre sample alone), and the band is taken only when its reach covers it:
// `0.75 norm <= F`, `F` the norm the reach was built for (`ellipse.x`, `ITEM_BAND_NORM`).
// A block is built once a pass and drawn under any group's matrix, so past that the edges
// a neighbour sample needs may be in no band of it, and every edge is walked instead --
// exact, as with no index.
ivec2 polygonBand(float y, float norm) {
    int nb = item.arcCaps.z;
    // plan-164-C: kinds 11 and 12 too -- their pieces are banded as edges.
    int bk = item.misc.x;
    if ((bk != 4 && bk != 10 && bk != 11 && bk != 12) || nb <= 0) { return ivec2(-1, item.misc.w); }
    if (!(norm * 0.75 <= fx(item.ellipse.x))) { return ivec2(-1, item.misc.w); }
    float k = floor((y - fx(item.arcCaps.x)) / fx(item.arcCaps.y));
    int band = int(clamp(k, 0.0, float(nb - 1)));
    int t = BAND_BASE + item.arcCaps.w;
    return ivec2(t + nb * 2 + edges.values[t + band * 2], edges.values[t + band * 2 + 1]);
}

// plan-163-C: a path clip's band, the clip's twin of `polygonBand`: its band header in
// `clipBand`, or every clip edge (band count 0) -- and, bug-1085, every clip edge for a
// pixel whose composed norm the band's reach does not cover (`gradFocal.w`,
// `ITEM_CLIP_BAND_NORM`). Metal's, line for line.
ivec2 clipBandAt(float y, float norm) {
    int nb = item.clipBand.z;
    if (nb <= 0) { return ivec2(-1, item.clipPath.y); }
    if (!(norm * 0.75 <= fx(item.gradFocal.w))) { return ivec2(-1, item.clipPath.y); }
    float k = floor((y - fx(item.clipBand.x)) / fx(item.clipBand.y));
    int band = int(clamp(k, 0.0, float(nb - 1)));
    int t = BAND_BASE + item.clipBand.w;
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

// Every shape kind but the polygon, the polyline, the curves and the glyph, in fp32 -- the
// twin of `__canvas_geoDistance` in the oracle, operation for operation, at the shape-space
// point `p` from the item block's float32 words. The arc's sweep planes are the oracle's two
// cross products against `arcCaps - centre` (the radius times its Taylor `(cos, sin)`),
// over the radius, and `arc.y` is its `turn`, decided in f64 by the emitter; the ellipse is
// the oracle's 24-step bisection. Metal's `shapeDistance`, line for line, where the cases
// are set out in full.
float shapeDistance(vec2 p) {
    int kind = item.misc.x;
    vec2 c = vec2(fx(item.shape.x), fx(item.shape.y));
    vec2 e = vec2(fx(item.shape.z), fx(item.shape.w));
    float radius = fx(item.misc.y);
    vec2 d = p - c;
    if (kind == 0 || kind == 9 || kind == 13) { return rectDistance(p, c, e) - radius; }
    if (kind == 1) { return length(d) - e.x - radius; }
    if (kind == 2) {
        if (item.arc.w == 1) { return segmentDistance(p, c, e) - radius; }
        // plan-163-B: Square -- `__canvas_segmentDistanceSquare`.
        if (item.arc.w == 2) { return segmentDistanceSquare(p, c, e, radius); }
        return segmentDistanceButt(p, c, e, radius);
    }
    if (kind == 3) {
        vec2 s = vec2(fx(item.arcCaps.x), fx(item.arcCaps.y)) - c;
        vec2 t = vec2(fx(item.arcCaps.z), fx(item.arcCaps.w)) - c;
        // bug-1088: the band cut by the sweep's wedge, the oracle's exact cone distance:
        // inside both end half-planes and ahead of the centre along a radius, the nearer
        // end line's distance, else the nearer end ray's; the reflex complement of the
        // gap's cone; a whole turn (`arc.y` 2) the band alone. `s`, `t` are the radius
        // times the oracle's unit vectors, so they are divided by it first.
        float band = abs(length(d) - e.x) - radius;
        if (item.arc.y != 2) {
            vec2 u = s / e.x;
            vec2 v = t / e.x;
            float startPlane = u.y * d.x - u.x * d.y;
            float endPlane = v.x * d.y - v.y * d.x;
            float alongS = d.x * u.x + d.y * u.y;
            float alongE = d.x * v.x + d.y * v.y;
            float rays = min(length(d - max(alongS, 0.0) * u), length(d - max(alongE, 0.0) * v));
            bool ahead = alongS >= 0.0 || alongE >= 0.0;
            float sweep = rays;
            if (item.arc.y == 1) {
                sweep = -rays;
                if (startPlane >= 0.0 && endPlane >= 0.0 && ahead) { sweep = min(startPlane, endPlane); }
            } else if (startPlane <= 0.0 && endPlane <= 0.0 && ahead) {
                sweep = max(startPlane, endPlane);
            }
            band = max(band, sweep);
        }
        if (item.arc.w == 0) { return band; }
        for (int k = 0; k < 2; ++k) {
            vec2 r = k == 0 ? s : t;
            vec2 o = d - r;
            if (item.arc.w == 2) {
                // plan-163-B: `__canvas_arcSquareCap` -- the half-width box past the end,
                // along the tangent; `along` is measured outward from the end.
                float rl = length(r);
                if (!(rl > 0.0)) { continue; }
                vec2 u = r / rl;
                float along = o.x * u.y - o.y * u.x;
                if (k == 1) { along = -along; }
                float across = o.x * u.x + o.y * u.y;
                band = min(band, max(max(abs(across) - radius, along - radius), -along));
            } else {
                band = min(band, length(o) - radius);
            }
        }
        return band;
    }
    if (kind == 7) {
        float ca = fx(item.ellipse.x);
        float sa = fx(item.ellipse.y);
        vec2 q = abs(vec2(d.x * ca + d.y * sa, d.y * ca - d.x * sa));
        if (item.shape.z == item.shape.w) { return length(q) - e.x - radius; }
        vec2 c0 = vec2(1.0, 0.0);
        vec2 c1 = vec2(0.0, 1.0);
        vec2 m = c0;
        for (int k = 0; k < 24; ++k) {
            vec2 cs = c0 + c1;
            m = cs / length(cs);
            float g = (q.x - e.x * m.x) * (-e.x * m.y) + (q.y - e.y * m.y) * (e.y * m.x);
            if (g > 0.0) { c0 = m; } else { c1 = m; }
        }
        float dist = length(q - e * m);
        vec2 u = q / e;
        if (u.x * u.x + u.y * u.y < 1.0) { dist = -dist; }
        return dist - radius;
    }
    return 1.0e6;
}

// A PolyLine's distance -- `__canvas_polylineScan`'s -- over the band's segments, and the
// join record at each segment's start. Metal's `polylineDistance`, line for line.
float polylineSegment(int e, vec2 p, float halfW) {
    int first = item.gradient.x;
    int last = item.gradient.y;
    int i = (item.arc.z + e) * 4;
    bool live = first == last || edges.values[i] != edges.values[i + 2]
        || edges.values[i + 1] != edges.values[i + 3];
    if (!live) { return 1.0e6; }
    vec2 a;
    vec2 b;
    edgeEnds(item.arc.z, e, a, b);
    vec2 v = b - a;
    float len2 = dot(v, v);
    vec2 w = p - a;
    float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;
    float at = clamp(t, 0.0, 1.0);
    bool square = item.arc.w == 2;
    // plan-163-B: a square OUTER end measures to the segment's line past its endpoint.
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
    return d;
}
float polylineDistance(ivec2 band, vec2 p) {
    int first = item.gradient.x;
    int last = item.gradient.y;
    bool joined = item.strokeStyle.y > 0;
    float halfW = fx(item.misc.y);
    float best = 1.0e6;
    for (int j = 0; j < band.y; ++j) {
        int e = band.x < 0 ? j : edges.values[band.x + j];
        if (e < first || e > last) { continue; }
        best = min(best, polylineSegment(e, p, halfW));
        int kind = joined ? joinKind(e) : 0;
        if (kind != 0) { best = min(best, joinPieceAt(e, kind, p - joinPoint(e, 0))); }
    }
    return best;
}

// plan-163-B: dashes (`helper_dashes.rs`). An item's dash words ride the dash region from
// `strokeStyle.z`, `strokeStyle.w` of them (0 for a solid stroke), as float32 bit
// patterns: the phase, the entry count, the period, each entry's END along the period,
// then the kind's perimeter data -- three words per outline edge (its start and end
// position and its ring's length, negative for an open path), an ellipse's arc-length
// table, an arc's start angle and sweep. Each function is the oracle's in fp32, at
// offsets from the item's own outline. Metal's twins, line for line.
float dashWord(int k) { return intBitsToFloat(edges.values[DASH_BASE + item.strokeStyle.z + k]); }
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
// plan-164-C: the curve shapes' functions. Generated -- edit `curves.tpl`, not this.
// @CURVES-BEGIN generated from curves.tpl by tools/canvas-curve-shader/generate.py
vec2 curvePoint(int slot, int w) {
  int i = slot * 4 + w;
  return vec2(fx(edges.values[i]), fx(edges.values[i + 1]));
}
float curveWeight(int slot) {
  return intBitsToFloat(edges.values[slot * 4 + 2]);
}
// plan-164-C: the curve shapes (`helper_curves.rs`). A `PolyCurve` (kind 11) and a
// `CurvePolygon` (kind 12) are drawn from y-monotone rational-quadratic PIECES, which
// ride the edge region two slots a piece: piece `e`'s chord `x0, y0, x2, y2` in slot
// `arc.z + e` -- where the band index reads it as an edge, a piece's two ends being its
// whole y-range -- and its control point and weight in slot `arc.z + misc.w + e`, as `x1, y1`, the fp32 BITS
// of `w1` and a zero. Each function below is the oracle's of the same name
// (`__canvas_pieceNearest`, `__canvas_pieceCrossX`, `__canvas_pieceCut`, ...), in
// fp32, on coordinates taken RELATIVE TO THE PIXEL first. The two shaders' text is
// generated from one template (`tools/canvas-curve-shader`), so they are line for line.
struct MfbPiece { vec2 p0; vec2 p1; vec2 p2; float w; };
struct MfbPieceQ { vec2 a; vec2 b; vec2 c; float wb; };
MfbPiece curvePiece(int e, vec2 pt) {
  int s = item.arc.z + e;
  int c = s + item.misc.w;
  MfbPiece k;
  k.p0 = curvePoint(s, 0) - pt;
  k.p2 = curvePoint(s, 2) - pt;
  k.p1 = curvePoint(c, 0) - pt;
  k.w = curveWeight(c);
  return k;
}
// With Q(t) = a t^2 + b t + c the piece's numerator less the pixel times its
// denominator W(t) = 1 + wb t (1 - t), the squared distance is Q.Q / W^2 and its slope
// has the sign of g = (Q.Q')W - (Q.Q)W'. `order` 0 is g, 1 its derivative, 2 its second.
float pcF(MfbPieceQ k, int order, float t) {
  vec2 q = (k.a * t + k.b) * t + k.c;
  vec2 d = 2.0 * k.a * t + k.b;
  float w = 1.0 + k.wb * t * (1.0 - t);
  float dw = k.wb * (1.0 - 2.0 * t);
  float qd = dot(q, d);
  if (order == 0) { return qd * w - dot(q, q) * dw; }
  if (order == 1) { return (dot(d, d) + 2.0 * dot(q, k.a)) * w - qd * dw + 2.0 * k.wb * dot(q, q); }
  return 6.0 * dot(d, k.a) * w + 6.0 * k.wb * qd;
}
// The root between `lo` and `hi`, where the function changes sign: Newton's step when it
// lands in the bracket -- its ends included, so a step that has arrived stays -- and the
// midpoint when not. Twelve steps, `__CANVAS_PIECE_STEPS`.
float pcRoot(MfbPieceQ k, int order, float lo, float hi) {
  bool rising = pcF(k, order, lo) < 0.0;
  float t = (lo + hi) * 0.5;
  for (int i = 0; i < 12; ++i) {
    float f = pcF(k, order, t);
    if ((f < 0.0) == rising) { lo = t; } else { hi = t; }
    float slope = pcF(k, order + 1, t);
    float mid = (lo + hi) * 0.5;
    if (abs(f) < abs(slope)) {
      float moved = t - f / slope;
      if (moved >= lo && moved <= hi) { mid = moved; }
    }
    t = mid;
  }
  return t;
}
// `best` is (squared distance, parameter); `t` as a candidate for it.
vec2 pcTry(MfbPieceQ k, float t, vec2 best) {
  vec2 q = (k.a * t + k.b) * t + k.c;
  float w = 1.0 + k.wb * t * (1.0 - t);
  float d2 = dot(q, q) / (w * w);
  return d2 < best.x ? vec2(d2, t) : best;
}
// (distance, parameter of the nearest point) of a piece from the pixel. The roots are
// isolated, not searched for: g'' is a quadratic, fitted through its values at 0, 1/2
// and 1 and solved; between its roots g' is monotone and holds at most one root; between
// those g is monotone, and a stretch that goes from negative to positive holds exactly
// one minimum.
vec2 pieceNearest(MfbPiece p) {
  MfbPieceQ k;
  vec2 q1 = p.w * p.p1;
  k.a = p.p0 - 2.0 * q1 + p.p2;
  k.b = 2.0 * (q1 - p.p0);
  k.c = p.p0;
  k.wb = 2.0 * (p.w - 1.0);
  vec2 best = vec2(dot(p.p0, p.p0), 0.0);
  vec3 h = vec3(0.0);
  for (int i = 0; i < 3; ++i) { h[i] = pcF(k, 2, 0.5 * float(i)); }
  float alpha = 2.0 * (h.x + h.z) - 4.0 * h.y;
  float beta = h.z - h.x - alpha;
  float r1 = 2.0;
  float r2 = 2.0;
  float disc = beta * beta - 4.0 * alpha * h.x;
  if (disc >= 0.0) {
    float s = sqrt(disc);
    float q = beta < 0.0 ? -(beta - s) * 0.5 : -(beta + s) * 0.5;
    if (abs(q) < abs(alpha) && (q > 0.0) == (alpha > 0.0) && q != 0.0) { r1 = q / alpha; }
    if (abs(h.x) < abs(q) && (h.x > 0.0) == (q > 0.0) && h.x != 0.0) {
      float other = h.x / q;
      if (other < r1) { r2 = r1; r1 = other; } else { r2 = other; }
    }
  }
  vec4 turn = vec4(2.0, 2.0, 2.0, 1.0);
  vec3 from = vec3(0.0, r1, r2);
  vec3 to = vec3(min(r1, 1.0), min(r2, 1.0), 1.0);
  for (int i = 0; i < 3; ++i) {
    if (from[i] < 1.0) {
      float flo = pcF(k, 1, from[i]);
      float fhi = pcF(k, 1, to[i]);
      if ((flo < 0.0 && fhi > 0.0) || (flo > 0.0 && fhi < 0.0)) { turn[i] = pcRoot(k, 1, from[i], to[i]); }
    }
  }
  float lo = 0.0;
  for (int i = 0; i < 4; ++i) {
    float hi = turn[i];
    if (hi > 1.5) { continue; }
    if (pcF(k, 0, lo) < 0.0 && pcF(k, 0, hi) > 0.0) { best = pcTry(k, pcRoot(k, 0, lo, hi), best); }
    best = pcTry(k, hi, best);
    lo = hi;
  }
  return vec2(sqrt(best.x), best.y);
}
// A bound below the piece's distance: the pixel's distance to the box round its three
// control points.
float pieceBound(MfbPiece p) {
  vec2 lo = min(min(p.p0, p.p1), p.p2);
  vec2 hi = max(max(p.p0, p.p1), p.p2);
  return length(max(max(lo, -hi), vec2(0.0)));
}
// What a piece adds to the pixel's winding: 0 unless its ends are on opposite sides of
// the pixel's row and it passes LEFT of the pixel there; then +1 running down the screen
// and -1 running up. The crossing is the root of (a - 2b + c) t^2 + 2 (b - a) t + a taken
// as a / (a - b + s), s = sqrt(b^2 - a c) signed as (a - c): the form that divides by
// the larger candidate.
int pieceCrossing(MfbPiece p) {
  float a = p.p0.y;
  float c = p.p2.y;
  if ((a > 0.0) == (c > 0.0)) { return 0; }
  float t = 0.0;
  if (a != 0.0) {
    float b = p.w * p.p1.y;
    float s = sqrt(b * b - a * c);
    t = a / (a - b + (a < c ? -s : s));
  }
  t = clamp(t, 0.0, 1.0);
  float u = 1.0 - t;
  float k1 = 2.0 * p.w * u * t;
  float xr = (u * u * p.p0.x + k1 * p.p1.x + t * t * p.p2.x) / (u * u + k1 + t * t);
  if (!(0.0 < xr)) { return 0; }
  return c > a ? 1 : -1;
}
// One end of a piece as (along, side, has a direction): how far the pixel is along the
// curve's direction there, from the end, and how far from the tangent line.
vec3 pieceEnd(MfbPiece p, bool atEnd) {
  vec2 e = atEnd ? p.p2 : p.p0;
  vec2 t = atEnd ? p.p2 - p.p1 : p.p1 - p.p0;
  if (t.x == 0.0 && t.y == 0.0) { t = p.p2 - p.p0; }
  float len2 = dot(t, t);
  if (!(len2 > 0.0)) { return vec3(0.0); }
  float len_ = sqrt(len2);
  return vec3(-dot(e, t) / len_, abs(e.y * t.x - e.x * t.y) / len_, 1.0);
}
// `__canvas_pieceCut`: one end of a piece's band, cut as `cap` says (0 Butt, 2 Square),
// given its round band there and whether the nearest point of the piece IS that end.
float pieceCut(MfbPiece p, float halfW, bool atEnd, int cap, float band, bool nearest) {
  vec3 f = pieceEnd(p, atEnd);
  if (f.z == 0.0) {
    if (!nearest) { return band; }
    if (cap == 2) { return max(abs(p.p0.x), abs(p.p0.y)) - halfW; }
    return 1.0e6;
  }
  float plane = atEnd ? f.x : -f.x;
  if (cap == 2) { plane -= halfW; }
  if (nearest) { return cap == 2 ? max(f.y - halfW, plane) : max(band, plane); }
  float zone = halfW * 1.5 + 2.0;
  if (plane > -1.0 && f.x * f.x + f.y * f.y <= zone * zone) { return max(band, plane); }
  return band;
}
// A piece's outline position at `t`, from its arc-length row (`row` in the dash words):
// the position at each sixteenth of the piece, read off the straight line between two.
float pieceArc(int row, float t) {
  float f = t * 16.0;
  int k = int(clamp(floor(f), 0.0, 15.0));
  float c0 = dashWord(row + k);
  float c1 = dashWord(row + k + 1);
  return c0 + (c1 - c0) * (f - float(k));
}
// The point of a piece at outline position `s`, less the pixel.
vec2 pieceAtArc(MfbPiece p, int row, float s) {
  int lo = 0;
  int hi = 16;
  for (int i = 0; i < 4; ++i) {
    int mid = (lo + hi) / 2;
    if (dashWord(row + mid) <= s) { lo = mid; } else { hi = mid; }
  }
  float c0 = dashWord(row + lo);
  float c1 = dashWord(row + lo + 1);
  float f = c1 > c0 ? clamp((s - c0) / (c1 - c0), 0.0, 1.0) : 0.0;
  float t = (float(lo) + f) / 16.0;
  float u = 1.0 - t;
  float k1 = 2.0 * p.w * u * t;
  return (u * u * p.p0 + k1 * p.p1 + t * t * p.p2) / (u * u + k1 + t * t);
}
// Every distance a curve shape is drawn from, in one walk over the band's pieces so
// that `pieceNearest` has one call site. `mode` 0 is a CurvePolygon's signed distance
// (`__canvas_curveDistance`); 1 a PolyCurve's stroke (`__canvas_curveScan`); 2 a
// CurvePolygon's stroke with join records (`__canvas_curveJoined`); 3 either's dashed
// stroke (`__canvas_dashedCurves`).
float curveWalk(ivec2 band, vec2 pt, int mode, float halfW, int cap) {
  int total = item.misc.w;
  bool joins = item.strokeStyle.y > 0;
  int inner = joins ? 0 : 1;
  float reachW = mode == 0 ? 0.0 : halfW * 1.4142135;
  float best = 1.0e6;
  int winding = 0;
  int data = 0;
  float phase = 0.0;
  float ext = cap == 2 ? halfW : 0.0;
  if (mode == 3) {
    data = 3 + int(dashWord(1));
    phase = dashWord(0);
    // A chain that is a single point is the dot its cap draws (`__canvas_dashDot`).
    if (total == 1 && !(dashWord(data + 16) > dashWord(data))) {
      MfbPiece dot0 = curvePiece(0, pt);
      if (cap == 0 || item.misc.x != 11 || !dashOnAt(phase)) { return 1.0e6; }
      if (cap == 2) { return max(abs(dot0.p0.x), abs(dot0.p0.y)) - halfW; }
      return length(dot0.p0) - halfW;
    }
  }
  for (int j = 0; j < band.y; ++j) {
    int e = band.x < 0 ? j : edges.values[band.x + j];
    MfbPiece p = curvePiece(e, pt);
    if (mode == 0) { winding += pieceCrossing(p); }
    int row = data + e * 18;
    float s0 = 0.0;
    float s1 = 0.0;
    float ring = 0.0;
    if (mode == 3) {
      s0 = dashWord(row);
      s1 = dashWord(row + 16);
      ring = dashWord(row + 17);
      if (!(s1 > s0)) { continue; }
    }
    if (pieceBound(p) - reachW < best) {
      vec2 n = pieceNearest(p);
      float d = n.x - (mode == 0 ? 0.0 : halfW);
      if (mode == 3) {
        vec3 f = vec3(0.0);
        if (n.y <= 0.0) { f = pieceEnd(p, false); } else if (n.y >= 1.0) { f = pieceEnd(p, true); }
        float u = 0.0;
        if (f.z != 0.0) {
          u = (n.y <= 0.0 ? s0 : s1) + f.x;
          d = f.y - halfW;
        } else {
          u = pieceArc(row, n.y);
        }
        vec2 ends;
        bvec2 has;
        d = max(d, dashAlong(u, s0, s1, abs(ring), ring > 0.0, ext, ends, has));
        if (cap == 1) {
          if (has.x) { d = min(d, length(pieceAtArc(p, row, ends.x)) - halfW); }
          if (has.y) { d = min(d, length(pieceAtArc(p, row, ends.y)) - halfW); }
        }
      } else if (mode != 0) {
        int startCap = mode == 2 ? 0 : (e == 0 ? cap : inner);
        int endCap = mode == 2 ? 0 : (e == total - 1 ? cap : inner);
        if (startCap != 1) { d = pieceCut(p, halfW, false, startCap, d, n.y <= 0.0); }
        if (endCap != 1) { d = pieceCut(p, halfW, true, endCap, d, n.y >= 1.0); }
      }
      best = min(best, d);
    }
    // The record at this piece's start: a disc where the outline is smooth, the paint's
    // join where two curves meet at an angle. Dashed, only where a dash runs through.
    if (mode != 0 && joins) {
      int jk = joinKind(e);
      bool drawn = jk != 0;
      if (drawn && mode == 3) {
        float before = (ring > 0.0 && s0 == 0.0) ? abs(ring) : s0;
        drawn = dashOnBefore(before + phase) && dashOnAt(s0 + phase);
      }
      if (drawn) {
        best = min(best, joinPieceAt(e, jk, pt - joinPoint(e, 0)));
      }
    }
  }
  if (mode != 0) { return best; }
  bool inside = item.arc.y != 0 ? winding != 0 : (winding & 1) != 0;
  return inside ? -best : best;
}
// @CURVES-END

// `__canvas_dashedEdges`: a polygon's or polyline's band edges, a line's one edge, or a
// rectangle's or picture's four sides, each band cut by the dashes on its own [s0, s1],
// with the corner piece wherever a dash runs through a vertex. An open path with no
// length is the dot its cap draws (`__canvas_dashDot`).
float dashedEdges(ivec2 band, vec2 p, float halfW, int cap) {
    int kind = item.misc.x;
    bool poly = kind == 4 || kind == 10;
    int count = poly ? band.y : (kind == 2 ? 1 : 4);
    int data = 3 + int(dashWord(1));
    float phase = dashWord(0);
    float ext = cap == 2 ? halfW : 0.0;
    vec2 c = vec2(fx(item.shape.x), fx(item.shape.y));
    vec2 h = vec2(fx(item.shape.z), fx(item.shape.w));
    if ((kind == 2 || kind == 10) && dashWord(data + 2) == 0.0) {
        vec2 o = c;
        if (kind == 10) {
            vec2 b0;
            edgeEnds(item.arc.z, 0, o, b0);
        }
        vec2 w0 = p - o;
        if (cap == 0 || !dashOnAt(phase)) { return 1.0e6; }
        if (cap == 2) { return max(abs(w0.x), abs(w0.y)) - halfW; }
        return length(w0) - halfW;
    }
    float best = 1.0e6;
    for (int j = 0; j < count; ++j) {
        int e = poly ? (band.x < 0 ? j : edges.values[band.x + j]) : j;
        vec2 a;
        vec2 b;
        if (poly) {
            edgeEnds(item.arc.z, e, a, b);
        } else if (kind == 2) {
            a = c;
            b = h;
        } else {
            joinedEdge(false, e, a, b);
        }
        vec2 v = b - a;
        float len2 = dot(v, v);
        if (!(len2 > 0.0)) { continue; }
        vec2 w = p - a;
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
                    best = min(best, joinPieceAt(e, jk, p - joinPoint(e, 0)));
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
        float r = fx(item.shape.z);
        float a = s / r + (kind == 3 ? dashWord(data) : 0.0);
        return r * vec2(dashCos(a), dashSin(a));
    }
    if (kind == 7) {
        float rx = fx(item.shape.z);
        float ry = fx(item.shape.w);
        float ca = fx(item.ellipse.x);
        float sa = fx(item.ellipse.y);
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
    float iw = fx(item.shape.z);
    float ih = fx(item.shape.w);
    float r = fx(item.misc.y);
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
float dashedCurve(vec2 p, float halfW, int cap) {
    int kind = item.misc.x;
    vec2 q = p - vec2(fx(item.shape.x), fx(item.shape.y));
    int data = 3 + int(dashWord(1));
    float bandD;
    float u;
    float len_;
    bool closed = true;
    if (kind == 1 || kind == 3) {
        float r = fx(item.shape.z);
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
        float rx = fx(item.shape.z);
        float ry = fx(item.shape.w);
        float ca = fx(item.ellipse.x);
        float sa = fx(item.ellipse.y);
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
        float iw = fx(item.shape.z);
        float ih = fx(item.shape.w);
        float r = fx(item.misc.y);
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
float dashedStroke(ivec2 band, vec2 p, float halfW) {
    int kind = item.misc.x;
    bool rounded = kind == 0 && item.misc.y != 0;
    bool curve = kind == 1 || kind == 3 || kind == 7 || rounded;
    return curve ? dashedCurve(p, halfW, item.arc.w) : dashedEdges(band, p, halfW, item.arc.w);
}

// One sample's distance at the shape-space point `p`, for every kind a distance draws: a
// path clip's or a polygon's edge walk, a curve's pieces, a dashed stroke, a polyline, or
// `shapeDistance`. One call site each; `shapeCoverage` calls this from two.
float sampleDistance(ivec2 band, vec2 p, bool clip) {
    int kind = item.misc.x;
    // plan-163-C: a path clip's edges under its own rule, through the polygon's walk -- one
    // `edgeDistance` call site, for lavapipe's compile time.
    if (clip || kind == 4) {
        return edgeDistance(clip ? item.clipPath.x : item.arc.z, band.y, band.x, p,
                            clip ? (item.clipPath.z >> 2) : item.arc.y);
    }
    // plan-164-C: a CurvePolygon's signed distance, or a PolyCurve's stroke -- dashed or
    // not -- from its pieces. A PolyCurve is stroke-as-fill, its half-width the radius.
    if (kind == 11 || kind == 12) {
        return curveWalk(band, p, kind == 12 ? 0 : (item.strokeStyle.w > 0 ? 3 : 1),
                         fx(item.misc.y), item.arc.w);
    }
    // plan-163-B: a dashed Line, Arc or PolyLine is its dashed stroke.
    if (item.strokeStyle.w > 0 && (kind == 2 || kind == 3 || kind == 10)) {
        return dashedStroke(band, p, fx(item.misc.y));
    }
    if (kind == 10) { return polylineDistance(band, p); }
    return shapeDistance(p);
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

// bug-1079: which of `n` texels holds `t`, the texels `d` apart: `floor(t / d)`, made
// independent of how the driver rounds `/`. Vulkan allows a quotient 2.5 ULP of error,
// and one real driver takes it (it multiplies by a reciprocal), so where `t` is exactly a
// whole number of `d` -- a texel boundary on a pixel centre -- `floor` of the quotient
// alone is the texel on either side. The quotient is within one of the answer whatever
// its last bits are, so it is corrected by at most one from the products, which every
// backend rounds correctly and which are exact wherever the quotient is a whole number.
// The oracle's picture arm (`helper_items.rs`) and the MSL take the same three steps.
int pictureTexel(float t, float d, int n) {
    float u = floor(t / d);
    if ((u + 1.0) * d <= t) { u += 1.0; } else if (u * d > t) { u -= 1.0; }
    return clamp(int(u), 0, n - 1);
}

int tileTexel(float t, float d) {
    float u = floor(t / d);
    if ((u + 1.0) * d <= t) { u += 1.0; } else if (u * d > t) { u -= 1.0; }
    return int(u);
}

// A picture's fill colour at the SHAPE-space point `q`, as an `ivec4` of 0..255
// (bug-484) — the oracle's picture arm in `helper_items.rs`, operation for operation.
//
// Sampled NEAREST, the plan-116-C §4.5 rule glyphs follow: `floor` and `+ - * /` only,
// in the oracle's order `(q - x0) * iw / dw`, with the index taken by `pictureTexel`
// so it does not hang on the division's last bit; every renderer picks the same texel.
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
    int u = pictureTexel((q.x - x0) * float(iw), dw, iw);
    int v = pictureTexel((q.y - y0) * float(ih), dh, ih);
    int word = edges.values[GLYPH_BASE + item.arc.z + v * iw + u];
    ivec4 tex = ivec4(word & 255, (word >> 8) & 255, (word >> 16) & 255, (word >> 24) & 255);
    // plan-163-C: towards `texel * fill / 255` by `fill.a / 255`, rounded half away from
    // zero, and the alpha faded by the transparency step (`__canvas_tintChannel`).
    ivec4 tint = (tex * item.fill) / 255;
    int a = item.fill.w;
    ivec4 outc = tex;
    for (int c = 0; c < 3; ++c) {
        int dc = (tint[c] - tex[c]) * a;
        outc[c] = tex[c] + (dc >= 0 ? (dc + 127) / 255 : -((127 - dc) / 255));
    }
    outc.w = (tex.w * (255 - item.clipPath.w) + 127) / 255;
    return outc;
}

// plan-173: direct source-rectangle mapping. Unlike Picture, a sample outside the
// image is transparent and the surviving portion is not stretched over the destination.
ivec4 tileColour(vec2 q) {
    int iw = item.misc.w;
    int ih = item.arc.x;
    if (iw <= 0 || ih <= 0) { return ivec4(0); }
    float x0 = fx(item.shape.x) - fx(item.shape.z);
    float y0 = fx(item.shape.y) - fx(item.shape.w);
    float dw = fx(item.shape.z) * 2.0;
    float dh = fx(item.shape.w) * 2.0;
    float sx = fx(item.gradient.x);
    float sy = fx(item.gradient.y);
    float sw = fx(item.gradient.z);
    float sh = fx(item.gradient.w);
    int u = tileTexel(sx * dw + (q.x - x0) * sw, dw);
    int v = tileTexel(sy * dh + (q.y - y0) * sh, dh);
    // bug-1093: clamp to the texels the source rectangle touches, before the image test.
    u = clamp(u, int(floor(sx)), int(ceil(sx + sw)) - 1);
    v = clamp(v, int(floor(sy)), int(ceil(sy + sh)) - 1);
    if (u < 0 || v < 0 || u >= iw || v >= ih) { return ivec4(0); }
    int word = edges.values[GLYPH_BASE + item.arc.z + v * iw + u];
    return ivec4(word & 255, (word >> 8) & 255, (word >> 16) & 255, (word >> 24) & 255);
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
// The stops are five words each in the buffer's third region: the offset as float32 bits,
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

// plan-163-D: a ramp position past its ends, per the spread -- `__canvas_gradientSpread`,
// operation for operation: Pad (0) leaves it for `gradientAt`'s clamp, Repeat (2) is
// `t - floor(t)`, Reflect (1) folds `t - 2 floor(t / 2)` back at 1. Metal's, line for line.
float gradientSpread(float t, int spread) {
    if (spread == 0) { return t; }
    float tt = clamp(t, -1.0e9, 1.0e9);
    if (spread == 2) { return tt - floor(tt); }
    float u = tt - 2.0 * floor(tt / 2.0);
    return u > 1.0 ? 2.0 - u : u;
}

// plan-163-D: a focal radial's ramp position at the gradient-space point `p` --
// `__canvas_focalT`, operation for operation: the larger root of `|p - f - t (c - f)|^2 =
// (fr + t (r - fr))^2`, whose leading coefficient the builder's clamps keep below zero.
// Metal's, line for line.
float focalT(vec2 p, vec2 c, float r, vec2 o, float fr) {
    if (!(r > 0.0)) { return 0.0; }
    vec2 f = c + o;
    vec2 e = p - f;
    vec2 w = c - f;
    float dr = r - fr;
    float a = w.x * w.x + w.y * w.y - dr * dr;
    float b = e.x * w.x + e.y * w.y + fr * dr;
    float cc = e.x * e.x + e.y * e.y - fr * fr;
    float disc = max(b * b - a * cc, 0.0);
    return (b - sqrt(disc)) / a;
}

// plan-163-D: the ramp at the SHAPE-space point `q` (the inverse-mapped point under a
// transform), so the item's transform and group carry it; a linear gradient's own
// transform is folded into its axis at build, a radial one's maps `q` through
// `gradInv0`/`gradInv1` here, and `surface.w` packs `kind | spread << 1 | apply << 3`.
ivec4 gradientColour(vec2 q) {
    vec2 from = vec2(fx(item.gradient.x), fx(item.gradient.y));
    vec2 to = vec2(fx(item.gradient.z), fx(item.gradient.w));
    vec2 axis = to - from;
    float len2 = dot(axis, axis);
    float t = 0.0;
    if ((item.surface.w & 1) == 1) {
        // Radial: distance from the centre, over the axis length. A zero-length axis
        // leaves t at 0, so the first stop fills — defined, not a divide by zero.
        vec2 g = q;
        if (item.gradInv1.z != 0) {
            g = vec2(intBitsToFloat(item.gradInv0.x) * q.x + intBitsToFloat(item.gradInv0.z) * q.y
                         + intBitsToFloat(item.gradInv1.x),
                     intBitsToFloat(item.gradInv0.y) * q.x + intBitsToFloat(item.gradInv0.w) * q.y
                         + intBitsToFloat(item.gradInv1.y));
        }
        vec2 o = vec2(fx(item.gradFocal.x), fx(item.gradFocal.y));
        float fr = fx(item.gradFocal.z);
        float len = sqrt(len2);
        if (o.x != 0.0 || o.y != 0.0 || fr != 0.0) {
            t = focalT(g, from, len, o, fr);
        } else if (len > 0.0) {
            t = length(g - from) / len;
        }
    } else {
        if (len2 > 0.0) { t = dot(q - from, axis) / len2; }
    }
    return gradientAt(gradientSpread(t, (item.surface.w >> 1) & 3));
}

// plan-163-D: a ramp colour tinted by `by` at the strength of its alpha -- the picture
// tint (`__canvas_tintChannel`) -- keeping the ramp's alpha. A stroke the ramp paints
// (`FillAndStroke`) is this of the stroke colour; a Line's or Arc's, of its fill slots.
ivec4 tintRamp(ivec4 ramp, ivec4 by) {
    ivec4 tint = (ramp * by) / 255;
    ivec4 outc = ramp;
    for (int c = 0; c < 3; ++c) {
        int dc = (tint[c] - ramp[c]) * by.w;
        outc[c] = ramp[c] + (dc >= 0 ? (dc + 127) / 255 : -((127 - dc) / 255));
    }
    return outc;
}

vec4 covered(ivec4 rgba, int coverage) {
    float a = float((rgba.w * coverage) / 255) / 255.0;
    return vec4(srgbToLinear(float(rgba.x)) * a,
                srgbToLinear(float(rgba.y)) * a,
                srgbToLinear(float(rgba.z)) * a, a);
}

// The clip rectangle's own antialiased coverage, 0..255 (plan-116-B): the same rectangle
// distance and the same `clamp(0.5 - d, 0, 1)` quantization the shape edges use, so a
// fractional clip edge is antialiased identically to a shape edge. plan-163-C: at the
// SHAPE-space point (the group offset taken off), because the clip is in the item's own
// coordinates, and only for a mode-1 clip, whose zero-area rectangle encloses nothing and
// hides the item. `__canvas_clipCoverage` is the same lines; Metal's `clipRect`, line for
// line.
int clipRect(vec2 p) {
    if (!(fx(item.clip.x) < fx(item.clip.z)) || !(fx(item.clip.y) < fx(item.clip.w))) { return 0; }
    vec2 lo = vec2(fx(item.clip.x), fx(item.clip.y));
    vec2 hi = vec2(fx(item.clip.z), fx(item.clip.w));
    return coverage(rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5));
}

// plan-172-H: a rectangle clip's signed distance at the item's own point. `clipRect` above
// answers coverage and is right while the rectangle is axis-aligned on the surface; under
// a group matrix it is not, so its distance is sampled like a path clip's and divided by
// its own gradient (`__canvas_clipRectSurface`). A rectangle with no area encloses
// nothing. Metal's `clipRectDistance`, line for line.
float clipRectDistance(vec2 p) {
    if (!(fx(item.clip.x) < fx(item.clip.z)) || !(fx(item.clip.y) < fx(item.clip.w))) { return 1.0e6; }
    vec2 lo = vec2(fx(item.clip.x), fx(item.clip.y));
    vec2 hi = vec2(fx(item.clip.z), fx(item.clip.w));
    return rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5);
}

// An item's fill and stroke coverage, 0..255 each, and whether it strokes, from its
// distance at the shape-space point `p`. The oracle's `__canvas_drawGeometry`, operation
// for operation: the inverse map with the float32 terms, the central differences at 0.5
// (all five on the centre's band), `d / ||grad d||`, and the stroke's half-width taken off
// in shape space before the scale. The CENTRE sample is straight-line code and every other
// sample sits behind one branch (bug-688, below), so `sampleDistance` has two call sites. A
// transformed CENTRE sample past `half + 0.5 * max(F, 1)` (F the inverse matrix's Frobenius
// norm, which bounds the gradient) decides the coverage by its sign and the four neighbours
// are skipped. A clip's centre sample past `0.5 * max(F, 1)` decides the clip's coverage
// the same way (bug-995). Metal's `shapeCoverage`, line for line, where both bounds are
// argued.
//
// plan-172-H: `p` arrives already through the groups' inverse linear part when `grouped`,
// and `gi` is that part: the four neighbour samples are half a SURFACE pixel away, which
// in the group's space is `gi` times that. An item inside a group with a matrix takes the
// transformed path whether or not it has a transform of its own — `mapped` — while
// `itemMapped` alone says whether its own inverse applies.
ivec4 shapeCoverage(vec2 p) {
    bool itemMapped = hasTransform();
    bool mapped = itemMapped || grouped;
    float halfW = fx(item.misc.z);
    // The oracle strokes exactly when its f64 half-width is above 0. The word's float32
    // bits, read as a signed int, are positive exactly when the value is above +0, and
    // the emitter makes a positive one that narrows to +0 the smallest float32
    // (`emit_item_stroke_sign`) -- so this tests bits, which a GPU that flushes that
    // denormal to zero still reads as positive.
    bool stroked = item.misc.z > 0;
    float reach = 0.5 + max(halfW, 0.0);
    // bug-995: the clip's reach -- the shape's without the stroke half, and for any kind
    // (a clip's distance is 1-Lipschitz whatever the item is, an arc's too).
    float clipReach = 0.5;
    // bug-1085: the composed inverse's norm, which the band lookups hold to the norm the
    // band's reach was built for; 0 for a pixel that takes its centre sample alone.
    float norm = 0.0;
    if (mapped) {
        vec2 c0 = itemMapped ? vec2(intBitsToFloat(item.xform0.x), intBitsToFloat(item.xform0.y)) : vec2(1.0, 0.0);
        vec2 c1 = itemMapped ? vec2(intBitsToFloat(item.xform0.z), intBitsToFloat(item.xform0.w)) : vec2(0.0, 1.0);
        // The composed inverse's norm is at most the product of the two norms.
        norm = sqrt(dot(c0, c0) + dot(c1, c1));
        if (grouped) { norm = (itemMapped ? norm : 1.0) * length(gi); }
        float frob = norm * (1.0 + 1.0 / 256.0) + 1.0 / 256.0;
        reach = max(halfW, 0.0) + 0.5 * max(frob, 1.0);
        clipReach = 0.5 * max(frob, 1.0);
    }
    vec2 cq = itemMapped ? inverseMap(p) : p;
    ivec2 band = polygonBand(cq.y, norm);
    // plan-163-B: a polygon, rectangle or picture with join records strokes from its own
    // distance (`joinedStroke`), which a miter can reach past `half` from the outline --
    // so the centre's shape distance cannot decide the neighbours there.
    // plan-163-B: so can a dashed one, which strokes from `dashedStroke`.
    bool dashed = item.strokeStyle.w > 0;
    // plan-164-C: nor a PolyCurve, stroke-as-fill like a PolyLine: `sampleDistance` is
    // its whole stroke.
    bool joined = (item.strokeStyle.y > 0 || dashed) && item.misc.x != 10 && item.misc.x != 11;
    // plan-163-C: samples 5..9 are a path clip's distance at the same five points, so the
    // clip is divided by ITS OWN gradient (`__canvas_clipPathSurface`), not the shape's.
    // bug-995: the clip's CENTRE sample (5) past `clipReach` decides its coverage by its
    // sign, exactly as the shape's does, and its four neighbours are skipped: their
    // central difference `cg` is at most `frob` (or the fallback 1.0), so
    // `|d5| > 0.5 * max(frob, 1)` puts `|d5 / cg|` past 0.5, where `coverage` has
    // clamped. The bound is argued in Metal's twin.
    int clipMode = item.clipPath.z & 3;
    bool clipMapped = itemMapped || grouped;
    // plan-172-H: a rectangle clip is sampled too, under a group matrix.
    bool clipSampled = clipMode == 2 || (clipMode == 1 && grouped);
    ivec2 cband = clipMode == 2 ? clipBandAt(cq.y, norm) : ivec2(-1, 0);
    float d[10];
    // bug-688: the shape's centre sample is taken here, outside the loop, and the loop is
    // entered only by a pixel that needs another sample -- the shape's four neighbours
    // (`mapped`, once the centre has not decided it) or the clip's (`clipSampled`). An
    // untransformed, unclipped pixel therefore runs one sample and no loop. It did in the
    // one ten-pass loop this replaces too, as nine `continue`s, and a driver that does
    // not turn a skipped pass into skipped work charged it for all ten (box 206's Intel
    // driver: 10,000 16 px squares rendered in 106 ms a frame, and in 58 this way).
    // Metal's, line for line.
    d[0] = sampleDistance(band, cq, false);
    if (!joined && abs(d[0]) > reach) { mapped = false; }
    if (mapped || clipSampled) {
        for (int k = 1; k < 10; ++k) {
            bool clipK = k >= 5;
            int j = clipK ? k - 5 : k;
            if (clipK ? (!clipSampled || j >= (clipMapped ? 5 : 1)) : !mapped) { continue; }
            float ox = j == 1 ? 0.5 : (j == 2 ? -0.5 : 0.0);
            float oy = j == 3 ? 0.5 : (j == 4 ? -0.5 : 0.0);
            vec2 s = p + (grouped ? vec2(gi.x * ox + gi.z * oy, gi.y * ox + gi.w * oy) : vec2(ox, oy));
            vec2 q = itemMapped ? inverseMap(s) : s;
            if (clipK && clipMode == 1) {
                d[k] = clipRectDistance(q);
            } else {
                d[k] = sampleDistance(clipK ? cband : band, q, clipK);
            }
            if (k == 5 && abs(d[5]) > clipReach) { clipMapped = false; }
        }
    }
    float g = 1.0;
    if (mapped) {
        g = length(vec2(d[1] - d[2], d[3] - d[4]));
        if (!(g > 0.000001)) { g = 1.0; }
    }
    int fill = coverage(mapped ? d[0] / g : d[0]);
    int stroke = 0;
    if (stroked) {
        // plan-164-C: a CurvePolygon's joined or dashed stroke is its pieces' (`curveWalk`).
        float s = (item.misc.x == 12 && joined)
            ? curveWalk(band, cq, dashed ? 3 : 2, halfW, item.arc.w)
            : (dashed ? dashedStroke(band, cq, halfW)
            : (joined ? joinedStroke(band, cq, halfW) : abs(d[0]) - halfW));
        stroke = coverage(mapped ? s / g : s);
    }
    int clipCov = 255;
    if (clipMode == 1 && !grouped) { clipCov = clipRect(p); }
    if (clipSampled) {
        float cg = 1.0;
        if (clipMapped) {
            cg = length(vec2(d[6] - d[7], d[8] - d[9]));
            if (!(cg > 0.000001)) { cg = 1.0; }
        }
        clipCov = coverage(clipMapped ? d[5] / cg : d[5]);
    }
    return ivec4(fill, stroke, stroked ? 1 : 0, clipCov);
}

void main() {
    // First, before anything reads it: everything below, and both helpers above, work
    // off this one record.
    item = itemBuf.blocks[vItem];
    int at = NODE_BASE + 8 * vNode;
    draw.offset = ivec4(edges.values[at], edges.values[at + 1], edges.values[at + 2], edges.values[at + 3]);
    draw.inverse = ivec4(edges.values[at + 4], edges.values[at + 5], edges.values[at + 6], edges.values[at + 7]);
    // The clip multiplies the shape's own coverage, exactly as it does in the oracle's
    // `(coverage * clipCov) / 255`. Integer, and by 255 rather than a shift, so the two
    // quantize identically — a float multiply here would disagree on the boundary
    // pixels, which are the only ones a clip can affect.
    // plan-116-H: the shape-space point. The vertex stage moved the quad by +offset, so
    // the distance field is evaluated at p - offset — the same translation-as-inverse-map
    // rule plan-116-C established for `Paint.transform` and plan-116-G §4.5 restated for
    // groups. One rule, three renderers.
    vec2 p = gl_FragCoord.xy - vec2(fx(draw.offset.x), fx(draw.offset.y));
    // plan-172-H: and, under a group matrix, through the groups' inverse linear part — so
    // `p` is the point in the GROUP's space, where an item's own placement lives, and
    // every consumer below (the item's own inverse map included) is unchanged.
    grouped = (draw.inverse.x | draw.inverse.y | draw.inverse.z | draw.inverse.w) != 0;
    gi = vec4(intBitsToFloat(draw.inverse.x), intBitsToFloat(draw.inverse.y),
              intBitsToFloat(draw.inverse.z), intBitsToFloat(draw.inverse.w));
    if (grouped) { p = vec2(gi.x * p.x + gi.z * p.y, gi.y * p.x + gi.w * p.y); }
    // plan-163-C: the clip is evaluated in `shapeCoverage` at the SHAPE-space point -- it
    // travels with the item's transform and its group -- for every kind, glyphs included.
    ivec4 cov = shapeCoverage(p);
    int clipCov = cov.w;
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
    // Both coverages come from `shapeCoverage` at `p`. The fill uses the surface distance
    // and the stroke subtracts `half` BEFORE converting, so the outline scales with the
    // shape (§4.3).
    // plan-116-F: the gradient replaces the fill COLOUR and nothing else — the shape's
    // distance, its coverage and its stroke are untouched.
    // plan-163-D: at the shape-space point `q`, so the transform carries the ramp; and
    // with `FillAndStroke` (`surface.w & 8`) the stroke -- a Line's or Arc's fill -- is the
    // ramp tinted by its colour.
    vec2 q = hasTransform() ? inverseMap(p) : p;
    ivec4 ramp = item.ellipse.z >= 2 ? gradientColour(q) : item.fill;
    bool rampStroke = item.ellipse.z >= 2 && (item.surface.w & 8) != 0;
    ivec4 fillRgba = ramp;
    if (rampStroke && (item.misc.x == 2 || item.misc.x == 3 || item.misc.x == 11)) { fillRgba = tintRamp(ramp, item.fill); }
    // plan-154-C: a PolyLine's stops are its per-point colours, and its ramp runs along
    // the path -- the parameter of the nearest point, in shape space, from the same band
    // the distance used.
    if (item.misc.x == 10 && item.ellipse.z >= 2) {
        fillRgba = gradientAt(polylineScan(polygonBand(q.y, 0.0), q).y);
    }
    // bug-484: a picture's image replaces the fill colour the same way — sampled at the
    // shape-space point, the inverse-mapped one under a transform.
    if (item.misc.x == 9) {
        fillRgba = pictureColour(q);
    }
    if (item.misc.x == 13) {
        fillRgba = tileColour(q);
    }
    // plan-163-D: a fill alpha of -1 is the stroke record of a split blended item
    // (`emit_split_or_publish`): no fill here, whatever its colour would be.
    vec4 colour = item.fill.w < 0 ? vec4(0.0) : covered(fillRgba, (cov.x * clipCov) / 255);
    if (cov.z != 0) {
        // Stroke over fill, then the hardware puts that over the destination — which
        // is what makes this one fragment equal to the software path's two sequential
        // writes, since `over` is associative.
        //
        // plan-116-B: that identity is `Normal`-ONLY, which is why a non-`Normal`
        // stroked-and-filled item is emitted as two adjacent instances instead (§4.3).
        // By the time such an item reaches here it is either fill-only or stroke-only,
        // so this branch composes two sources only under `Normal`, where it is exact.
        vec4 s = covered(rampStroke ? tintRamp(ramp, item.stroke) : item.stroke, (cov.y * clipCov) / 255);
        colour = s + colour * (1.0 - s.w);
    }
    fragColor = colour;
}
