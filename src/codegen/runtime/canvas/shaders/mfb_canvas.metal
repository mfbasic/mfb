// The canvas shaders for Metal (MSL): the vertex stage, then the fragment stage. The
// twin of mfb_canvas.vert and mfb_canvas.frag. Metal compiles this text at run time:
// metal.rs (METAL_SHADER_SOURCE) puts the region bases in front of it and replaces the
// curve-shader marker line with the curve functions (CURVE_SHADER_MARK, curves.tpl).

struct MfbItem {
  int4 quad; // bounds minX, minY, maxX, maxY (16.16 px)
  int4 shape; // p0..p3 (16.16 px)
  int4 fill; // RGBA 0..255
  int4 stroke; // RGBA 0..255
  int4 misc; // kind, radius (16.16), strokeHalf (16.16), edgeCount
  // glyph/picture height, arc sweep turn (bug-735, bug-1088) / polygon fill rule (plan-163-A),
  // edgeBase, capStyle
  int4 arc;
  int2 surface; // width, height (px)
  int blendMode; // the BlendMode tag 0..3 (plan-116-B)
  // 0 linear, 1 radial (plan-116-F); was the block's pad. plan-163-D: `kind | spread
  // << 1 | apply << 3`.
  int gradientKind;
  int4 clip; // clip x0,y0,x1,y1 (16.16 px); zero-area = unclipped
  int4 xform0; // inverse transform ia,ib,ic,id as float32 BITS
  // itx, ity (float32 bits), hasTransform (0 or 1); then (bug-1083) how much of the
  // quad's x sides is rim, 16.16, which only the vertex stage reads
  int4 xform1;
  int4 arcCaps; // an arc's two sweep endpoints startX,startY,endX,endY (16.16)
  int4 ellipse; // ellipse cos, sin (16.16); gradient stop count and base
  int4 gradient; // a gradient's axis startX,startY,endX,endY (16.16)
  // bug-735: the residual words (units of 2^-47 px) of every field a distance reads.
  int4 shapeLo; // of shape.xyzw
  int4 capsLo; // of arcCaps.xyzw
  int4 clipLo; // of clip.xyzw
  int4 miscLo; // of misc.y (radius), misc.z (strokeHalf), ellipse.x, ellipse.y
  // plan-163-B: join records' first edge slot and count, dash pattern's first entry
  // and count.
  int4 strokeStyle;
  // plan-163-C: a path clip's band header, then its edge base, edge count, mode | rule
  // << 2 and a picture's transparency step.
  int4 clipBand;
  int4 clipPath;
  // plan-163-D: a radial gradient's own inverse transform (float32 bits) and flag, then
  // its focal offset and focal radius (16.16). bug-1083: `gradInv1.w` is how much of
  // the quad's y sides is rim, 16.16 (`xform1.w` has the x sides').
  int4 gradInv0;
  int4 gradInv1;
  int4 gradFocal;
};
// plan-116-A: the index travels to the fragment stage as a flat varying, because
// `[[instance_id]]` does not exist there. `[[flat]]` and not the default: the value
// is an index, and interpolating an index across a quad produces a *plausible*
// picture drawn from the wrong blocks rather than a failure.
struct VOut { float4 pos [[position]]; uint item [[flat]]; };
static float fx(int v) { return float(v) / FIXED; }
// plan-172-H: what one draw binds, to both stages -- 32 bytes. `offset` is the
// translation of the enclosing groups' composed affine: two 16.16 words, then
// (bug-735) their residual words. `inverse` is the linear part of that affine's
// INVERSE as four float32 bit patterns -- `ia`, `ib`, `ic`, `id`, so a point maps as
// `(ia * x + ic * y, ib * x + id * y)` -- and all-zero bits when the groups only
// translate, the convention every `canvas::Transform` has. An inverse is never all
// zero, so the test cannot mistake one.
struct MfbDraw { int4 offset; int4 inverse; };
static bool drawGrouped(constant MfbDraw &draw) {
  return (draw.inverse.x | draw.inverse.y | draw.inverse.z | draw.inverse.w) != 0;
}
static float4 drawInverse(constant MfbDraw &draw) {
  return float4(as_type<float>(draw.inverse.x), as_type<float>(draw.inverse.y),
                as_type<float>(draw.inverse.z), as_type<float>(draw.inverse.w));
}
// `[[instance_id]]` **already includes `baseInstance`**, so a run that begins partway
// through the item buffer indexes it directly and needs no other arithmetic — the
// same property Vulkan's `gl_InstanceIndex` has. That is measured, not assumed, and
// it is the opposite of what plan-116-A predicted: adding a separate
// `[[base_instance]]` on top of it double-counted, so `baseInstance = 0` drew
// correctly and every non-zero base indexed past the end of the scene's blocks and
// drew nothing at all (plan-116-A Correction C5).
//
// Indexing here rather than binding the buffer at `base * ITEM_BLOCK_SIZE` also
// sidesteps `MTLBuffer` offset alignment, which the item stride does not satisfy
// (`ITEM_BLOCK_SIZE`, 272 since bug-735; 112 when this was written).
// plan-116-H: the group offset moves the QUAD, and the fragment stage moves the
// query point the other way. Same split the software renderer makes (plan-116-G
// section 4.5) and the same one `mfb_canvas.vert` makes -- bounds by +offset,
// distance evaluated at p - offset -- so one rule covers all three renderers.
//
// No clamp to the surface. The quad may extend past it and the rasteriser discards
// what falls outside clip space; clamping would shrink a quad the fragment stage
// still expects to span the item's full extent.
//
// `buffer(1)` and not a bigger `MfbItem`: the offset varies per DRAW while the item
// varies per instance, so putting it in the item block would mean rewriting every
// block whenever a group moved.
vertex VOut mfbVertex(uint vid [[vertex_id]],
                      uint iid [[instance_id]],
                      constant MfbItem *items [[buffer(0)]],
                      constant MfbDraw &draw [[buffer(1)]]) {
  uint index = iid;
  constant MfbItem &item = items[index];
  float2 corner = float2(fx((vid & 1) == 0 ? item.quad.x : item.quad.z),
                         fx((vid & 2) == 0 ? item.quad.y : item.quad.w))
                + float2(fx(draw.offset.x), fx(draw.offset.y));
  // plan-172-H: under a group matrix the quad is the item's box carried through the
  // FORWARD linear part -- the inverse of the 2x2 the draw carries -- which makes it
  // a parallelogram, and four mapped corners of a triangle strip are exactly that.
  //
  // bug-1083: the box carried is not the block's as it stands. Its rim -- `xform1.w`
  // on the x sides, `gradInv1.w` on the y sides, zero for a box that is not an extent
  // and a rim -- was sized for the item drawn with no group, and a pixel takes
  // coverage only within half the COMPOSED inverse's norm of the item, which is at
  // most the product of the item's norm and the groups'. So the rim is replaced by
  // itself scaled by the groups' norm `|M|`: the lower sides are `(bound + rim) - rim
  // * |M|`, the upper `(bound - rim) + rim * |M|`. `__canvas_drawBound` has the
  // derivation, kind of rim by kind of rim, and takes the same two lines.
  //
  // Then the pixel: the box is grown by `|M|` once more on every side. That is at
  // least one pixel on the surface, because the forward map shrinks nothing by more
  // than `1 / |M|`, and it is added here, before the map, because the quad is the
  // mapped parallelogram and not its hull (the oracle pads its hull by a pixel
  // instead). The fragment stage decides coverage; a quad can only be too small.
  if (drawGrouped(draw)) {
    float4 gi = drawInverse(draw);
    float pad = length(gi);
    float2 rim = float2(fx(item.xform1.w), fx(item.gradInv1.w));
    float2 grow = rim * pad - rim + pad;
    float2 local = float2(fx((vid & 1) == 0 ? item.quad.x : item.quad.z) + ((vid & 1) == 0 ? -grow.x : grow.x),
                          fx((vid & 2) == 0 ? item.quad.y : item.quad.w) + ((vid & 2) == 0 ? -grow.y : grow.y));
    float det = gi.x * gi.w - gi.y * gi.z;
    corner = float2((gi.w * local.x - gi.z * local.y) / det, (gi.x * local.y - gi.y * local.x) / det)
             + float2(fx(draw.offset.x), fx(draw.offset.y));
  }
  VOut o;
  o.item = index;
  o.pos = float4(corner.x / float(item.surface.x) * 2.0 - 1.0,
                 1.0 - corner.y / float(item.surface.y) * 2.0, 0.0, 1.0);
  return o;
}
static float rectDistance(float2 p, float2 c, float2 h) {
  float2 d = abs(p - c) - h;
  return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}
static float segmentDistance(float2 p, float2 a, float2 b) {
  float2 v = b - a, w = p - a;
  float len2 = dot(v, v);
  float t = len2 > 0.0 ? clamp(dot(w, v) / len2, 0.0, 1.0) : 0.0;
  return length(w - v * t);
}
// plan-116-D: the butt-capped twin, and the twin of `segmentDistanceButt` in
// `runtime/canvas/shaders/mfb_canvas.frag`. `half` comes off BEFORE the two `max`es
// because a butt stroke is the round band intersected with the slab between the end
// planes — subtracting after would compare each plane against the half-width rather
// than against zero, so the cap would not bite until a pixel was more than `half`
// past the endpoint.
static float segmentDistanceButt(float2 p, float2 a, float2 b, float halfW) {
  float2 v = b - a, w = p - a;
  float len2 = dot(v, v);
  if (len2 <= 0.0) { return 1.0e6; }
  float len = sqrt(len2);
  float t = dot(w, v) / len2;
  float d = length(w - v * clamp(t, 0.0, 1.0)) - halfW;
  d = max(d, -t * len);
  return max(d, (t - 1.0) * len);
}
// plan-163-B: the SQUARE-ended twin (`__canvas_segmentDistanceSquare`, and
// `segmentDistanceSquare` in `mfb_canvas.frag`): the rectangle `half` wider than the
// segment on every side; a zero-length one is the axis-aligned square.
static float segmentDistanceSquare(float2 p, float2 a, float2 b, float halfW) {
  float2 v = b - a, w = p - a;
  float len2 = dot(v, v);
  if (len2 <= 0.0) { return max(abs(w.x), abs(w.y)) - halfW; }
  float len = sqrt(len2);
  float t = dot(w, v) / len2;
  float d = abs(w.x * v.y - w.y * v.x) / len - halfW;
  d = max(d, -t * len - halfW);
  return max(d, (t - 1.0) * len - halfW);
}
// bug-687: double-float arithmetic -- a value as an unevaluated sum `hi + lo` of two
// floats, about 48 bits. A polygon's distance is evaluated in it because fp32 is not
// the oracle's f64: where the exact distance lies within ~1e-5 px of a coverage
// quantisation boundary the two round to neighbouring steps, and near black one step
// is 3-4 sRGB levels, past GPU_DEFAULT.
//
// The error terms only exist if the compiler keeps the operations in the order written.
// Metal compiles with fast math, which may reassociate `(a - (s - v)) + (b - v)` to 0
// (measured: every TwoSum error term came back wrong), so each helper turns
// reassociation off for its own body. `fma` is exact by the MSL specification, which is
// what makes TwoProd one instruction.
static float2 dfTwoSum(float a, float b) {
#pragma clang fp reassociate(off)
  float s = a + b;
  float v = s - a;
  return float2(s, (a - (s - v)) + (b - v));
}
static float2 dfQuickSum(float a, float b) {
#pragma clang fp reassociate(off)
  float s = a + b;
  return float2(s, b - (s - a));
}
static float2 dfTwoProd(float a, float b) {
#pragma clang fp reassociate(off)
  float p = a * b;
  return float2(p, fma(a, b, -p));
}
static float2 dfAdd(float2 a, float2 b) {
#pragma clang fp reassociate(off)
  float2 s = dfTwoSum(a.x, b.x);
  float2 t = dfTwoSum(a.y, b.y);
  s = dfQuickSum(s.x, s.y + t.x);
  return dfQuickSum(s.x, s.y + t.y);
}
static float2 dfSub(float2 a, float2 b) { return dfAdd(a, -b); }
static float2 dfMul(float2 a, float2 b) {
#pragma clang fp reassociate(off)
  float2 p = dfTwoProd(a.x, b.x);
  return dfQuickSum(p.x, p.y + (a.x * b.y + a.y * b.x));
}
static float2 dfMulF(float2 a, float b) {
#pragma clang fp reassociate(off)
  float2 p = dfTwoProd(a.x, b);
  return dfQuickSum(p.x, p.y + a.y * b);
}
static float2 dfDiv(float2 a, float2 b) {
#pragma clang fp reassociate(off)
  float q1 = a.x / b.x;
  float2 r = dfSub(a, dfMulF(b, q1));
  float q2 = r.x / b.x;
  r = dfSub(r, dfMulF(b, q2));
  float q3 = r.x / b.x;
  return dfAdd(dfQuickSum(q1, q2), float2(q3, 0.0));
}
static float2 dfSqrt(float2 a) {
#pragma clang fp reassociate(off)
  if (a.x <= 0.0) { return float2(0.0, 0.0); }
  float x = rsqrt(a.x);
  float y = a.x * x;
  float2 d = dfSub(a, dfTwoProd(y, y));
  return dfTwoSum(y, d.x * (x * 0.5));
}
static bool dfLess(float2 a, float2 b) { return a.x < b.x || (a.x == b.x && a.y < b.y); }
// A 16.16 word as a double-float, exactly: the two halves are exact floats (at most 23
// and 8 significant bits) and the scale is a power of two.
static float2 dfWord(int v) { return dfTwoSum(float(v & ~255) / FIXED, float(v & 255) / FIXED); }
// A residual word (bug-687, `EDGE_RESIDUAL_SCALE`) in px: units of 2^-47.
static float residualPx(int r) { return float(r) * (1.0 / 140737488355328.0); }
// bug-735: the residual encoding for every item-block field a distance reads -- the
// 16.16 word and its residual word in the block's `*Lo` `int4`s -- as a double-float,
// and the few double-float helpers every shape kind shares.
static float2 dfFixed(int word, int residual) { return dfAdd(dfWord(word), float2(residualPx(residual), 0.0)); }
static float2 dfAbs(float2 a) { return a.x < 0.0 ? -a : a; }
static float2 dfMax(float2 a, float2 b) { return dfLess(a, b) ? b : a; }
static float2 dfMin(float2 a, float2 b) { return dfLess(b, a) ? b : a; }
static float2 dfLength(float2 x, float2 y) { return dfSqrt(dfAdd(dfMul(x, x), dfMul(y, y))); }
// `a < b` for two (word, residual) pairs, exactly: the pair is order-preserving, since
// a residual is at most half a 16.16 step.
static bool fixedLess(int wa, int ra, int wb, int rb) { return wa < wb || (wa == wb && ra < rb); }
// One vertex coordinate as the CPU had it: the 16.16 word plus its residual.
static float2 edgeCoord(constant int *edges, int i, int r) { return dfFixed(edges[i], edges[r]); }
// `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` -- the quantisation every shape uses -- for a
// double-float `d`, rounded exactly.
static int dfCoverage(float2 d) {
  float2 c = dfSub(float2(0.5, 0.0), d);
  if (!dfLess(float2(0.0, 0.0), c)) { return 0; }
  if (!dfLess(c, float2(1.0, 0.0))) { return 255; }
  float2 x = dfAdd(dfMulF(c, 255.0), float2(0.5, 0.0));
  float k = floor(x.x);
  if (x.x == k && x.y < 0.0) { k -= 1.0; }
  return int(k);
}
// plan-116-A: `base` is the polygon's first-edge index into the frame buffer's edge
// region. bug-686: `list` is where this pixel's band's edge numbers start in `edges`,
// and `count` how many there are -- or `list` is -1 and `count` the polygon's edge
// count, the loop over every edge (`polygonBand`). An edge left out of a band is one
// whose y-span misses the pixel's row (so it cannot toggle `inside`) and that is
// further than the band's reach from it (so it cannot change the clamped coverage
// `best` feeds); `min` and a parity toggle are both order-independent.
//
// bug-687: the twin of `__canvas_edgeDistance`, in double-float, at the shape-space
// point (`x`, `y`). Each endpoint is its 16.16 word plus its residual, so the polygon
// is the oracle's to ~2^-41 px; the edge vector is the integer difference of the
// words plus the residuals' difference. The crossing test is the oracle's
// `px < ax + (py - ay) / dy * dx`, multiplied out by `dy` so it needs no division.
//
// Double-float is spent only where fp32 cannot decide, and that is NOT a tolerance:
// the value returned is the double-float value, pixel for pixel. `edgeSlack` is an
// ERROR BOUND on evaluating one edge in fp32 -- the 16.16 words' conversion, the
// dropped residual (at most 2^-17 px), and the subtractions, products, division and
// square root, each a few ulps of the largest coordinate involved (a segment's
// distance is 1-Lipschitz in its endpoints). It is 2^-18 of the six coordinates'
// magnitudes, 64 ulps of each, plus 2^-12 px. So each edge's double-float distance
// lies in [d32 - slack, d32 + slack], and pass 1, in fp32 over the band, gives:
// * `bound`, the least `d32 + slack`, at least the true minimum: in pass 2 an edge
//   whose `d32 - slack` exceeds it cannot be the nearest, so only the rest -- one or
//   two -- are evaluated in double-float (a box test skips far ones without a root);
// * `low`, the least `d32 - slack`, at most the true minimum: if it is past `reach`
//   (0.5 + the stroke's half-width; unbounded under a transform, whose gradient uses
//   all five values) the coverage is 0 or 255 whatever the distance, so pass 2 is
//   skipped and `low` is returned in its place;
// * the parity: a row test is exact in fp32 unless an endpoint is within the slack
//   of the row, and the side of a crossing unless `|cross|` is within the slack
//   times `|v|1 + 2|w|1` (its sensitivity to moved endpoints; the L1 norm only
//   overstates it). Those `edgeUnsure` edges are left out of pass 1's parity and
//   decided in pass 2, in double-float.
// Each double-float helper is inlined at ONE call site: lavapipe (Mesa, LLVM)
// compiles the whole fragment shader at pipeline creation, and a version with four
// sites per call and six calls did not finish compiling in 16 minutes on box 2226.
static float edgeSlack(float2 p, float2 a, float2 b) {
  return (abs(p.x) + abs(p.y) + abs(a.x) + abs(a.y) + abs(b.x) + abs(b.y)) * (1.0 / 262144.0) + (1.0 / 4096.0);
}
static bool edgeUnsure(float2 p, float2 a, float2 b, float slack) {
  if (abs(a.y - p.y) <= slack || abs(b.y - p.y) <= slack) { return true; }
  if ((a.y > p.y) == (b.y > p.y)) { return false; }
  float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
  return abs(cross) <= slack * (abs(b.x - a.x) + abs(b.y - a.y) + 2.0 * (abs(p.x - a.x) + abs(p.y - a.y)));
}
// plan-163-A: an edge's signed crossing -- 0 when it does not cross the pixel's row
// left of the pixel, else +1 for an edge running down the screen (`b.y > a.y`) and -1
// for one running up: `__canvas_edgeDistance`'s winding convention.
static int edgeCrossing32(float2 p, float2 a, float2 b) {
  if ((a.y > p.y) == (b.y > p.y)) { return 0; }
  float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
  if (b.y > a.y) { return cross < 0.0 ? 1 : 0; }
  return cross > 0.0 ? -1 : 0;
}
// bug-735: `__canvas_rectDistance` in double-float, from the point's offset to the centre
// and the half-extents: a shape's rectangle (less its corner radius) and the clip's.
static float2 dfRectDistance(float2 dx, float2 dy, float2 hx, float2 hy) {
  float2 ox = dfSub(dfAbs(dx), hx);
  float2 oy = dfSub(dfAbs(dy), hy);
  float2 outside = dfLength(dfMax(ox, float2(0.0, 0.0)), dfMax(oy, float2(0.0, 0.0)));
  return dfAdd(outside, dfMin(dfMax(ox, oy), float2(0.0, 0.0)));
}
// bug-735: a segment's distance, its UNCLAMPED parameter `t` and `len2`, from `w = p - a`
// and `v = b - a` -- `__canvas_segmentDistance`'s arithmetic (and the butt twin's `t`),
// so a polygon edge, a line and a polyline segment share it. `t` stays 0 for a
// zero-length segment, whose distance is to its point.
struct DfSegment { float2 d; float2 t; float2 len2; };
static DfSegment dfSegment(float2 wx, float2 wy, float2 vx, float2 vy) {
  DfSegment s;
  s.len2 = dfAdd(dfMul(vx, vx), dfMul(vy, vy));
  s.t = float2(0.0, 0.0);
  float2 c = float2(0.0, 0.0);
  if (s.len2.x > 0.0) {
    s.t = dfDiv(dfAdd(dfMul(wx, vx), dfMul(wy, vy)), s.len2);
    c = s.t;
    if (dfLess(c, float2(0.0, 0.0))) { c = float2(0.0, 0.0); }
    if (dfLess(float2(1.0, 0.0), c)) { c = float2(1.0, 0.0); }
  }
  s.d = dfLength(dfSub(wx, dfMul(vx, c)), dfSub(wy, dfMul(vy, c)));
  return s;
}
struct DfEdge { float2 ax; float2 ay; float2 vx; float2 vy; float2 by; };
static DfEdge dfEdge(constant int *edges, int base, int e) {
  int i = (base + e) * 4 + METAL_EDGE_BASE;
  int r = (base + e) * 4 + METAL_EDGE_RESIDUAL_BASE;
  DfEdge g;
  g.ax = edgeCoord(edges, i, r);
  g.ay = edgeCoord(edges, i + 1, r + 1);
  // The end itself, not `ay + vy`: two edges that share a vertex must read ONE value
  // for it, or a pixel row through that vertex counts the corner twice (`dfCrossing`).
  g.by = edgeCoord(edges, i + 3, r + 3);
  g.vx = dfAdd(dfWord(edges[i + 2] - edges[i]),
               dfTwoSum(residualPx(edges[r + 2]), -residualPx(edges[r])));
  g.vy = dfAdd(dfWord(edges[i + 3] - edges[i + 1]),
               dfTwoSum(residualPx(edges[r + 3]), -residualPx(edges[r + 1])));
  return g;
}
static float2 dfSegmentDistance(DfEdge g, float2 x, float2 y) {
  return dfSegment(dfSub(x, g.ax), dfSub(y, g.ay), g.vx, g.vy).d;
}
static int dfCrossing(DfEdge g, float2 x, float2 y) {
  if (dfLess(y, g.ay) == dfLess(y, g.by)) { return 0; }
  float2 lhs = dfMul(dfSub(x, g.ax), g.vy);
  float2 rhs = dfMul(dfSub(y, g.ay), g.vx);
  if (g.vy.x > 0.0) { return dfLess(lhs, rhs) ? 1 : 0; }
  return dfLess(rhs, lhs) ? -1 : 0;
}
// plan-163-A: `nonZero` is the polygon's fill rule (`arc.y`). The crossings are summed
// as a winding number; NonZero is inside when it is not zero, EvenOdd when it is odd,
// which is the parity the toggles it replaced computed.
static float2 edgeDistance(constant int *edges, int base, int count, int list, float2 x, float2 y, float reach, int nonZero) {
  float2 p = float2(x.x, y.x);
  float bound = 1.0e6;
  float low = 1.0e6;
  bool unsure = false;
  int winding = 0;
  for (int j = 0; j < count; ++j) {
    int i = (base + (list < 0 ? j : edges[list + j])) * 4 + METAL_EDGE_BASE;
    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));
    float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));
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
  float2 best = distant ? float2(low, 0.0) : float2(1.0e6, 0.0);
  for (int j = 0; j < count && (!distant || unsure); ++j) {
    int e = list < 0 ? j : edges[list + j];
    int i = (base + e) * 4 + METAL_EDGE_BASE;
    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));
    float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));
    float slack = edgeSlack(p, a, b);
    float2 gap = max(max(min(a, b) - p, p - max(a, b)), float2(0.0, 0.0));
    float within = bound + slack;
    bool candidate = !distant && dot(gap, gap) <= within * within && segmentDistance(p, a, b) - slack <= bound;
    bool unsureEdge = unsure && edgeUnsure(p, a, b, slack);
    if (!candidate && !unsureEdge) { continue; }
    DfEdge g = dfEdge(edges, base, e);
    if (candidate) {
      float2 d = dfSegmentDistance(g, x, y);
      if (dfLess(d, best)) { best = d; }
    }
    if (unsureEdge) { winding += dfCrossing(g, x, y); }
  }
  bool inside = nonZero != 0 ? winding != 0 : (winding & 1) != 0;
  return inside ? -best : best;
}
// plan-163-B: join records (`helper_joins.rs`), riding the edge region after the item's
// own edges -- one record per outline edge, the corner at its START, as two edge slots
// (`strokeStyle.x` the first, `strokeStyle.y` the count): four points V, A, P, B as
// eight 16.16 words with their residuals. Its corner PIECE, from exact equalities:
// none when A = V, the disc of radius |A - V| about V when A = B, that disc cut by the
// chord A-B on V's side when P = V (a bevel), the disc with the polygon V, A, P, B
// otherwise (a miter). The twins of `mfb_canvas.frag`'s, line for line.
static int joinWord(constant MfbItem &item, int r, int w) { return (item.strokeStyle.x + r * 2) * 4 + w; }
static bool joinSame(constant MfbItem &item, constant int *edges, int r, int a, int b) {
  int i = joinWord(item, r, a * 2) + METAL_EDGE_BASE;
  int j = joinWord(item, r, b * 2) + METAL_EDGE_BASE;
  int ri = joinWord(item, r, a * 2) + METAL_EDGE_RESIDUAL_BASE;
  int rj = joinWord(item, r, b * 2) + METAL_EDGE_RESIDUAL_BASE;
  return edges[i] == edges[j] && edges[i + 1] == edges[j + 1]
      && edges[ri] == edges[rj] && edges[ri + 1] == edges[rj + 1];
}
static int joinKind(constant MfbItem &item, constant int *edges, int r) {
  if (joinSame(item, edges, r, 1, 0)) { return 0; }
  if (joinSame(item, edges, r, 1, 3)) { return 1; }
  return joinSame(item, edges, r, 2, 0) ? 2 : 3;
}
static float2 joinPoint32(constant MfbItem &item, constant int *edges, int r, int k) {
  int i = joinWord(item, r, k * 2) + METAL_EDGE_BASE;
  return float2(fx(edges[i]), fx(edges[i + 1]));
}
static float2 joinCoord(constant MfbItem &item, constant int *edges, int r, int w) {
  int i = joinWord(item, r, w);
  return edgeCoord(edges, i + METAL_EDGE_BASE, i + METAL_EDGE_RESIDUAL_BASE);
}
// A record's piece distance in fp32 as (least, most) the double-float one can be,
// widened by `edgeSlack`'s bound.
static float2 joinBounds32(constant MfbItem &item, constant int *edges, int r, int kind, float2 p) {
  float2 v = joinPoint32(item, edges, r, 0);
  float2 a = joinPoint32(item, edges, r, 1);
  float ds = 2.0 * edgeSlack(p, v, a);
  float disc = length(p - v) - length(a - v);
  float2 piece = float2(disc - ds, disc + ds);
  if (kind == 1) { return piece; }
  float2 b = joinPoint32(item, edges, r, 3);
  if (kind == 2) {
    float2 c = b - a;
    float cs = 2.0 * edgeSlack(p, a, b);
    float o = (c.x * (p.y - a.y) - c.y * (p.x - a.x)) / length(c);
    if (c.x * (v.y - a.y) - c.y * (v.x - a.x) > 0.0) { o = -o; }
    return float2(max(piece.x, o - cs), max(piece.y, o + cs));
  }
  float d = 1.0e6;
  float slack = 0.0;
  bool unsure = false;
  int winding = 0;
  for (int k = 0; k < 4; ++k) {
    float2 s0 = joinPoint32(item, edges, r, k);
    float2 s1 = joinPoint32(item, edges, r, (k + 1) & 3);
    float sl = edgeSlack(p, s0, s1);
    slack = max(slack, sl);
    d = min(d, segmentDistance(p, s0, s1));
    if (edgeUnsure(p, s0, s1, sl)) { unsure = true; } else { winding += edgeCrossing32(p, s0, s1); }
  }
  float2 quad = unsure ? float2(-d - slack, d + slack)
      : ((winding & 1) != 0 ? float2(-d - slack, -d + slack) : float2(d - slack, d + slack));
  return float2(min(piece.x, quad.x), min(piece.y, quad.y));
}
// The piece in double-float, for the disc and the chord; the polygon's distance `quad`
// (negative inside) comes from the caller's shared `dfSegment` loop.
static float2 dfJoinPiece(constant MfbItem &item, constant int *edges, int r, int kind, float2 x, float2 y, float2 quad) {
  float2 vx = joinCoord(item, edges, r, 0);
  float2 vy = joinCoord(item, edges, r, 1);
  float2 ax = joinCoord(item, edges, r, 2);
  float2 ay = joinCoord(item, edges, r, 3);
  float2 disc = dfSub(dfLength(dfSub(x, vx), dfSub(y, vy)), dfLength(dfSub(ax, vx), dfSub(ay, vy)));
  if (kind == 1) { return disc; }
  if (kind == 3) { return dfMin(disc, quad); }
  float2 cx = dfSub(joinCoord(item, edges, r, 6), ax);
  float2 cy = dfSub(joinCoord(item, edges, r, 7), ay);
  float2 o = dfDiv(dfSub(dfMul(cx, dfSub(y, ay)), dfMul(cy, dfSub(x, ax))), dfLength(cx, cy));
  float2 ov = dfSub(dfMul(cx, dfSub(vy, ay)), dfMul(cy, dfSub(vx, ax)));
  if (dfLess(float2(0.0, 0.0), ov)) { o = -o; }
  return dfMax(disc, o);
}
// One outline edge of a joined stroke, in fp32: a polygon's band edge `e`, or a
// rectangle's side `e` from its centre and half-extents (`__canvas_rectSides`).
static void joinedEdge32(constant MfbItem &item, constant int *edges, bool poly, int e, thread float2 &a, thread float2 &b) {
  if (poly) {
    int i = (item.arc.z + e) * 4 + METAL_EDGE_BASE;
    a = float2(fx(edges[i]), fx(edges[i + 1]));
    b = float2(fx(edges[i + 2]), fx(edges[i + 3]));
    return;
  }
  float2 c = float2(fx(item.shape.x), fx(item.shape.y));
  float2 h = float2(fx(item.shape.z), fx(item.shape.w));
  float2 corner[4] = { c - h, float2(c.x + h.x, c.y - h.y), c + h, float2(c.x - h.x, c.y + h.y) };
  a = corner[e];
  b = corner[(e + 1) & 3];
}
// The stroke distance of a polygon (its band's edges) or a rectangle or picture (its
// four sides) with join records -- `__canvas_joinedStroke`: the min of each outline
// edge's BUTT band and the record at its start, with bug-687's two passes. Every
// double-float segment goes through ONE `dfSegment` call, `k` 0 the outline edge and
// 1..4 the record's edges.
static float2 joinedStroke(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float2 half2, float reach) {
  bool poly = item.misc.x == 4;
  int count = poly ? band.y : 4;
  float2 p = float2(x.x, y.x);
  float halfW = half2.x;
  float bound = 1.0e6;
  float low = 1.0e6;
  for (int j = 0; j < count; ++j) {
    int e = poly ? (band.x < 0 ? j : edges[band.x + j]) : j;
    float2 a;
    float2 b;
    joinedEdge32(item, edges, poly, e, a, b);
    float slack = edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0);
    float d32 = segmentDistanceButt(p, a, b, halfW);
    bound = min(bound, d32 + slack);
    low = min(low, d32 - slack);
    int kind = joinKind(item, edges, e);
    if (kind != 0) {
      float2 jb = joinBounds32(item, edges, e, kind, p);
      bound = min(bound, jb.y);
      low = min(low, jb.x);
    }
  }
  if (low > reach) { return float2(low, 0.0); }
  float2 cx = dfFixed(item.shape.x, item.shapeLo.x);
  float2 cy = dfFixed(item.shape.y, item.shapeLo.y);
  float2 hx = dfFixed(item.shape.z, item.shapeLo.z);
  float2 hy = dfFixed(item.shape.w, item.shapeLo.w);
  float2 best = float2(1.0e6, 0.0);
  for (int j = 0; j < count; ++j) {
    int e = poly ? (band.x < 0 ? j : edges[band.x + j]) : j;
    float2 a;
    float2 b;
    joinedEdge32(item, edges, poly, e, a, b);
    float slack = edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0);
    bool edgeNear = segmentDistanceButt(p, a, b, halfW) - slack <= bound;
    int kind = joinKind(item, edges, e);
    bool joinNear = kind != 0 && joinBounds32(item, edges, e, kind, p).x <= bound;
    if (!edgeNear && !joinNear) { continue; }
    float2 jd = float2(1.0e6, 0.0);
    int winding = 0;
    for (int k = edgeNear ? 0 : 1; k < (joinNear && kind == 3 ? 5 : 1); ++k) {
      DfEdge g;
      if (k == 0 && poly) {
        g = dfEdge(edges, item.arc.z, e);
      } else if (k == 0) {
        int s0 = e;
        int s1 = (e + 1) & 3;
        float2 ax = (s0 == 0 || s0 == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
        float2 ay = (s0 < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
        float2 bx = (s1 == 0 || s1 == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
        float2 by = (s1 < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
        g.ax = ax;
        g.ay = ay;
        g.vx = dfSub(bx, ax);
        g.vy = dfSub(by, ay);
        g.by = by;
      } else {
        g.ax = joinCoord(item, edges, e, (k - 1) * 2);
        g.ay = joinCoord(item, edges, e, (k - 1) * 2 + 1);
        g.vx = dfSub(joinCoord(item, edges, e, (k & 3) * 2), g.ax);
        g.by = joinCoord(item, edges, e, (k & 3) * 2 + 1);
        g.vy = dfSub(g.by, g.ay);
      }
      DfSegment s = dfSegment(dfSub(x, g.ax), dfSub(y, g.ay), g.vx, g.vy);
      if (k == 0) {
        if (s.len2.x > 0.0) {
          float2 len = dfSqrt(s.len2);
          float2 d = dfMax(dfSub(s.d, half2), -dfMul(s.t, len));
          d = dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len));
          if (dfLess(d, best)) { best = d; }
        }
      } else {
        if (dfLess(s.d, jd)) { jd = s.d; }
        winding += dfCrossing(g, x, y);
      }
    }
    if (joinNear) {
      if ((winding & 1) != 0) { jd = -jd; }
      float2 piece = dfJoinPiece(item, edges, e, kind, x, y, jd);
      if (dfLess(piece, best)) { best = piece; }
    }
  }
  return best;
}
// plan-116-C: the inverse transform, decoded from the float32 bits the item block
// carries. `as_type<float>` is a reinterpret, not a conversion -- the CPU already
// did the narrowing (`__canvas_float32Bits`), because this compiler's assemblers
// have no double->single convert.
static bool hasTransform(constant MfbItem &item) { return item.xform1.z != 0; }
// bug-686: the edge list a polygon's shape-space point at height `y` visits, as
// (first word in `edges`, length), or (-1, edgeCount) for "every edge" -- any other
// kind, or a polygon the emitter gave no band index (`arcCaps.z` = 0). The band
// header is `arcCaps` = (top, height in 16.16, band count, table offset), written by
// `emit_band_index`; the table is (first, count) per band and the list follows it.
// A point above the first band or below the last uses the end band, which the
// emitter built to hold every edge that can matter there.
//
// bug-1085: `norm` is the pixel's composed inverse norm (0 for a pixel that takes
// its centre sample alone), and the band is taken only when its reach covers it:
// `0.75 norm <= F`, `F` the norm the reach was built for (`capsLo.w`,
// `ITEM_BAND_NORM`). A block is built once a pass and drawn under any group's
// matrix, so past that the edges a neighbour sample needs may be in no band of it,
// and every edge is walked instead -- exact, as with no index.
static int2 polygonBand(constant MfbItem &item, constant int *edges, float y, float norm) {
  int nb = item.arcCaps.z;
  // plan-164-C: kinds 11 and 12 too -- their pieces are banded as edges.
  int bk = item.misc.x;
  if ((bk != 4 && bk != 10 && bk != 11 && bk != 12) || nb <= 0) { return int2(-1, item.misc.w); }
  if (!(norm * 0.75 <= fx(item.capsLo.w))) { return int2(-1, item.misc.w); }
  float k = floor((y - fx(item.arcCaps.x)) / fx(item.arcCaps.y));
  int band = int(clamp(k, 0.0, float(nb - 1)));
  int t = METAL_BAND_BASE + item.arcCaps.w;
  return int2(t + nb * 2 + edges[t + band * 2], edges[t + band * 2 + 1]);
}
// plan-154-B: a PolyLine's distance and path parameter, as (d, t) -- the twin of
// `__canvas_polylineScan` in the oracle and of `polylineScan` in
// `runtime/canvas/shaders/mfb_canvas.frag`. The minimum over the segments' round
// bands is the union that makes every corner round; only the first and last LIVE
// segments (`gradient.x`/`.y`, whole numbers) take a butt plane, each on its outer
// end. A zero-length segment is skipped unless it is the whole path (first == last),
// where it is the round dot. `t` is (segment + clamped t) / segments, continuous at a
// corner, and the same walk sets both, so the colour's segment is the distance's.
// plan-163-C: a path clip's band, the clip's twin of `polygonBand`: its band header
// in `clipBand`, or every clip edge (band count 0) -- and, bug-1085, every clip edge
// for a pixel whose composed norm the band's reach does not cover (`gradFocal.w`,
// `ITEM_CLIP_BAND_NORM`).
static int2 clipBandAt(constant MfbItem &item, constant int *edges, float y, float norm) {
  int nb = item.clipBand.z;
  if (nb <= 0) { return int2(-1, item.clipPath.y); }
  if (!(norm * 0.75 <= fx(item.gradFocal.w))) { return int2(-1, item.clipPath.y); }
  float k = floor((y - fx(item.clipBand.x)) / fx(item.clipBand.y));
  int band = int(clamp(k, 0.0, float(nb - 1)));
  int t = METAL_BAND_BASE + item.clipBand.w;
  return int2(t + nb * 2 + edges[t + band * 2], edges[t + band * 2 + 1]);
}
static float2 polylineScan(constant MfbItem &item, constant int *edges, int2 band, float2 p) {
  int first = item.gradient.x;
  int last = item.gradient.y;
  float halfW = fx(item.misc.y);
  bool butt = item.arc.w == 0;
  bool square = item.arc.w == 2;
  float best = 1.0e6;
  float param = 0.0;
  for (int j = 0; j < band.y; ++j) {
    int e = band.x < 0 ? j : edges[band.x + j];
    if (e < first || e > last) { continue; }
    int i = (item.arc.z + e) * 4 + METAL_EDGE_BASE;
    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));
    float2 v = float2(fx(edges[i + 2]), fx(edges[i + 3])) - a;
    float len2 = dot(v, v);
    if (!(len2 > 0.0) && first != last) { continue; }
    float2 w = p - a;
    float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;
    float c = clamp(t, 0.0, 1.0);
    float at = c;
    if (square && ((e == first && t < 0.0) || (e == last && t > 1.0))) { at = t; }
    float d = length(w - v * at) - halfW;
    if (butt) {
      float len = sqrt(len2);
      if (e == first) { d = max(d, -t * len); }
      if (e == last) { d = max(d, (t - 1.0) * len); }
    }
    if (square) {
      if (len2 > 0.0) {
        float len = sqrt(len2);
        if (e == first) { d = max(d, -t * len - halfW); }
        if (e == last) { d = max(d, (t - 1.0) * len - halfW); }
      } else {
        d = max(abs(w.x), abs(w.y)) - halfW;
      }
    }
    if (d < best) { best = d; param = (float(e) + c) / float(item.misc.w); }
  }
  return float2(best, param);
}
static float2 inverseMap(constant MfbItem &item, float2 p) {
  return float2(as_type<float>(item.xform0.x) * p.x + as_type<float>(item.xform0.z) * p.y + as_type<float>(item.xform1.x),
                as_type<float>(item.xform0.y) * p.x + as_type<float>(item.xform0.w) * p.y + as_type<float>(item.xform1.y));
}
// bug-735: every shape kind but the polygon and the glyph, in double-float -- the twin of
// `__canvas_geoDistance` in the oracle, operation for operation, on inputs carried as
// their 16.16 words plus residuals (`ITEM_OFFSET_SHAPE_LO` and the rest), so each input
// is the oracle's to ~2^-41 px and the arithmetic to ~2^-44 of its magnitude. fp32 from
// 16.16 missed the oracle by up to ~1e-4 px and so rounded a pixel near a coverage step to
// the neighbouring step: 3-12 sRGB levels near black.
//
// * rectangle (0), picture (9): the rectangle distance minus the corner radius;
// * circle (1): the centre distance minus the radius and the stroke radius;
// * line (2): the segment distance, Round (1) minus the half-width, Butt (0) cut by the two
//   end planes -- the half-width off BEFORE the planes (`__canvas_segmentDistanceButt`);
// * arc (3): the ring band cut by the sweep -- `max` with the two end radii's signed
//   cone distance (the reflex complement of the gap's) -- and a Round cap's two discs
//   or a Square cap's two boxes (bug-1088). The planes are the oracle's two cross
//   products, against `arcCaps - centre`, the radius times the oracle's Taylor
//   `(cos, sin)`, divided by the radius, so no GPU trigonometry is involved; `arc.y` is
//   the oracle's `turn` (`__canvas_arcTurn`), decided in f64 by the emitter;
// * ellipse (7): the oracle's 24-step bisection, every step in double-float. In fp32 a
//   step's sign test took the other branch wherever `g` was within fp32's error of zero
//   -- every late step, on every pixel -- which is what put the ellipse 8 pixels past
//   GPU_DEFAULT. The circle case (equal radii) is the oracle's too.
static float2 dfShapeDistance(constant MfbItem &item, float2 x, float2 y) {
  int kind = item.misc.x;
  float2 cx = dfFixed(item.shape.x, item.shapeLo.x);
  float2 cy = dfFixed(item.shape.y, item.shapeLo.y);
  float2 ex = dfFixed(item.shape.z, item.shapeLo.z);
  float2 ey = dfFixed(item.shape.w, item.shapeLo.w);
  float2 radius = dfFixed(item.misc.y, item.miscLo.x);
  float2 dx = dfSub(x, cx);
  float2 dy = dfSub(y, cy);
  if (kind == 0 || kind == 9 || kind == 13) {
    return dfSub(dfRectDistance(dx, dy, ex, ey), radius);
  }
  if (kind == 1) { return dfSub(dfSub(dfLength(dx, dy), ex), radius); }
  if (kind == 2) {
    DfSegment s = dfSegment(dx, dy, dfSub(ex, cx), dfSub(ey, cy));
    // plan-163-B: Square -- `__canvas_segmentDistanceSquare`.
    if (item.arc.w == 1) { return dfSub(s.d, radius); }
    if (item.arc.w == 2) {
      if (!(s.len2.x > 0.0)) { return dfSub(dfMax(dfAbs(dx), dfAbs(dy)), radius); }
      float2 sl = dfSqrt(s.len2);
      float2 perp = dfDiv(dfAbs(dfSub(dfMul(dx, dfSub(ey, cy)), dfMul(dy, dfSub(ex, cx)))), sl);
      float2 sd = dfMax(dfSub(perp, radius), dfSub(-dfMul(s.t, sl), radius));
      return dfMax(sd, dfSub(dfMul(dfSub(s.t, float2(1.0, 0.0)), sl), radius));
    }
    if (!(s.len2.x > 0.0)) { return float2(1.0e6, 0.0); }
    float2 len = dfSqrt(s.len2);
    float2 d = dfMax(dfSub(s.d, radius), -dfMul(s.t, len));
    return dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len));
  }
  if (kind == 3) {
    float2 sx = dfSub(dfFixed(item.arcCaps.x, item.capsLo.x), cx);
    float2 sy = dfSub(dfFixed(item.arcCaps.y, item.capsLo.y), cy);
    float2 tx = dfSub(dfFixed(item.arcCaps.z, item.capsLo.z), cx);
    float2 ty = dfSub(dfFixed(item.arcCaps.w, item.capsLo.w), cy);
    // bug-1088: the band cut by the sweep's wedge, the oracle's exact cone distance:
    // inside both end half-planes and ahead of the centre along a radius, the nearer end
    // line's distance, else the nearer end ray's; the reflex complement of the gap's cone;
    // a whole turn (`arc.y` 2) the band alone. `s`, `t` are the radius times the oracle's
    // unit vectors, so they are divided by it first.
    float2 band = dfSub(dfAbs(dfSub(dfLength(dx, dy), ex)), radius);
    if (item.arc.y != 2) {
      float2 ux = dfDiv(sx, ex);
      float2 uy = dfDiv(sy, ex);
      float2 vx = dfDiv(tx, ex);
      float2 vy = dfDiv(ty, ex);
      float2 startPlane = dfSub(dfMul(uy, dx), dfMul(ux, dy));
      float2 endPlane = dfSub(dfMul(vx, dy), dfMul(vy, dx));
      float2 alongS = dfAdd(dfMul(dx, ux), dfMul(dy, uy));
      float2 alongE = dfAdd(dfMul(dx, vx), dfMul(dy, vy));
      float2 onS = dfMax(alongS, float2(0.0, 0.0));
      float2 onE = dfMax(alongE, float2(0.0, 0.0));
      float2 rays = dfMin(dfLength(dfSub(dx, dfMul(onS, ux)), dfSub(dy, dfMul(onS, uy))),
                          dfLength(dfSub(dx, dfMul(onE, vx)), dfSub(dy, dfMul(onE, vy))));
      bool ahead = !dfLess(alongS, float2(0.0, 0.0)) || !dfLess(alongE, float2(0.0, 0.0));
      float2 sweep = rays;
      if (item.arc.y == 1) {
        sweep = -rays;
        if (!dfLess(startPlane, float2(0.0, 0.0)) && !dfLess(endPlane, float2(0.0, 0.0)) && ahead) { sweep = dfMin(startPlane, endPlane); }
      } else if (!dfLess(float2(0.0, 0.0), startPlane) && !dfLess(float2(0.0, 0.0), endPlane) && ahead) {
        sweep = dfMax(startPlane, endPlane);
      }
      band = dfMax(band, sweep);
    }
    // plan-163-B: a Square end is `__canvas_arcSquareCap` -- the half-width box past the
    // end along the tangent, `along` measured outward from the end.
    if (item.arc.w == 0) { return band; }
    for (int k = 0; k < 2; ++k) {
      float2 rx = k == 0 ? sx : tx;
      float2 ry = k == 0 ? sy : ty;
      float2 ox = dfSub(dx, rx);
      float2 oy = dfSub(dy, ry);
      if (item.arc.w == 2) {
        float2 rl = dfLength(rx, ry);
        if (!(rl.x > 0.0)) { continue; }
        float2 ux = dfDiv(rx, rl);
        float2 uy = dfDiv(ry, rl);
        float2 along = dfSub(dfMul(ox, uy), dfMul(oy, ux));
        if (k == 1) { along = -along; }
        float2 across = dfAdd(dfMul(ox, ux), dfMul(oy, uy));
        float2 box = dfMax(dfMax(dfSub(dfAbs(across), radius), dfSub(along, radius)), -along);
        band = dfMin(band, box);
      } else {
        band = dfMin(band, dfSub(dfLength(ox, oy), radius));
      }
    }
    return band;
  }
  if (kind == 7) {
    float2 ca = dfFixed(item.ellipse.x, item.miscLo.z);
    float2 sa = dfFixed(item.ellipse.y, item.miscLo.w);
    float2 qx = dfAbs(dfAdd(dfMul(dx, ca), dfMul(dy, sa)));
    float2 qy = dfAbs(dfSub(dfMul(dy, ca), dfMul(dx, sa)));
    if (item.shape.z == item.shape.w && item.shapeLo.z == item.shapeLo.w) {
      return dfSub(dfSub(dfLength(qx, qy), ex), radius);
    }
    float2 c0 = float2(1.0, 0.0);
    float2 s0 = float2(0.0, 0.0);
    float2 c1 = float2(0.0, 0.0);
    float2 s1 = float2(1.0, 0.0);
    float2 cm = c0;
    float2 sm = s0;
    for (int k = 0; k < 24; ++k) {
      float2 cs = dfAdd(c0, c1);
      float2 ss = dfAdd(s0, s1);
      float2 nn = dfLength(cs, ss);
      cm = dfDiv(cs, nn);
      sm = dfDiv(ss, nn);
      float2 g = dfAdd(dfMul(dfSub(qx, dfMul(ex, cm)), -dfMul(ex, sm)),
                       dfMul(dfSub(qy, dfMul(ey, sm)), dfMul(ey, cm)));
      if (dfLess(float2(0.0, 0.0), g)) { c0 = cm; s0 = sm; } else { c1 = cm; s1 = sm; }
    }
    float2 d = dfLength(dfSub(qx, dfMul(ex, cm)), dfSub(qy, dfMul(ey, sm)));
    float2 ux = dfDiv(qx, ex);
    float2 uy = dfDiv(qy, ey);
    if (dfLess(dfAdd(dfMul(ux, ux), dfMul(uy, uy)), float2(1.0, 0.0))) { d = -d; }
    return dfSub(d, radius);
  }
  return float2(1.0e6, 0.0);
}
// bug-735: the fp32 pre-pass for the same kinds -- a value `v` such that, whenever
// `|v| > reach`, the distance has `v`'s sign and at least its magnitude, so its fill and
// stroke coverage are `v`'s and double-float is not needed. Like `edgeSlack`, an ERROR
// BOUND, not a tolerance: the value returned for a far pixel is past the clamp either
// way, and every other pixel gets the double-float value.
//
// * rectangle, circle, line: the fp32 distance, moved toward zero by `slack` -- 2^-18 of
//   the coordinates' magnitudes plus 2^-12 px, the bound `edgeSlack` uses (each is
//   1-Lipschitz in its inputs, which the 16.16 words and the residual move by <= 2^-17);
// * arc: a lower bound only, `min(ring band, cap discs)` with the sweep ignored -- the
//   oracle's band is `max(ring band, sweep planes)` (bug-1088), never less than the ring;
// * ellipse: `rmin * |s - 1|` for `s = |(qx / rx, qy / ry)|`, a RIGOROUS bound on the
//   distance to the ellipse (the ellipse and its scale by `s` are `rmin * |s - 1|` apart:
//   map by `diag(1 / rx, 1 / ry)`, whose inverse stretches no vector by more than
//   `rmin`), with a slack of 2^-14 of the magnitudes for the 16.16 inputs; the oracle's
//   bisection lands on the ellipse, so its distance is never below the true one;
// * a butt line shorter than fp32 resolves, and any other kind: no shortcut or 1e6.
static float farBound(constant MfbItem &item, float2 p) {
  int kind = item.misc.x;
  float2 c = float2(fx(item.shape.x), fx(item.shape.y));
  float2 e = float2(fx(item.shape.z), fx(item.shape.w));
  float r = fx(item.misc.y);
  float slack = (abs(p.x) + abs(p.y) + abs(c.x) + abs(c.y) + abs(e.x) + abs(e.y) + abs(r)) * (1.0 / 262144.0) + (1.0 / 4096.0);
  if (kind == 7) {
    float2 d = p - c;
    float ca = fx(item.ellipse.x);
    float sa = fx(item.ellipse.y);
    float s = length(abs(float2(d.x * ca + d.y * sa, -d.x * sa + d.y * ca)) / e);
    float rmin = min(e.x, e.y);
    float sl = (abs(d.x) + abs(d.y) + e.x + e.y + 1.0) * (1.0 / 16384.0) + (1.0 / 4096.0);
    return s >= 1.0 ? rmin * (s - 1.0) - r - sl : rmin * (s - 1.0) - r + sl;
  }
  if (kind == 3) {
    float2 cs = float2(fx(item.arcCaps.x), fx(item.arcCaps.y));
    float2 ce = float2(fx(item.arcCaps.z), fx(item.arcCaps.w));
    float m = abs(length(p - c) - e.x) - r;
    // plan-163-B: a Square end lies within `r * sqrt(2)` of its endpoint (1.4143 > sqrt 2,
    // so this stays a lower bound).
    float capR = item.arc.w == 2 ? r * 1.4143 : r;
    if (item.arc.w != 0) { m = min(m, min(length(p - cs), length(p - ce)) - capR); }
    slack += (abs(cs.x) + abs(cs.y) + abs(ce.x) + abs(ce.y)) * (1.0 / 262144.0);
    return max(m - slack, 0.0);
  }
  float d;
  if (kind == 0 || kind == 9 || kind == 13) {
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
// bug-735: a PolyLine's distance in double-float -- `__canvas_polylineScan`'s, with
// bug-687's two passes. Pass 1 is fp32 over the band: `bound` (the least `d32 + slack`)
// and `low` (the least `d32 - slack`). Past `reach` it returns `low`; otherwise pass 2
// evaluates in double-float only the segments that can be the nearest
// (`d32 - slack <= bound`), each endpoint its 16.16 word plus its residual. A segment is
// live when its endpoints' words or residuals differ -- the oracle's `len2 > 0`, exactly
// -- or it is the whole path (`first == last`); one shorter than fp32 resolves gets an
// unbounded slack, so it is always a candidate and never tightens `bound`.
static float polylineSegment32(constant MfbItem &item, constant int *edges, int e, float2 p, float halfW, thread float &slack) {
  int first = item.gradient.x;
  int last = item.gradient.y;
  int i = (item.arc.z + e) * 4 + METAL_EDGE_BASE;
  int ri = (item.arc.z + e) * 4 + METAL_EDGE_RESIDUAL_BASE;
  bool live = first == last || edges[i] != edges[i + 2] || edges[i + 1] != edges[i + 3]
      || edges[ri] != edges[ri + 2] || edges[ri + 1] != edges[ri + 3];
  if (!live) { slack = -1.0; return 1.0e6; }
  float2 a = float2(fx(edges[i]), fx(edges[i + 1]));
  float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));
  float2 v = b - a;
  float len2 = dot(v, v);
  float2 w = p - a;
  float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;
  float at = clamp(t, 0.0, 1.0);
  bool square = item.arc.w == 2;
  if (square && ((e == first && t < 0.0) || (e == last && t > 1.0))) { at = t; }
  float d = length(w - v * at) - halfW;
  if (item.arc.w == 0 && len2 > 0.0) {
    float len = sqrt(len2);
    if (e == first) { d = max(d, -t * len); }
    if (e == last) { d = max(d, (t - 1.0) * len); }
  }
  if (square) {
    if (len2 > 0.0) {
      float len = sqrt(len2);
      if (e == first) { d = max(d, -t * len - halfW); }
      if (e == last) { d = max(d, (t - 1.0) * len - halfW); }
    } else {
      d = max(abs(w.x), abs(w.y)) - halfW;
    }
  }
  // plan-163-B: with join records the inner ends are cut butt; the corners are the
  // records.
  if (item.strokeStyle.y > 0 && len2 > 0.0) {
    float len = sqrt(len2);
    if (e != first) { d = max(d, -t * len); }
    if (e != last) { d = max(d, (t - 1.0) * len); }
  }
  slack = len2 > 0.0 ? edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0) : 1.0e30;
  return d;
}
static float2 polylineDistance(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float reach) {
  int first = item.gradient.x;
  int last = item.gradient.y;
  bool joined = item.strokeStyle.y > 0;
  float halfW = fx(item.misc.y);
  float2 p = float2(x.x, y.x);
  float bound = 1.0e6;
  float low = 1.0e6;
  for (int j = 0; j < band.y; ++j) {
    int e = band.x < 0 ? j : edges[band.x + j];
    if (e < first || e > last) { continue; }
    float slack;
    float d32 = polylineSegment32(item, edges, e, p, halfW, slack);
    int kind = joined ? joinKind(item, edges, e) : 0;
    if (kind != 0) {
      float2 jb = joinBounds32(item, edges, e, kind, p);
      bound = min(bound, jb.y);
      low = min(low, jb.x);
    }
    if (slack < 0.0) { continue; }
    bound = min(bound, d32 + slack);
    low = min(low, d32 - slack);
  }
  if (low > reach) { return float2(low, 0.0); }
  float2 half2 = dfFixed(item.misc.y, item.miscLo.x);
  float2 best = float2(1.0e6, 0.0);
  for (int j = 0; j < band.y; ++j) {
    int e = band.x < 0 ? j : edges[band.x + j];
    if (e < first || e > last) { continue; }
    float slack;
    float d32 = polylineSegment32(item, edges, e, p, halfW, slack);
    bool segNear = slack >= 0.0 && d32 - slack <= bound;
    int kind = joined ? joinKind(item, edges, e) : 0;
    bool joinNear = kind != 0 && joinBounds32(item, edges, e, kind, p).x <= bound;
    if (!segNear && !joinNear) { continue; }
    // plan-163-B: `k` 0 the segment, 1..4 the edges of the join record at its start,
    // through one `dfSegment` call.
    float2 jd = float2(1.0e6, 0.0);
    int winding = 0;
    for (int k = segNear ? 0 : 1; k < (joinNear && kind == 3 ? 5 : 1); ++k) {
    DfEdge g;
    if (k == 0) {
      g = dfEdge(edges, item.arc.z, e);
    } else {
      g.ax = joinCoord(item, edges, e, (k - 1) * 2);
      g.ay = joinCoord(item, edges, e, (k - 1) * 2 + 1);
      g.vx = dfSub(joinCoord(item, edges, e, (k & 3) * 2), g.ax);
      g.by = joinCoord(item, edges, e, (k & 3) * 2 + 1);
      g.vy = dfSub(g.by, g.ay);
    }
    float2 wx = dfSub(x, g.ax);
    float2 wy = dfSub(y, g.ay);
    DfSegment s = dfSegment(wx, wy, g.vx, g.vy);
    if (k > 0) {
      if (dfLess(s.d, jd)) { jd = s.d; }
      winding += dfCrossing(g, x, y);
      continue;
    }
    float2 d = dfSub(s.d, half2);
    if (item.arc.w == 0 && s.len2.x > 0.0) {
      float2 len = dfSqrt(s.len2);
      if (e == first) { d = dfMax(d, -dfMul(s.t, len)); }
      if (e == last) { d = dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len)); }
    }
    // plan-163-B: a square OUTER end -- to the line past the endpoint, then the end plane
    // moved out by the half-width; a lone zero-length segment is the square.
    if (item.arc.w == 2) {
      if (s.len2.x > 0.0) {
        float2 len = dfSqrt(s.len2);
        bool before = e == first && dfLess(s.t, float2(0.0, 0.0));
        bool after = e == last && dfLess(float2(1.0, 0.0), s.t);
        if (before || after) {
          d = dfSub(dfDiv(dfAbs(dfSub(dfMul(wx, g.vy), dfMul(wy, g.vx))), len), half2);
        }
        if (e == first) { d = dfMax(d, dfSub(-dfMul(s.t, len), half2)); }
        if (e == last) { d = dfMax(d, dfSub(dfMul(dfSub(s.t, float2(1.0, 0.0)), len), half2)); }
      } else {
        d = dfSub(dfMax(dfAbs(wx), dfAbs(wy)), half2);
      }
    }
    if (joined && s.len2.x > 0.0) {
      float2 len = dfSqrt(s.len2);
      if (e != first) { d = dfMax(d, -dfMul(s.t, len)); }
      if (e != last) { d = dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len)); }
    }
    if (dfLess(d, best)) { best = d; }
    }
    if (joinNear) {
      if ((winding & 1) != 0) { jd = -jd; }
      float2 piece = dfJoinPiece(item, edges, e, kind, x, y, jd);
      if (dfLess(piece, best)) { best = piece; }
    }
  }
  return best;
}
// bug-735: one sample's distance at the double-float shape-space point, for every kind a
// distance draws: the polygon's edge walk (bug-687), the polyline's, or the pre-pass and
// then `dfShapeDistance`. One call site each; `shapeCoverage` calls this from two.
// plan-163-B: dashes (`helper_dashes.rs`). An item's dash words ride the dash region from
// `strokeStyle.z`, `strokeStyle.w` of them (0 for a solid stroke), as float32 bit
// patterns: the phase, the entry count, the period, each entry's END along the period,
// then the kind's perimeter data -- three words per outline edge (its start and end
// position and its ring's length, negative for an open path), an ellipse's arc-length
// table, an arc's start angle and sweep. Each function is the oracle's in fp32, at
// offsets from the item's own outline taken in double-float, so the precision is the
// outline's, not the coordinates'. Metal's twins, line for line.
static float dashWord(constant MfbItem &item, constant int *edges, int k) { return as_type<float>(edges[METAL_DASH_BASE + item.strokeStyle.z + k]); }
static float dfF(float2 v) { return v.x + v.y; }
static float dashWrap(constant MfbItem &item, constant int *edges, float x) {
  float period = dashWord(item, edges, 2);
  return x - floor(x / period) * period;
}
static bool dashOnAt(constant MfbItem &item, constant int *edges, float x) {
  float w = dashWrap(item, edges, x);
  int n = int(dashWord(item, edges, 1));
  for (int j = 0; j < n; ++j) {
    if (w < dashWord(item, edges, 3 + j)) { return (j & 1) == 0; }
  }
  return true;
}
static bool dashOnBefore(constant MfbItem &item, constant int *edges, float x) {
  float w = dashWrap(item, edges, x);
  if (w <= 0.0) { w = dashWord(item, edges, 2); }
  int n = int(dashWord(item, edges, 1));
  for (int j = 0; j < n; ++j) {
    if (w <= dashWord(item, edges, 3 + j)) { return (j & 1) == 0; }
  }
  return false;
}
// `__canvas_dashLocate` then `__canvas_dashAlongPiece`: the signed distance along the
// outline from `u` to the dashes on the piece [s0, s1] (negative inside one), a dash
// extended by `ext` past a TRUE end; those ends near `u` into `ends`, flagged by `has`.
static float dashAlong(constant MfbItem &item, constant int *edges, float u, float s0, float s1, float len_, bool closed, float ext, thread float2 &ends, thread bool2 &has) {
  float at = min(max(u, s0), s1);
  float phase = dashWord(item, edges, 0);
  float period = dashWord(item, edges, 2);
  int n = int(dashWord(item, edges, 1));
  float w = dashWrap(item, edges, at + phase);
  float lowW = 0.0;
  float highW = period;
  int j = 0;
  bool found = false;
  for (; j < n; ++j) {
    float c = dashWord(item, edges, 3 + j);
    if (w < c) { highW = c; found = true; break; }
    lowW = c;
  }
  bool on = (j & 1) == 0;
  if (!found) { on = true; lowW = period; highW = period + dashWord(item, edges, 3); }
  float lo = at - (w - lowW);
  float hi = at + (highW - w);
  if (closed) {
    bool onStart = dashOnAt(item, edges, phase);
    bool onEnd = dashOnBefore(item, edges, len_ + phase);
    if (lo < 0.0) { lo = on == onEnd ? -1.0e9 : 0.0; }
    if (hi > len_) { hi = on == onStart ? 1.0e9 : len_; }
  } else {
    if (lo < 0.0) { lo = on ? 0.0 : -1.0e9; }
    if (hi > len_) { hi = on ? len_ : 1.0e9; }
  }
  ends = float2(0.0);
  has = bool2(false);
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
static float dashAtan(float y, float x) {
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
static float dashWrapAngle(float a) { return a - 2.0 * PI * floor((a + PI) / (2.0 * PI)); }
static float dashFold(float a) {
  float x = dashWrapAngle(a);
  if (x > PI / 2.0) { return PI - x; }
  if (x < -PI / 2.0) { return -PI - x; }
  return x;
}
static float dashSin(float a) {
  float x = dashFold(a);
  float x2 = x * x;
  return x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))));
}
static float dashCos(float a) {
  float x = dashFold(a);
  float x2 = x * x;
  float m = 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0 * (1.0 - x2 / 90.0))));
  return abs(dashWrapAngle(a)) > PI / 2.0 ? -m : m;
}
// The direction of `q` in [0, 2pi), from +X towards +Y.
static float dashAngle(float2 q) { return dashAtan(q.y, q.x); }
// A join record's corner piece at `w`, the pixel less its vertex V, from its points
// relative to V (`__canvas_joinPieceDistance`).
static float2 joinRel(constant MfbItem &item, constant int *edges, int r, int k) {
  return float2(dfF(dfSub(joinCoord(item, edges, r, k * 2), joinCoord(item, edges, r, 0))),
        dfF(dfSub(joinCoord(item, edges, r, k * 2 + 1), joinCoord(item, edges, r, 1))));
}
static float joinPieceAt(constant MfbItem &item, constant int *edges, int r, int kind, float2 w) {
  float2 a = joinRel(item, edges, r, 1);
  float disc = length(w) - length(a);
  if (kind == 1) { return disc; }
  float2 b = joinRel(item, edges, r, 3);
  if (kind == 2) {
    float2 c = b - a;
    float o = (c.x * (w.y - a.y) - c.y * (w.x - a.x)) / length(c);
    if (c.x * (-a.y) - c.y * (-a.x) > 0.0) { o = -o; }
    return max(disc, o);
  }
  float2 pts[4] = { float2(0.0), a, joinRel(item, edges, r, 2), b };
  float d = 1.0e6;
  bool inside = false;
  for (int k = 0; k < 4; ++k) {
    float2 s0 = pts[k];
    float2 s1 = pts[(k + 1) & 3];
    d = min(d, segmentDistance(w, s0, s1));
    if ((s0.y > w.y) != (s1.y > w.y) && w.x < s0.x + (w.y - s0.y) / (s1.y - s0.y) * (s1.x - s0.x)) {
      inside = !inside;
    }
  }
  return min(disc, inside ? -d : d);
}
// plan-164-C: the curve shapes' functions, generated from `shaders/curves.tpl`
// (`runtime/canvas/curve_shader.rs`) and spliced in here when the source is built.
@CURVE_SHADER@
// `__canvas_dashedEdges`: a polygon's or polyline's band edges, a line's one edge, or a
// rectangle's or picture's four sides, each band cut by the dashes on its own [s0, s1],
// with the corner piece wherever a dash runs through a vertex. An open path with no
// length is the dot its cap draws (`__canvas_dashDot`).
static float dashedEdges(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float halfW, int cap) {
  int kind = item.misc.x;
  bool poly = kind == 4 || kind == 10;
  int count = poly ? band.y : (kind == 2 ? 1 : 4);
  int data = 3 + int(dashWord(item, edges, 1));
  float phase = dashWord(item, edges, 0);
  float ext = cap == 2 ? halfW : 0.0;
  float2 cx = dfFixed(item.shape.x, item.shapeLo.x);
  float2 cy = dfFixed(item.shape.y, item.shapeLo.y);
  float2 hx = dfFixed(item.shape.z, item.shapeLo.z);
  float2 hy = dfFixed(item.shape.w, item.shapeLo.w);
  if ((kind == 2 || kind == 10) && dashWord(item, edges, data + 2) == 0.0) {
    float2 ox = cx;
    float2 oy = cy;
    if (kind == 10) {
      DfEdge g0 = dfEdge(edges, item.arc.z, 0);
      ox = g0.ax;
      oy = g0.ay;
    }
    float2 w0 = float2(dfF(dfSub(x, ox)), dfF(dfSub(y, oy)));
    if (cap == 0 || !dashOnAt(item, edges, phase)) { return 1.0e6; }
    if (cap == 2) { return max(abs(w0.x), abs(w0.y)) - halfW; }
    return length(w0) - halfW;
  }
  float best = 1.0e6;
  for (int j = 0; j < count; ++j) {
    int e = poly ? (band.x < 0 ? j : edges[band.x + j]) : j;
    float2 ax;
    float2 ay;
    float2 v;
    if (poly) {
      DfEdge g = dfEdge(edges, item.arc.z, e);
      ax = g.ax;
      ay = g.ay;
      v = float2(dfF(g.vx), dfF(g.vy));
    } else if (kind == 2) {
      ax = cx;
      ay = cy;
      v = float2(dfF(dfSub(hx, cx)), dfF(dfSub(hy, cy)));
    } else {
      int f = (e + 1) & 3;
      ax = (e == 0 || e == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
      ay = (e < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
      float2 bx = (f == 0 || f == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);
      float2 by = (f < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);
      v = float2(dfF(dfSub(bx, ax)), dfF(dfSub(by, ay)));
    }
    float len2 = dot(v, v);
    if (!(len2 > 0.0)) { continue; }
    float2 w = float2(dfF(dfSub(x, ax)), dfF(dfSub(y, ay)));
    float s0 = dashWord(item, edges, data + e * 3);
    float s1 = dashWord(item, edges, data + e * 3 + 1);
    float ring = dashWord(item, edges, data + e * 3 + 2);
    float el = sqrt(len2);
    float u = s0 + dot(w, v) / el;
    float bandD = abs(w.x * v.y - w.y * v.x) / el - halfW;
    if (max(bandD, max(s0 - u, u - s1) - halfW) < best) {
      float2 ends;
      bool2 has;
      float piece = max(bandD, dashAlong(item, edges, u, s0, s1, abs(ring), ring > 0.0, ext, ends, has));
      if (cap == 1) {
        if (has.x) { piece = min(piece, length(w - v * ((ends.x - s0) / el)) - halfW); }
        if (has.y) { piece = min(piece, length(w - v * ((ends.y - s0) / el)) - halfW); }
      }
      best = min(best, piece);
    }
    if (item.strokeStyle.y > 0) {
      int jk = joinKind(item, edges, e);
      if (jk != 0) {
        float before = (ring > 0.0 && s0 == 0.0) ? abs(ring) : s0;
        if (dashOnBefore(item, edges, before + phase) && dashOnAt(item, edges, s0 + phase)) {
          float2 wv = float2(dfF(dfSub(x, joinCoord(item, edges, e, 0))), dfF(dfSub(y, joinCoord(item, edges, e, 1))));
          best = min(best, joinPieceAt(item, edges, e, jk, wv));
        }
      }
    }
  }
  return best;
}
// `__canvas_rrectPosition`: a rounded rectangle's perimeter position of the outline point
// nearest `q` (from its centre), from the top side's left end, clockwise.
static float rrectPosition(float2 q, float iw, float ih, float r) {
  float qr = r * (PI / 2.0);
  float ax = abs(q.x) - iw;
  float ay = abs(q.y) - ih;
  if (ax > 0.0 && ay > 0.0) {
    if (q.x > 0.0 && q.y < 0.0) {
      return 2.0 * iw + r * min(max(dashAngle(float2(q.x - iw, q.y + ih)) - 1.5 * PI, 0.0), PI / 2.0);
    }
    if (q.x > 0.0) {
      return 2.0 * iw + qr + 2.0 * ih + r * min(dashAngle(float2(q.x - iw, q.y - ih)), PI / 2.0);
    }
    if (q.y > 0.0) {
      return 4.0 * iw + 2.0 * qr + 2.0 * ih + r * min(max(dashAngle(float2(q.x + iw, q.y - ih)) - PI / 2.0, 0.0), PI / 2.0);
    }
    return 4.0 * iw + 3.0 * qr + 4.0 * ih + r * min(max(dashAngle(float2(q.x + iw, q.y + ih)) - PI, 0.0), PI / 2.0);
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
static float2 dashCurvePoint(constant MfbItem &item, constant int *edges, int kind, int data, float s) {
  if (kind == 1 || kind == 3) {
    float r = dfF(dfFixed(item.shape.z, item.shapeLo.z));
    float a = s / r + (kind == 3 ? dashWord(item, edges, data) : 0.0);
    return r * float2(dashCos(a), dashSin(a));
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
      if (dashWord(item, edges, data + mid) <= s) { lo = mid; } else { hi = mid; }
    }
    float t0 = dashWord(item, edges, data + lo);
    float t1 = dashWord(item, edges, data + lo + 1);
    float t = (float(lo) + (s - t0) / (t1 - t0)) * (2.0 * PI / float(steps));
    float ox = rx * dashCos(t);
    float oy = ry * dashSin(t);
    return float2(ox * ca - oy * sa, ox * sa + oy * ca);
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
  if (s < 2.0 * iw) { return float2(-iw + s, -ih - r); }
  if (s < right) {
    float a = 1.5 * PI + (s - 2.0 * iw) / r;
    return float2(iw, -ih) + r * float2(dashCos(a), dashSin(a));
  }
  if (s < bottomRight) { return float2(iw + r, -ih + (s - right)); }
  if (s < bottom) {
    float a = (s - bottomRight) / r;
    return float2(iw, ih) + r * float2(dashCos(a), dashSin(a));
  }
  if (s < bottomLeft) { return float2(iw - (s - bottom), ih + r); }
  if (s < left) {
    float a = PI / 2.0 + (s - bottomLeft) / r;
    return float2(-iw, ih) + r * float2(dashCos(a), dashSin(a));
  }
  if (s < topLeft) { return float2(-iw - r, ih - (s - left)); }
  float a = PI + (s - topLeft) / r;
  return float2(-iw, -ih) + r * float2(dashCos(a), dashSin(a));
}
// `__canvas_dashedCurve`: a circle, arc, ellipse or rounded rectangle, cut at the
// perimeter position of the pixel's nearest outline point.
static float dashedCurve(constant MfbItem &item, constant int *edges, float2 x, float2 y, float halfW, int cap) {
  int kind = item.misc.x;
  float2 q = float2(dfF(dfSub(x, dfFixed(item.shape.x, item.shapeLo.x))),
         dfF(dfSub(y, dfFixed(item.shape.y, item.shapeLo.y))));
  int data = 3 + int(dashWord(item, edges, 1));
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
      float start = dashWord(item, edges, data);
      float sweep = dashWord(item, edges, data + 1);
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
    float2 o = float2(q.x * ca + q.y * sa, q.y * ca - q.x * sa);
    float t;
    if (rx == ry) {
      t = dashAngle(o);
      bandD = abs(length(o) - rx) - halfW;
    } else {
      float2 a = abs(o);
      float2 c0 = float2(1.0, 0.0);
      float2 c1 = float2(0.0, 1.0);
      float2 m = c0;
      for (int k = 0; k < 24; ++k) {
        m = normalize(c0 + c1);
        float g = (a.x - rx * m.x) * (-rx * m.y) + (a.y - ry * m.y) * (ry * m.x);
        if (g > 0.0) { c0 = m; } else { c1 = m; }
      }
      float dist = length(a - float2(rx, ry) * m);
      if (dot(a / float2(rx, ry), a / float2(rx, ry)) < 1.0) { dist = -dist; }
      bandD = abs(dist) - halfW;
      t = dashAngle(float2(o.x < 0.0 ? -m.x : m.x, o.y < 0.0 ? -m.y : m.y));
    }
    int steps = item.strokeStyle.w - data - 1;
    float f = t / (2.0 * PI / float(steps));
    int k = clamp(int(f), 0, steps - 1);
    float t0 = dashWord(item, edges, data + k);
    float t1 = dashWord(item, edges, data + k + 1);
    u = t0 + (t1 - t0) * (f - float(k));
    len_ = dashWord(item, edges, data + steps);
  } else {
    float iw = dfF(dfFixed(item.shape.z, item.shapeLo.z));
    float ih = dfF(dfFixed(item.shape.w, item.shapeLo.w));
    float r = dfF(dfFixed(item.misc.y, item.miscLo.x));
    bandD = abs(rectDistance(q, float2(0.0), float2(iw, ih)) - r) - halfW;
    u = rrectPosition(q, iw, ih, r);
    len_ = 4.0 * iw + 4.0 * ih + 4.0 * (r * (PI / 2.0));
  }
  float2 ends;
  bool2 has;
  float d = max(bandD, dashAlong(item, edges, u, 0.0, len_, len_, closed, cap == 2 ? halfW : 0.0, ends, has));
  if (cap == 1) {
    if (has.x) { d = min(d, length(q - dashCurvePoint(item, edges, kind, data, ends.x)) - halfW); }
    if (has.y) { d = min(d, length(q - dashCurvePoint(item, edges, kind, data, ends.y)) - halfW); }
  }
  return d;
}
// `__canvas_dashedStroke`: a dashed item's stroke distance at the shape-space point --
// for a Line, Arc or PolyLine its whole distance, for every other kind its band's.
static float2 dashedStroke(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float halfW) {
  int kind = item.misc.x;
  bool rounded = kind == 0 && (item.misc.y != 0 || item.miscLo.x != 0);
  bool curve = kind == 1 || kind == 3 || kind == 7 || rounded;
  return float2(curve ? dashedCurve(item, edges, x, y, halfW, item.arc.w) : dashedEdges(item, edges, band, x, y, halfW, item.arc.w), 0.0);
}
static float2 sampleDistance(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float reach, bool clip) {
  int kind = item.misc.x;
  // plan-163-C: a path clip's edges under its own rule, through the polygon's walk -- one
  // `edgeDistance` call site, for the shader compilers' sake.
  if (clip || kind == 4) {
    return edgeDistance(edges, clip ? item.clipPath.x : item.arc.z, band.y, band.x, x, y, reach,
                        clip ? (item.clipPath.z >> 2) : item.arc.y);
  }
  // plan-164-C: a CurvePolygon's signed distance, or a PolyCurve's stroke -- dashed or
  // not -- from its pieces. A PolyCurve is stroke-as-fill, its half-width the radius.
  if (kind == 11 || kind == 12) {
    return float2(curveWalk(item, edges, band, x, y, kind == 12 ? 0 : (item.strokeStyle.w > 0 ? 3 : 1),
                            dfF(dfFixed(item.misc.y, item.miscLo.x)), item.arc.w), 0.0);
  }
  // plan-163-B: a dashed Line, Arc or PolyLine is its dashed stroke.
  if (item.strokeStyle.w > 0 && (kind == 2 || kind == 3 || kind == 10)) {
    return dashedStroke(item, edges, band, x, y, dfF(dfFixed(item.misc.y, item.miscLo.x)));
  }
  if (kind == 10) { return polylineDistance(item, edges, band, x, y, reach); }
  float v = farBound(item, float2(x.x, y.x));
  if (abs(v) > reach) { return float2(v, 0.0); }
  return dfShapeDistance(item, x, y);
}
static float2 dfMapX(constant MfbItem &item, float2 x, float2 y) {
  return dfAdd(dfAdd(dfMulF(x, as_type<float>(item.xform0.x)), dfMulF(y, as_type<float>(item.xform0.z))),
               float2(as_type<float>(item.xform1.x), 0.0));
}
static float2 dfMapY(constant MfbItem &item, float2 x, float2 y) {
  return dfAdd(dfAdd(dfMulF(x, as_type<float>(item.xform0.y)), dfMulF(y, as_type<float>(item.xform0.w))),
               float2(as_type<float>(item.xform1.y), 0.0));
}
// The double-float shape-space point: the surface point with the group offset -- its
// 16.16 word plus, since bug-735, its residual -- taken off exactly.
static float4 dfShapePoint(float2 surface, int4 offset) {
  return float4(dfSub(float2(surface.x, 0.0), dfFixed(offset.x, offset.z)), dfSub(float2(surface.y, 0.0), dfFixed(offset.y, offset.w)));
}
// The clip rectangle's own antialiased coverage, 0..255 (plan-116-B): the same
// rectangle SDF and the same quantization the shape edges use, so a fractional clip
// edge is antialiased identically to a shape edge. plan-163-C: at the SHAPE-space
// point (`x`, `y`, the group offset taken off), because the clip is in the item's own
// coordinates, and only for a mode-1 clip, whose zero-area rectangle encloses nothing
// and hides the item. `__canvas_clipCoverage` and `mfb_canvas.frag`'s `clipRect` are
// the same lines.
//
// bug-735: in double-float, from the clip's words plus residuals, wherever the fp32
// rectangle distance (with `edgeSlack`'s bound) cannot say the pixel is wholly in or
// out.
static int clipRect(constant MfbItem &item, float2 x, float2 y) {
  if (!fixedLess(item.clip.x, item.clipLo.x, item.clip.z, item.clipLo.z)
      || !fixedLess(item.clip.y, item.clipLo.y, item.clip.w, item.clipLo.w)) { return 0; }
  float2 p = float2(x.x, y.x);
  float2 lo = float2(fx(item.clip.x), fx(item.clip.y));
  float2 hi = float2(fx(item.clip.z), fx(item.clip.w));
  float d = rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5);
  float slack = edgeSlack(p, lo, hi);
  if (d - slack > 0.5) { return 0; }
  if (d + slack < -0.5) { return 255; }
  float2 x0 = dfFixed(item.clip.x, item.clipLo.x);
  float2 y0 = dfFixed(item.clip.y, item.clipLo.y);
  float2 x1 = dfFixed(item.clip.z, item.clipLo.z);
  float2 y1 = dfFixed(item.clip.w, item.clipLo.w);
  float2 dx = dfSub(x, dfMulF(dfAdd(x0, x1), 0.5));
  float2 dy = dfSub(y, dfMulF(dfAdd(y0, y1), 0.5));
  return dfCoverage(dfRectDistance(dx, dy, dfMulF(dfSub(x1, x0), 0.5), dfMulF(dfSub(y1, y0), 0.5)));
}
// bug-687, extended to every kind by bug-735: an item's fill and stroke coverage, 0..255
// each, and whether it strokes, from its distance in double-float at the double-float
// shape-space point `pt` = (x.hi, x.lo, y.hi, y.lo). The oracle's
// `__canvas_drawGeometry`, operation for operation: the inverse map with the float32
// terms, the central differences at 0.5 (all five on the centre's band), `d / ||grad
// d||`, and the stroke's half-width -- its word plus its residual, stroking exactly when
// the oracle's `half > 0` -- taken off in shape space before the scale. The CENTRE
// sample is straight-line code and every other sample sits behind one branch
// (bug-688, at the samples below), so `sampleDistance` has two call sites.
//
// A sample past `reach` has its coverage decided by its sign (the pre-passes return a
// bound there, not the distance). Untransformed that is `0.5 + half`. Transformed,
// the CENTRE sample may take it too: the scale `g` the coverage divides by is a
// central difference of a 1-Lipschitz distance, so it is at most the inverse
// matrix's Frobenius norm `F` (or the fallback 1.0), and a centre past
// `half + 0.5 * max(F, 1)` is past the clamp whatever the four neighbours say --
// they are then not evaluated. `F` is widened by 2^-8 for the ellipse's bisection,
// which is Lipschitz only to its own ~1e-4 px. An arc is 1-Lipschitz since bug-1088
// (its sweep is two signed half-planes, not a jump to 1e6), so it takes the shortcut
// like every other kind. No neighbour sample does, whose exact value the gradient
// needs. The clip's centre
// sample takes the same shortcut at `0.5 * max(F, 1)` (bug-995, argued at
// `clipReach` in `shapeCoverage` below).
// plan-172-H: a rectangle clip's signed distance in double-float at the item's own
// point. `clipRect` above answers coverage and is right while the rectangle is
// axis-aligned on the surface; under a group matrix it is not, so its distance is
// sampled like a path clip's and divided by its own gradient
// (`__canvas_clipRectSurface`). A rectangle with no area encloses nothing.
static float2 clipRectDistance(constant MfbItem &item, float2 x, float2 y) {
  if (!fixedLess(item.clip.x, item.clipLo.x, item.clip.z, item.clipLo.z)
      || !fixedLess(item.clip.y, item.clipLo.y, item.clip.w, item.clipLo.w)) { return float2(1.0e6, 0.0); }
  float2 x0 = dfFixed(item.clip.x, item.clipLo.x);
  float2 y0 = dfFixed(item.clip.y, item.clipLo.y);
  float2 x1 = dfFixed(item.clip.z, item.clipLo.z);
  float2 y1 = dfFixed(item.clip.w, item.clipLo.w);
  float2 dx = dfSub(x, dfMulF(dfAdd(x0, x1), 0.5));
  float2 dy = dfSub(y, dfMulF(dfAdd(y0, y1), 0.5));
  return dfRectDistance(dx, dy, dfMulF(dfSub(x1, x0), 0.5), dfMulF(dfSub(y1, y0), 0.5));
}
// plan-172-H: `pt` arrives already through the groups' inverse linear part when
// `grouped`, and `gi` is that part: the four neighbour samples are half a SURFACE
// pixel away, which in the group's space is `gi` times that. An item inside a group
// with a matrix takes the transformed path whether or not it has a transform of its
// own -- `mapped` -- while `itemMapped` alone says whether its own inverse applies.
static int4 shapeCoverage(constant MfbItem &item, constant int *edges, float4 pt, float4 gi, bool grouped) {
  float2 x = pt.xy;
  float2 y = pt.zw;
  bool itemMapped = hasTransform(item);
  bool mapped = itemMapped || grouped;
  float2 half2 = dfFixed(item.misc.z, item.miscLo.y);
  bool stroked = dfLess(float2(0.0, 0.0), half2);
  float reach = 0.5 + max(half2.x, 0.0);
  // bug-995: the clip's reach -- the shape's without the stroke half, and for any kind
  // (a clip's distance is 1-Lipschitz whatever the item is, an arc's too).
  float clipReach = 0.5;
  // bug-1085: the composed inverse's norm, which the band lookups hold to the norm the
  // band's reach was built for; 0 for a pixel that takes its centre sample alone.
  float norm = 0.0;
  if (mapped) {
    float2 c0 = itemMapped ? float2(as_type<float>(item.xform0.x), as_type<float>(item.xform0.y)) : float2(1.0, 0.0);
    float2 c1 = itemMapped ? float2(as_type<float>(item.xform0.z), as_type<float>(item.xform0.w)) : float2(0.0, 1.0);
    // The composed inverse's norm is at most the product of the two norms.
    norm = sqrt(dot(c0, c0) + dot(c1, c1));
    if (grouped) { norm = (itemMapped ? norm : 1.0) * length(gi); }
    float frob = norm * (1.0 + 1.0 / 256.0) + 1.0 / 256.0;
    reach = max(half2.x, 0.0) + 0.5 * max(frob, 1.0);
    clipReach = 0.5 * max(frob, 1.0);
  }
  int2 band = polygonBand(item, edges, itemMapped ? dfMapY(item, x, y).x : y.x, norm);
  // plan-163-B: a polygon, rectangle or picture with join records strokes from its own
  // distance (`joinedStroke`), which a miter can reach past `half` from the outline --
  // so the centre's shape distance cannot decide the neighbours there.
  // plan-163-B: so can a dashed one, which strokes from `dashedStroke`.
  bool dashed = item.strokeStyle.w > 0;
  // plan-164-C: nor a PolyCurve, stroke-as-fill like a PolyLine: `sampleDistance` is
  // its whole stroke.
  bool joined = (item.strokeStyle.y > 0 || dashed) && item.misc.x != 10 && item.misc.x != 11;
  float strokeReach = mapped ? reach - max(half2.x, 0.0) : 0.5;
  // plan-163-C: samples 5..9 are a path clip's distance at the same five points, so the
  // clip is divided by ITS OWN gradient (`__canvas_clipPathSurface`), not the shape's.
  //
  // bug-995: the clip's CENTRE sample (5) takes the shape's shortcut. Its coverage is
  // `dfCoverage(d5 / cg)`, `cg` the central difference `|(d6 - d7, d8 - d9)|` of the
  // clip's distance, or the fallback 1.0 when that is under 1e-6. The bound:
  //
  // * A clip's distance is 1-Lipschitz in shape space: a path's is the distance to its
  //   edges with a sign that changes only where an edge is crossed (where it is 0), a
  //   rectangle's the box distance. The neighbours are `q +- M (0.5, 0)` and
  //   `q +- M (0, 0.5)`, `M` the composed inverse (the item's, after the groups'), so
  //   `|d6 - d7| <= |M e1|` and `|d8 - d9| <= |M e2|`, hence `cg <= ||M||_F`. The norm
  //   of a composition is at most the product of the norms, which is `norm`, and
  //   `frob` widens it by 2^-8 for the float32 it is computed in.
  // * A path clip's samples walk the centre's band, which holds every edge within its
  //   reach of the band (`emit_band_index`, `r >= 0.5 + 1.5 F`), or every clip edge
  //   when that reach does not cover the composed `norm` (`clipBandAt`, bug-1085):
  //   the neighbours' crossings and nearest edges are in it, so what is sampled is
  //   Lipschitz too. This is the shape's premise as well (`polygonBand`).
  // * So with `clipReach = 0.5 * max(frob, 1)`, `|d5| > clipReach` gives
  //   `|d5 / cg| > 0.5 * max(frob, 1) / cg >= 0.5` when `cg` is the difference, and
  //   `|d5| > 0.5` when it is the fallback. Past 0.5 `dfCoverage` has clamped: 0
  //   outside, 255 inside, which is `dfCoverage(d5)` -- the four neighbours cannot
  //   change it and are not evaluated (`clipMapped` goes false, as `mapped` does).
  // * Sample 5 is therefore asked with reach `clipReach`, not an unbounded one: where
  //   `edgeDistance`'s fp32 pass proves the distance past it, it returns that bound
  //   with the exact sign, which is past `clipReach` and takes the same branch.
  //
  // Untransformed and ungrouped `clipReach` is 0.5, the one sample there always was.
  int clipMode = item.clipPath.z & 3;
  bool clipMapped = itemMapped || grouped;
  // plan-172-H: a rectangle clip is sampled too, under a group matrix.
  bool clipSampled = clipMode == 2 || (clipMode == 1 && grouped);
  int2 cband = clipMode == 2 ? clipBandAt(item, edges, itemMapped ? dfMapY(item, x, y).x : y.x, norm) : int2(-1, 0);
  float2 d[10];
  // bug-688: the shape's centre sample is taken here, outside the loop, and the loop is
  // entered only by a pixel that needs another sample -- the shape's four neighbours
  // (`mapped`, once the centre has not decided it) or the clip's (`clipSampled`). An
  // untransformed, unclipped pixel therefore runs one sample and no loop. It did in the
  // one ten-pass loop this replaces too, as nine `continue`s, and a driver that does
  // not turn a skipped pass into skipped work charged it for all ten (box 206's Intel
  // Vulkan driver, on the GLSL twin: 10,000 16 px squares rendered in 106 ms a frame,
  // and in 58 this way). The samples and their order are unchanged; the centre point
  // is `x`, `y` itself where the loop added a zero offset to it, which a double-float
  // sum returns unchanged.
  float2 cqx = itemMapped ? dfMapX(item, x, y) : x;
  float2 cqy = itemMapped ? dfMapY(item, x, y) : y;
  d[0] = sampleDistance(item, edges, band, cqx, cqy, reach, false);
  if (!joined && abs(d[0].x) > reach) { mapped = false; }
  if (mapped || clipSampled) {
    for (int k = 1; k < 10; ++k) {
      bool clipK = k >= 5;
      int j = clipK ? k - 5 : k;
      if (clipK ? (!clipSampled || j >= (clipMapped ? 5 : 1)) : !mapped) { continue; }
      float ox = j == 1 ? 0.5 : (j == 2 ? -0.5 : 0.0);
      float oy = j == 3 ? 0.5 : (j == 4 ? -0.5 : 0.0);
      float2 sx = dfAdd(x, float2(grouped ? gi.x * ox + gi.z * oy : ox, 0.0));
      float2 sy = dfAdd(y, float2(grouped ? gi.y * ox + gi.w * oy : oy, 0.0));
      float2 qx = itemMapped ? dfMapX(item, sx, sy) : sx;
      float2 qy = itemMapped ? dfMapY(item, sx, sy) : sy;
      if (clipK && clipMode == 1) {
        d[k] = clipRectDistance(item, qx, qy);
      } else {
        d[k] = sampleDistance(item, edges, clipK ? cband : band, qx, qy, k == 5 ? clipReach : 1.0e30, clipK);
      }
      if (k == 5 && abs(d[5].x) > clipReach) { clipMapped = false; }
    }
  }
  float2 dRaw = d[0];
  float2 g = float2(1.0, 0.0);
  if (mapped) {
    float2 gx = dfSub(d[1], d[2]);
    float2 gy = dfSub(d[3], d[4]);
    g = dfLength(gx, gy);
    if (!(g.x > 0.000001)) { g = float2(1.0, 0.0); }
  }
  int fill = dfCoverage(mapped ? dfDiv(dRaw, g) : dRaw);
  int stroke = 0;
  if (stroked) {
    // plan-164-C: a CurvePolygon's joined or dashed stroke is its pieces' (`curveWalk`).
    float2 s = (item.misc.x == 12 && joined)
        ? float2(curveWalk(item, edges, band, cqx, cqy, dashed ? 3 : 2, dfF(half2), item.arc.w), 0.0)
        : (dashed ? dashedStroke(item, edges, band, cqx, cqy, dfF(half2))
        : (joined ? joinedStroke(item, edges, band, cqx, cqy, half2, strokeReach) : dfSub(dfAbs(dRaw), half2)));
    stroke = dfCoverage(mapped ? dfDiv(s, g) : s);
  }
  int clipCov = 255;
  if (clipMode == 1 && !grouped) { clipCov = clipRect(item, x, y); }
  if (clipSampled) {
    float2 cg = float2(1.0, 0.0);
    if (clipMapped) {
      cg = dfLength(dfSub(d[6], d[7]), dfSub(d[8], d[9]));
      if (!(cg.x > 0.000001)) { cg = float2(1.0, 0.0); }
    }
    clipCov = dfCoverage(clipMapped ? dfDiv(d[5], cg) : d[5]);
  }
  return int4(fill, stroke, stroked ? 1 : 0, clipCov);
}
static float srgbToLinear(float c) {
  c = c / 255.0;
  return c <= 0.04045 ? (c / 12.92) : pow((c + 0.055) / 1.055, 2.4);
}
// plan-116-F: the colour a gradient shows at `p`, as 0..255 — the twin of
// `gradientColour` in `runtime/canvas/shaders/mfb_canvas.frag` and of
// `__canvas_gradientColor` in the oracle. The lerp is in LINEAR light, which is the
// one property that must match; the result is returned encoded so it drops straight
// into `covered`, which is what decodes.
// One entry of the oracle's forward table, recomputed rather than uploaded: the
// curve ROUNDED to integers on a 0..65535 scale, which is the space its lerp
// happens in. The oracle's table is `__color_srgbTable`, in the `color` package
// since plan-122-B. This reproduction spells neither its name nor its constant,
// so a grep for the table will not find it; if the quantisation ever changes,
// `the_gpu_draws_the_gradient_scene_the_reference_shows` is what catches it.
static float srgbTable(int i) {
  return floor(srgbToLinear(float(i)) * 65535.0 + 0.5);
}
// One channel, reproducing `__canvas_gradientChannel` exactly. `num` is already
// quantised to 1/4096 by TRUNCATION because the oracle quantises there; a
// continuous fraction here biases the whole ramp by up to a step. The answer is the
// byte nearest in LINEAR value, the rule the oracle's binary search applies.
static int gradientChannel(int loSrgb, int hiSrgb, int num) {
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
// plan-154-B: the stop lookup at a ramp parameter `t`, split from the axis that
// computes `t` so a PolyLine can ask it at its path parameter instead.
static int4 gradientAt(float t, constant int *edges, constant MfbItem &item) {
  int count = item.ellipse.z;
  int base = METAL_GRADIENT_BASE + item.ellipse.w * 5;
  t = clamp(t, 0.0, 1.0);
  int idx = count;
  for (int i = 0; i < count; ++i) {
    if (idx == count && fx(edges[base + i * 5]) >= t) { idx = i; }
  }
  if (idx >= count) {
    int last = base + (count - 1) * 5;
    return int4(edges[last + 1], edges[last + 2], edges[last + 3], edges[last + 4]);
  }
  if (idx <= 0) {
    return int4(edges[base + 1], edges[base + 2], edges[base + 3], edges[base + 4]);
  }
  int lo = base + (idx - 1) * 5;
  int hi = base + idx * 5;
  float loOff = fx(edges[lo]);
  float hiOff = fx(edges[hi]);
  // Quantised to 1/4096 by truncation, exactly as `__canvas_gradientColor` does.
  int num = hiOff > loOff ? int((t - loOff) / (hiOff - loOff) * 4096.0) : 0;
  num = clamp(num, 0, 4096);
  int loA = edges[lo + 4];
  int hiA = edges[hi + 4];
  return int4(gradientChannel(edges[lo + 1], edges[hi + 1], num),
              gradientChannel(edges[lo + 2], edges[hi + 2], num),
              gradientChannel(edges[lo + 3], edges[hi + 3], num),
              loA + int(trunc(float(hiA - loA) * float(num) / 4096.0)));
}
// plan-163-D: the spread past the ramp's ends, the focal radial's position, the ramp
// at the SHAPE-space point `q` (radial through its own inverse; `gradientKind` packs
// `kind | spread << 1 | apply << 3`), and the stroke tint -- `mfb_canvas.frag`'s
// `gradientSpread`, `focalT`, `gradientColour` and `tintRamp`, line for line.
static float gradientSpread(float t, int spread) {
  if (spread == 0) { return t; }
  float tt = clamp(t, -1.0e9, 1.0e9);
  if (spread == 2) { return tt - floor(tt); }
  float u = tt - 2.0 * floor(tt / 2.0);
  return u > 1.0 ? 2.0 - u : u;
}
static float focalT(float2 p, float2 c, float r, float2 o, float fr) {
  if (!(r > 0.0)) { return 0.0; }
  float2 f = c + o;
  float2 e = p - f;
  float2 w = c - f;
  float dr = r - fr;
  float a = w.x * w.x + w.y * w.y - dr * dr;
  float b = e.x * w.x + e.y * w.y + fr * dr;
  float cc = e.x * e.x + e.y * e.y - fr * fr;
  float disc = max(b * b - a * cc, 0.0);
  return (b - sqrt(disc)) / a;
}
static int4 gradientColour(float2 q, constant int *edges, constant MfbItem &item) {
  float2 gfrom = float2(fx(item.gradient.x), fx(item.gradient.y));
  float2 gto = float2(fx(item.gradient.z), fx(item.gradient.w));
  float2 axis = gto - gfrom;
  float len2 = dot(axis, axis);
  float t = 0.0;
  if ((item.gradientKind & 1) == 1) {
    float2 g = q;
    if (item.gradInv1.z != 0) {
      g = float2(as_type<float>(item.gradInv0.x) * q.x + as_type<float>(item.gradInv0.z) * q.y
                     + as_type<float>(item.gradInv1.x),
                 as_type<float>(item.gradInv0.y) * q.x + as_type<float>(item.gradInv0.w) * q.y
                     + as_type<float>(item.gradInv1.y));
    }
    float2 o = float2(fx(item.gradFocal.x), fx(item.gradFocal.y));
    float fr = fx(item.gradFocal.z);
    float len = sqrt(len2);
    if (o.x != 0.0 || o.y != 0.0 || fr != 0.0) {
      t = focalT(g, gfrom, len, o, fr);
    } else if (len > 0.0) {
      t = length(g - gfrom) / len;
    }
  } else {
    if (len2 > 0.0) { t = dot(q - gfrom, axis) / len2; }
  }
  return gradientAt(gradientSpread(t, (item.gradientKind >> 1) & 3), edges, item);
}
static int4 tintRamp(int4 ramp, int4 by) {
  int4 tint = (ramp * by) / 255;
  int4 out = ramp;
  for (int c = 0; c < 3; ++c) {
    int dc = (tint[c] - ramp[c]) * by.w;
    out[c] = ramp[c] + (dc >= 0 ? (dc + 127) / 255 : -((127 - dc) / 255));
  }
  return out;
}
// bug-1079: which of `n` texels holds `t`, the texels `d` apart -- `floor(t / d)`
// corrected by at most one from the exact products, so the index does not hang on
// the last bit of the quotient (Vulkan allows `/` 2.5 ULP and one driver takes it;
// Apple's GPUs divide exactly, and this is the same rule so the copies stay one).
// The GLSL's `pictureTexel`, line for line, and the oracle's picture arm.
static int pictureTexel(float t, float d, int n) {
  float u = floor(t / d);
  if ((u + 1.0) * d <= t) { u += 1.0; } else if (u * d > t) { u -= 1.0; }
  return clamp(int(u), 0, n - 1);
}
static int tileTexel(float t, float d) {
  float u = floor(t / d);
  if ((u + 1.0) * d <= t) { u += 1.0; } else if (u * d > t) { u -= 1.0; }
  return int(u);
}
// bug-484: a picture's fill colour at the SHAPE-space point `q` -- its image sampled
// NEAREST and tinted by the fill (plan-163-C: towards `texel * fill / 255` by
// `fill.a / 255`, rounded half away from zero, and its alpha faded by the transparency
// step in `clipPath.w` -- `__canvas_tintChannel`), the twin of the `isPicture` arm of
// `__canvas_drawGeometry`. Same operation order as the oracle, `(q - x0) * iw / dw`,
// through `pictureTexel`, so all renderers pick the same texel; the index is
// CLAMPED because the antialiased pixels just outside the destination belong to the
// border texel. The texels ride the glyph region one packed `r | g<<8 | b<<16 | a<<24`
// word each, and the block names its slice as a glyph's does: width in `misc.w`,
// height in `arc.x`, base in `arc.z`. A zero width is an image the emitter could not
// upload (destroyed, or over the frame's cap): transparent, like the oracle's
// `shadowTexel` answer for a missing block.
static int4 pictureColour(constant MfbItem &item, constant int *edges, float2 q) {
  int iw = item.misc.w;
  int ih = item.arc.x;
  if (iw <= 0 || ih <= 0) { return int4(0); }
  float hw = fx(item.shape.z), hh = fx(item.shape.w);
  float x0 = fx(item.shape.x) - hw, y0 = fx(item.shape.y) - hh;
  float dw = hw * 2.0, dh = hh * 2.0;
  int u = pictureTexel((q.x - x0) * float(iw), dw, iw);
  int v = pictureTexel((q.y - y0) * float(ih), dh, ih);
  uint word = as_type<uint>(edges[METAL_GLYPH_BASE + item.arc.z + v * iw + u]);
  int4 tex = int4(int(word & 255u), int((word >> 8) & 255u),
                  int((word >> 16) & 255u), int((word >> 24) & 255u));
  int4 tint = (tex * item.fill) / 255;
  int a = item.fill.w;
  int4 out = tex;
  for (int c = 0; c < 3; ++c) {
    int dc = (tint[c] - tex[c]) * a;
    out[c] = tex[c] + (dc >= 0 ? (dc + 127) / 255 : -((127 - dc) / 255));
  }
  out.w = (tex.w * (255 - item.clipPath.w) + 127) / 255;
  return out;
}
// plan-173: the requested source rectangle maps directly; samples outside the image
// are transparent instead of clamped or remapped.
static int4 tileColour(constant MfbItem &item, constant int *edges, float2 q) {
  int iw = item.misc.w, ih = item.arc.x;
  if (iw <= 0 || ih <= 0) { return int4(0); }
  float hw = fx(item.shape.z), hh = fx(item.shape.w);
  float x0 = fx(item.shape.x) - hw, y0 = fx(item.shape.y) - hh;
  float dw = hw * 2.0, dh = hh * 2.0;
  float sx = fx(item.gradient.x), sy = fx(item.gradient.y);
  float sw = fx(item.gradient.z), sh = fx(item.gradient.w);
  int u = tileTexel(sx * dw + (q.x - x0) * sw, dw);
  int v = tileTexel(sy * dh + (q.y - y0) * sh, dh);
  // bug-1093: clamp to the texels the source rectangle touches, before the image test.
  u = clamp(u, int(floor(sx)), int(ceil(sx + sw)) - 1);
  v = clamp(v, int(floor(sy)), int(ceil(sy + sh)) - 1);
  if (u < 0 || v < 0 || u >= iw || v >= ih) { return int4(0); }
  uint word = as_type<uint>(edges[METAL_GLYPH_BASE + item.arc.z + v * iw + u]);
  return int4(int(word & 255u), int((word >> 8) & 255u),
              int((word >> 16) & 255u), int((word >> 24) & 255u));
}
// The oracle quantizes coverage to a whole 0..255 and then takes an integer
// `(colourAlpha * coverage) / 255`. Matching that here rather than blending in
// float is not pedantry: near full coverage the sRGB encode is so steep that ONE
// step of coverage moves a dark channel by up to 13 output steps, so a float
// coverage that merely rounds differently produces a visible disagreement on
// every antialiased edge. Quantizing the same way leaves only the pixels within
// a rounding boundary of each other.
static float4 covered(int4 rgba, int coverage) {
  float a = float((rgba.w * coverage) / 255) / 255.0;
  return float4(srgbToLinear(float(rgba.x)) * a,
                srgbToLinear(float(rgba.y)) * a,
                srgbToLinear(float(rgba.z)) * a, a);
}
// `items` and `edges` are the SAME buffer bound twice, at offset zero both times:
// one view typed as blocks, one as raw words. Two bindings rather than one pointer
// cast because the edge region is reached by a word index and the block region by a
// struct index, and letting each keep its own element type is what stops a packing
// mistake -- the class of bug that yields a plausible wrong picture, not a fault.
// Glyph coverage is the fourth region of the same buffer (bug-670): a text item is
// ONE instanced draw, so a per-glyph payload bound before it would be overwritten
// by the next glyph's; each glyph's block names its own slice instead.
fragment float4 mfbFragment(VOut in [[stage_in]],
                            constant MfbItem *items [[buffer(0)]],
                            constant int *edges [[buffer(1)]],
                            constant MfbDraw &draw [[buffer(3)]]) {
  constant MfbItem &item = items[in.item];
  // plan-116-H: `p` is the SHAPE-space point -- the surface point with the group's
  // offset taken back off -- and every distance, gradient, glyph lookup and (since
  // plan-163-C) the clip below uses it.
  float2 p = in.pos.xy - float2(fx(draw.offset.x), fx(draw.offset.y));
  // plan-172-H: and, under a group matrix, through the groups' inverse linear part --
  // so `p` is the point in the GROUP's space, where an item's own placement lives, and
  // every consumer below (the item's own inverse map included) is unchanged.
  bool grouped = drawGrouped(draw);
  float4 gi = drawInverse(draw);
  if (grouped) { p = float2(gi.x * p.x + gi.z * p.y, gi.y * p.x + gi.w * p.y); }
  // A glyph has coverage, not a distance: the CPU rasterised its outline once and
  // cached the bitmap (plan-98-G Phase 2), so the GPU's job here is a lookup. It
  // returns before `geoDistance` for that reason, and it is fill-only — a stroked
  // text item became an outline polygon in the geometry builder.
  //
  // Outside the bitmap is zero rather than clamped: the quad can cover a pixel the
  // bitmap does not, and clamping would smear the border row outward.
  // The clip multiplies the shape's own coverage, exactly as the oracle's
  // `(coverage * clipCov) / 255` does. Integer, and by 255 rather than a shift, so
  // the two quantize identically -- a float multiply here would disagree on the
  // boundary pixels, which are the only ones a clip can affect.
  // plan-163-C: the clip is evaluated in `shapeCoverage` at the SHAPE-space point -- it
  // travels with the item's transform and its group -- for every kind, glyphs included.
  float4 pt = dfShapePoint(in.pos.xy, draw.offset);
  if (grouped) {
    pt = float4(dfAdd(dfMulF(pt.xy, gi.x), dfMulF(pt.zw, gi.z)), dfAdd(dfMulF(pt.xy, gi.y), dfMulF(pt.zw, gi.w)));
  }
  int4 cov = shapeCoverage(item, edges, pt, gi, grouped);
  int clipCov = cov.w;
  // plan-116-C section 4.5: a transformed glyph samples its bitmap at the
  // inverse-mapped point, nearest. The index arithmetic is already whole-pixel, so
  // mapping the query point is the whole change -- the cache stays untransformed and
  // one entry serves every transform.
  if (item.misc.x == 6) {
    float2 gp = hasTransform(item) ? inverseMap(item, p) : p;
    // `floor`, not a cast: a cast truncates toward zero, and a transformed glyph maps
    // to NEGATIVE shape-space coordinates (ink runs up from the pen), where truncation
    // picks the texel on the wrong side. Untransformed, `gp` is a surface pixel centre
    // and always positive, so this is the same value the cast gave.
    int ix = int(floor(gp.x)) - item.shape.x;
    int iy = int(floor(gp.y)) - item.shape.y;
    int cov = (ix < 0 || iy < 0 || ix >= item.misc.w || iy >= item.arc.x)
      ? 0 : edges[METAL_GLYPH_BASE + item.arc.z + iy * item.misc.w + ix];
    return covered(item.fill, (cov * clipCov) / 255);
  }
  // Both coverages come from `shapeCoverage` in double-float at the double-float twin
  // of `p` -- the surface point with the group offset (word and residual) taken off,
  // exactly: a polygon since bug-687, every other kind since bug-735. The fill uses
  // the surface distance and the stroke subtracts `half` BEFORE converting, so the
  // outline scales with the shape (section 4.3).
  // plan-116-F: the gradient replaces the fill COLOUR and nothing else.
  // plan-163-D: at the shape-space point `q`, so the transform carries the ramp; and
  // with `FillAndStroke` the stroke -- a Line's or Arc's fill -- is the ramp tinted by
  // its colour.
  float2 q = hasTransform(item) ? inverseMap(item, p) : p;
  int4 ramp = item.ellipse.z >= 2 ? gradientColour(q, edges, item) : item.fill;
  bool rampStroke = item.ellipse.z >= 2 && (item.gradientKind & 8) != 0;
  int4 fillRgba = ramp;
  if (rampStroke && (item.misc.x == 2 || item.misc.x == 3 || item.misc.x == 11)) { fillRgba = tintRamp(ramp, item.fill); }
  // plan-154-B: a PolyLine's stops are its per-point colours, and its ramp runs along
  // the path -- the parameter of the nearest point, in shape space, from the same band
  // the distance used.
  if (item.misc.x == 10 && item.ellipse.z >= 2) {
    fillRgba = gradientAt(polylineScan(item, edges, polygonBand(item, edges, q.y, 0.0), q).y, edges, item);
  }
  // bug-484: a picture's image replaces the fill COLOUR the same way, sampled at the
  // inverse-mapped point under a transform (plan-116-C section 4.5, as a glyph is).
  if (item.misc.x == 9) {
    fillRgba = pictureColour(item, edges, q);
  }
  if (item.misc.x == 13) {
    fillRgba = tileColour(item, edges, q);
  }
  // plan-163-D: a fill alpha of -1 is the stroke record of a split blended item.
  float4 colour = item.fill.w < 0 ? float4(0.0) : covered(fillRgba, (cov.x * clipCov) / 255);
  if (cov.z != 0) {
    // plan-116-B: this stroke-over-fill composition equals the oracle's two sequential
    // blends only under `Normal`, which is why a non-`Normal` item that both fills and
    // strokes is emitted as two adjacent instances (`emit_split_or_publish`). By the
    // time such an item reaches here it is fill-only or stroke-only.
    float4 s = covered(rampStroke ? tintRamp(ramp, item.stroke) : item.stroke, (cov.y * clipCov) / 255);
    colour = s + colour * (1.0 - s.w);
  }
  return colour;
}
