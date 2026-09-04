//! The Metal render pipeline (plan-98-E Phase 1).
//!
//! One pipeline, many shapes — the same decision the software rasteriser made, so
//! the GPU path predicts the oracle's output through the same structure rather than
//! a parallel one.
//!
//! ## Why this lives in the macOS app module
//!
//! The device probe (`canvas::metalAvailable`) is a plain C call and lives with the
//! rest of the renderer under `codegen/runtime/canvas/`. Everything below is
//! Objective-C message sends, and the selector machinery that makes those readable —
//! `Asm::load_selector`, the `SEL_*` C-string data objects and the table that emits
//! them — is here. Duplicating it into the runtime module to keep the Metal code
//! together would mean two copies of the seam that has to stay in step with the
//! reconcile data-object list, so the code goes where the machinery already is.
//!
//! ## The shaders are compiled at run time
//!
//! `newLibraryWithSource:options:error:` compiles the MSL below at first frame,
//! rather than a build step producing a `.metallib` to embed. The plan's Open
//! Decision offered "hand-write vs a glslang→SPIRV-Cross toolchain" and recommended
//! hand-writing for one or two shaders; it did not ask *when* the source becomes a
//! library. Runtime is the answer that keeps the no-dependency constraint intact: a
//! build-time `xcrun metal` step would make compiling a *user's* program depend on an
//! installed Xcode toolchain, which is a much heavier thing to require than one
//! compile at startup.
//!
//! ## The colour chain is set up now, not retrofitted
//!
//! plan-98-E §3 calls the sRGB/linear-blend chain "non-negotiable and painful to
//! retrofit", so Phase 1 pins all three links even though a single opaque quad would
//! survive getting them wrong:
//!
//! * the render target is `BGRA8Unorm_sRGB`, so the GPU does the linear→sRGB encode
//!   on write, exactly where the software path's `__COLOR_SRGB` table does it;
//! * the fragment shader emits **linear** premultiplied colour, matching the space
//!   the software blend runs in;
//! * the blend state is `One` / `OneMinusSourceAlpha` — the premultiplied-alpha
//!   `over` the software path implements by hand.
//!
//! ## Ownership
//!
//! Every object created here is `+1` — `new…` and `alloc`/`init` both return owned
//! references — so this needs no autorelease pool, which matters because it runs on
//! the graphics thread and that thread has none (an unpooled autorelease there does
//! not merely leak: the thread-exit drain aborts in libmalloc). They are created once
//! and live for the process, so nothing releases them either.

use super::*;
use crate::codegen::runtime::canvas::metal::{LIB_METAL, MTL_CREATE_DEVICE};
use crate::codegen::runtime::canvas::{
    BLEND_MODE_COUNT, CANVAS_MAX_FRAME_ITEMS, GRAPHICS_OFFSET_MTL_DEVICE,
    GRAPHICS_OFFSET_MTL_ITEM_BUFFER, GRAPHICS_OFFSET_MTL_ITEM_CONTENTS,
    GRAPHICS_OFFSET_MTL_PIPELINE, GRAPHICS_OFFSET_MTL_PIPELINE_MODES, GRAPHICS_OFFSET_MTL_QUEUE,
    GRAPHICS_OFFSET_MTL_READY, GRAPHICS_OFFSET_MTL_TEXTURE, GRAPHICS_OFFSET_MTL_TEX_HEIGHT,
    GRAPHICS_OFFSET_MTL_TEX_WIDTH, GRAPHICS_STATE_SYMBOL, ITEM_ARC_EDGE_BASE, METAL_BUFFER_BYTES,
    METAL_EDGE_BASE_WORDS, METAL_MAX_FRAME_EDGES,
};
use crate::codegen::runtime::canvas::{
    EDGE_SLOTS, FIXED_POINT_SCALE, GEO_KIND_POLYGON, GEO_KIND_TEXT, GLYPH_META_H, GLYPH_META_SLOTS,
    GLYPH_META_START, GLYPH_META_W, GLYPH_META_X0, GLYPH_META_Y0, GLYPH_RUN_SLOTS,
    GRADIENT_STOP_WORDS, HEADER_AUX0, HEADER_AUX1, HEADER_BLEND, HEADER_BOUNDS, HEADER_CAP,
    HEADER_CAP_END_X, HEADER_CAP_START_X, HEADER_CLIP_X0, HEADER_CLIP_X1, HEADER_CLIP_Y0,
    HEADER_CLIP_Y1, HEADER_ELLIPSE_COS, HEADER_ELLIPSE_SIN, HEADER_FILL_R, HEADER_GRADIENT_COUNT,
    HEADER_GRADIENT_FROM_X, HEADER_GRADIENT_KIND, HEADER_HAS_TRANSFORM, HEADER_KIND, HEADER_RADIUS,
    HEADER_SHAPE, HEADER_SLOTS, HEADER_STROKE_HALF, HEADER_STROKE_R, HEADER_TRANSFORM_IA,
    HEADER_TRANSFORM_IB, HEADER_TRANSFORM_IC, HEADER_TRANSFORM_ID, HEADER_TRANSFORM_ITX,
    HEADER_TRANSFORM_ITY, ITEM_ARC_CAP, ITEM_ARC_GLYPH_HEIGHT, ITEM_BLOCK_SIZE,
    ITEM_ELLIPSE_GRADIENT_BASE, ITEM_ELLIPSE_GRADIENT_COUNT, ITEM_OFFSET_ARC, ITEM_OFFSET_ARC_CAPS,
    ITEM_OFFSET_CLIP, ITEM_OFFSET_ELLIPSE, ITEM_OFFSET_FILL, ITEM_OFFSET_GRADIENT,
    ITEM_OFFSET_MISC, ITEM_OFFSET_QUAD, ITEM_OFFSET_SHAPE, ITEM_OFFSET_STROKE, ITEM_OFFSET_SURFACE,
    ITEM_OFFSET_TRANSFORM, ITEM_SURFACE_BLEND, ITEM_SURFACE_GRADIENT_KIND, MAX_EDGES,
    MAX_FRAME_GRADIENT_STOPS, METAL_GRADIENT_BASE_WORDS, METAL_MAX_GLYPH_SAMPLES,
};

/// The one-time setup helper's symbol.
pub(super) const METAL_INIT_SYMBOL: &str = "_mfb_macapp_metal_init";

/// The MSL for the single pipeline.
///
/// One pipeline, many shapes: the vertex stage expands four vertices over the item's
/// **bounds** and the fragment stage evaluates that item's signed distance field.
/// This is the same structure the software rasteriser uses — one loop, one distance
/// function switched on `kind` — which is what makes the oracle predict this
/// backend's output rather than merely resemble it.
///
/// `[[position]]` in a fragment is the framebuffer pixel *centre* with a top-left
/// origin, which is exactly the software path's `px = x + 0.5, py = y + 0.5`. So the
/// fragment stage needs no surface size and no Y flip; only the vertex stage does.
///
/// ## Why the parameter block is integers
///
/// The geometry header is `Float`, i.e. IEEE double, and MSL has no double — so the
/// values have to narrow somewhere. They narrow on the CPU, into **16.16 fixed
/// point**, because the AArch64 assembler this backend emits through has no
/// double→single convert and no 32-bit floating-point store: producing an `f32`
/// buffer would mean adding two instructions to the shared ISA layer (and their
/// x86-64 and riscv64 counterparts) purely to feed a macOS GPU buffer.
///
/// Fixed point is not a compromise for what this carries. Pixel-space geometry needs
/// a range of a few thousand and a resolution far below one pixel; 16.16 gives
/// ±32768 px at 1/65536 px, which is finer than `float`'s own resolution above 512
/// px. The colours are exempt — the header already stores them as whole 0–255 values
/// — and so is `invLenSq`, which is why the polygon edge buffer carries endpoints and
/// the shader recomputes the edge vector (see `edgeDistance`).
///
/// ## Every member is an `int4`
///
/// So the CPU-side offsets and MSL's own packing cannot disagree. A mixed struct
/// would put the burden of predicting MSL's alignment rules on the emitter, and a
/// wrong prediction there is not a compile error — it is a scene that draws with its
/// fields shifted.
pub(super) const METAL_SHADER_SOURCE: &str = concat!(
    "#include <metal_stdlib>\n",
    "using namespace metal;\n",
    "constant float FIXED = 65536.0;\n",
    "constant float PI = 3.141592653589793;\n",
    // Where the frame buffer's edge region starts, in 32-bit words -- i.e. immediately
    // past `CANVAS_MAX_FRAME_ITEMS` item blocks, so it MOVES whenever `ITEM_BLOCK_SIZE`
    // does (114688 -> 131072 when plan-116-B widened the block to 128 bytes). Spelled
    // as a literal because
    // `METAL_SHADER_SOURCE` is a `concat!` of string literals and cannot interpolate a
    // computed value; `the_metal_shader_region_bases_match_the_buffer_layout` is what
    // keeps it equal to `METAL_EDGE_BASE_WORDS`. A disagreement would not fail
    // anywhere -- every polygon would simply read edges from the wrong place in a
    // buffer that is entirely valid memory.
    "constant int METAL_EDGE_BASE = 212992;\n",
    "constant int METAL_GRADIENT_BASE = 278528;\n",
    "struct MfbItem {\n",
    "  int4 quad;\n",     // bounds minX, minY, maxX, maxY (16.16 px)
    "  int4 shape;\n",    // p0..p3 (16.16 px)
    "  int4 fill;\n",     // RGBA 0..255
    "  int4 stroke;\n",   // RGBA 0..255
    "  int4 misc;\n",     // kind, radius (16.16), strokeHalf (16.16), edgeCount
    "  int4 arc;\n",      // startAngle, endAngle (16.16 rad), edgeBase, capStyle
    "  int2 surface;\n",  // width, height (px)
    "  int blendMode;\n", // the BlendMode tag 0..3 (plan-116-B)
    "  int gradientKind;\n", // 0 linear, 1 radial (plan-116-F); was the block's pad
    "  int4 clip;\n",   // clip x0,y0,x1,y1 (16.16 px); zero-area = unclipped
    "  int4 xform0;\n", // inverse transform ia,ib,ic,id as float32 BITS
    "  int4 xform1;\n", // itx, ity (float32 bits), hasTransform (0 or 1), unused
    "  int4 arcCaps;\n", // an arc's two sweep endpoints startX,startY,endX,endY (16.16)
    "  int4 ellipse;\n", // ellipse cos, sin (16.16); gradient stop count and base
    "  int4 gradient;\n", // a gradient's axis startX,startY,endX,endY (16.16)
    "};\n",
    // plan-116-A: the index travels to the fragment stage as a flat varying, because
    // `[[instance_id]]` does not exist there. `[[flat]]` and not the default: the value
    // is an index, and interpolating an index across a quad produces a *plausible*
    // picture drawn from the wrong blocks rather than a failure.
    "struct VOut { float4 pos [[position]]; uint item [[flat]]; };\n",
    "static float fx(int v) { return float(v) / FIXED; }\n",
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
    // (`ITEM_BLOCK_SIZE`, 208 since plan-116-F; 112 when this was written).
    "vertex VOut mfbVertex(uint vid [[vertex_id]],\n",
    "                      uint iid [[instance_id]],\n",
    "                      constant MfbItem *items [[buffer(0)]]) {\n",
    "  uint index = iid;\n",
    "  constant MfbItem &item = items[index];\n",
    "  float2 corner = float2(fx((vid & 1) == 0 ? item.quad.x : item.quad.z),\n",
    "                         fx((vid & 2) == 0 ? item.quad.y : item.quad.w));\n",
    "  VOut o;\n",
    "  o.item = index;\n",
    "  o.pos = float4(corner.x / float(item.surface.x) * 2.0 - 1.0,\n",
    "                 1.0 - corner.y / float(item.surface.y) * 2.0, 0.0, 1.0);\n",
    "  return o;\n",
    "}\n",
    "static float rectDistance(float2 p, float2 c, float2 h) {\n",
    "  float2 d = abs(p - c) - h;\n",
    "  return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);\n",
    "}\n",
    "static float segmentDistance(float2 p, float2 a, float2 b) {\n",
    "  float2 v = b - a, w = p - a;\n",
    "  float len2 = dot(v, v);\n",
    "  float t = len2 > 0.0 ? clamp(dot(w, v) / len2, 0.0, 1.0) : 0.0;\n",
    "  return length(w - v * t);\n",
    "}\n",
    // plan-116-D: the butt-capped twin, and the twin of `segmentDistanceButt` in
    // `runtime/canvas/shaders/mfb_canvas.frag`. `half` comes off BEFORE the two `max`es
    // because a butt stroke is the round band intersected with the slab between the end
    // planes — subtracting after would compare each plane against the half-width rather
    // than against zero, so the cap would not bite until a pixel was more than `half`
    // past the endpoint.
    "static float segmentDistanceButt(float2 p, float2 a, float2 b, float halfW) {\n",
    "  float2 v = b - a, w = p - a;\n",
    "  float len2 = dot(v, v);\n",
    "  if (len2 <= 0.0) { return 1.0e6; }\n",
    "  float len = sqrt(len2);\n",
    "  float t = dot(w, v) / len2;\n",
    "  float d = length(w - v * clamp(t, 0.0, 1.0)) - halfW;\n",
    "  d = max(d, -t * len);\n",
    "  return max(d, (t - 1.0) * len);\n",
    "}\n",
    // plan-116-E: the ellipse SDF, the twin of `ellipseDistance` in
    // `runtime/canvas/shaders/mfb_canvas.frag`. 24 bisection halvings on the folded
    // first quadrant -- the same count and arithmetic the software oracle uses -- and
    // no trigonometry, because `ca`/`sa` are the CPU's deterministic Taylor pair
    // carried in the item block.
    "static float ellipseDistance(float2 p, float2 c, float rx, float ry, float ca, float sa) {\n",
    "  float2 d = p - c;\n",
    "  float2 q = abs(float2(d.x * ca + d.y * sa, -d.x * sa + d.y * ca));\n",
    "  if (rx == ry) { return length(q) - rx; }\n",
    "  float2 a = float2(1.0, 0.0);\n",
    "  float2 b = float2(0.0, 1.0);\n",
    "  float2 m = a;\n",
    "  for (int i = 0; i < 24; ++i) {\n",
    "    m = normalize(a + b);\n",
    "    float g = (q.x - rx * m.x) * (-rx * m.y) + (q.y - ry * m.y) * (ry * m.x);\n",
    "    if (g > 0.0) { a = m; } else { b = m; }\n",
    "  }\n",
    "  float dist = length(q - float2(rx * m.x, ry * m.y));\n",
    "  float2 u = float2(q.x / rx, q.y / ry);\n",
    "  return dot(u, u) < 1.0 ? -dist : dist;\n",
    "}\n",
    "static bool arcInSweep(float2 d, float2 s, float2 e, bool reflex) {\n",
    "  bool afterStart = s.x * d.y - s.y * d.x >= 0.0;\n",
    "  bool beforeEnd  = e.x * d.y - e.y * d.x <= 0.0;\n",
    "  return reflex ? (afterStart || beforeEnd) : (afterStart && beforeEnd);\n",
    "}\n",
    // plan-116-A: `base` is the polygon's first-edge index into the frame buffer's edge
    // region, the same word (`ITEM_ARC_EDGE_BASE`) Vulkan has always carried. Metal used
    // to leave it zero because `setFragmentBytes:` copied each item's edges into the
    // command buffer, so every polygon's array started at 0; one buffer now serves the
    // whole frame, so each polygon reads its own slice.
    "static float edgeDistance(constant int *edges, int base, int count, float2 p) {\n",
    "  float best = 1.0e6;\n",
    "  bool inside = false;\n",
    "  for (int e = 0; e < count; ++e) {\n",
    "    int i = (base + e) * 4 + METAL_EDGE_BASE;\n",
    "    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "    float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));\n",
    "    best = min(best, segmentDistance(p, a, b));\n",
    "    if ((a.y > p.y) != (b.y > p.y)) {\n",
    "      float u = (p.y - a.y) / (b.y - a.y);\n",
    "      if (p.x < a.x + u * (b.x - a.x)) inside = !inside;\n",
    "    }\n",
    "  }\n",
    "  return inside ? -best : best;\n",
    "}\n",
    // plan-116-C: the inverse transform, decoded from the float32 bits the item block
    // carries. `as_type<float>` is a reinterpret, not a conversion -- the CPU already
    // did the narrowing (`__canvas_float32Bits`), because this compiler's assemblers
    // have no double->single convert.
    "static bool hasTransform(constant MfbItem &item) { return item.xform1.z != 0; }\n",
    "static float2 inverseMap(constant MfbItem &item, float2 p) {\n",
    "  return float2(as_type<float>(item.xform0.x) * p.x + as_type<float>(item.xform0.z) * p.y + as_type<float>(item.xform1.x),\n",
    "                as_type<float>(item.xform0.y) * p.x + as_type<float>(item.xform0.w) * p.y + as_type<float>(item.xform1.y));\n",
    "}\n",
    "static float geoDistance(constant MfbItem &item, constant int *edges, float2 p) {\n",
    "  float radius = fx(item.misc.y);\n",
    "  float2 c = float2(fx(item.shape.x), fx(item.shape.y));\n",
    "  if (item.misc.x == 0) {\n",
    "    return rectDistance(p, c, float2(fx(item.shape.z), fx(item.shape.w))) - radius;\n",
    "  }\n",
    "  if (item.misc.x == 1) { return length(p - c) - fx(item.shape.z) - radius; }\n",
    // Round is 1 and is what a Line did before plan-116-D, so it reads as the straight
    // path; the butt arm returns the finished band distance and does not subtract
    // `radius` again.
    "  if (item.misc.x == 2) {\n",
    "    if (item.arc.w == 1) {\n",
    "      return segmentDistance(p, c, float2(fx(item.shape.z), fx(item.shape.w))) - radius;\n",
    "    }\n",
    "    return segmentDistanceButt(p, c, float2(fx(item.shape.z), fx(item.shape.w)), radius);\n",
    "  }\n",
    "  if (item.misc.x == 7) {\n",
    "    return ellipseDistance(p, c, fx(item.shape.z), fx(item.shape.w),\n",
    "                           fx(item.ellipse.x), fx(item.ellipse.y)) - radius;\n",
    "  }\n",
    "  if (item.misc.x == 3) {\n",
    "    float2 d = p - c;\n",
    "    float a0 = fx(item.arc.x), a1 = fx(item.arc.y);\n",
    "    float2 s = float2(cos(a0), sin(a0));\n",
    "    float2 e = float2(cos(a1), sin(a1));\n",
    "    float band = arcInSweep(d, s, e, (a1 - a0) > PI)\n",
    "        ? abs(length(d) - fx(item.shape.z)) - radius : 1.0e6;\n",
    // Butt is 0 and is what an Arc did before plan-116-D, so it returns untouched;
    // Round unions a disc of the stroke's half-width at each sweep endpoint, and a
    // union of SDFs is their min. The endpoints are per-shape constants the CPU wrote,
    // so this costs two distances and no trigonometry. Twin of the block in
    // `runtime/canvas/shaders/mfb_canvas.frag`.
    "    if (item.arc.w == 0) { return band; }\n",
    "    float2 cs = float2(fx(item.arcCaps.x), fx(item.arcCaps.y));\n",
    "    float2 ce = float2(fx(item.arcCaps.z), fx(item.arcCaps.w));\n",
    "    band = min(band, length(p - cs) - radius);\n",
    "    return min(band, length(p - ce) - radius);\n",
    "  }\n",
    "  return edgeDistance(edges, item.arc.z, item.misc.w, p);\n",
    "}\n",
    // The shape-space distance and the local scale of the mapping, as (d, s).
    //
    // Untransformed this is the distance and 1.0 -- the single evaluation the shader
    // always did. Transformed it is the distance at the inverse-mapped point and
    // ||grad d|| by CENTRAL DIFFERENCES at epsilon 0.5, so the /2e divisor is exactly 1.
    // The epsilon is part of the specified result, not a tuning knob: the oracle uses
    // the same one and Phase 1's measurement is against this value. Central differences
    // rather than fwidth for the reason 06_canvas.md gives -- a hardware derivative
    // differs between platforms; this uses only + - * / and sqrt.
    "static float2 shapeDistanceAndScale(constant MfbItem &item, constant int *edges, float2 p) {\n",
    "  if (!hasTransform(item)) { return float2(geoDistance(item, edges, p), 1.0); }\n",
    "  float d = geoDistance(item, edges, inverseMap(item, p));\n",
    "  float gx = geoDistance(item, edges, inverseMap(item, p + float2(0.5, 0.0)))\n",
    "           - geoDistance(item, edges, inverseMap(item, p - float2(0.5, 0.0)));\n",
    "  float gy = geoDistance(item, edges, inverseMap(item, p + float2(0.0, 0.5)))\n",
    "           - geoDistance(item, edges, inverseMap(item, p - float2(0.0, 0.5)));\n",
    "  float g = sqrt(gx * gx + gy * gy);\n",
    "  return float2(d, g > 0.000001 ? g : 1.0);\n",
    "}\n",
    "static float srgbToLinear(float c) {\n",
    "  c = c / 255.0;\n",
    "  return c <= 0.04045 ? (c / 12.92) : pow((c + 0.055) / 1.055, 2.4);\n",
    "}\n",
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
    "static float srgbTable(int i) {\n",
    "  return floor(srgbToLinear(float(i)) * 65535.0 + 0.5);\n",
    "}\n",
    // One channel, reproducing `__canvas_gradientChannel` exactly. `num` is already
    // quantised to 1/4096 by TRUNCATION because the oracle quantises there; a
    // continuous fraction here biases the whole ramp by up to a step. The answer is the
    // byte nearest in LINEAR value, the rule the oracle's binary search applies.
    "static int gradientChannel(int loSrgb, int hiSrgb, int num) {\n",
    "  float loLin = srgbTable(loSrgb);\n",
    "  float hiLin = srgbTable(hiSrgb);\n",
    "  float lin = loLin + trunc((hiLin - loLin) * float(num) / 4096.0);\n",
    "  float e = lin <= 205.4 ? (lin / 65535.0 * 12.92)\n",
    "                         : (1.055 * pow(lin / 65535.0, 1.0 / 2.4) - 0.055);\n",
    "  int c = int(clamp(e * 255.0, 0.0, 255.0));\n",
    "  if (c > 0 && lin <= floor((srgbTable(c - 1) + srgbTable(c)) * 0.5)) {\n",
    "    return c - 1;\n",
    "  }\n",
    "  if (c < 255 && lin > floor((srgbTable(c) + srgbTable(c + 1)) * 0.5)) {\n",
    "    return c + 1;\n",
    "  }\n",
    "  return c;\n",
    "}\n",
    "static int4 gradientColour(float2 p, constant int *edges, constant MfbItem &item) {\n",
    "  int count = item.ellipse.z;\n",
    "  int base = METAL_GRADIENT_BASE + item.ellipse.w * 5;\n",
    "  float2 gfrom = float2(fx(item.gradient.x), fx(item.gradient.y));\n",
    "  float2 gto = float2(fx(item.gradient.z), fx(item.gradient.w));\n",
    "  float2 axis = gto - gfrom;\n",
    "  float len2 = dot(axis, axis);\n",
    "  float t = 0.0;\n",
    "  if (item.gradientKind == 1) {\n",
    "    float len = sqrt(len2);\n",
    "    if (len > 0.0) { t = length(p - gfrom) / len; }\n",
    "  } else {\n",
    "    if (len2 > 0.0) { t = dot(p - gfrom, axis) / len2; }\n",
    "  }\n",
    "  t = clamp(t, 0.0, 1.0);\n",
    "  int idx = count;\n",
    "  for (int i = 0; i < count; ++i) {\n",
    "    if (idx == count && fx(edges[base + i * 5]) >= t) { idx = i; }\n",
    "  }\n",
    "  if (idx >= count) {\n",
    "    int last = base + (count - 1) * 5;\n",
    "    return int4(edges[last + 1], edges[last + 2], edges[last + 3], edges[last + 4]);\n",
    "  }\n",
    "  if (idx <= 0) {\n",
    "    return int4(edges[base + 1], edges[base + 2], edges[base + 3], edges[base + 4]);\n",
    "  }\n",
    "  int lo = base + (idx - 1) * 5;\n",
    "  int hi = base + idx * 5;\n",
    "  float loOff = fx(edges[lo]);\n",
    "  float hiOff = fx(edges[hi]);\n",
    // Quantised to 1/4096 by truncation, exactly as `__canvas_gradientColor` does.
    "  int num = hiOff > loOff ? int((t - loOff) / (hiOff - loOff) * 4096.0) : 0;\n",
    "  num = clamp(num, 0, 4096);\n",
    "  int loA = edges[lo + 4];\n",
    "  int hiA = edges[hi + 4];\n",
    "  return int4(gradientChannel(edges[lo + 1], edges[hi + 1], num),\n",
    "              gradientChannel(edges[lo + 2], edges[hi + 2], num),\n",
    "              gradientChannel(edges[lo + 3], edges[hi + 3], num),\n",
    "              loA + int(trunc(float(hiA - loA) * float(num) / 4096.0)));\n",
    "}\n",
    "static float4 covered(int4 rgba, int coverage) {\n",
    "  float a = float((rgba.w * coverage) / 255) / 255.0;\n",
    "  return float4(srgbToLinear(float(rgba.x)) * a,\n",
    "                srgbToLinear(float(rgba.y)) * a,\n",
    "                srgbToLinear(float(rgba.z)) * a, a);\n",
    "}\n",
    "static float4 premultiplied(int4 rgba, float distance) {\n",
    // The oracle quantizes coverage to a whole 0..255 and then takes an integer
    // `(colourAlpha * coverage) / 255`. Matching that here rather than blending in
    // float is not pedantry: near full coverage the sRGB encode is so steep that ONE
    // step of coverage moves a dark channel by up to 13 output steps, so a float
    // coverage that merely rounds differently produces a visible disagreement on
    // every antialiased edge. Quantizing the same way leaves only the pixels within
    // a rounding boundary of each other.
    "  return covered(rgba, int(clamp(0.5 - distance, 0.0, 1.0) * 255.0 + 0.5));\n",
    "}\n",
    // The clip's own antialiased coverage, 0..255 (plan-116-B). The same rectangle SDF
    // and the same quantization the shape edges use, so a fractional clip edge is
    // antialiased identically to a shape edge -- which is what lets the oracle and this
    // shader agree on it rather than merely come close. `__canvas_clipCoverage` in
    // `helper_items.rs` and `clipCoverage` in `mfb_canvas.frag` are the same three
    // lines. A zero-area rectangle means unclipped and returns 255, matching
    // `__canvas_hasClip`: testing both extents also rejects a negative one, which
    // `Bounds` cannot forbid.
    "static int clipCoverage(constant MfbItem &item, float2 p) {\n",
    "  if (item.clip.x >= item.clip.z || item.clip.y >= item.clip.w) { return 255; }\n",
    "  float2 lo = float2(fx(item.clip.x), fx(item.clip.y));\n",
    "  float2 hi = float2(fx(item.clip.z), fx(item.clip.w));\n",
    "  float d = rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5);\n",
    "  return int(clamp(0.5 - d, 0.0, 1.0) * 255.0 + 0.5);\n",
    "}\n",
    // `items` and `edges` are the SAME buffer bound twice, at offset zero both times:
    // one view typed as blocks, one as raw words. Two bindings rather than one pointer
    // cast because the edge region is reached by a word index and the block region by a
    // struct index, and letting each keep its own element type is what stops a packing
    // mistake -- the class of bug that yields a plausible wrong picture, not a fault.
    // The glyph bitmap stays a per-glyph `setFragmentBytes:` payload: a text item is
    // already N separate draws, so its payload never has to survive an instanced run.
    "fragment float4 mfbFragment(VOut in [[stage_in]],\n",
    "                            constant MfbItem *items [[buffer(0)]],\n",
    "                            constant int *edges [[buffer(1)]],\n",
    "                            constant uchar *glyph [[buffer(2)]]) {\n",
    "  constant MfbItem &item = items[in.item];\n",
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
    "  int clipCov = clipCoverage(item, in.pos.xy);\n",
    // plan-116-C section 4.5: a transformed glyph samples its bitmap at the
    // inverse-mapped point, nearest. The index arithmetic is already whole-pixel, so
    // mapping the query point is the whole change -- the cache stays untransformed and
    // one entry serves every transform.
    "  if (item.misc.x == 6) {\n",
    "    float2 gp = hasTransform(item) ? inverseMap(item, in.pos.xy) : in.pos.xy;\n",
    // `floor`, not a cast: a cast truncates toward zero, and a transformed glyph maps
    // to NEGATIVE shape-space coordinates (ink runs up from the pen), where truncation
    // picks the texel on the wrong side. Untransformed, `gp` is a surface pixel centre
    // and always positive, so this is the same value the cast gave.
    "    int ix = int(floor(gp.x)) - item.shape.x;\n",
    "    int iy = int(floor(gp.y)) - item.shape.y;\n",
    "    int cov = (ix < 0 || iy < 0 || ix >= item.misc.w || iy >= item.arc.x)\n",
    "      ? 0 : int(glyph[iy * item.misc.w + ix]);\n",
    "    return covered(item.fill, (cov * clipCov) / 255);\n",
    "  }\n",
    // `dRaw` is in SHAPE space and `dScale` the local scale; the fill uses the surface
    // distance and the stroke subtracts `half` BEFORE converting, so the outline scales
    // with the shape (section 4.3). Untransformed, `dScale` is 1.0 and both collapse to
    // the expressions this shader had.
    "  float2 ds = shapeDistanceAndScale(item, edges, in.pos.xy);\n",
    "  float dRaw = ds.x;\n",
    "  float dScale = ds.y;\n",
    "  float d = dRaw / dScale;\n",
    // plan-116-F: the gradient replaces the fill COLOUR and nothing else.
    "  int4 fillRgba = item.ellipse.z >= 2 ? gradientColour(in.pos.xy, edges, item) : item.fill;\n",
    "  float4 colour = covered(fillRgba,\n",
    "    (int(clamp(0.5 - d, 0.0, 1.0) * 255.0 + 0.5) * clipCov) / 255);\n",
    "  float halfWidth = fx(item.misc.z);\n",
    "  if (halfWidth > 0.0) {\n",
    // plan-116-B: this stroke-over-fill composition equals the oracle's two sequential
    // blends only under `Normal`, which is why a non-`Normal` item that both fills and
    // strokes is emitted as two adjacent instances (`emit_split_or_publish`). By the
    // time such an item reaches here it is fill-only or stroke-only.
    "    float4 s = covered(item.stroke,\n",
    "      (int(clamp(0.5 - (abs(dRaw) - halfWidth) / dScale, 0.0, 1.0) * 255.0 + 0.5) * clipCov) / 255);\n",
    "    colour = s + colour * (1.0 - s.w);\n",
    "  }\n",
    "  return colour;\n",
    "}\n",
);

/// The MSL source, as a C string data object.
pub(super) const STR_METAL_SHADER: (&str, &str) = ("_mfb_macapp_metal_shader", METAL_SHADER_SOURCE);
/// The two entry-point names, looked up in the compiled library.
pub(super) const STR_METAL_VERTEX_FN: (&str, &str) = ("_mfb_macapp_metal_vertex_fn", "mfbVertex");
pub(super) const STR_METAL_FRAGMENT_FN: (&str, &str) =
    ("_mfb_macapp_metal_fragment_fn", "mfbFragment");

pub(super) const SEL_NEW_COMMAND_QUEUE: (&str, &str) =
    ("_mfb_macapp_sel_newCommandQueue", "newCommandQueue");
pub(super) const SEL_NEW_LIBRARY_WITH_SOURCE: (&str, &str) = (
    "_mfb_macapp_sel_newLibraryWithSource",
    "newLibraryWithSource:options:error:",
);
pub(super) const SEL_NEW_FUNCTION_WITH_NAME: (&str, &str) = (
    "_mfb_macapp_sel_newFunctionWithName",
    "newFunctionWithName:",
);
pub(super) const SEL_SET_VERTEX_FUNCTION: (&str, &str) =
    ("_mfb_macapp_sel_setVertexFunction", "setVertexFunction:");
pub(super) const SEL_SET_FRAGMENT_FUNCTION: (&str, &str) = (
    "_mfb_macapp_sel_setFragmentFunction",
    "setFragmentFunction:",
);
pub(super) const SEL_COLOR_ATTACHMENTS: (&str, &str) =
    ("_mfb_macapp_sel_colorAttachments", "colorAttachments");
pub(super) const SEL_OBJECT_AT_INDEXED: (&str, &str) = (
    "_mfb_macapp_sel_objectAtIndexedSubscript",
    "objectAtIndexedSubscript:",
);
pub(super) const SEL_SET_PIXEL_FORMAT: (&str, &str) =
    ("_mfb_macapp_sel_setPixelFormat", "setPixelFormat:");
pub(super) const SEL_SET_BLENDING_ENABLED: (&str, &str) =
    ("_mfb_macapp_sel_setBlendingEnabled", "setBlendingEnabled:");
pub(super) const SEL_SET_SRC_RGB_FACTOR: (&str, &str) = (
    "_mfb_macapp_sel_setSourceRGBBlendFactor",
    "setSourceRGBBlendFactor:",
);
pub(super) const SEL_SET_SRC_ALPHA_FACTOR: (&str, &str) = (
    "_mfb_macapp_sel_setSourceAlphaBlendFactor",
    "setSourceAlphaBlendFactor:",
);
pub(super) const SEL_SET_DST_RGB_FACTOR: (&str, &str) = (
    "_mfb_macapp_sel_setDestinationRGBBlendFactor",
    "setDestinationRGBBlendFactor:",
);
pub(super) const SEL_SET_DST_ALPHA_FACTOR: (&str, &str) = (
    "_mfb_macapp_sel_setDestinationAlphaBlendFactor",
    "setDestinationAlphaBlendFactor:",
);
pub(super) const SEL_NEW_PIPELINE_STATE: (&str, &str) = (
    "_mfb_macapp_sel_newRenderPipelineState",
    "newRenderPipelineStateWithDescriptor:error:",
);

pub(super) const SEL_TEXTURE_2D_DESCRIPTOR: (&str, &str) = (
    "_mfb_macapp_sel_texture2DDescriptor",
    "texture2DDescriptorWithPixelFormat:width:height:mipmapped:",
);
pub(super) const SEL_SET_USAGE: (&str, &str) = ("_mfb_macapp_sel_setUsage", "setUsage:");
pub(super) const SEL_SET_STORAGE_MODE: (&str, &str) =
    ("_mfb_macapp_sel_setStorageMode", "setStorageMode:");
pub(super) const SEL_NEW_TEXTURE_WITH_DESCRIPTOR: (&str, &str) = (
    "_mfb_macapp_sel_newTextureWithDescriptor",
    "newTextureWithDescriptor:",
);
pub(super) const SEL_RENDER_PASS_DESCRIPTOR: (&str, &str) = (
    "_mfb_macapp_sel_renderPassDescriptor",
    "renderPassDescriptor",
);
pub(super) const SEL_SET_TEXTURE: (&str, &str) = ("_mfb_macapp_sel_setTexture", "setTexture:");
pub(super) const SEL_SET_LOAD_ACTION: (&str, &str) =
    ("_mfb_macapp_sel_setLoadAction", "setLoadAction:");
pub(super) const SEL_SET_STORE_ACTION: (&str, &str) =
    ("_mfb_macapp_sel_setStoreAction", "setStoreAction:");
pub(super) const SEL_COMMAND_BUFFER: (&str, &str) =
    ("_mfb_macapp_sel_commandBuffer", "commandBuffer");
pub(super) const SEL_RENDER_COMMAND_ENCODER: (&str, &str) = (
    "_mfb_macapp_sel_renderCommandEncoder",
    "renderCommandEncoderWithDescriptor:",
);
pub(super) const SEL_SET_RENDER_PIPELINE_STATE: (&str, &str) = (
    "_mfb_macapp_sel_setRenderPipelineState",
    "setRenderPipelineState:",
);
/// The only draw this backend issues (plan-116-A) — one call for a whole run of
/// consecutive non-text items, and one per glyph.
///
/// It replaced `drawPrimitives:vertexStart:vertexCount:` and
/// `setVertexBytes:length:atIndex:` outright, and both are *deleted* rather than kept
/// for a caller that might want them: every selector in `metal_data_objects` is a C
/// string emitted into every canvas binary and registered with the ObjC runtime at
/// startup, so an unsent one is not free.
///
/// `baseInstance:` is the load-bearing part, not `instanceCount:`: it is what lets a run
/// that begins partway through the item buffer name its own blocks. MSL's
/// `[[instance_id]]` **already includes it** — the same property Vulkan's
/// `gl_InstanceIndex` has — so the shader indexes with `[[instance_id]]` alone and adds
/// nothing. That is measured rather than assumed; plan-116-A predicted the opposite, and
/// the extra `[[base_instance]]` it called for double-counted (Correction C5).
///
/// The alternative — binding the buffer at `base * ITEM_BLOCK_SIZE` — would put an
/// `MTLBuffer` offset-alignment requirement on a stride that does not meet it
/// (`ITEM_BLOCK_SIZE`, 208 since plan-116-F; it was 112 when this was written, and
/// neither value meets the alignment, so the conclusion is unchanged).
pub(super) const SEL_DRAW_PRIMITIVES_INSTANCED: (&str, &str) = (
    "_mfb_macapp_sel_drawPrimitivesInstanced",
    "drawPrimitives:vertexStart:vertexCount:instanceCount:baseInstance:",
);
/// `[device newBufferWithLength:options:]` for the frame buffer, and `contents` for
/// the CPU pointer it is written through.
pub(super) const SEL_NEW_BUFFER: (&str, &str) = (
    "_mfb_macapp_sel_newBufferWithLength",
    "newBufferWithLength:options:",
);
pub(super) const SEL_CONTENTS: (&str, &str) = ("_mfb_macapp_sel_contents", "contents");
/// Bind the frame buffer to a stage, once per frame rather than once per item.
pub(super) const SEL_SET_VERTEX_BUFFER: (&str, &str) = (
    "_mfb_macapp_sel_setVertexBuffer",
    "setVertexBuffer:offset:atIndex:",
);
pub(super) const SEL_SET_FRAGMENT_BUFFER: (&str, &str) = (
    "_mfb_macapp_sel_setFragmentBuffer",
    "setFragmentBuffer:offset:atIndex:",
);
pub(super) const SEL_END_ENCODING: (&str, &str) = ("_mfb_macapp_sel_endEncoding", "endEncoding");
pub(super) const SEL_COMMIT: (&str, &str) = ("_mfb_macapp_sel_commit", "commit");
pub(super) const SEL_WAIT_UNTIL_COMPLETED: (&str, &str) =
    ("_mfb_macapp_sel_waitUntilCompleted", "waitUntilCompleted");
pub(super) const SEL_GET_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_getBytes",
    "getBytes:bytesPerRow:fromRegion:mipmapLevel:",
);

pub(super) const SEL_SET_FRAGMENT_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_setFragmentBytes",
    "setFragmentBytes:length:atIndex:",
);

pub(crate) const CLASS_MTL_TEXTURE_DESCRIPTOR: &str = "_OBJC_CLASS_$_MTLTextureDescriptor";
pub(crate) const CLASS_MTL_RENDER_PASS_DESCRIPTOR: &str = "_OBJC_CLASS_$_MTLRenderPassDescriptor";

pub(crate) const CLASS_MTL_RENDER_PIPELINE_DESCRIPTOR: &str =
    "_OBJC_CLASS_$_MTLRenderPipelineDescriptor";

/// `MTLPixelFormatBGRA8Unorm_sRGB`. The GPU applies the sRGB encode on write, which
/// is the same transform the software path's `__COLOR_SRGB` table applies on the
/// way out — so the two agree by construction rather than by a matching pair of
/// hand-written conversions. It is also a `CAMetalLayer`-supported format, so the
/// offscreen target this pipeline is proved against and the on-screen drawable it
/// eventually presents to use *one* pipeline, not two that could drift.
pub(super) const MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB: &str = "81";
/// `MTLBlendFactorOne` — premultiplied source.
const MTL_BLEND_FACTOR_ONE: &str = "1";
/// `MTLBlendFactorOneMinusSourceAlpha`.
const MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: &str = "5";
/// `MTLBlendFactorOneMinusSourceColor` (3) and `MTLBlendFactorDestinationColor` (6) —
/// the two extra factors plan-116-B's `Screen` and `Multiply` pipelines need.
///
/// From `MTLBlendFactor`: 0 Zero, 1 One, 2 SourceColor, 3 OneMinusSourceColor,
/// 4 SourceAlpha, 5 OneMinusSourceAlpha, 6 DestinationColor. Note these are NOT the
/// same numbers as Vulkan's `VkBlendFactor` — there `DstColor` is 4 and
/// `OneMinusSrcAlpha` is 7 — so the two backends' tables cannot be copied between each
/// other, and a value that looks familiar from the other file is probably wrong.
const MTL_BLEND_FACTOR_ONE_MINUS_SRC_COLOR: &str = "3";
const MTL_BLEND_FACTOR_DST_COLOR: &str = "6";

/// `MTLResourceStorageModeShared` — CPU and GPU see one allocation, no explicit copy.
///
/// The frame buffer is written by the CPU every frame and read by the GPU in the same
/// frame, which is exactly what shared storage is for. `Managed` would need a
/// `didModifyRange:` per frame and `Private` could not be written from the CPU at all.
const MTL_RESOURCE_STORAGE_MODE_SHARED: &str = "0";

/// `int _mfb_macapp_metal_init(void)` — build the device, queue and pipeline once.
///
/// Returns 1 once the pipeline exists and 0 if any step failed, and remembers which
/// in `GRAPHICS_OFFSET_MTL_READY` so a machine with no Metal device pays the probe
/// and the shader compile once rather than per frame. Failure is a real outcome
/// rather than an abort: no Metal device, or an MSL compile error, must fall back to
/// the software renderer rather than take the program down.
///
/// The `error:` out-parameters are passed `NULL`. That is deliberate and not
/// laziness: an `NSError**` would have to be read, formatted and routed somewhere
/// from the graphics thread, which has no console, and the actionable signal — "the
/// pipeline did not build, use software" — is already the return value.
pub(super) fn emit_metal_init() -> CodeFunction {
    let mut asm = Asm::new(METAL_INIT_SYMBOL);
    let frame = 64;
    let fail = format!("{METAL_INIT_SYMBOL}_fail");
    let done = format!("{METAL_INIT_SYMBOL}_done");
    let build = format!("{METAL_INIT_SYMBOL}_build");
    // `LOCAL[5]` joins the saved set for plan-116-B: the four-pipeline loop needs a
    // register that survives `local_address` to hold each handle between creating it
    // and storing it. The frame is 64 bytes and offsets 48 and 56 were spare.
    let saves: [(&str, usize); 6] = [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
    ];

    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in saves {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }

    // Already tried? Report what happened last time. `ready` is 0 = untried,
    // 1 = built, 2 = failed, so a failed probe is remembered as a fact rather than
    // re-derived from `pipeline == 0` (which is also what "untried" looks like).
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::LOCAL[1],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_READY,
    ));
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&build));
    // Both answers are materialized before the compare that selects between them, so
    // nothing sits between the `cmp` and its branch. Putting a `mov` there would work
    // today — AArch64 `movz` leaves the flags alone — but it makes the branch depend
    // on a property of an instruction chosen elsewhere.
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "1"));
    asm.push(abi::compare_immediate(abi::LOCAL[1], "1"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&build));

    // device = MTLCreateSystemDefaultDevice()
    asm.call_external(MTL_CREATE_DEVICE, LIB_METAL);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&fail));
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0))); // device

    // queue = [device newCommandQueue]
    asm.load_selector(SEL_NEW_COMMAND_QUEUE.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&fail));
    asm.push(abi::move_register(abi::LOCAL[2], abi::c_arg(0))); // queue

    // library = [device newLibraryWithSource:@(MSL) options:nil error:NULL]
    build_nsstring_from_cstring(&mut asm, abi::LOCAL[3], STR_METAL_SHADER.0);
    asm.push(abi::move_register(abi::LOCAL[3], abi::c_arg(0))); // NSString source
    asm.load_selector(SEL_NEW_LIBRARY_WITH_SOURCE.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[3]));
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // options: nil
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0")); // error: NULL
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&fail));
    asm.push(abi::move_register(abi::LOCAL[3], abi::c_arg(0))); // library

    // descriptor = [[MTLRenderPipelineDescriptor alloc] init]
    asm.external_data(
        abi::LOCAL[4],
        CLASS_MTL_RENDER_PIPELINE_DESCRIPTOR,
        LIB_METAL,
    );
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[4]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // Park the allocation before asking for the next selector: `load_selector`
    // resolves through `sel_registerName`, whose return lands in the same register
    // the receiver has to be in, so leaving it there loses it.
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0)));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[4], abi::c_arg(0))); // descriptor

    // [descriptor setVertexFunction:[library newFunctionWithName:@"mfbVertex"]]
    // and the same for the fragment function. A missing entry point returns nil,
    // which `setVertexFunction:` would accept silently and the pipeline build would
    // then reject with an error nobody reads — so the nil is caught here.
    for (name_symbol, setter) in [
        (STR_METAL_VERTEX_FN.0, SEL_SET_VERTEX_FUNCTION.0),
        (STR_METAL_FRAGMENT_FN.0, SEL_SET_FRAGMENT_FUNCTION.0),
    ] {
        build_nsstring_from_cstring(&mut asm, abi::LOCAL[0], name_symbol);
        asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0)));
        asm.load_selector(SEL_NEW_FUNCTION_WITH_NAME.0);
        asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[0]));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
        asm.push(abi::branch_eq(&fail));
        asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0)));
        asm.load_selector(setter);
        asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[0]));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[4]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

    // attachment = [descriptor colorAttachments][0]
    asm.load_selector(SEL_COLOR_ATTACHMENTS.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[4]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0)));
    asm.load_selector(SEL_OBJECT_AT_INDEXED.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // The attachment lands in `LOCAL[3]`, not `LOCAL[0]`: it has to stay live across
    // all four pipeline creations below, and `LOCAL[0]` is the objc temporary every one
    // of those sends clobbers. `LOCAL[3]` held the shader library, which the
    // `newFunctionWithName:` sends above already consumed.
    asm.push(abi::move_register(abi::LOCAL[3], abi::c_arg(0))); // attachment

    // The colour chain that is the SAME for every mode: sRGB target, blending on, and
    // the alpha factors. The alpha pair stays `One`/`OneMinusSourceAlpha` under every
    // mode — the modes are defined on COLOUR, and the oracle writes surface alpha 255
    // everywhere, so a mode that also rewrote alpha would make the two disagree about a
    // channel neither is trying to blend.
    for (setter, value) in [
        (SEL_SET_PIXEL_FORMAT.0, MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB),
        (SEL_SET_BLENDING_ENABLED.0, "1"),
        (SEL_SET_SRC_ALPHA_FACTOR.0, MTL_BLEND_FACTOR_ONE),
        (
            SEL_SET_DST_ALPHA_FACTOR.0,
            MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        ),
    ] {
        asm.load_selector(setter);
        asm.push(abi::move_immediate(abi::c_arg(2), "Integer", value));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

    // --- one pipeline per blend mode (plan-116-B) ----------------------------------
    // A blend mode is per-PIPELINE state here exactly as it is on Vulkan: it lives on
    // the descriptor's colour attachment, which is baked in at creation. So "per-item
    // blend" is four pipelines chosen per draw, not a shader branch. All four share one
    // vertex function, one fragment function and one descriptor — only the two RGB
    // factors below differ, which is why the descriptor is edited and re-submitted
    // rather than rebuilt.
    let modes = [
        (
            0usize,
            MTL_BLEND_FACTOR_ONE,
            MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        ),
        (
            1,
            MTL_BLEND_FACTOR_DST_COLOR,
            MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        ),
        (
            2,
            MTL_BLEND_FACTOR_ONE,
            MTL_BLEND_FACTOR_ONE_MINUS_SRC_COLOR,
        ),
        (3, MTL_BLEND_FACTOR_ONE, MTL_BLEND_FACTOR_ONE),
    ];
    // The frame path indexes the pipeline array by the blend tag with no bounds check,
    // so a table shorter than the variant set binds a neighbouring state slot as a
    // pipeline handle. Tying the literal to the constant is what makes adding a
    // `BlendMode` variant fail here rather than in a frame.
    debug_assert_eq!(
        modes.len(),
        BLEND_MODE_COUNT,
        "one pipeline per BlendMode variant"
    );
    for (mode, src_rgb, dst_rgb) in modes {
        for (setter, value) in [
            (SEL_SET_SRC_RGB_FACTOR.0, src_rgb),
            (SEL_SET_DST_RGB_FACTOR.0, dst_rgb),
        ] {
            asm.load_selector(setter);
            asm.push(abi::move_immediate(abi::c_arg(2), "Integer", value));
            asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
            asm.call_external("_objc_msgSend", LIB_OBJC);
        }

        // pipeline = [device newRenderPipelineStateWithDescriptor:descriptor error:NULL]
        asm.load_selector(SEL_NEW_PIPELINE_STATE.0);
        asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[4]));
        asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
        asm.push(abi::branch_eq(&fail));
        asm.push(abi::move_register(abi::LOCAL[5], abi::c_arg(0)));
        asm.local_address(abi::c_arg(1), GRAPHICS_STATE_SYMBOL);
        asm.push(abi::store_u64(
            abi::LOCAL[5],
            abi::c_arg(1),
            GRAPHICS_OFFSET_MTL_PIPELINE_MODES + mode * 8,
        ));
        if mode == 0 {
            // `Normal`'s handle is what the publish block below stores into the legacy
            // `…_MTL_PIPELINE` slot, LAST, so a frame racing this still sees a non-zero
            // pipeline only once everything it needs is already there.
            asm.push(abi::move_register(abi::LOCAL[0], abi::LOCAL[5]));
        }
    }

    // --- the frame buffer (plan-116-A) ---------------------------------------------
    // `[device newBufferWithLength:METAL_BUFFER_BYTES options:MTLResourceStorageModeShared]`.
    // Created with the DEVICE and not with the target: its size does not depend on the
    // surface, so a resize must not tear it down and rebuild it — the same lifecycle
    // rule the Vulkan edge and item buffers follow.
    //
    // `LOCAL[3]` and `LOCAL[4]` are reused here rather than the frame growing two more
    // saves: `LOCAL[3]` held the shader library and `LOCAL[4]` the pipeline descriptor,
    // and the pipeline that consumed both was created just above, so neither is live.
    asm.load_selector(SEL_NEW_BUFFER.0);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &METAL_BUFFER_BYTES.to_string(),
    ));
    asm.push(abi::move_immediate(
        abi::c_arg(3),
        "Integer",
        MTL_RESOURCE_STORAGE_MODE_SHARED,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1])); // device
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&fail));
    asm.push(abi::move_register(abi::LOCAL[3], abi::c_arg(0))); // buffer

    // `contents` once, not once per item: the frame path writes item blocks and edges
    // through a plain pointer, and a message send per item would put an `objc_msgSend`
    // in the inner loop of every scene.
    asm.load_selector(SEL_CONTENTS.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&fail));
    asm.push(abi::move_register(abi::LOCAL[4], abi::c_arg(0))); // contents

    // Publish device, queue and pipeline for the frame path, and record success.
    // The pipeline is stored last: a frame that races this sees a non-zero pipeline
    // only once the device and queue it needs are already there — and now the frame
    // buffer too, which the frame path dereferences without checking for the same
    // reason it does not check the device.
    asm.local_address(abi::c_arg(1), GRAPHICS_STATE_SYMBOL);
    asm.push(abi::store_u64(
        abi::LOCAL[3],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_ITEM_BUFFER,
    ));
    asm.push(abi::store_u64(
        abi::LOCAL[4],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_ITEM_CONTENTS,
    ));
    asm.push(abi::store_u64(
        abi::LOCAL[1],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_DEVICE,
    ));
    asm.push(abi::store_u64(
        abi::LOCAL[2],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_QUEUE,
    ));
    asm.push(abi::store_u64(
        abi::LOCAL[0],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_PIPELINE,
    ));
    asm.push(abi::move_immediate(abi::LOCAL[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::LOCAL[0],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_READY,
    ));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "1"));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&fail));
    asm.local_address(abi::c_arg(1), GRAPHICS_STATE_SYMBOL);
    asm.push(abi::move_immediate(abi::LOCAL[0], "Integer", "2"));
    asm.push(abi::store_u64(
        abi::LOCAL[0],
        abi::c_arg(1),
        GRAPHICS_OFFSET_MTL_READY,
    ));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));

    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saves {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.metal.init".to_string(),
        symbol: METAL_INIT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// The frame renderer's symbol.
pub(super) const METAL_DRAW_SYMBOL: &str = "_mfb_macapp_metal_draw";

/// `MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget`.
const MTL_TEXTURE_USAGE: &str = "5";
/// `MTLStorageModeShared` — one allocation both the GPU renders into and the CPU
/// reads back from. On Apple Silicon there is no separate device memory to copy
/// across, so a `Managed` texture would add a blit-and-synchronize for nothing.
const MTL_STORAGE_MODE_SHARED: &str = "0";
/// `MTLLoadActionClear` / `MTLStoreActionStore`.
///
/// Clear rather than DontCare because the surface has a defined starting colour:
/// `canvas::newSurface` documents opaque black, and Metal's default clear colour is
/// exactly `(0, 0, 0, 1)` — so the two backends start from the same pixels without
/// this having to name the colour twice.
const MTL_LOAD_ACTION_CLEAR: &str = "2";
/// `setClearColor:` — the colour `MTLLoadActionClear` clears the attachment to.
///
/// Set explicitly rather than left to the default. `MTLRenderPassAttachmentDescriptor`
/// is documented to default to opaque black, but the canvas surface is composited into
/// a window through a `CALayer`, so "what the default is" decides whether an unpainted
/// pixel is black or the window showing through — and the software path is unambiguous
/// about it (`canvas::newSurface` fills opaque black). Naming it here makes the three
/// backends agree by construction instead of by three defaults happening to match.
pub(super) const SEL_SET_CLEAR_COLOR: (&str, &str) =
    ("_mfb_macapp_sel_setClearColor", "setClearColor:");
const MTL_STORE_ACTION_STORE: &str = "1";
/// `MTLPrimitiveTypeTriangleStrip` — four vertices, two triangles, no index buffer.
///
/// 4, not 3: the enum runs Point, Line, LineStrip, Triangle, TriangleStrip, so 3 is
/// the triangle *list*. A list with four vertices is not an error — it draws one
/// triangle and ignores the fourth vertex, which renders exactly half the quad and
/// looks like a geometry bug rather than an enum one.
const MTL_PRIMITIVE_TRIANGLE_STRIP: &str = "4";

// The frame. `OFF_REGION` holds the 48-byte `MTLRegion` that
// `getBytes:bytesPerRow:fromRegion:mipmapLevel:` takes by value in C. AAPCS64 rule
// B.4 turns a composite argument larger than 16 bytes into a **pointer to a
// caller-allocated copy** before register assignment ever happens, so the region is
// passed as an address in the next argument register — not spilled to an outgoing
// stack area. Getting that wrong is not a subtle mismatch: the callee dereferences
// whatever is in that register, and a zero there faults inside
// `-[IOGPUMetalTexture getBytes:…]` with none of our frames in the trace.
//
// plan-116-A: the frame no longer carries a `MAX_EDGES * 16` edge staging area. Edges
// are written straight into the frame buffer's edge region, so the stack shrinks by
// 4 KiB and the per-item `setFragmentBytes:` that copied that area into the command
// buffer is gone with it.
const DRAW_FRAME: usize = 576;
const OFF_REGION: usize = 0;
const OFF_LR: usize = 64;
const OFF_SAVES: usize = 72;
const OFF_SURFACE: usize = 136;
const OFF_WIDTH: usize = 144;
const OFF_HEIGHT: usize = 152;
const OFF_POOL: usize = 160;
const OFF_ITEM: usize = 192;
const OFF_TEXTURE: usize = 400;
/// The glyph cache's two payload pointers, and the per-glyph loop's state.
///
/// On the stack rather than in `LOCAL` registers because the glyph loop makes two
/// `objc_msgSend` calls per glyph and the low `LOCAL`s are the objc temporaries.
const OFF_GLYPH_META: usize = 408;
const OFF_GLYPH_COV: usize = 416;
const OFF_GLYPH_INDEX: usize = 424;
const OFF_GLYPH_COUNT: usize = 432;
const OFF_GLYPH_HEADER: usize = 440;
const OFF_GLYPH_W: usize = 448;
const OFF_GLYPH_H: usize = 456;
const OFF_GLYPH_X: usize = 464;
const OFF_GLYPH_Y: usize = 472;
/// The pointer handed straight to `setFragmentBytes:` — into the coverage cache
/// itself. Metal copies at record time, so the bitmap needs no staging buffer of its
/// own; the cache's bytes for one glyph are already contiguous.
const OFF_GLYPH_SRC: usize = 480;
/// `[frameBuffer contents]`, loaded once per frame from the graphics state.
///
/// Parked rather than kept in a `LOCAL`: every item makes at least one
/// `objc_msgSend`, and the low `LOCAL`s are the objc temporaries.
const OFF_CONTENTS: usize = 488;
/// The frame's item-buffer cursor — one block per drawn QUAD, so a shape takes one and
/// a glyph run takes one per glyph — and the base of the instanced run currently being
/// accumulated. `OFF_RUN_COUNT` is where the flush computes `cursor - base`, which has
/// to live somewhere the argument staging cannot clobber.
const OFF_ITEM_CURSOR: usize = 496;
const OFF_RUN_START: usize = 504;
const OFF_RUN_COUNT: usize = 512;
/// The frame's running edge cursor, in edges. Each polygon appends here and records
/// where it started in its own item block — exactly what the Vulkan emitter has always
/// done, and what Metal could not do while its edges rode a per-item payload.
const OFF_EDGE_CURSOR: usize = 520;
/// Where the glyph currently being drawn published its block, parked so the draw's
/// `baseInstance:` is staged from memory rather than from a register the staging of an
/// earlier argument would have overwritten.
const OFF_GLYPH_INSTANCE: usize = 528;
/// The blend mode currently bound, this item's, and the `strokeHalf` parked across the
/// two-instance split (plan-116-B).
///
/// `strokeHalf` is on the stack rather than in a register for a reason that cost a
/// debugging round on the Vulkan side: `emit_item_publish` uses the low `SCRATCH`
/// registers, so a value saved across it comes back as a mapped address — and as a
/// stroke width that reads like an enormous band the oracle never drew.
const OFF_BOUND_MODE: usize = 536;
const OFF_ITEM_MODE: usize = 544;
const OFF_SAVED_STROKE: usize = 552;
/// The frame's gradient-stop cursor, in STOPS — the third region's twin of
/// `OFF_EDGE_CURSOR` (plan-116-F).
const OFF_GRAD_CURSOR: usize = 560;

/// `void _mfb_macapp_metal_draw(pixels, width, height, geometry, offsets, count)` —
/// render one frame on the GPU and read it back into `pixels`.
///
/// The arguments arrive in the MFB argument registers, staged by
/// `canvas::metalDrawScene`: the surface's RGBA8 payload pointer, its dimensions, the
/// geometry cache's `Float` payload, the payload of the draw-order offset list, and
/// how many offsets there are.
///
/// It reads the frame back rather than presenting it, so the finished pixels go out
/// through the same `canvas::blitSurface` the software path uses. That is what makes
/// the backends comparable: the tolerance comparator diffs an RGBA8 buffer, and a
/// frame that only ever existed in a drawable is not one.
///
/// The whole body runs inside one autorelease pool. The graphics thread has none of
/// its own, and `renderPassDescriptor`, `commandBuffer` and
/// `renderCommandEncoderWithDescriptor:` all return autoreleased objects — without a
/// pool those do not merely leak, they abort the thread in libmalloc when it exits.
pub(super) fn emit_metal_draw() -> CodeFunction {
    let mut asm = Asm::new(METAL_DRAW_SYMBOL);
    let restore = format!("{METAL_DRAW_SYMBOL}_restore");
    let release_pool = format!("{METAL_DRAW_SYMBOL}_release_pool");
    let make_texture = format!("{METAL_DRAW_SYMBOL}_make_texture");
    let allocate_texture = format!("{METAL_DRAW_SYMBOL}_allocate_texture");
    let have_texture = format!("{METAL_DRAW_SYMBOL}_have_texture");
    let item_head = format!("{METAL_DRAW_SYMBOL}_item_head");
    let item_done = format!("{METAL_DRAW_SYMBOL}_item_done");
    let item_next = format!("{METAL_DRAW_SYMBOL}_item_next");
    let text_item = format!("{METAL_DRAW_SYMBOL}_text_item");
    let swizzle_head = format!("{METAL_DRAW_SYMBOL}_swizzle_head");
    let swizzle_done = format!("{METAL_DRAW_SYMBOL}_swizzle_done");

    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(DRAW_FRAME));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_LR,
    ));
    for slot in 0..8 {
        asm.push(abi::store_u64(
            abi::LOCAL[slot],
            abi::stack_pointer(),
            OFF_SAVES + slot * 8,
        ));
    }
    // Park the arguments before the first call clobbers them.
    asm.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::store_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        OFF_WIDTH,
    ));
    asm.push(abi::store_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        OFF_HEIGHT,
    ));
    asm.push(abi::move_register(abi::LOCAL[3], abi::mfb_arg(3))); // geometry payload
    asm.push(abi::move_register(abi::LOCAL[4], abi::mfb_arg(4))); // offsets payload
    asm.push(abi::move_register(abi::LOCAL[5], abi::mfb_arg(5))); // offset count
    for (argument, slot) in [(6usize, OFF_GLYPH_META), (7, OFF_GLYPH_COV)] {
        asm.push(abi::store_u64(
            abi::mfb_arg(argument),
            abi::stack_pointer(),
            slot,
        ));
    }

    // The pipeline, built on first use. A failure here leaves the surface exactly as
    // `canvas::newSurface` made it, which is the cleared frame — not garbage.
    asm.call_internal(METAL_INIT_SYMBOL);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&restore));

    asm.call_external("_objc_autoreleasePoolPush", LIB_OBJC);
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_POOL,
    ));

    // --- the offscreen render target, reused until the surface resizes -----------
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::LOCAL[1],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_TEXTURE,
    ));
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&make_texture));
    for (slot, parked) in [
        (GRAPHICS_OFFSET_MTL_TEX_WIDTH, OFF_WIDTH),
        (GRAPHICS_OFFSET_MTL_TEX_HEIGHT, OFF_HEIGHT),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::LOCAL[0], slot));
        asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        asm.push(abi::branch_ne(&make_texture));
    }
    asm.push(abi::store_u64(
        abi::LOCAL[1],
        abi::stack_pointer(),
        OFF_TEXTURE,
    ));
    asm.push(abi::branch(&have_texture));

    asm.push(abi::label(&make_texture));
    // Release the outgoing texture before allocating its replacement — a resize that
    // leaked one would leak the whole surface's worth of pixels per drag event.
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&allocate_texture));
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // Forget it before allocating the replacement. If that allocation fails, the
    // frame gives up — and the next frame would find this slot still pointing at the
    // texture just released, and either release it a second time or render into it.
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_TEXTURE,
    ));

    asm.push(abi::label(&allocate_texture));
    // descriptor = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:… ]
    asm.external_data(abi::LOCAL[1], CLASS_MTL_TEXTURE_DESCRIPTOR, LIB_METAL);
    asm.load_selector(SEL_TEXTURE_2D_DESCRIPTOR.0);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(3),
        abi::stack_pointer(),
        OFF_WIDTH,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(4),
        abi::stack_pointer(),
        OFF_HEIGHT,
    ));
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0")); // mipmapped: NO
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0)));
    for (setter, value) in [
        (SEL_SET_USAGE.0, MTL_TEXTURE_USAGE),
        (SEL_SET_STORAGE_MODE.0, MTL_STORAGE_MODE_SHARED),
    ] {
        asm.load_selector(setter);
        asm.push(abi::move_immediate(abi::c_arg(2), "Integer", value));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }
    // texture = [device newTextureWithDescriptor:descriptor]
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::LOCAL[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_DEVICE,
    ));
    asm.load_selector(SEL_NEW_TEXTURE_WITH_DESCRIPTOR.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[1]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_TEXTURE,
    ));
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&release_pool));
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_TEXTURE,
    ));
    for (slot, parked) in [
        (GRAPHICS_OFFSET_MTL_TEX_WIDTH, OFF_WIDTH),
        (GRAPHICS_OFFSET_MTL_TEX_HEIGHT, OFF_HEIGHT),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::LOCAL[0], slot));
    }

    asm.push(abi::label(&have_texture));

    // --- the render pass ---------------------------------------------------------
    asm.external_data(abi::LOCAL[0], CLASS_MTL_RENDER_PASS_DESCRIPTOR, LIB_METAL);
    asm.load_selector(SEL_RENDER_PASS_DESCRIPTOR.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0))); // pass descriptor

    asm.load_selector(SEL_COLOR_ATTACHMENTS.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0)));
    asm.load_selector(SEL_OBJECT_AT_INDEXED.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0))); // colour attachment

    asm.load_selector(SEL_SET_TEXTURE.0);
    asm.push(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_TEXTURE,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    for (setter, value) in [
        (SEL_SET_LOAD_ACTION.0, MTL_LOAD_ACTION_CLEAR),
        (SEL_SET_STORE_ACTION.0, MTL_STORE_ACTION_STORE),
    ] {
        asm.load_selector(setter);
        asm.push(abi::move_immediate(abi::c_arg(2), "Integer", value));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

    // [attachment setClearColor:MTLClearColorMake(0, 0, 0, 1)] — opaque black, the
    // colour `canvas::newSurface` fills and the colour the Vulkan clear value carries.
    // `MTLClearColor` is four C doubles, so on AArch64 they arrive in d0..d3 rather
    // than in the integer bank; the receiver and selector still go in x0/x1.
    //
    // `FP_SCRATCH[k]`, never the literal `"d0"`. The architecture guard
    // (`shared_lowering_names_no_physical_register`) rejects a raw register spelling in
    // an emission context, and it is right to: the neutral token is what lets the pool
    // be realized differently per target, and a literal is invisible to every later
    // sweep that reasons about register use.
    asm.load_selector(SEL_SET_CLEAR_COLOR.0);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for channel in 0..3 {
        asm.push(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[channel],
            abi::SCRATCH[0],
        ));
    }
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[0],
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // buffer = [queue commandBuffer]; encoder = [buffer renderCommandEncoder…]
    asm.local_address(abi::LOCAL[7], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::LOCAL[7],
        abi::LOCAL[7],
        GRAPHICS_OFFSET_MTL_QUEUE,
    ));
    asm.load_selector(SEL_COMMAND_BUFFER.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[7]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[7], abi::c_arg(0))); // command buffer

    asm.load_selector(SEL_RENDER_COMMAND_ENCODER.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[0]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[7]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[6], abi::c_arg(0))); // encoder

    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::LOCAL[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_PIPELINE,
    ));
    asm.load_selector(SEL_SET_RENDER_PIPELINE_STATE.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[0]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // --- the frame buffer, bound once (plan-116-A) --------------------------------
    // Once per frame, not once per item, which is the whole point: a binding that
    // changed per item could not be shared by the instances of one draw.
    //
    // The SAME buffer goes to three places — vertex index 0, fragment index 0, and
    // fragment index 1 — all at offset zero. Indices 0 read it as `MfbItem` blocks and
    // index 1 as raw words for the edge region, and each keeping its own element type
    // is what stops a hand-packed reinterpretation in the shader. Offset zero
    // throughout is deliberate too: it sidesteps `MTLBuffer` offset alignment, which a
    // 112-byte block stride would not satisfy, and it is why the edge region is reached
    // by adding `METAL_EDGE_BASE` in the shader instead.
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_ITEM_CONTENTS,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_CONTENTS,
    ));
    asm.push(abi::load_u64(
        abi::LOCAL[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_ITEM_BUFFER,
    ));
    for (setter, index) in [
        (SEL_SET_VERTEX_BUFFER.0, "0"),
        (SEL_SET_FRAGMENT_BUFFER.0, "0"),
        (SEL_SET_FRAGMENT_BUFFER.0, "1"),
    ] {
        asm.load_selector(setter);
        asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[0])); // buffer
        asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // offset
        asm.push(abi::move_immediate(abi::c_arg(4), "Integer", index));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

    // --- one quad per item -------------------------------------------------------
    asm.push(abi::move_immediate(abi::LOCAL[2], "Integer", "0"));
    // The frame's cursors: the item buffer's next free block, the run currently being
    // accumulated, and the edge region's next free edge.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    // `OFF_BOUND_MODE` starts at 0 because the once-per-frame
    // `setRenderPipelineState:` above bound `Normal`'s pipeline, so an all-`Normal`
    // scene issues exactly the one bind it always did.
    for slot in [
        OFF_ITEM_CURSOR,
        OFF_RUN_START,
        OFF_EDGE_CURSOR,
        // plan-116-F. A cursor left at the previous frame's value would put this
        // frame's stops past the region's end on the very first gradient, and the
        // over-cap arm then stores a count of 0 -- which draws the flat fill.
        OFF_GRAD_CURSOR,
        OFF_BOUND_MODE,
    ] {
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
    }
    asm.push(abi::label(&item_head));
    asm.push(abi::compare_registers(abi::LOCAL[2], abi::LOCAL[5]));
    asm.push(abi::branch_ge(&item_done));

    // header = geometry + offsets[i] * 8
    asm.push(abi::shift_left_immediate(abi::SCRATCH[0], abi::LOCAL[2], 3));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::LOCAL[4],
        abi::SCRATCH[0],
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::LOCAL[3],
        abi::SCRATCH[0],
    ));

    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));

    // --- the blend mode, before the kind fork so a glyph run takes it too -----------
    // A mode change ends the instanced run and binds that mode's pipeline. Ending the
    // run first is what preserves paint order: the quads already published draw under
    // the pipeline they were recorded with. Batching is adjacent-run only, so nothing
    // is ever reordered — a scene that alternates modes just issues more binds.
    {
        let same_mode = format!("{METAL_DRAW_SYMBOL}_same_mode");
        asm.push(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[0],
            HEADER_BLEND * 8,
        ));
        asm.push(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_ITEM_MODE,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[2],
            abi::stack_pointer(),
            OFF_BOUND_MODE,
        ));
        asm.push(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[2]));
        asm.push(abi::branch_eq(&same_mode));

        emit_run_flush(&mut asm, "mode");
        // handle = *(state + …_MTL_PIPELINE_MODES + mode * 8) — contiguous and 0-based,
        // so a shift and an add rather than a four-way branch.
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            OFF_ITEM_MODE,
        ));
        asm.push(abi::shift_left_immediate(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            3,
        ));
        asm.local_address(abi::SCRATCH[1], GRAPHICS_STATE_SYMBOL);
        asm.push(abi::add_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[1],
            abi::SCRATCH[0],
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            GRAPHICS_OFFSET_MTL_PIPELINE_MODES,
        ));
        // Parked before `load_selector`, which calls `sel_registerName` and clobbers
        // the whole scratch bank.
        asm.push(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            OFF_SAVED_STROKE,
        ));
        asm.load_selector(SEL_SET_RENDER_PIPELINE_STATE.0);
        asm.push(abi::load_u64(
            abi::c_arg(2),
            abi::stack_pointer(),
            OFF_SAVED_STROKE,
        ));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
        asm.call_external("_objc_msgSend", LIB_OBJC);

        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            OFF_ITEM_MODE,
        ));
        asm.push(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            OFF_BOUND_MODE,
        ));
        asm.push(abi::label(&same_mode));
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            OFF_GLYPH_HEADER,
        ));
    }

    // A glyph run is not one draw: it forks here, before the item block is built,
    // because the block a glyph needs describes the *glyph* and not the run.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
        HEADER_KIND * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_TEXT));
    asm.push(abi::branch_eq(&text_item));

    // The block first, then the edges: `emit_item_block` writes all four words of
    // `ITEM_OFFSET_ARC`, so an edge base stored before it would be overwritten. Same
    // ordering as the Vulkan emitter.
    emit_item_block(&mut asm);
    emit_edge_buffer(&mut asm);
    emit_gradient_buffer(&mut asm);
    // Published, not drawn. The draw happens at the end of the run this item joins,
    // which is what makes consecutive shapes one instanced draw instead of N — and
    // there is nothing left to bind per item now that the edges ride the frame buffer
    // too.
    emit_split_or_publish(&mut asm, &item_next);
    asm.push(abi::branch(&item_next));

    // A glyph run ends the instanced run: its quads are N draws rather than N instances
    // (still one block each, at the same cursor), so the shapes accumulated so far have
    // to reach the command stream before them or they would be drawn on top of the text
    // instead of under it.
    asm.push(abi::label(&text_item));
    emit_run_flush(&mut asm, "text");
    emit_glyph_draws(&mut asm);
    // The glyphs consumed item-buffer slots of their own, so the next run of shapes
    // begins after them — not where the flush above left the base. Without this the
    // trailing shapes are drawn as one run *starting at the first glyph*, so every
    // glyph quad is drawn a second time.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM_CURSOR,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_RUN_START,
    ));

    asm.push(abi::label(&item_next));
    asm.push(abi::add_immediate(abi::LOCAL[2], abi::LOCAL[2], 1));
    asm.push(abi::branch(&item_head));
    asm.push(abi::label(&item_done));

    // The scene's last run — everything published since the final glyph run, or the
    // whole frame when it contains no text. Without this the trailing shapes are
    // written into the buffer and never drawn.
    emit_run_flush(&mut asm, "tail");

    // --- submit and wait ---------------------------------------------------------
    for (selector, receiver) in [
        (SEL_END_ENCODING.0, abi::LOCAL[6]),
        (SEL_COMMIT.0, abi::LOCAL[7]),
        (SEL_WAIT_UNTIL_COMPLETED.0, abi::LOCAL[7]),
    ] {
        asm.load_selector(selector);
        asm.push(abi::move_register(abi::c_arg(0), receiver));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

    // [texture getBytes:pixels bytesPerRow:width*4 fromRegion:{0,0,0,w,h,1} mipmapLevel:0]
    asm.load_selector(SEL_GET_BYTES.0);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for offset in [OFF_REGION, OFF_REGION + 8, OFF_REGION + 16] {
        asm.push(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            offset,
        ));
    }
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_WIDTH,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_REGION + 24,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_HEIGHT,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_REGION + 32,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_REGION + 40,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "4"));
    asm.push(abi::multiply_registers(
        abi::c_arg(3),
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::add_immediate(
        abi::c_arg(4),
        abi::stack_pointer(),
        OFF_REGION,
    ));
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0")); // mipmapLevel
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_TEXTURE,
    ));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // --- BGRA -> RGBA ------------------------------------------------------------
    // The pipeline writes the layer's format, so the readback is B,G,R,A while the
    // software surface — and every consumer of it, from the blit to the goldens — is
    // R,G,B,A. Swapping here rather than giving the offscreen path its own
    // RGBA-format pipeline is what keeps "one pipeline" true: the texture this is
    // proved against and the drawable it will present to share a format.
    asm.push(abi::load_u64(
        abi::LOCAL[0],
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_WIDTH,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_HEIGHT,
    ));
    asm.push(abi::multiply_registers(
        abi::LOCAL[1],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_immediate(abi::LOCAL[2], "Integer", "0"));
    asm.push(abi::label(&swizzle_head));
    asm.push(abi::compare_registers(abi::LOCAL[2], abi::LOCAL[1]));
    asm.push(abi::branch_ge(&swizzle_done));
    asm.push(abi::load_u8(abi::SCRATCH[0], abi::LOCAL[0], 0));
    asm.push(abi::load_u8(abi::SCRATCH[1], abi::LOCAL[0], 2));
    asm.push(abi::store_u8(abi::SCRATCH[1], abi::LOCAL[0], 0));
    asm.push(abi::store_u8(abi::SCRATCH[0], abi::LOCAL[0], 2));
    asm.push(abi::add_immediate(abi::LOCAL[0], abi::LOCAL[0], 4));
    asm.push(abi::add_immediate(abi::LOCAL[2], abi::LOCAL[2], 1));
    asm.push(abi::branch(&swizzle_head));
    asm.push(abi::label(&swizzle_done));

    asm.push(abi::label(&release_pool));
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), OFF_POOL));
    asm.call_external("_objc_autoreleasePoolPop", LIB_OBJC);

    asm.push(abi::label(&restore));
    asm.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_LR,
    ));
    for slot in 0..8 {
        asm.push(abi::load_u64(
            abi::LOCAL[slot],
            abi::stack_pointer(),
            OFF_SAVES + slot * 8,
        ));
    }
    asm.push(abi::add_stack(DRAW_FRAME));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.metal.draw".to_string(),
        symbol: METAL_DRAW_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// Fill the parameter block at `sp + OFF_ITEM` from the geometry header whose
/// address is in `SCRATCH[0]`, and the edge buffer at `sp + OFF_EDGES` from its tail.
///
/// The quad is the header's **bounds**, not the shape's own extent: the bounds carry
/// the `strokeHalf + 1` pad the software rasteriser gives itself so its coverage ramp
/// has pixels to run over, and the SDF fragment stage needs exactly the same margin.
/// (Phase 1 used the exact extent because a flat fill has no ramp to make room for.)
///
/// Positions narrow to 16.16 fixed point, colours cross as the whole 0–255 values the
/// header already stores. Nothing here rounds a colour, so a fill is exact.
/// One quad per glyph, for a `__CANVAS_GEO_TEXT` item.
///
/// The Metal twin of `emit_glyph_draws` in `runtime/canvas/vulkan.rs`, and simpler for
/// one reason: `setFragmentBytes:` copies into the command buffer at record time, so a
/// glyph's bitmap can be handed over **in place** — a pointer into the coverage cache —
/// where Vulkan has to copy it into a frame-wide buffer and pass an offset. The price is
/// the payload's 4 KiB cap, which is `METAL_MAX_GLYPH_SAMPLES` and is why
/// `__canvas_metalRenderable` declines a scene with a glyph bigger than about 64x64.
///
/// The item block is built once for the run — fill, stroke and surface are the same for
/// every glyph in it — and then edited per glyph.
fn emit_glyph_draws(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_glyph_head");
    let done = format!("{METAL_DRAW_SYMBOL}_glyph_done");
    let next = format!("{METAL_DRAW_SYMBOL}_glyph_next");

    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    emit_item_block(asm);

    // The run's glyph count, and a zeroed index.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
        HEADER_AUX0 * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_GLYPH_COUNT,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_GLYPH_INDEX,
    ));

    // kind = 6, radius = 0, strokeHalf = 0. A glyph is fill-only: a stroked text item
    // became an outline polygon in the geometry builder and never reaches here.
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        GEO_KIND_TEXT,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in 1..3 {
        asm.push(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_MISC + word * 4,
        ));
    }

    asm.push(abi::label(&head));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_INDEX,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_GLYPH_COUNT,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_ge(&done));

    // run = header + HEADER_SLOTS + index * GLYPH_RUN_SLOTS, in doubles.
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &GLYPH_RUN_SLOTS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::add_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        HEADER_SLOTS * 8,
    ));
    for (slot, register) in [
        (0usize, abi::SCRATCH[2]),
        (1, abi::SCRATCH[3]),
        (2, abi::SCRATCH[4]),
    ] {
        asm.push(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[0],
            slot * 8,
        ));
        asm.push(abi::float_convert_to_signed_x(register, abi::FP_SCRATCH[1]));
    }
    // A cache entry of -1 is a glyph the eviction pass dropped after this run was
    // built: it draws nothing rather than indexing the metadata out of range.
    asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
    asm.push(abi::branch_lt(&next));

    // meta = glyphMeta + entry * GLYPH_META_SLOTS, in 8-byte Integers.
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        &GLYPH_META_SLOTS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[5],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        3,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_META,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[5],
        abi::SCRATCH[2],
    ));

    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_X0 * 8,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[3],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_X,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_Y0 * 8,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[4],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_Y,
    ));
    for (slot, parked) in [(GLYPH_META_W, OFF_GLYPH_W), (GLYPH_META_H, OFF_GLYPH_H)] {
        asm.push(abi::load_u64(abi::SCRATCH[5], abi::SCRATCH[2], slot * 8));
        asm.push(abi::store_u64(
            abi::SCRATCH[5],
            abi::stack_pointer(),
            parked,
        ));
    }
    // src = coverage + covStart — handed to `setFragmentBytes:` as it is.
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_START * 8,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_GLYPH_COV,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_SRC,
    ));

    // An empty bitmap — a space, or a glyph with no contours — draws nothing, and
    // `setFragmentBytes:length:` will not take a zero length anyway.
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_W,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[5], "0"));
    asm.push(abi::branch_le(&next));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_GLYPH_H,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "0"));
    asm.push(abi::branch_le(&next));
    // Bigger than the payload is a scene `__canvas_metalRenderable` should already have
    // declined. Skipping rather than truncating: a clipped glyph is a different glyph.
    asm.push(abi::multiply_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &METAL_MAX_GLYPH_SAMPLES.to_string(),
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[6], abi::SCRATCH[7]));
    asm.push(abi::branch_gt(&next));

    // --- the glyph's own item block ------------------------------------------------
    // quad is 16.16 like every other kind's; shape.x/.y are WHOLE pixels, because the
    // shader indexes the bitmap with them.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_X,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_GLYPH_Y,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_SHAPE,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_SHAPE + 4,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_GLYPH_W,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        OFF_GLYPH_H,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[0],
        abi::SCRATCH[2],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
    ));
    // The per-glyph quad narrows the item's quad to this one glyph's box, which is only
    // valid UNTRANSFORMED — the box is in shape space, so under a transform the GPU
    // would rasterise a region the glyph no longer occupies and draw nothing. See the
    // twin of this block in `runtime/canvas/vulkan.rs` for the cost of the alternative.
    {
        let keep_hull = format!("{METAL_DRAW_SYMBOL}_glyph_hull");
        asm.push(abi::load_u32(
            abi::SCRATCH[6],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_TRANSFORM + 24,
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[6], "0"));
        asm.push(abi::branch_ne(&keep_hull));
        for (register, word) in [
            (abi::SCRATCH[0], 0usize),
            (abi::SCRATCH[1], 1),
            (abi::SCRATCH[4], 2),
            (abi::SCRATCH[5], 3),
        ] {
            asm.push(abi::shift_left_immediate(abi::SCRATCH[6], register, 16));
            asm.push(abi::store_u32(
                abi::SCRATCH[6],
                abi::stack_pointer(),
                OFF_ITEM + ITEM_OFFSET_QUAD + word * 4,
            ));
        }
        asm.push(abi::label(&keep_hull));
    }
    // misc.w = width, arc.x = height. Metal needs no bitmap offset: its payload starts
    // at the glyph, where Vulkan's is one region shared by the whole frame.
    asm.push(abi::store_u32(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 12,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));

    // --- publish and draw ------------------------------------------------------------
    // This glyph's block goes into the frame buffer like any other quad's, and the draw
    // names it through `baseInstance:`. The index is parked *before* the publish,
    // because publishing advances the cursor past it.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM_CURSOR,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_INSTANCE,
    ));
    emit_item_publish(asm, &next);

    // The edge binding needs no per-glyph send any more: the frame buffer is bound at
    // fragment index 1 once for the whole frame, and the glyph arm returns before
    // `geoDistance` would read it anyway. The bitmap below is the ONLY per-draw payload
    // left on this path — it stays, because a text item is already N separate draws
    // (`GEO_KIND_TEXT`), so it never has to survive an instanced run.
    asm.load_selector(SEL_SET_FRAGMENT_BYTES.0);
    asm.push(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_GLYPH_SRC,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_W,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_GLYPH_H,
    ));
    asm.push(abi::multiply_registers(
        abi::c_arg(3),
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "2"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // One instance, not a run: a glyph run is N draws by design, and folding it into
    // the instancing scheme is a change of shape rather than of transport. The block
    // still rides the buffer, so `baseInstance:` is all that identifies it.
    asm.load_selector(SEL_DRAW_PRIMITIVES_INSTANCED.0);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        MTL_PRIMITIVE_TRIANGLE_STRIP,
    ));
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // vertexStart
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "4")); // vertexCount
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "1")); // instanceCount
    asm.push(abi::load_u64(
        abi::c_arg(6),
        abi::stack_pointer(),
        OFF_GLYPH_INSTANCE,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label(&next));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_INDEX,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_INDEX,
    ));
    asm.push(abi::branch(&head));
    asm.push(abi::label(&done));
}

fn emit_item_block(asm: &mut Asm) {
    let header = abi::SCRATCH[0];
    let scale = abi::FP_SCRATCH[0];
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[1]));

    // The 16.16 fields: bounds, shape parameters, corner/stroke radii, arc angles.
    for (item_offset, slots) in [
        (
            ITEM_OFFSET_QUAD,
            [
                HEADER_BOUNDS,
                HEADER_BOUNDS + 1,
                HEADER_BOUNDS + 2,
                HEADER_BOUNDS + 3,
            ],
        ),
        (
            ITEM_OFFSET_SHAPE,
            [
                HEADER_SHAPE,
                HEADER_SHAPE + 1,
                HEADER_SHAPE + 2,
                HEADER_SHAPE + 3,
            ],
        ),
        (
            ITEM_OFFSET_ARC,
            [HEADER_AUX0, HEADER_AUX1, HEADER_AUX1, HEADER_AUX1],
        ),
        // plan-116-D: an arc's two sweep endpoints, four consecutive header slots
        // narrowing to 16.16 exactly like the bounds — so the new `ivec4` rides this
        // loop rather than adding a hand-written store per coordinate. Written for
        // every kind; only a Round-capped arc reads them.
        (
            ITEM_OFFSET_ARC_CAPS,
            [
                HEADER_CAP_START_X,
                HEADER_CAP_START_X + 1,
                HEADER_CAP_END_X,
                HEADER_CAP_END_X + 1,
            ],
        ),
        // plan-116-E: an ellipse's rotation as cos, sin. The trailing pair repeats the
        // sine rather than naming an unused slot, because this loop writes four words
        // and the shader reads only x and y — the same shape the arc row above uses.
        (
            ITEM_OFFSET_ELLIPSE,
            [
                HEADER_ELLIPSE_COS,
                HEADER_ELLIPSE_SIN,
                HEADER_ELLIPSE_SIN,
                HEADER_ELLIPSE_SIN,
            ],
        ),
        // plan-116-F: the gradient's axis, four consecutive header slots narrowing to
        // 16.16 exactly like the bounds and the arc caps.
        (
            ITEM_OFFSET_GRADIENT,
            [
                HEADER_GRADIENT_FROM_X,
                HEADER_GRADIENT_FROM_X + 1,
                HEADER_GRADIENT_FROM_X + 2,
                HEADER_GRADIENT_FROM_X + 3,
            ],
        ),
        // plan-116-B: the clip is already RESOLVED to x0,y0,x1,y1 in the header, so it
        // rides this loop unchanged — four consecutive slots narrowing to 16.16 like
        // the bounds above, and no arithmetic repeated per item.
        (
            ITEM_OFFSET_CLIP,
            [
                HEADER_CLIP_X0,
                HEADER_CLIP_Y0,
                HEADER_CLIP_X1,
                HEADER_CLIP_Y1,
            ],
        ),
    ] {
        for (index, slot) in slots.into_iter().enumerate() {
            asm.push(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
            asm.push(abi::float_multiply_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                scale,
            ));
            asm.push(abi::float_round_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            asm.push(abi::store_u32(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                OFF_ITEM + item_offset + index * 4,
            ));
        }
    }

    // The whole-number fields: both colours, then kind and the edge count.
    for (item_offset, first) in [
        (ITEM_OFFSET_FILL, HEADER_FILL_R),
        (ITEM_OFFSET_STROKE, HEADER_STROKE_R),
    ] {
        for channel in 0..4 {
            asm.push(abi::load_double(
                abi::FP_SCRATCH[1],
                header,
                (first + channel) * 8,
            ));
            asm.push(abi::float_convert_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            asm.push(abi::store_u32(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                OFF_ITEM + item_offset + channel * 4,
            ));
        }
    }

    // misc = { kind, radius (16.16), strokeHalf (16.16), edgeCount }
    for (index, slot, fixed) in [
        (0usize, HEADER_KIND, false),
        (1, HEADER_RADIUS, true),
        (2, HEADER_STROKE_HALF, true),
        (3, HEADER_AUX0, false),
    ] {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        if fixed {
            asm.push(abi::float_multiply_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                scale,
            ));
            asm.push(abi::float_round_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
        } else {
            asm.push(abi::float_convert_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
        }
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_MISC + index * 4,
        ));
    }

    for (index, parked) in [OFF_WIDTH, OFF_HEIGHT].into_iter().enumerate() {
        asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_SURFACE + index * 4,
        ));
    }

    // The inverse transform (plan-116-C). The header already holds these as float32
    // BIT PATTERNS — `__canvas_float32Bits` narrowed them once, in MFBASIC, because
    // this assembler has no double→single convert — so the emitter's whole job is a
    // whole-number read and a 32-bit store. Seven slots: `ia..ity` then the flag.
    for (index, slot) in [
        HEADER_TRANSFORM_IA,
        HEADER_TRANSFORM_IB,
        HEADER_TRANSFORM_IC,
        HEADER_TRANSFORM_ID,
        HEADER_TRANSFORM_ITX,
        HEADER_TRANSFORM_ITY,
        HEADER_HAS_TRANSFORM,
    ]
    .into_iter()
    .enumerate()
    {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        asm.push(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_TRANSFORM + index * 4,
        ));
    }

    // The blend tag, a whole 0..3 beside the surface size (plan-116-B). `Normal` is 0,
    // so an item that never set `Paint.blend` writes the value the pipeline it selects
    // has always had.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_BLEND * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_SURFACE + ITEM_SURFACE_BLEND,
    ));

    // The cap tag, in the per-kind block's last free word (plan-116-D) — the twin of
    // the block in `runtime/canvas/vulkan.rs`, and unconditional for the same reason.
    // plan-116-F: the gradient's kind, a whole 0 or 1 in the block's last spare word.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_KIND * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_SURFACE + ITEM_SURFACE_GRADIENT_KIND,
    ));

    asm.push(abi::load_double(abi::FP_SCRATCH[1], header, HEADER_CAP * 8));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_CAP,
    ));
}

/// Copy the item block just built on the stack into the frame buffer at the cursor, and
/// advance the cursor by one quad.
///
/// This is what replaced the two `setVertexBytes:`/`setFragmentBytes:` sends that used
/// to push the block per draw (plan-116-A). The block is still built at `OFF_ITEM`
/// exactly as before — `emit_item_block` is untouched — and then lands in the buffer
/// instead of in the command stream, at the index the shaders read it back from
/// through `[[instance_id]]`, which includes the draw's `baseInstance:`.
///
/// **The cursor counts quads, not scene items.** A shape is one, a glyph run is one per
/// glyph. That is the same number `__canvas_metalRenderable` sums against
/// `CANVAS_MAX_FRAME_ITEMS`, so the two cannot disagree about what "full" means.
///
/// Branches to `full` without writing or advancing when the frame is at capacity —
/// unreachable, because the predicate already declined such a scene to software, and
/// kept because the alternative is a write past the buffer.
///
/// No `objc_msgSend` happens here, so the scratch bank is safe across it.
fn emit_item_publish(asm: &mut Asm, full: &str) {
    let cursor = abi::SCRATCH[0];
    let target = abi::SCRATCH[1];
    let value = abi::SCRATCH[2];

    asm.push(abi::load_u64(cursor, abi::stack_pointer(), OFF_ITEM_CURSOR));
    asm.push(abi::compare_immediate(
        cursor,
        &CANVAS_MAX_FRAME_ITEMS.to_string(),
    ));
    asm.push(abi::branch_ge(full));

    // target = contents + cursor * ITEM_BLOCK_SIZE. A multiply rather than a second
    // byte-cursor kept in step with this one: two cursors that must never diverge is
    // exactly the invariant that breaks silently.
    asm.push(abi::move_immediate(
        target,
        "Integer",
        &ITEM_BLOCK_SIZE.to_string(),
    ));
    asm.push(abi::multiply_registers(target, cursor, target));
    asm.push(abi::load_u64(value, abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::add_registers(target, value, target));

    debug_assert_eq!(
        ITEM_BLOCK_SIZE % 8,
        0,
        "the item block is copied to the buffer eight bytes at a time"
    );
    for word in 0..ITEM_BLOCK_SIZE / 8 {
        asm.push(abi::load_u64(
            value,
            abi::stack_pointer(),
            OFF_ITEM + word * 8,
        ));
        asm.push(abi::store_u64(value, target, word * 8));
    }

    asm.push(abi::add_immediate(cursor, cursor, 1));
    asm.push(abi::store_u64(
        cursor,
        abi::stack_pointer(),
        OFF_ITEM_CURSOR,
    ));
}

/// Publish this item's block — as **two** records when the fragment shader's
/// stroke-over-fill composition would not equal the oracle's two sequential blends.
///
/// The MSL composes stroke over fill in-shader and hands the hardware one source,
/// which equals the oracle's two writes only because `over` is associative. **That is
/// `Normal`-only.** The oracle applies the mode twice per pixel — fill into the
/// surface, then stroke into the result — and
/// `M(M(D, fill), stroke) = M(D, over(stroke, fill))` holds for `over` and for none of
/// `Multiply`, `Screen` or `Add` wherever the stroke band covers filled pixels.
///
/// So a non-`Normal` item that both fills and strokes becomes two adjacent records:
/// the first with `strokeHalf` zeroed (fill only), the second with the fill alpha
/// zeroed (stroke only), in that order. Each reaches the fixed-function unit as a
/// single source, and paint order is exactly the oracle's. The shader needs no change
/// — a zero `strokeHalf` skips the stroke arm, a zero fill alpha premultiplies to
/// nothing.
///
/// The twin of `emit_split_or_publish` in `runtime/canvas/vulkan.rs`; the two must
/// split the same items or the backends disagree on exactly the scenes this letter
/// adds.
fn emit_split_or_publish(asm: &mut Asm, full: &str) {
    let single = format!("{METAL_DRAW_SYMBOL}_publish_single");
    let done = format!("{METAL_DRAW_SYMBOL}_publish_done");

    // Split only when the mode is not `Normal`...
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM_MODE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&single));
    // ...and the item actually strokes (`strokeHalf` > 0, in 16.16)...
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 8,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_le(&single));
    // ...and actually fills. A `Line` or an `Arc` arrives with its stroke colour
    // already moved into the fill slots and `strokeHalf` negative
    // (`__canvas_strokeAsFill`), so it is fill-only and takes the single path.
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_FILL + 12,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_le(&single));

    // Record one: the fill, with the stroke switched off. `strokeHalf` is parked on the
    // STACK — `emit_item_publish` owns the low scratch registers, so a register saved
    // across it comes back holding a mapped address.
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 8,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SAVED_STROKE,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 8,
    ));
    emit_item_publish(asm, full);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SAVED_STROKE,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 8,
    ));

    // Record two: the stroke, with the fill made fully transparent.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_FILL + 12,
    ));
    emit_item_publish(asm, full);
    asm.push(abi::branch(&done));

    asm.push(abi::label(&single));
    emit_item_publish(asm, full);
    asm.push(abi::label(&done));
}

/// Draw every quad published since the last flush as **one instanced draw**, and start
/// a new run.
///
/// `[encoder drawPrimitives:TriangleStrip vertexStart:0 vertexCount:4
/// instanceCount:count baseInstance:base]`. MSL's `[[instance_id]]` includes
/// `baseInstance`, so each instance reads exactly the block this run published into it,
/// with no index arithmetic in the shader — see `SEL_DRAW_PRIMITIVES_INSTANCED`.
///
/// A run ends at a glyph run or at the end of the scene, and nowhere else. Nothing
/// per-item is bound between instances any more: the edges became a buffer region in
/// this same letter and the item block became one beside them, so consecutive shapes
/// have nothing left to separate them.
///
/// The encoder is read from `LOCAL[6]`, and the count and base from the stack —
/// `load_selector` calls through `sel_registerName` and clobbers the whole scratch
/// bank, so a count computed into a register before it would not survive.
fn emit_run_flush(asm: &mut Asm, label: &str) {
    let empty = format!("{METAL_DRAW_SYMBOL}_run_empty_{label}");

    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM_CURSOR,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_RUN_START,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_le(&empty));
    asm.push(abi::subtract_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_RUN_COUNT,
    ));

    asm.load_selector(SEL_DRAW_PRIMITIVES_INSTANCED.0);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        MTL_PRIMITIVE_TRIANGLE_STRIP,
    ));
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // vertexStart
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "4")); // vertexCount
    asm.push(abi::load_u64(
        abi::c_arg(5),
        abi::stack_pointer(),
        OFF_RUN_COUNT,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(6),
        abi::stack_pointer(),
        OFF_RUN_START,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label(&empty));
    // The next run starts wherever this frame has published to, whether or not anything
    // was drawn just now.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM_CURSOR,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_RUN_START,
    ));
}

/// Append a polygon's cached edge tail to the **frame buffer's edge region**, and
/// record where it landed in the item block's `ITEM_ARC_EDGE_BASE`.
///
/// **This used to be a per-item `setFragmentBytes:` payload** copied into the command
/// buffer at record time, which is why Metal's edge base was always zero: every
/// polygon's array started at 0 because every polygon got its own copy. plan-116-A had
/// to change that, and not for tidiness — an instanced draw cannot rebind a per-item
/// payload between instances, so every polygon would have ended the instanced run, and
/// plan-116-F's gradient stops and plan-116-H's one-draw-per-group would each have
/// collided with the same fact. Both backends now carry edges identically.
///
/// Runs **after** `emit_item_block`, which writes all four words of `ITEM_OFFSET_ARC`
/// and would otherwise overwrite the base this stores. That is the same ordering the
/// Vulkan emitter uses.
///
/// The header is reloaded from `OFF_GLYPH_HEADER` rather than taken from `SCRATCH[0]`,
/// because `emit_item_block` runs in between and owns the scratch bank.
///
/// The cache stores each edge as `x0, y0, dx, dy, invLenSq`; the buffer carries the two
/// **endpoints** instead, and the shader recomputes the edge vector. That is not lost
/// work: the cache keeps `invLenSq` to keep a reciprocal off the software path's
/// per-pixel loop, the GPU has the divide for free, and `invLenSq` is the one header
/// quantity 16.16 fixed point represents badly — a 100-px edge gives 1e-4, which is 6
/// in 16.16.
///
/// A non-polygon zeroes both the count and the base and reads nothing: slot 20 is the
/// arc's start angle for an arc, so walking a tail that is not there would read the
/// *next* item's header as edge coordinates.
/// Copy this item's gradient stops into the frame buffer's third region, and record
/// where they landed in the item block.
///
/// The twin of `emit_edge_buffer`, and of `emit_gradient_upload` in
/// `runtime/canvas/vulkan.rs` — one buffer, three regions, a per-item base index.
///
/// The stops sit at the END of the geometry record (`slot1 − count * 5`), because
/// `__canvas_tailFor` appends them after whatever other tail the kind has. Deriving the
/// base from the record's own length is what makes this work for a gradient-filled
/// polygon, whose tail is edges *then* stops, without the emitter knowing an edge count.
fn emit_gradient_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_grad_head");
    let done = format!("{METAL_DRAW_SYMBOL}_grad_done");
    let empty = format!("{METAL_DRAW_SYMBOL}_grad_empty");
    let convert = format!("{METAL_DRAW_SYMBOL}_grad_convert");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let scale = abi::FP_SCRATCH[0];

    asm.push(abi::load_u64(
        header,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    // Fewer than two stops is not a gradient, and the header already says so: the count
    // slot is 0 for no gradient, one stop, and any kind with no interior alike.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(count, "2"));
    asm.push(abi::branch_lt(&empty));
    // Would this item's stops fit the frame's region? Unreachable — the predicate
    // declines a frame whose stops sum past the cap — and kept for the reason the edge
    // one is: the alternative to declining is writing past the buffer.
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GRAD_CURSOR,
    ));
    asm.push(abi::add_registers(abi::SCRATCH[6], abi::SCRATCH[5], count));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[6],
        &MAX_FRAME_GRADIENT_STOPS.to_string(),
    ));
    asm.push(abi::branch_le(&convert));

    asm.push(abi::label(&empty));
    asm.push(abi::move_immediate(count, "Integer", "0"));
    asm.push(abi::store_u32(
        count,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    asm.push(abi::store_u32(
        count,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_BASE,
    ));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&convert));
    asm.push(abi::store_u32(
        count,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_BASE,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_GRAD_CURSOR,
    ));

    // source = header + (slot1 - count * 5) * 8
    asm.push(abi::load_double(abi::FP_SCRATCH[1], header, 8));
    asm.push(abi::float_convert_to_signed_x(source, abi::FP_SCRATCH[1]));
    asm.push(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &GRADIENT_STOP_WORDS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[8],
        count,
        abi::SCRATCH[7],
    ));
    asm.push(abi::subtract_registers(source, source, abi::SCRATCH[8]));
    asm.push(abi::shift_left_immediate(source, source, 3));
    asm.push(abi::add_registers(source, header, source));

    // target = contents + METAL_GRADIENT_BASE_WORDS * 4 + base * 20. The region offset
    // goes through a register: it is far past the 12-bit immediate an `add` encodes.
    asm.push(abi::multiply_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[7],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        2,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_CONTENTS,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        &(METAL_GRADIENT_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
    ));

    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[5]));
    asm.push(abi::move_immediate(index, "Integer", "0"));

    asm.push(abi::label(&head));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&done));
    // The offset in 16.16, then four whole 0..255 channels.
    asm.push(abi::load_double(abi::FP_SCRATCH[1], source, 0));
    asm.push(abi::float_multiply_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        scale,
    ));
    asm.push(abi::float_round_to_signed_x(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[5], abi::SCRATCH[6], 0));
    asm.push(abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 4));
    for channel in 1..=4usize {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], source, channel * 8));
        asm.push(abi::float_convert_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[5], abi::SCRATCH[6], 0));
        asm.push(abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 4));
    }
    asm.push(abi::add_immediate(source, source, GRADIENT_STOP_WORDS * 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&head));

    asm.push(abi::label(&done));
}

fn emit_edge_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_edge_head");
    let done = format!("{METAL_DRAW_SYMBOL}_edge_done");
    let empty = format!("{METAL_DRAW_SYMBOL}_edge_empty");
    let convert = format!("{METAL_DRAW_SYMBOL}_edge_convert");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let scale = abi::FP_SCRATCH[0];

    asm.push(abi::load_u64(
        header,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::move_immediate(count, "Integer", "0"));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_KIND * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[5], GEO_KIND_POLYGON));
    asm.push(abi::branch_ne(&empty));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_AUX0 * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(count, &MAX_EDGES.to_string()));
    asm.push(abi::branch_gt(&empty));
    // Would this polygon's edges fit the frame's region? Unreachable — the same
    // `__canvas_metalRenderable` that declines an over-long polygon now also declines a
    // frame whose polygons sum past `METAL_MAX_FRAME_EDGES`, so the whole scene went to
    // software. Kept because the alternative to declining is a write past the buffer.
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    asm.push(abi::add_registers(abi::SCRATCH[6], abi::SCRATCH[5], count));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[6],
        &METAL_MAX_FRAME_EDGES.to_string(),
    ));
    asm.push(abi::branch_le(&convert));

    // Not a polygon, over the per-item cap, or past the frame's region: draw no edges
    // and leave the base at zero. Clamping would render a *different polygon*.
    asm.push(abi::label(&empty));
    asm.push(abi::move_immediate(count, "Integer", "0"));
    asm.push(abi::store_u32(
        count,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&convert));
    // The pre-advance cursor is this polygon's first-edge index; the shader reaches its
    // slice through it. `SCRATCH[5]` still holds it.
    asm.push(abi::store_u32(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    // target = contents + METAL_EDGE_BASE_WORDS * 4 + base * 16.
    //
    // The region offset goes through a register rather than `add_immediate`: it is
    // 458752 bytes, far past the 12-bit immediate an AArch64 `add` encodes.
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        4,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_CONTENTS,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        &(METAL_EDGE_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
    ));

    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[5]));
    asm.push(abi::add_immediate(source, header, HEADER_SLOTS * 8));
    asm.push(abi::move_immediate(index, "Integer", "0"));

    asm.push(abi::label(&head));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&done));
    // out[0..1] = (x0, y0); out[2..3] = (x0 + dx, y0 + dy)
    for (slot, delta) in [(0usize, None), (1, None), (0, Some(2usize)), (1, Some(3))] {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
        if let Some(delta) = delta {
            asm.push(abi::load_double(abi::FP_SCRATCH[2], source, delta * 8));
            asm.push(abi::float_add_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[2],
            ));
        }
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[5], abi::SCRATCH[6], 0));
        asm.push(abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 4));
    }
    asm.push(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&head));
    asm.push(abi::label(&done));
}

/// The C strings this module's sends need, for the reconcile data-object list.
pub(super) fn metal_data_objects() -> Vec<(&'static str, &'static str)> {
    vec![
        STR_METAL_SHADER,
        STR_METAL_VERTEX_FN,
        STR_METAL_FRAGMENT_FN,
        SEL_NEW_COMMAND_QUEUE,
        SEL_NEW_LIBRARY_WITH_SOURCE,
        SEL_NEW_FUNCTION_WITH_NAME,
        SEL_SET_VERTEX_FUNCTION,
        SEL_SET_FRAGMENT_FUNCTION,
        SEL_COLOR_ATTACHMENTS,
        SEL_OBJECT_AT_INDEXED,
        SEL_SET_PIXEL_FORMAT,
        SEL_SET_BLENDING_ENABLED,
        SEL_SET_SRC_RGB_FACTOR,
        SEL_SET_SRC_ALPHA_FACTOR,
        SEL_SET_DST_RGB_FACTOR,
        SEL_SET_DST_ALPHA_FACTOR,
        SEL_NEW_PIPELINE_STATE,
        SEL_TEXTURE_2D_DESCRIPTOR,
        SEL_SET_USAGE,
        SEL_SET_STORAGE_MODE,
        SEL_NEW_TEXTURE_WITH_DESCRIPTOR,
        SEL_RENDER_PASS_DESCRIPTOR,
        SEL_SET_TEXTURE,
        SEL_SET_LOAD_ACTION,
        SEL_SET_STORE_ACTION,
        SEL_SET_CLEAR_COLOR,
        SEL_COMMAND_BUFFER,
        SEL_RENDER_COMMAND_ENCODER,
        SEL_SET_RENDER_PIPELINE_STATE,
        SEL_END_ENCODING,
        SEL_COMMIT,
        SEL_WAIT_UNTIL_COMPLETED,
        SEL_GET_BYTES,
        SEL_SET_FRAGMENT_BYTES,
        SEL_DRAW_PRIMITIVES_INSTANCED,
        SEL_NEW_BUFFER,
        SEL_CONTENTS,
        SEL_SET_VERTEX_BUFFER,
        SEL_SET_FRAGMENT_BUFFER,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `objc_msgSend` in the pipeline setup sets its receiver *after* the
    /// selector lookup that precedes it.
    ///
    /// This is a structural check for a real bug, not a hypothetical one: the first
    /// version of `emit_metal_init` sent `init` to the `MTLRenderPipelineDescriptor`
    /// it had just allocated without re-staging the receiver, and
    /// `Asm::load_selector` — which resolves through `sel_registerName`, whose
    /// return value lands in the receiver register — had overwritten it. The call
    /// then ran as `objc_msgSend(SEL, SEL)` and segfaulted inside `objc_msgSend`
    /// reading an isa out of a selector name, with no frame of ours in the trace.
    ///
    /// The rule is exact rather than approximate: `load_selector` leaves the
    /// selector in the *second* argument register and clobbers the first, so between
    /// a `bl _sel_registerName` and the `bl _objc_msgSend` it feeds there must be a
    /// write to the first argument register. Anything else is the bug above.
    #[test]
    fn every_msg_send_stages_its_receiver_after_the_selector_lookup() {
        for func in [emit_metal_init(), emit_metal_draw()] {
            assert_receivers_staged(&func);
        }
    }

    fn assert_receivers_staged(func: &CodeFunction) {
        let receiver = [
            render_field("dst", abi::c_arg(0)),
            render_field("dst", abi::mfb_arg(0)),
        ];
        let mut staged = false;
        let mut sends = 0usize;
        for instruction in &func.instructions {
            let target = instruction.get("target").unwrap_or_default();
            if target == "_sel_registerName" {
                staged = false;
                continue;
            }
            if target == "_objc_msgSend" {
                sends += 1;
                assert!(
                    staged,
                    "objc_msgSend #{sends} in {METAL_INIT_SYMBOL} runs with the \
                     receiver register still holding the selector \
                     `sel_registerName` returned"
                );
                continue;
            }
            if let Some(dst) = instruction.get("dst") {
                if receiver.contains(&dst) {
                    staged = true;
                }
            }
        }
        assert!(
            sends >= 12,
            "expected {} to send at least a dozen messages, saw {sends} — the walk is \
             matching nothing",
            func.symbol,
        );
    }

    /// The frame renderer pushes exactly one autorelease pool and pops it once.
    ///
    /// It runs on the graphics thread, which has no pool of its own, and
    /// `renderPassDescriptor`, `commandBuffer` and `renderCommandEncoderWithDescriptor:`
    /// are all autoreleased. Without the push those do not merely leak — the thread
    /// aborts inside libmalloc when it exits. Without the pop they accumulate a
    /// command buffer and an encoder per frame for the process lifetime.
    #[test]
    fn the_frame_renderer_balances_its_autorelease_pool() {
        let func = emit_metal_draw();
        for (symbol, expected) in [
            ("_objc_autoreleasePoolPush", 1usize),
            ("_objc_autoreleasePoolPop", 1),
        ] {
            let count = func
                .relocations
                .iter()
                .filter(|r| r.to.as_str() == symbol && r.kind == RelocIntent::Call)
                .count();
            assert_eq!(
                count, expected,
                "{} calls {symbol} {count} time(s), expected {expected}",
                func.symbol,
            );
        }
    }

    /// The frame is committed and then **waited on**, in that order, exactly once.
    ///
    /// This is what makes D's frame counter a real completion signal (plan-98-E
    /// Phase 3). `__canvas_renderLoop` advances the counter by calling
    /// `canvas::frameDone()` *after* `__canvas_renderFrame()` returns, so as long as
    /// this function does not return until `waitUntilCompleted` has, the counter
    /// cannot move before the GPU has finished — and every consumer D built on it
    /// (the scene ring's retirement gate, `MFB_CANVAS_SYNC`) inherits that ordering
    /// unchanged.
    ///
    /// It also underwrites the texture free. The offscreen target is released at the
    /// *start* of a later frame, which is after this wait returned, so no GPU work
    /// can still be reading it. Drop the wait — to make the present asynchronous, say
    /// — and both properties go with it silently: frames would still render and the
    /// tests that diff pixels would still pass, because the CPU would simply read a
    /// texture the GPU had not finished writing *most* of the time.
    #[test]
    fn the_frame_is_committed_then_waited_on() {
        let func = emit_metal_draw();
        let order: Vec<&str> = func
            .relocations
            .iter()
            .filter(|r| r.kind == RelocIntent::DataAddrHi)
            .map(|r| r.to.as_str())
            .filter(|name| *name == SEL_COMMIT.0 || *name == SEL_WAIT_UNTIL_COMPLETED.0)
            .collect();
        assert_eq!(
            order,
            vec![SEL_COMMIT.0, SEL_WAIT_UNTIL_COMPLETED.0],
            "the frame renderer must -commit the command buffer and then \
             -waitUntilCompleted it, once each and in that order; got {order:?}"
        );
    }

    /// A resize releases the outgoing texture before allocating its replacement.
    ///
    /// A leak here is a whole surface's worth of pixels per resize step — several
    /// megabytes per frame of a window drag, which is the one moment the renderer is
    /// asked to reallocate repeatedly.
    #[test]
    fn a_resize_releases_the_texture_it_replaces() {
        let func = emit_metal_draw();
        let order: Vec<&str> = func
            .relocations
            .iter()
            .filter(|r| r.kind == RelocIntent::DataAddrHi)
            .map(|r| r.to.as_str())
            .filter(|name| *name == SEL_RELEASE.0 || *name == SEL_NEW_TEXTURE_WITH_DESCRIPTOR.0)
            .collect();
        let release = order.iter().position(|name| *name == SEL_RELEASE.0);
        let allocate = order
            .iter()
            .position(|name| *name == SEL_NEW_TEXTURE_WITH_DESCRIPTOR.0)
            .expect("the frame renderer must be able to allocate a texture");
        assert_eq!(
            release,
            Some(0),
            "the -release of the outgoing texture must precede the allocation of its \
             replacement; the sends were {order:?} and the allocation is at {allocate}"
        );
    }

    /// The two entry points the setup looks up are the two the shader defines.
    ///
    /// `newFunctionWithName:` answers nil for a name the library does not export,
    /// which the setup treats as "no Metal" and silently falls back to software — so
    /// a rename on one side of this pair would not fail, it would quietly stop using
    /// the GPU.
    #[test]
    fn the_shader_defines_both_entry_points_the_setup_looks_up() {
        for (kind, name) in [
            ("vertex", STR_METAL_VERTEX_FN.1),
            ("fragment", STR_METAL_FRAGMENT_FN.1),
        ] {
            assert!(
                METAL_SHADER_SOURCE.contains(&format!("{kind} VOut {name}("))
                    || METAL_SHADER_SOURCE.contains(&format!("{kind} float4 {name}(")),
                "the MSL must define a {kind} entry point named `{name}`, which is \
                 what `newFunctionWithName:` asks for"
            );
        }
    }

    /// The frame's hand-assigned stack slots do not overlap, and the item block fits.
    ///
    /// `DRAW_FRAME` and every `OFF_*` are hand-written byte offsets, so widening
    /// anything they hold is a silent overrun rather than a compile error. plan-116-B
    /// walked straight into it: taking `ITEM_BLOCK_SIZE` from 112 to 128 made
    /// `emit_item_publish`'s copy run 16 bytes past `OFF_ITEM` into `OFF_TEXTURE`, and
    /// the symptom was an entirely BLACK GPU frame — the texture handle destroyed, so
    /// nothing was drawn into, with the renderer still reporting success.
    ///
    /// Checked as a sorted sweep rather than a list of pairwise asserts so a slot added
    /// later is covered without anyone remembering to extend this.
    #[test]
    fn the_draw_frame_slots_do_not_overlap() {
        // (offset, size, name) for every hand-assigned slot in the frame.
        let mut slots = vec![
            (OFF_REGION, 48, "region"),
            (OFF_LR, 8, "lr"),
            (OFF_SAVES, 8 * 8, "saves"),
            (OFF_SURFACE, 8, "surface"),
            (OFF_WIDTH, 8, "width"),
            (OFF_HEIGHT, 8, "height"),
            (OFF_POOL, 8, "pool"),
            (OFF_ITEM, ITEM_BLOCK_SIZE, "item"),
            (OFF_TEXTURE, 8, "texture"),
            (OFF_GLYPH_META, 8, "glyphMeta"),
            (OFF_GLYPH_COV, 8, "glyphCov"),
            (OFF_GLYPH_INDEX, 8, "glyphIndex"),
            (OFF_GLYPH_COUNT, 8, "glyphCount"),
            (OFF_GLYPH_HEADER, 8, "glyphHeader"),
            (OFF_GLYPH_W, 8, "glyphW"),
            (OFF_GLYPH_H, 8, "glyphH"),
            (OFF_GLYPH_X, 8, "glyphX"),
            (OFF_GLYPH_Y, 8, "glyphY"),
            (OFF_GLYPH_SRC, 8, "glyphSrc"),
            (OFF_CONTENTS, 8, "contents"),
            (OFF_ITEM_CURSOR, 8, "itemCursor"),
            (OFF_RUN_START, 8, "runStart"),
            (OFF_RUN_COUNT, 8, "runCount"),
            (OFF_EDGE_CURSOR, 8, "edgeCursor"),
            (OFF_GRAD_CURSOR, 8, "gradCursor"),
            (OFF_GLYPH_INSTANCE, 8, "glyphInstance"),
            (OFF_BOUND_MODE, 8, "boundMode"),
            (OFF_ITEM_MODE, 8, "itemMode"),
            (OFF_SAVED_STROKE, 8, "savedStroke"),
        ];
        slots.sort_by_key(|&(offset, _, _)| offset);

        for pair in slots.windows(2) {
            let (offset, size, name) = pair[0];
            let (next_offset, _, next_name) = pair[1];
            assert!(
                offset + size <= next_offset,
                "`{name}` at {offset} is {size} bytes, so it runs to {} and overlaps \
                 `{next_name}` at {next_offset} — a hand-assigned frame slot was \
                 widened without moving the ones above it",
                offset + size,
            );
        }

        let (last_offset, last_size, last_name) = *slots.last().expect("slots is not empty");
        assert!(
            last_offset + last_size <= DRAW_FRAME,
            "`{last_name}` runs to {} but DRAW_FRAME is only {DRAW_FRAME}",
            last_offset + last_size,
        );
        assert_eq!(DRAW_FRAME % 16, 0, "AAPCS64 wants a 16-byte-aligned frame");
    }

    /// The MSL's `METAL_EDGE_BASE` is `METAL_EDGE_BASE_WORDS`.
    ///
    /// The shader cannot see a Rust constant — `METAL_SHADER_SOURCE` is a `concat!` of
    /// string literals, so the number is spelled twice — and this is the only thing
    /// standing between the two. A disagreement would not fail anywhere: every polygon
    /// would simply read its edges from the wrong place in a buffer that is entirely
    /// valid memory, and the frame would come back with plausible-looking wrong shapes.
    /// That is the exact failure mode `the_shaders_glyph_base_matches_the_buffer_layout`
    /// exists for on the Vulkan side.
    #[test]
    fn the_metal_shader_region_bases_match_the_buffer_layout() {
        assert!(
            METAL_SHADER_SOURCE.contains(&format!(
                "constant int METAL_EDGE_BASE = {METAL_EDGE_BASE_WORDS};"
            )),
            "the MSL declares an edge-region base that is not METAL_EDGE_BASE_WORDS \
             ({METAL_EDGE_BASE_WORDS}); every polygon would read edges from the wrong \
             offset of a buffer that is entirely valid memory"
        );
        assert!(
            METAL_SHADER_SOURCE.contains(&format!(
                "constant int METAL_GRADIENT_BASE = {METAL_GRADIENT_BASE_WORDS};"
            )),
            "the MSL declares a gradient-region base that is not \
             METAL_GRADIENT_BASE_WORDS ({METAL_GRADIENT_BASE_WORDS}); every gradient \
             would read its stops from the wrong offset of a buffer that is entirely \
             valid memory, and render a plausible wrong ramp"
        );
        // A running chain rather than a sum, so a fourth region means extending it
        // rather than rewriting an equation — plan-116-F added the third and found this
        // asserting the two-region total.
        assert_eq!(
            METAL_GRADIENT_BASE_WORDS * 4,
            METAL_EDGE_BASE_WORDS * 4 + METAL_MAX_FRAME_EDGES * 16,
            "the gradient region must start where the edge region ends"
        );
        assert_eq!(
            METAL_BUFFER_BYTES,
            METAL_GRADIENT_BASE_WORDS * 4 + MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4,
            "the buffer must be exactly its three regions, with nothing past the last"
        );
    }

    /// The render target is an sRGB format.
    ///
    /// plan-98-E §3 calls this link "non-negotiable and painful to retrofit": the
    /// fragment shader emits linear premultiplied colour, so a non-sRGB target would
    /// write those linear values straight out and every pixel would come back too
    /// dark — a whole-image mismatch the tolerance comparator cannot absorb, and one
    /// that would look like a blend bug rather than a format one.
    #[test]
    fn the_pipeline_target_is_an_srgb_format() {
        assert_eq!(
            MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB, "81",
            "MTLPixelFormatBGRA8Unorm_sRGB is 81; 80 is the non-sRGB BGRA8Unorm and \
             would skip the encode the software oracle applies through __COLOR_SRGB"
        );
    }

    fn render_field(name: &'static str, operand: impl Into<Operand>) -> String {
        CodeInstruction::new("mov")
            .field(name, operand)
            .get(name)
            .expect("the field was just set")
    }
}
