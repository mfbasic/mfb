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
use crate::codegen::error::constants::error_constants::{
    COLLECTION_HEADER_SIZE, COLLECTION_OFFSET_COUNT,
};
// plan-163-B: the stroke-style words and the join-record header slots.
use crate::codegen::runtime::canvas::curve_shader::{
    curve_shader, ShaderLanguage, CURVE_SHADER_MARK,
};
use crate::codegen::runtime::canvas::metal::{LIB_METAL, MTL_CREATE_DEVICE};
use crate::codegen::runtime::canvas::{
    emit_item_arc_flags, emit_item_polygon_rule, emit_item_residual_field, emit_item_stroke_sign,
    emit_square_cap_reach, BAND_LIST_PER_EDGE, BAND_MAX, BAND_MIN_EDGES, ITEM_BAND_COUNT,
    ITEM_BAND_HEIGHT, ITEM_BAND_START, ITEM_BAND_TOP, ITEM_OFFSET_MISC_LO, ITEM_RESIDUAL_FIELDS,
    METAL_BAND_BASE_WORDS, METAL_MAX_FRAME_BAND_WORDS,
};
use crate::codegen::runtime::canvas::{
    BLEND_MODE_COUNT, EDGE_RESIDUAL_SCALE, GRAPHICS_OFFSET_MTL_DEVICE,
    GRAPHICS_OFFSET_MTL_ITEM_BUFFER, GRAPHICS_OFFSET_MTL_ITEM_CONTENTS,
    GRAPHICS_OFFSET_MTL_PIPELINE, GRAPHICS_OFFSET_MTL_PIPELINE_MODES, GRAPHICS_OFFSET_MTL_QUEUE,
    GRAPHICS_OFFSET_MTL_READY, GRAPHICS_OFFSET_MTL_TEXTURE, GRAPHICS_OFFSET_MTL_TEX_HEIGHT,
    GRAPHICS_OFFSET_MTL_TEX_WIDTH, GRAPHICS_STATE_SYMBOL, ITEM_ARC_EDGE_BASE, METAL_BUFFER_BYTES,
    METAL_EDGE_BASE_WORDS, METAL_EDGE_RESIDUAL_BASE_WORDS, METAL_MAX_FRAME_EDGES,
    METAL_MAX_FRAME_ITEMS,
};
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_ENTRY_COUNT_SHIFT, CANVAS_DRAW_ENTRY_MODE, CANVAS_DRAW_ENTRY_SHIFT,
    CURVE_PIECE_SLOTS, EDGE_SLOTS, FIXED_POINT_SCALE, GEO_KIND_CURVEPOLYGON, GEO_KIND_PICTURE,
    GEO_KIND_POLYCURVE, GEO_KIND_POLYGON, GEO_KIND_POLYLINE, GEO_KIND_TEXT, GLYPH_META_H,
    GLYPH_META_SLOTS, GLYPH_META_START, GLYPH_META_W, GLYPH_META_X0, GLYPH_META_Y0,
    GLYPH_RUN_SLOTS, GRADIENT_STOP_WORDS, HEADER_AUX0, HEADER_AUX1, HEADER_BLEND, HEADER_BOUNDS,
    HEADER_CAP, HEADER_FILL_R, HEADER_GRADIENT_COUNT, HEADER_GRADIENT_FROM_X, HEADER_GRADIENT_KIND,
    HEADER_HAS_TRANSFORM, HEADER_KIND, HEADER_PICTURE_SHADOW_HI, HEADER_PICTURE_SHADOW_LO,
    HEADER_POLYLINE_FIRST, HEADER_POLYLINE_LAST, HEADER_RADIUS, HEADER_SLOTS, HEADER_STROKE_HALF,
    HEADER_STROKE_R, HEADER_TRANSFORM_IA, HEADER_TRANSFORM_IB, HEADER_TRANSFORM_IC,
    HEADER_TRANSFORM_ID, HEADER_TRANSFORM_ITX, HEADER_TRANSFORM_ITY, ITEM_ARC_CAP,
    ITEM_ARC_GLYPH_HEIGHT, ITEM_BLOCK_SIZE, ITEM_ELLIPSE_GRADIENT_BASE,
    ITEM_ELLIPSE_GRADIENT_COUNT, ITEM_OFFSET_ARC, ITEM_OFFSET_ARC_CAPS, ITEM_OFFSET_ELLIPSE,
    ITEM_OFFSET_FILL, ITEM_OFFSET_GRADIENT, ITEM_OFFSET_MISC, ITEM_OFFSET_QUAD, ITEM_OFFSET_SHAPE,
    ITEM_OFFSET_STROKE, ITEM_OFFSET_SURFACE, ITEM_OFFSET_TRANSFORM, ITEM_SURFACE_BLEND,
    ITEM_SURFACE_GRADIENT_KIND, METAL_GLYPH_BASE_WORDS, METAL_GRADIENT_BASE_WORDS,
    METAL_MAX_FRAME_GLYPH_SAMPLES, METAL_MAX_FRAME_GRADIENT_STOPS, METAL_PICTURE_INDEX_OFFSET,
    METAL_PICTURE_INDEX_SLOTS, METAL_PICTURE_RECORDS, METAL_PICTURE_RECORDS_OFFSET,
    PICTURE_RECORD_BYTES, PICTURE_SHADOW_SPLIT_BITS,
};
use crate::codegen::runtime::canvas::{
    GRAPHICS_OFFSET_MTL_DRAWABLE_H, GRAPHICS_OFFSET_MTL_DRAWABLE_W, GRAPHICS_OFFSET_MTL_LAYER,
    GRAPHICS_OFFSET_MTL_LAYER_READY, GRAPHICS_OFFSET_MTL_LAYER_SHOW,
    GRAPHICS_OFFSET_MTL_LAYER_SHOWN,
};
use crate::codegen::runtime::canvas::{
    HEADER_CLIP_EDGE_COUNT, HEADER_CLIP_MODE, HEADER_CLIP_RULE, HEADER_DASH_COUNT,
    HEADER_DASH_PHASE, HEADER_GRADIENT_APPLY, HEADER_GRADIENT_FOCAL_X, HEADER_GRADIENT_HAS_INV,
    HEADER_GRADIENT_INV, HEADER_GRADIENT_SPREAD, HEADER_JOIN_COUNT, HEADER_JOIN_REACH,
    HEADER_PICTURE_TRANSPARENCY, ITEM_CLIP_PATH_BASE, ITEM_CLIP_PATH_COUNT, ITEM_CLIP_PATH_MODE,
    ITEM_CLIP_PATH_PICTURE_T, ITEM_OFFSET_CLIP_BAND, ITEM_OFFSET_CLIP_PATH,
    ITEM_OFFSET_GRADIENT_FOCAL, ITEM_OFFSET_GRADIENT_INV, ITEM_OFFSET_STROKE_STYLE,
    ITEM_STROKE_DASH_BASE, ITEM_STROKE_DASH_COUNT, ITEM_STROKE_JOIN_BASE, ITEM_STROKE_JOIN_COUNT,
    METAL_DASH_BASE_WORDS, METAL_MAX_FRAME_DASH_WORDS,
};
use std::sync::LazyLock;

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
///
/// ## The region bases are generated, not spelled
///
/// The frame buffer's regions (`METAL_EDGE_BASE`, `METAL_BAND_BASE`,
/// `METAL_GRADIENT_BASE`, `METAL_GLYPH_BASE`) start wherever the layout constants in
/// `runtime/canvas` put them, and every one MOVES when a cap before it or
/// `ITEM_BLOCK_SIZE` changes. They
/// were integer literals in a `concat!` until bug-686, kept equal to the Rust layout
/// only by `the_metal_shader_region_bases_match_the_buffer_layout`; a stale one does
/// not fail anywhere — every polygon, ramp or glyph reads from the wrong offset of a
/// buffer that is entirely valid memory. The source is now formatted once, on first
/// use, with the numbers taken from the constants themselves, so there is no second
/// spelling to fall out of step. The test still reads the formatted text back.
pub(super) static METAL_SHADER_SOURCE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "#include <metal_stdlib>\n\
         using namespace metal;\n\
         constant float FIXED = 65536.0;\n\
         constant float PI = 3.141592653589793;\n\
         constant int METAL_EDGE_BASE = {METAL_EDGE_BASE_WORDS};\n\
         constant int METAL_EDGE_RESIDUAL_BASE = {METAL_EDGE_RESIDUAL_BASE_WORDS};\n\
         constant int METAL_BAND_BASE = {METAL_BAND_BASE_WORDS};\n\
         constant int METAL_GRADIENT_BASE = {METAL_GRADIENT_BASE_WORDS};\n\
         constant int METAL_DASH_BASE = {METAL_DASH_BASE_WORDS};\n\
         constant int METAL_GLYPH_BASE = {METAL_GLYPH_BASE_WORDS};\n\
         {METAL_SHADER_BODY}"
    )
    .replace(CURVE_SHADER_MARK, &curve_shader(ShaderLanguage::Msl))
});

/// Everything in the MSL after the region bases — the part with no layout number in it.
const METAL_SHADER_BODY: &str = concat!(
    "struct MfbItem {\n",
    "  int4 quad;\n",     // bounds minX, minY, maxX, maxY (16.16 px)
    "  int4 shape;\n",    // p0..p3 (16.16 px)
    "  int4 fill;\n",     // RGBA 0..255
    "  int4 stroke;\n",   // RGBA 0..255
    "  int4 misc;\n",     // kind, radius (16.16), strokeHalf (16.16), edgeCount
    // glyph/picture height, arc reflex flag (bug-735) / polygon fill rule (plan-163-A),
    // edgeBase, capStyle
    "  int4 arc;\n",
    "  int2 surface;\n",  // width, height (px)
    "  int blendMode;\n", // the BlendMode tag 0..3 (plan-116-B)
    // 0 linear, 1 radial (plan-116-F); was the block's pad. plan-163-D: `kind | spread
    // << 1 | apply << 3`.
    "  int gradientKind;\n",
    "  int4 clip;\n",   // clip x0,y0,x1,y1 (16.16 px); zero-area = unclipped
    "  int4 xform0;\n", // inverse transform ia,ib,ic,id as float32 BITS
    "  int4 xform1;\n", // itx, ity (float32 bits), hasTransform (0 or 1), unused
    "  int4 arcCaps;\n", // an arc's two sweep endpoints startX,startY,endX,endY (16.16)
    "  int4 ellipse;\n", // ellipse cos, sin (16.16); gradient stop count and base
    "  int4 gradient;\n", // a gradient's axis startX,startY,endX,endY (16.16)
    // bug-735: the residual words (units of 2^-47 px) of every field a distance reads.
    "  int4 shapeLo;\n", // of shape.xyzw
    "  int4 capsLo;\n",  // of arcCaps.xyzw
    "  int4 clipLo;\n",  // of clip.xyzw
    "  int4 miscLo;\n",  // of misc.y (radius), misc.z (strokeHalf), ellipse.x, ellipse.y
    // plan-163-B: join records' first edge slot and count, dash pattern's first entry
    // and count.
    "  int4 strokeStyle;\n",
    // plan-163-C: a path clip's band header, then its edge base, edge count, mode | rule
    // << 2 and a picture's transparency step.
    "  int4 clipBand;\n",
    "  int4 clipPath;\n",
    // plan-163-D: a radial gradient's own inverse transform (float32 bits) and flag, then
    // its focal offset and focal radius (16.16).
    "  int4 gradInv0;\n",
    "  int4 gradInv1;\n",
    "  int4 gradFocal;\n",
    "};\n",
    // plan-116-A: the index travels to the fragment stage as a flat varying, because
    // `[[instance_id]]` does not exist there. `[[flat]]` and not the default: the value
    // is an index, and interpolating an index across a quad produces a *plausible*
    // picture drawn from the wrong blocks rather than a failure.
    "struct VOut { float4 pos [[position]]; uint item [[flat]]; };\n",
    "static float fx(int v) { return float(v) / FIXED; }\n",
    // plan-172-H: what one draw binds, to both stages -- 32 bytes. `offset` is the
    // translation of the enclosing groups' composed affine: two 16.16 words, then
    // (bug-735) their residual words. `inverse` is the linear part of that affine's
    // INVERSE as four float32 bit patterns -- `ia`, `ib`, `ic`, `id`, so a point maps as
    // `(ia * x + ic * y, ib * x + id * y)` -- and all-zero bits when the groups only
    // translate, the convention every `canvas::Transform` has. An inverse is never all
    // zero, so the test cannot mistake one.
    "struct MfbDraw { int4 offset; int4 inverse; };\n",
    "static bool drawGrouped(constant MfbDraw &draw) {\n",
    "  return (draw.inverse.x | draw.inverse.y | draw.inverse.z | draw.inverse.w) != 0;\n",
    "}\n",
    "static float4 drawInverse(constant MfbDraw &draw) {\n",
    "  return float4(as_type<float>(draw.inverse.x), as_type<float>(draw.inverse.y),\n",
    "                as_type<float>(draw.inverse.z), as_type<float>(draw.inverse.w));\n",
    "}\n",
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
    "vertex VOut mfbVertex(uint vid [[vertex_id]],\n",
    "                      uint iid [[instance_id]],\n",
    "                      constant MfbItem *items [[buffer(0)]],\n",
    "                      constant MfbDraw &draw [[buffer(1)]]) {\n",
    "  uint index = iid;\n",
    "  constant MfbItem &item = items[index];\n",
    "  float2 corner = float2(fx((vid & 1) == 0 ? item.quad.x : item.quad.z),\n",
    "                         fx((vid & 2) == 0 ? item.quad.y : item.quad.w))\n",
    "                + float2(fx(draw.offset.x), fx(draw.offset.y));\n",
    // plan-172-H: under a group matrix the quad is the item's box carried through the
    // FORWARD linear part -- the inverse of the 2x2 the draw carries -- which makes it
    // a parallelogram, and four mapped corners of a triangle strip are exactly that.
    // The box is first grown by the inverse's Frobenius norm on every side: its own
    // antialiasing rim is in the group's space, and a group that scales down would
    // shrink it under the pixel it has to cover. Growing by `|M|` in the group's space
    // is at least one pixel on the surface, because the forward map shrinks nothing by
    // more than `1 / |M|`. The fragment stage decides coverage; a quad can only be too
    // small.
    "  if (drawGrouped(draw)) {\n",
    "    float4 gi = drawInverse(draw);\n",
    "    float pad = length(gi);\n",
    "    float2 local = float2(fx((vid & 1) == 0 ? item.quad.x : item.quad.z) + ((vid & 1) == 0 ? -pad : pad),\n",
    "                          fx((vid & 2) == 0 ? item.quad.y : item.quad.w) + ((vid & 2) == 0 ? -pad : pad));\n",
    "    float det = gi.x * gi.w - gi.y * gi.z;\n",
    "    corner = float2((gi.w * local.x - gi.z * local.y) / det, (gi.x * local.y - gi.y * local.x) / det)\n",
    "             + float2(fx(draw.offset.x), fx(draw.offset.y));\n",
    "  }\n",
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
    // plan-163-B: the SQUARE-ended twin (`__canvas_segmentDistanceSquare`, and
    // `segmentDistanceSquare` in `mfb_canvas.frag`): the rectangle `half` wider than the
    // segment on every side; a zero-length one is the axis-aligned square.
    "static float segmentDistanceSquare(float2 p, float2 a, float2 b, float halfW) {\n",
    "  float2 v = b - a, w = p - a;\n",
    "  float len2 = dot(v, v);\n",
    "  if (len2 <= 0.0) { return max(abs(w.x), abs(w.y)) - halfW; }\n",
    "  float len = sqrt(len2);\n",
    "  float t = dot(w, v) / len2;\n",
    "  float d = abs(w.x * v.y - w.y * v.x) / len - halfW;\n",
    "  d = max(d, -t * len - halfW);\n",
    "  return max(d, (t - 1.0) * len - halfW);\n",
    "}\n",
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
    "static float2 dfTwoSum(float a, float b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float s = a + b;\n",
    "  float v = s - a;\n",
    "  return float2(s, (a - (s - v)) + (b - v));\n",
    "}\n",
    "static float2 dfQuickSum(float a, float b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float s = a + b;\n",
    "  return float2(s, b - (s - a));\n",
    "}\n",
    "static float2 dfTwoProd(float a, float b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float p = a * b;\n",
    "  return float2(p, fma(a, b, -p));\n",
    "}\n",
    "static float2 dfAdd(float2 a, float2 b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float2 s = dfTwoSum(a.x, b.x);\n",
    "  float2 t = dfTwoSum(a.y, b.y);\n",
    "  s = dfQuickSum(s.x, s.y + t.x);\n",
    "  return dfQuickSum(s.x, s.y + t.y);\n",
    "}\n",
    "static float2 dfSub(float2 a, float2 b) { return dfAdd(a, -b); }\n",
    "static float2 dfMul(float2 a, float2 b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float2 p = dfTwoProd(a.x, b.x);\n",
    "  return dfQuickSum(p.x, p.y + (a.x * b.y + a.y * b.x));\n",
    "}\n",
    "static float2 dfMulF(float2 a, float b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float2 p = dfTwoProd(a.x, b);\n",
    "  return dfQuickSum(p.x, p.y + a.y * b);\n",
    "}\n",
    "static float2 dfDiv(float2 a, float2 b) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  float q1 = a.x / b.x;\n",
    "  float2 r = dfSub(a, dfMulF(b, q1));\n",
    "  float q2 = r.x / b.x;\n",
    "  r = dfSub(r, dfMulF(b, q2));\n",
    "  float q3 = r.x / b.x;\n",
    "  return dfAdd(dfQuickSum(q1, q2), float2(q3, 0.0));\n",
    "}\n",
    "static float2 dfSqrt(float2 a) {\n",
    "#pragma clang fp reassociate(off)\n",
    "  if (a.x <= 0.0) { return float2(0.0, 0.0); }\n",
    "  float x = rsqrt(a.x);\n",
    "  float y = a.x * x;\n",
    "  float2 d = dfSub(a, dfTwoProd(y, y));\n",
    "  return dfTwoSum(y, d.x * (x * 0.5));\n",
    "}\n",
    "static bool dfLess(float2 a, float2 b) { return a.x < b.x || (a.x == b.x && a.y < b.y); }\n",
    // A 16.16 word as a double-float, exactly: the two halves are exact floats (at most 23
    // and 8 significant bits) and the scale is a power of two.
    "static float2 dfWord(int v) { return dfTwoSum(float(v & ~255) / FIXED, float(v & 255) / FIXED); }\n",
    // A residual word (bug-687, `EDGE_RESIDUAL_SCALE`) in px: units of 2^-47.
    "static float residualPx(int r) { return float(r) * (1.0 / 140737488355328.0); }\n",
    // bug-735: the residual encoding for every item-block field a distance reads -- the
    // 16.16 word and its residual word in the block's `*Lo` `int4`s -- as a double-float,
    // and the few double-float helpers every shape kind shares.
    "static float2 dfFixed(int word, int residual) { return dfAdd(dfWord(word), float2(residualPx(residual), 0.0)); }\n",
    "static float2 dfAbs(float2 a) { return a.x < 0.0 ? -a : a; }\n",
    "static float2 dfMax(float2 a, float2 b) { return dfLess(a, b) ? b : a; }\n",
    "static float2 dfMin(float2 a, float2 b) { return dfLess(b, a) ? b : a; }\n",
    "static float2 dfLength(float2 x, float2 y) { return dfSqrt(dfAdd(dfMul(x, x), dfMul(y, y))); }\n",
    // `a < b` for two (word, residual) pairs, exactly: the pair is order-preserving, since
    // a residual is at most half a 16.16 step.
    "static bool fixedLess(int wa, int ra, int wb, int rb) { return wa < wb || (wa == wb && ra < rb); }\n",
    // One vertex coordinate as the CPU had it: the 16.16 word plus its residual.
    "static float2 edgeCoord(constant int *edges, int i, int r) { return dfFixed(edges[i], edges[r]); }\n",
    // `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` -- the quantisation every shape uses -- for a
    // double-float `d`, rounded exactly.
    "static int dfCoverage(float2 d) {\n",
    "  float2 c = dfSub(float2(0.5, 0.0), d);\n",
    "  if (!dfLess(float2(0.0, 0.0), c)) { return 0; }\n",
    "  if (!dfLess(c, float2(1.0, 0.0))) { return 255; }\n",
    "  float2 x = dfAdd(dfMulF(c, 255.0), float2(0.5, 0.0));\n",
    "  float k = floor(x.x);\n",
    "  if (x.x == k && x.y < 0.0) { k -= 1.0; }\n",
    "  return int(k);\n",
    "}\n",
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
    "static float edgeSlack(float2 p, float2 a, float2 b) {\n",
    "  return (abs(p.x) + abs(p.y) + abs(a.x) + abs(a.y) + abs(b.x) + abs(b.y)) * (1.0 / 262144.0) + (1.0 / 4096.0);\n",
    "}\n",
    "static bool edgeUnsure(float2 p, float2 a, float2 b, float slack) {\n",
    "  if (abs(a.y - p.y) <= slack || abs(b.y - p.y) <= slack) { return true; }\n",
    "  if ((a.y > p.y) == (b.y > p.y)) { return false; }\n",
    "  float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);\n",
    "  return abs(cross) <= slack * (abs(b.x - a.x) + abs(b.y - a.y) + 2.0 * (abs(p.x - a.x) + abs(p.y - a.y)));\n",
    "}\n",
    // plan-163-A: an edge's signed crossing -- 0 when it does not cross the pixel's row
    // left of the pixel, else +1 for an edge running down the screen (`b.y > a.y`) and -1
    // for one running up: `__canvas_edgeDistance`'s winding convention.
    "static int edgeCrossing32(float2 p, float2 a, float2 b) {\n",
    "  if ((a.y > p.y) == (b.y > p.y)) { return 0; }\n",
    "  float cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);\n",
    "  if (b.y > a.y) { return cross < 0.0 ? 1 : 0; }\n",
    "  return cross > 0.0 ? -1 : 0;\n",
    "}\n",
    // bug-735: `__canvas_rectDistance` in double-float, from the point's offset to the centre
    // and the half-extents: a shape's rectangle (less its corner radius) and the clip's.
    "static float2 dfRectDistance(float2 dx, float2 dy, float2 hx, float2 hy) {\n",
    "  float2 ox = dfSub(dfAbs(dx), hx);\n",
    "  float2 oy = dfSub(dfAbs(dy), hy);\n",
    "  float2 outside = dfLength(dfMax(ox, float2(0.0, 0.0)), dfMax(oy, float2(0.0, 0.0)));\n",
    "  return dfAdd(outside, dfMin(dfMax(ox, oy), float2(0.0, 0.0)));\n",
    "}\n",
    // bug-735: a segment's distance, its UNCLAMPED parameter `t` and `len2`, from `w = p - a`
    // and `v = b - a` -- `__canvas_segmentDistance`'s arithmetic (and the butt twin's `t`),
    // so a polygon edge, a line and a polyline segment share it. `t` stays 0 for a
    // zero-length segment, whose distance is to its point.
    "struct DfSegment { float2 d; float2 t; float2 len2; };\n",
    "static DfSegment dfSegment(float2 wx, float2 wy, float2 vx, float2 vy) {\n",
    "  DfSegment s;\n",
    "  s.len2 = dfAdd(dfMul(vx, vx), dfMul(vy, vy));\n",
    "  s.t = float2(0.0, 0.0);\n",
    "  float2 c = float2(0.0, 0.0);\n",
    "  if (s.len2.x > 0.0) {\n",
    "    s.t = dfDiv(dfAdd(dfMul(wx, vx), dfMul(wy, vy)), s.len2);\n",
    "    c = s.t;\n",
    "    if (dfLess(c, float2(0.0, 0.0))) { c = float2(0.0, 0.0); }\n",
    "    if (dfLess(float2(1.0, 0.0), c)) { c = float2(1.0, 0.0); }\n",
    "  }\n",
    "  s.d = dfLength(dfSub(wx, dfMul(vx, c)), dfSub(wy, dfMul(vy, c)));\n",
    "  return s;\n",
    "}\n",
    "struct DfEdge { float2 ax; float2 ay; float2 vx; float2 vy; float2 by; };\n",
    "static DfEdge dfEdge(constant int *edges, int base, int e) {\n",
    "  int i = (base + e) * 4 + METAL_EDGE_BASE;\n",
    "  int r = (base + e) * 4 + METAL_EDGE_RESIDUAL_BASE;\n",
    "  DfEdge g;\n",
    "  g.ax = edgeCoord(edges, i, r);\n",
    "  g.ay = edgeCoord(edges, i + 1, r + 1);\n",
    // The end itself, not `ay + vy`: two edges that share a vertex must read ONE value
    // for it, or a pixel row through that vertex counts the corner twice (`dfCrossing`).
    "  g.by = edgeCoord(edges, i + 3, r + 3);\n",
    "  g.vx = dfAdd(dfWord(edges[i + 2] - edges[i]),\n",
    "               dfTwoSum(residualPx(edges[r + 2]), -residualPx(edges[r])));\n",
    "  g.vy = dfAdd(dfWord(edges[i + 3] - edges[i + 1]),\n",
    "               dfTwoSum(residualPx(edges[r + 3]), -residualPx(edges[r + 1])));\n",
    "  return g;\n",
    "}\n",
    "static float2 dfSegmentDistance(DfEdge g, float2 x, float2 y) {\n",
    "  return dfSegment(dfSub(x, g.ax), dfSub(y, g.ay), g.vx, g.vy).d;\n",
    "}\n",
    "static int dfCrossing(DfEdge g, float2 x, float2 y) {\n",
    "  if (dfLess(y, g.ay) == dfLess(y, g.by)) { return 0; }\n",
    "  float2 lhs = dfMul(dfSub(x, g.ax), g.vy);\n",
    "  float2 rhs = dfMul(dfSub(y, g.ay), g.vx);\n",
    "  if (g.vy.x > 0.0) { return dfLess(lhs, rhs) ? 1 : 0; }\n",
    "  return dfLess(rhs, lhs) ? -1 : 0;\n",
    "}\n",
    // plan-163-A: `nonZero` is the polygon's fill rule (`arc.y`). The crossings are summed
    // as a winding number; NonZero is inside when it is not zero, EvenOdd when it is odd,
    // which is the parity the toggles it replaced computed.
    "static float2 edgeDistance(constant int *edges, int base, int count, int list, float2 x, float2 y, float reach, int nonZero) {\n",
    "  float2 p = float2(x.x, y.x);\n",
    "  float bound = 1.0e6;\n",
    "  float low = 1.0e6;\n",
    "  bool unsure = false;\n",
    "  int winding = 0;\n",
    "  for (int j = 0; j < count; ++j) {\n",
    "    int i = (base + (list < 0 ? j : edges[list + j])) * 4 + METAL_EDGE_BASE;\n",
    "    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "    float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));\n",
    "    float slack = edgeSlack(p, a, b);\n",
    "    float d32 = segmentDistance(p, a, b);\n",
    "    bound = min(bound, d32 + slack);\n",
    "    low = min(low, d32 - slack);\n",
    "    if (edgeUnsure(p, a, b, slack)) {\n",
    "      unsure = true;\n",
    "    } else {\n",
    "      winding += edgeCrossing32(p, a, b);\n",
    "    }\n",
    "  }\n",
    "  bool distant = low > reach;\n",
    "  float2 best = distant ? float2(low, 0.0) : float2(1.0e6, 0.0);\n",
    "  for (int j = 0; j < count && (!distant || unsure); ++j) {\n",
    "    int e = list < 0 ? j : edges[list + j];\n",
    "    int i = (base + e) * 4 + METAL_EDGE_BASE;\n",
    "    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "    float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));\n",
    "    float slack = edgeSlack(p, a, b);\n",
    "    float2 gap = max(max(min(a, b) - p, p - max(a, b)), float2(0.0, 0.0));\n",
    "    float within = bound + slack;\n",
    "    bool candidate = !distant && dot(gap, gap) <= within * within && segmentDistance(p, a, b) - slack <= bound;\n",
    "    bool unsureEdge = unsure && edgeUnsure(p, a, b, slack);\n",
    "    if (!candidate && !unsureEdge) { continue; }\n",
    "    DfEdge g = dfEdge(edges, base, e);\n",
    "    if (candidate) {\n",
    "      float2 d = dfSegmentDistance(g, x, y);\n",
    "      if (dfLess(d, best)) { best = d; }\n",
    "    }\n",
    "    if (unsureEdge) { winding += dfCrossing(g, x, y); }\n",
    "  }\n",
    "  bool inside = nonZero != 0 ? winding != 0 : (winding & 1) != 0;\n",
    "  return inside ? -best : best;\n",
    "}\n",
    // plan-163-B: join records (`helper_joins.rs`), riding the edge region after the item's
    // own edges -- one record per outline edge, the corner at its START, as two edge slots
    // (`strokeStyle.x` the first, `strokeStyle.y` the count): four points V, A, P, B as
    // eight 16.16 words with their residuals. Its corner PIECE, from exact equalities:
    // none when A = V, the disc of radius |A - V| about V when A = B, that disc cut by the
    // chord A-B on V's side when P = V (a bevel), the disc with the polygon V, A, P, B
    // otherwise (a miter). The twins of `mfb_canvas.frag`'s, line for line.
    "static int joinWord(constant MfbItem &item, int r, int w) { return (item.strokeStyle.x + r * 2) * 4 + w; }\n",
    "static bool joinSame(constant MfbItem &item, constant int *edges, int r, int a, int b) {\n",
    "  int i = joinWord(item, r, a * 2) + METAL_EDGE_BASE;\n",
    "  int j = joinWord(item, r, b * 2) + METAL_EDGE_BASE;\n",
    "  int ri = joinWord(item, r, a * 2) + METAL_EDGE_RESIDUAL_BASE;\n",
    "  int rj = joinWord(item, r, b * 2) + METAL_EDGE_RESIDUAL_BASE;\n",
    "  return edges[i] == edges[j] && edges[i + 1] == edges[j + 1]\n",
    "      && edges[ri] == edges[rj] && edges[ri + 1] == edges[rj + 1];\n",
    "}\n",
    "static int joinKind(constant MfbItem &item, constant int *edges, int r) {\n",
    "  if (joinSame(item, edges, r, 1, 0)) { return 0; }\n",
    "  if (joinSame(item, edges, r, 1, 3)) { return 1; }\n",
    "  return joinSame(item, edges, r, 2, 0) ? 2 : 3;\n",
    "}\n",
    "static float2 joinPoint32(constant MfbItem &item, constant int *edges, int r, int k) {\n",
    "  int i = joinWord(item, r, k * 2) + METAL_EDGE_BASE;\n",
    "  return float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "}\n",
    "static float2 joinCoord(constant MfbItem &item, constant int *edges, int r, int w) {\n",
    "  int i = joinWord(item, r, w);\n",
    "  return edgeCoord(edges, i + METAL_EDGE_BASE, i + METAL_EDGE_RESIDUAL_BASE);\n",
    "}\n",
    // A record's piece distance in fp32 as (least, most) the double-float one can be,
    // widened by `edgeSlack`'s bound.
    "static float2 joinBounds32(constant MfbItem &item, constant int *edges, int r, int kind, float2 p) {\n",
    "  float2 v = joinPoint32(item, edges, r, 0);\n",
    "  float2 a = joinPoint32(item, edges, r, 1);\n",
    "  float ds = 2.0 * edgeSlack(p, v, a);\n",
    "  float disc = length(p - v) - length(a - v);\n",
    "  float2 piece = float2(disc - ds, disc + ds);\n",
    "  if (kind == 1) { return piece; }\n",
    "  float2 b = joinPoint32(item, edges, r, 3);\n",
    "  if (kind == 2) {\n",
    "    float2 c = b - a;\n",
    "    float cs = 2.0 * edgeSlack(p, a, b);\n",
    "    float o = (c.x * (p.y - a.y) - c.y * (p.x - a.x)) / length(c);\n",
    "    if (c.x * (v.y - a.y) - c.y * (v.x - a.x) > 0.0) { o = -o; }\n",
    "    return float2(max(piece.x, o - cs), max(piece.y, o + cs));\n",
    "  }\n",
    "  float d = 1.0e6;\n",
    "  float slack = 0.0;\n",
    "  bool unsure = false;\n",
    "  int winding = 0;\n",
    "  for (int k = 0; k < 4; ++k) {\n",
    "    float2 s0 = joinPoint32(item, edges, r, k);\n",
    "    float2 s1 = joinPoint32(item, edges, r, (k + 1) & 3);\n",
    "    float sl = edgeSlack(p, s0, s1);\n",
    "    slack = max(slack, sl);\n",
    "    d = min(d, segmentDistance(p, s0, s1));\n",
    "    if (edgeUnsure(p, s0, s1, sl)) { unsure = true; } else { winding += edgeCrossing32(p, s0, s1); }\n",
    "  }\n",
    "  float2 quad = unsure ? float2(-d - slack, d + slack)\n",
    "      : ((winding & 1) != 0 ? float2(-d - slack, -d + slack) : float2(d - slack, d + slack));\n",
    "  return float2(min(piece.x, quad.x), min(piece.y, quad.y));\n",
    "}\n",
    // The piece in double-float, for the disc and the chord; the polygon's distance `quad`
    // (negative inside) comes from the caller's shared `dfSegment` loop.
    "static float2 dfJoinPiece(constant MfbItem &item, constant int *edges, int r, int kind, float2 x, float2 y, float2 quad) {\n",
    "  float2 vx = joinCoord(item, edges, r, 0);\n",
    "  float2 vy = joinCoord(item, edges, r, 1);\n",
    "  float2 ax = joinCoord(item, edges, r, 2);\n",
    "  float2 ay = joinCoord(item, edges, r, 3);\n",
    "  float2 disc = dfSub(dfLength(dfSub(x, vx), dfSub(y, vy)), dfLength(dfSub(ax, vx), dfSub(ay, vy)));\n",
    "  if (kind == 1) { return disc; }\n",
    "  if (kind == 3) { return dfMin(disc, quad); }\n",
    "  float2 cx = dfSub(joinCoord(item, edges, r, 6), ax);\n",
    "  float2 cy = dfSub(joinCoord(item, edges, r, 7), ay);\n",
    "  float2 o = dfDiv(dfSub(dfMul(cx, dfSub(y, ay)), dfMul(cy, dfSub(x, ax))), dfLength(cx, cy));\n",
    "  float2 ov = dfSub(dfMul(cx, dfSub(vy, ay)), dfMul(cy, dfSub(vx, ax)));\n",
    "  if (dfLess(float2(0.0, 0.0), ov)) { o = -o; }\n",
    "  return dfMax(disc, o);\n",
    "}\n",
    // One outline edge of a joined stroke, in fp32: a polygon's band edge `e`, or a
    // rectangle's side `e` from its centre and half-extents (`__canvas_rectSides`).
    "static void joinedEdge32(constant MfbItem &item, constant int *edges, bool poly, int e, thread float2 &a, thread float2 &b) {\n",
    "  if (poly) {\n",
    "    int i = (item.arc.z + e) * 4 + METAL_EDGE_BASE;\n",
    "    a = float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "    b = float2(fx(edges[i + 2]), fx(edges[i + 3]));\n",
    "    return;\n",
    "  }\n",
    "  float2 c = float2(fx(item.shape.x), fx(item.shape.y));\n",
    "  float2 h = float2(fx(item.shape.z), fx(item.shape.w));\n",
    "  float2 corner[4] = { c - h, float2(c.x + h.x, c.y - h.y), c + h, float2(c.x - h.x, c.y + h.y) };\n",
    "  a = corner[e];\n",
    "  b = corner[(e + 1) & 3];\n",
    "}\n",
    // The stroke distance of a polygon (its band's edges) or a rectangle or picture (its
    // four sides) with join records -- `__canvas_joinedStroke`: the min of each outline
    // edge's BUTT band and the record at its start, with bug-687's two passes. Every
    // double-float segment goes through ONE `dfSegment` call, `k` 0 the outline edge and
    // 1..4 the record's edges.
    "static float2 joinedStroke(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float2 half2, float reach) {\n",
    "  bool poly = item.misc.x == 4;\n",
    "  int count = poly ? band.y : 4;\n",
    "  float2 p = float2(x.x, y.x);\n",
    "  float halfW = half2.x;\n",
    "  float bound = 1.0e6;\n",
    "  float low = 1.0e6;\n",
    "  for (int j = 0; j < count; ++j) {\n",
    "    int e = poly ? (band.x < 0 ? j : edges[band.x + j]) : j;\n",
    "    float2 a;\n",
    "    float2 b;\n",
    "    joinedEdge32(item, edges, poly, e, a, b);\n",
    "    float slack = edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0);\n",
    "    float d32 = segmentDistanceButt(p, a, b, halfW);\n",
    "    bound = min(bound, d32 + slack);\n",
    "    low = min(low, d32 - slack);\n",
    "    int kind = joinKind(item, edges, e);\n",
    "    if (kind != 0) {\n",
    "      float2 jb = joinBounds32(item, edges, e, kind, p);\n",
    "      bound = min(bound, jb.y);\n",
    "      low = min(low, jb.x);\n",
    "    }\n",
    "  }\n",
    "  if (low > reach) { return float2(low, 0.0); }\n",
    "  float2 cx = dfFixed(item.shape.x, item.shapeLo.x);\n",
    "  float2 cy = dfFixed(item.shape.y, item.shapeLo.y);\n",
    "  float2 hx = dfFixed(item.shape.z, item.shapeLo.z);\n",
    "  float2 hy = dfFixed(item.shape.w, item.shapeLo.w);\n",
    "  float2 best = float2(1.0e6, 0.0);\n",
    "  for (int j = 0; j < count; ++j) {\n",
    "    int e = poly ? (band.x < 0 ? j : edges[band.x + j]) : j;\n",
    "    float2 a;\n",
    "    float2 b;\n",
    "    joinedEdge32(item, edges, poly, e, a, b);\n",
    "    float slack = edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0);\n",
    "    bool edgeNear = segmentDistanceButt(p, a, b, halfW) - slack <= bound;\n",
    "    int kind = joinKind(item, edges, e);\n",
    "    bool joinNear = kind != 0 && joinBounds32(item, edges, e, kind, p).x <= bound;\n",
    "    if (!edgeNear && !joinNear) { continue; }\n",
    "    float2 jd = float2(1.0e6, 0.0);\n",
    "    int winding = 0;\n",
    "    for (int k = edgeNear ? 0 : 1; k < (joinNear && kind == 3 ? 5 : 1); ++k) {\n",
    "      DfEdge g;\n",
    "      if (k == 0 && poly) {\n",
    "        g = dfEdge(edges, item.arc.z, e);\n",
    "      } else if (k == 0) {\n",
    "        int s0 = e;\n",
    "        int s1 = (e + 1) & 3;\n",
    "        float2 ax = (s0 == 0 || s0 == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);\n",
    "        float2 ay = (s0 < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);\n",
    "        float2 bx = (s1 == 0 || s1 == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);\n",
    "        float2 by = (s1 < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);\n",
    "        g.ax = ax;\n",
    "        g.ay = ay;\n",
    "        g.vx = dfSub(bx, ax);\n",
    "        g.vy = dfSub(by, ay);\n",
    "        g.by = by;\n",
    "      } else {\n",
    "        g.ax = joinCoord(item, edges, e, (k - 1) * 2);\n",
    "        g.ay = joinCoord(item, edges, e, (k - 1) * 2 + 1);\n",
    "        g.vx = dfSub(joinCoord(item, edges, e, (k & 3) * 2), g.ax);\n",
    "        g.by = joinCoord(item, edges, e, (k & 3) * 2 + 1);\n",
    "        g.vy = dfSub(g.by, g.ay);\n",
    "      }\n",
    "      DfSegment s = dfSegment(dfSub(x, g.ax), dfSub(y, g.ay), g.vx, g.vy);\n",
    "      if (k == 0) {\n",
    "        if (s.len2.x > 0.0) {\n",
    "          float2 len = dfSqrt(s.len2);\n",
    "          float2 d = dfMax(dfSub(s.d, half2), -dfMul(s.t, len));\n",
    "          d = dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len));\n",
    "          if (dfLess(d, best)) { best = d; }\n",
    "        }\n",
    "      } else {\n",
    "        if (dfLess(s.d, jd)) { jd = s.d; }\n",
    "        winding += dfCrossing(g, x, y);\n",
    "      }\n",
    "    }\n",
    "    if (joinNear) {\n",
    "      if ((winding & 1) != 0) { jd = -jd; }\n",
    "      float2 piece = dfJoinPiece(item, edges, e, kind, x, y, jd);\n",
    "      if (dfLess(piece, best)) { best = piece; }\n",
    "    }\n",
    "  }\n",
    "  return best;\n",
    "}\n",
    // plan-116-C: the inverse transform, decoded from the float32 bits the item block
    // carries. `as_type<float>` is a reinterpret, not a conversion -- the CPU already
    // did the narrowing (`__canvas_float32Bits`), because this compiler's assemblers
    // have no double->single convert.
    "static bool hasTransform(constant MfbItem &item) { return item.xform1.z != 0; }\n",
    // bug-686: the edge list a polygon's shape-space point at height `y` visits, as
    // (first word in `edges`, length), or (-1, edgeCount) for "every edge" -- any other
    // kind, or a polygon the emitter gave no band index (`arcCaps.z` = 0). The band
    // header is `arcCaps` = (top, height in 16.16, band count, table offset), written by
    // `emit_band_index`; the table is (first, count) per band and the list follows it.
    // A point above the first band or below the last uses the end band, which the
    // emitter built to hold every edge that can matter there.
    "static int2 polygonBand(constant MfbItem &item, constant int *edges, float y) {\n",
    "  int nb = item.arcCaps.z;\n",
    // plan-164-C: kinds 11 and 12 too -- their pieces are banded as edges.
    "  int bk = item.misc.x;\n",
    "  if ((bk != 4 && bk != 10 && bk != 11 && bk != 12) || nb <= 0) { return int2(-1, item.misc.w); }\n",
    "  float k = floor((y - fx(item.arcCaps.x)) / fx(item.arcCaps.y));\n",
    "  int band = int(clamp(k, 0.0, float(nb - 1)));\n",
    "  int t = METAL_BAND_BASE + item.arcCaps.w;\n",
    "  return int2(t + nb * 2 + edges[t + band * 2], edges[t + band * 2 + 1]);\n",
    "}\n",
    // plan-154-B: a PolyLine's distance and path parameter, as (d, t) -- the twin of
    // `__canvas_polylineScan` in the oracle and of `polylineScan` in
    // `runtime/canvas/shaders/mfb_canvas.frag`. The minimum over the segments' round
    // bands is the union that makes every corner round; only the first and last LIVE
    // segments (`gradient.x`/`.y`, whole numbers) take a butt plane, each on its outer
    // end. A zero-length segment is skipped unless it is the whole path (first == last),
    // where it is the round dot. `t` is (segment + clamped t) / segments, continuous at a
    // corner, and the same walk sets both, so the colour's segment is the distance's.
    // plan-163-C: a path clip's band, the clip's twin of `polygonBand`: its band header
    // in `clipBand`, or every clip edge (band count 0).
    "static int2 clipBandAt(constant MfbItem &item, constant int *edges, float y) {\n",
    "  int nb = item.clipBand.z;\n",
    "  if (nb <= 0) { return int2(-1, item.clipPath.y); }\n",
    "  float k = floor((y - fx(item.clipBand.x)) / fx(item.clipBand.y));\n",
    "  int band = int(clamp(k, 0.0, float(nb - 1)));\n",
    "  int t = METAL_BAND_BASE + item.clipBand.w;\n",
    "  return int2(t + nb * 2 + edges[t + band * 2], edges[t + band * 2 + 1]);\n",
    "}\n",
    "static float2 polylineScan(constant MfbItem &item, constant int *edges, int2 band, float2 p) {\n",
    "  int first = item.gradient.x;\n",
    "  int last = item.gradient.y;\n",
    "  float halfW = fx(item.misc.y);\n",
    "  bool butt = item.arc.w == 0;\n",
    "  bool square = item.arc.w == 2;\n",
    "  float best = 1.0e6;\n",
    "  float param = 0.0;\n",
    "  for (int j = 0; j < band.y; ++j) {\n",
    "    int e = band.x < 0 ? j : edges[band.x + j];\n",
    "    if (e < first || e > last) { continue; }\n",
    "    int i = (item.arc.z + e) * 4 + METAL_EDGE_BASE;\n",
    "    float2 a = float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "    float2 v = float2(fx(edges[i + 2]), fx(edges[i + 3])) - a;\n",
    "    float len2 = dot(v, v);\n",
    "    if (!(len2 > 0.0) && first != last) { continue; }\n",
    "    float2 w = p - a;\n",
    "    float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;\n",
    "    float c = clamp(t, 0.0, 1.0);\n",
    "    float at = c;\n",
    "    if (square && ((e == first && t < 0.0) || (e == last && t > 1.0))) { at = t; }\n",
    "    float d = length(w - v * at) - halfW;\n",
    "    if (butt) {\n",
    "      float len = sqrt(len2);\n",
    "      if (e == first) { d = max(d, -t * len); }\n",
    "      if (e == last) { d = max(d, (t - 1.0) * len); }\n",
    "    }\n",
    "    if (square) {\n",
    "      if (len2 > 0.0) {\n",
    "        float len = sqrt(len2);\n",
    "        if (e == first) { d = max(d, -t * len - halfW); }\n",
    "        if (e == last) { d = max(d, (t - 1.0) * len - halfW); }\n",
    "      } else {\n",
    "        d = max(abs(w.x), abs(w.y)) - halfW;\n",
    "      }\n",
    "    }\n",
    "    if (d < best) { best = d; param = (float(e) + c) / float(item.misc.w); }\n",
    "  }\n",
    "  return float2(best, param);\n",
    "}\n",
    "static float2 inverseMap(constant MfbItem &item, float2 p) {\n",
    "  return float2(as_type<float>(item.xform0.x) * p.x + as_type<float>(item.xform0.z) * p.y + as_type<float>(item.xform1.x),\n",
    "                as_type<float>(item.xform0.y) * p.x + as_type<float>(item.xform0.w) * p.y + as_type<float>(item.xform1.y));\n",
    "}\n",
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
    // * arc (3): the ring band inside the sweep, else 1e6, and a Round cap's two discs. The
    //   sweep test is the oracle's two cross products, against `arcCaps - centre`, a positive
    //   multiple of the oracle's Taylor `(cos, sin)`, so no GPU trigonometry is involved and
    //   the sign agrees wherever it is decided by more than ~1e-13 px; `arc.y` is the
    //   oracle's `reflex`, decided in f64 by the emitter;
    // * ellipse (7): the oracle's 24-step bisection, every step in double-float. In fp32 a
    //   step's sign test took the other branch wherever `g` was within fp32's error of zero
    //   -- every late step, on every pixel -- which is what put the ellipse 8 pixels past
    //   GPU_DEFAULT. The circle case (equal radii) is the oracle's too.
    "static float2 dfShapeDistance(constant MfbItem &item, float2 x, float2 y) {\n",
    "  int kind = item.misc.x;\n",
    "  float2 cx = dfFixed(item.shape.x, item.shapeLo.x);\n",
    "  float2 cy = dfFixed(item.shape.y, item.shapeLo.y);\n",
    "  float2 ex = dfFixed(item.shape.z, item.shapeLo.z);\n",
    "  float2 ey = dfFixed(item.shape.w, item.shapeLo.w);\n",
    "  float2 radius = dfFixed(item.misc.y, item.miscLo.x);\n",
    "  float2 dx = dfSub(x, cx);\n",
    "  float2 dy = dfSub(y, cy);\n",
    "  if (kind == 0 || kind == 9) {\n",
    "    return dfSub(dfRectDistance(dx, dy, ex, ey), radius);\n",
    "  }\n",
    "  if (kind == 1) { return dfSub(dfSub(dfLength(dx, dy), ex), radius); }\n",
    "  if (kind == 2) {\n",
    "    DfSegment s = dfSegment(dx, dy, dfSub(ex, cx), dfSub(ey, cy));\n",
    // plan-163-B: Square -- `__canvas_segmentDistanceSquare`.
    "    if (item.arc.w == 1) { return dfSub(s.d, radius); }\n",
    "    if (item.arc.w == 2) {\n",
    "      if (!(s.len2.x > 0.0)) { return dfSub(dfMax(dfAbs(dx), dfAbs(dy)), radius); }\n",
    "      float2 sl = dfSqrt(s.len2);\n",
    "      float2 perp = dfDiv(dfAbs(dfSub(dfMul(dx, dfSub(ey, cy)), dfMul(dy, dfSub(ex, cx)))), sl);\n",
    "      float2 sd = dfMax(dfSub(perp, radius), dfSub(-dfMul(s.t, sl), radius));\n",
    "      return dfMax(sd, dfSub(dfMul(dfSub(s.t, float2(1.0, 0.0)), sl), radius));\n",
    "    }\n",
    "    if (!(s.len2.x > 0.0)) { return float2(1.0e6, 0.0); }\n",
    "    float2 len = dfSqrt(s.len2);\n",
    "    float2 d = dfMax(dfSub(s.d, radius), -dfMul(s.t, len));\n",
    "    return dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len));\n",
    "  }\n",
    "  if (kind == 3) {\n",
    "    float2 sx = dfSub(dfFixed(item.arcCaps.x, item.capsLo.x), cx);\n",
    "    float2 sy = dfSub(dfFixed(item.arcCaps.y, item.capsLo.y), cy);\n",
    "    float2 tx = dfSub(dfFixed(item.arcCaps.z, item.capsLo.z), cx);\n",
    "    float2 ty = dfSub(dfFixed(item.arcCaps.w, item.capsLo.w), cy);\n",
    "    bool afterStart = !dfLess(dfMul(sx, dy), dfMul(sy, dx));\n",
    "    bool beforeEnd = !dfLess(dfMul(ty, dx), dfMul(tx, dy));\n",
    "    bool inSweep = item.arc.y != 0 ? (afterStart || beforeEnd) : (afterStart && beforeEnd);\n",
    "    float2 band = inSweep ? dfSub(dfAbs(dfSub(dfLength(dx, dy), ex)), radius) : float2(1.0e6, 0.0);\n",
    // plan-163-B: a Square end is `__canvas_arcSquareCap` -- the half-width box past the
    // end along the tangent, `along` measured outward from the end.
    "    if (item.arc.w == 0) { return band; }\n",
    "    for (int k = 0; k < 2; ++k) {\n",
    "      float2 rx = k == 0 ? sx : tx;\n",
    "      float2 ry = k == 0 ? sy : ty;\n",
    "      float2 ox = dfSub(dx, rx);\n",
    "      float2 oy = dfSub(dy, ry);\n",
    "      if (item.arc.w == 2) {\n",
    "        float2 rl = dfLength(rx, ry);\n",
    "        if (!(rl.x > 0.0)) { continue; }\n",
    "        float2 ux = dfDiv(rx, rl);\n",
    "        float2 uy = dfDiv(ry, rl);\n",
    "        float2 along = dfSub(dfMul(ox, uy), dfMul(oy, ux));\n",
    "        if (k == 1) { along = -along; }\n",
    "        float2 across = dfAdd(dfMul(ox, ux), dfMul(oy, uy));\n",
    "        float2 box = dfMax(dfMax(dfSub(dfAbs(across), radius), dfSub(along, radius)), -along);\n",
    "        band = dfMin(band, box);\n",
    "      } else {\n",
    "        band = dfMin(band, dfSub(dfLength(ox, oy), radius));\n",
    "      }\n",
    "    }\n",
    "    return band;\n",
    "  }\n",
    "  if (kind == 7) {\n",
    "    float2 ca = dfFixed(item.ellipse.x, item.miscLo.z);\n",
    "    float2 sa = dfFixed(item.ellipse.y, item.miscLo.w);\n",
    "    float2 qx = dfAbs(dfAdd(dfMul(dx, ca), dfMul(dy, sa)));\n",
    "    float2 qy = dfAbs(dfSub(dfMul(dy, ca), dfMul(dx, sa)));\n",
    "    if (item.shape.z == item.shape.w && item.shapeLo.z == item.shapeLo.w) {\n",
    "      return dfSub(dfSub(dfLength(qx, qy), ex), radius);\n",
    "    }\n",
    "    float2 c0 = float2(1.0, 0.0);\n",
    "    float2 s0 = float2(0.0, 0.0);\n",
    "    float2 c1 = float2(0.0, 0.0);\n",
    "    float2 s1 = float2(1.0, 0.0);\n",
    "    float2 cm = c0;\n",
    "    float2 sm = s0;\n",
    "    for (int k = 0; k < 24; ++k) {\n",
    "      float2 cs = dfAdd(c0, c1);\n",
    "      float2 ss = dfAdd(s0, s1);\n",
    "      float2 nn = dfLength(cs, ss);\n",
    "      cm = dfDiv(cs, nn);\n",
    "      sm = dfDiv(ss, nn);\n",
    "      float2 g = dfAdd(dfMul(dfSub(qx, dfMul(ex, cm)), -dfMul(ex, sm)),\n",
    "                       dfMul(dfSub(qy, dfMul(ey, sm)), dfMul(ey, cm)));\n",
    "      if (dfLess(float2(0.0, 0.0), g)) { c0 = cm; s0 = sm; } else { c1 = cm; s1 = sm; }\n",
    "    }\n",
    "    float2 d = dfLength(dfSub(qx, dfMul(ex, cm)), dfSub(qy, dfMul(ey, sm)));\n",
    "    float2 ux = dfDiv(qx, ex);\n",
    "    float2 uy = dfDiv(qy, ey);\n",
    "    if (dfLess(dfAdd(dfMul(ux, ux), dfMul(uy, uy)), float2(1.0, 0.0))) { d = -d; }\n",
    "    return dfSub(d, radius);\n",
    "  }\n",
    "  return float2(1.0e6, 0.0);\n",
    "}\n",
    // bug-735: the fp32 pre-pass for the same kinds -- a value `v` such that, whenever
    // `|v| > reach`, the distance has `v`'s sign and at least its magnitude, so its fill and
    // stroke coverage are `v`'s and double-float is not needed. Like `edgeSlack`, an ERROR
    // BOUND, not a tolerance: the value returned for a far pixel is past the clamp either
    // way, and every other pixel gets the double-float value.
    //
    // * rectangle, circle, line: the fp32 distance, moved toward zero by `slack` -- 2^-18 of
    //   the coordinates' magnitudes plus 2^-12 px, the bound `edgeSlack` uses (each is
    //   1-Lipschitz in its inputs, which the 16.16 words and the residual move by <= 2^-17);
    // * arc: a lower bound only, `min(ring band, cap discs)` with the sweep ignored -- out of
    //   sweep the oracle answers 1e6, never less than the band;
    // * ellipse: `rmin * |s - 1|` for `s = |(qx / rx, qy / ry)|`, a RIGOROUS bound on the
    //   distance to the ellipse (the ellipse and its scale by `s` are `rmin * |s - 1|` apart:
    //   map by `diag(1 / rx, 1 / ry)`, whose inverse stretches no vector by more than
    //   `rmin`), with a slack of 2^-14 of the magnitudes for the 16.16 inputs; the oracle's
    //   bisection lands on the ellipse, so its distance is never below the true one;
    // * a butt line shorter than fp32 resolves, and any other kind: no shortcut or 1e6.
    "static float farBound(constant MfbItem &item, float2 p) {\n",
    "  int kind = item.misc.x;\n",
    "  float2 c = float2(fx(item.shape.x), fx(item.shape.y));\n",
    "  float2 e = float2(fx(item.shape.z), fx(item.shape.w));\n",
    "  float r = fx(item.misc.y);\n",
    "  float slack = (abs(p.x) + abs(p.y) + abs(c.x) + abs(c.y) + abs(e.x) + abs(e.y) + abs(r)) * (1.0 / 262144.0) + (1.0 / 4096.0);\n",
    "  if (kind == 7) {\n",
    "    float2 d = p - c;\n",
    "    float ca = fx(item.ellipse.x);\n",
    "    float sa = fx(item.ellipse.y);\n",
    "    float s = length(abs(float2(d.x * ca + d.y * sa, -d.x * sa + d.y * ca)) / e);\n",
    "    float rmin = min(e.x, e.y);\n",
    "    float sl = (abs(d.x) + abs(d.y) + e.x + e.y + 1.0) * (1.0 / 16384.0) + (1.0 / 4096.0);\n",
    "    return s >= 1.0 ? rmin * (s - 1.0) - r - sl : rmin * (s - 1.0) - r + sl;\n",
    "  }\n",
    "  if (kind == 3) {\n",
    "    float2 cs = float2(fx(item.arcCaps.x), fx(item.arcCaps.y));\n",
    "    float2 ce = float2(fx(item.arcCaps.z), fx(item.arcCaps.w));\n",
    "    float m = abs(length(p - c) - e.x) - r;\n",
    // plan-163-B: a Square end lies within `r * sqrt(2)` of its endpoint (1.4143 > sqrt 2,
    // so this stays a lower bound).
    "    float capR = item.arc.w == 2 ? r * 1.4143 : r;\n",
    "    if (item.arc.w != 0) { m = min(m, min(length(p - cs), length(p - ce)) - capR); }\n",
    "    slack += (abs(cs.x) + abs(cs.y) + abs(ce.x) + abs(ce.y)) * (1.0 / 262144.0);\n",
    "    return max(m - slack, 0.0);\n",
    "  }\n",
    "  float d;\n",
    "  if (kind == 0 || kind == 9) {\n",
    "    d = rectDistance(p, c, e) - r;\n",
    "  } else if (kind == 1) {\n",
    "    d = length(p - c) - e.x - r;\n",
    "  } else if (kind == 2) {\n",
    "    if (item.arc.w != 1 && !(dot(e - c, e - c) > 0.0)) { return 0.0; }\n",
    "    d = item.arc.w == 1 ? segmentDistance(p, c, e) - r\n",
    "      : (item.arc.w == 2 ? segmentDistanceSquare(p, c, e, r) : segmentDistanceButt(p, c, e, r));\n",
    "  } else {\n",
    "    return 1.0e6;\n",
    "  }\n",
    "  return d > 0.0 ? max(d - slack, 0.0) : min(d + slack, 0.0);\n",
    "}\n",
    // bug-735: a PolyLine's distance in double-float -- `__canvas_polylineScan`'s, with
    // bug-687's two passes. Pass 1 is fp32 over the band: `bound` (the least `d32 + slack`)
    // and `low` (the least `d32 - slack`). Past `reach` it returns `low`; otherwise pass 2
    // evaluates in double-float only the segments that can be the nearest
    // (`d32 - slack <= bound`), each endpoint its 16.16 word plus its residual. A segment is
    // live when its endpoints' words or residuals differ -- the oracle's `len2 > 0`, exactly
    // -- or it is the whole path (`first == last`); one shorter than fp32 resolves gets an
    // unbounded slack, so it is always a candidate and never tightens `bound`.
    "static float polylineSegment32(constant MfbItem &item, constant int *edges, int e, float2 p, float halfW, thread float &slack) {\n",
    "  int first = item.gradient.x;\n",
    "  int last = item.gradient.y;\n",
    "  int i = (item.arc.z + e) * 4 + METAL_EDGE_BASE;\n",
    "  int ri = (item.arc.z + e) * 4 + METAL_EDGE_RESIDUAL_BASE;\n",
    "  bool live = first == last || edges[i] != edges[i + 2] || edges[i + 1] != edges[i + 3]\n",
    "      || edges[ri] != edges[ri + 2] || edges[ri + 1] != edges[ri + 3];\n",
    "  if (!live) { slack = -1.0; return 1.0e6; }\n",
    "  float2 a = float2(fx(edges[i]), fx(edges[i + 1]));\n",
    "  float2 b = float2(fx(edges[i + 2]), fx(edges[i + 3]));\n",
    "  float2 v = b - a;\n",
    "  float len2 = dot(v, v);\n",
    "  float2 w = p - a;\n",
    "  float t = len2 > 0.0 ? dot(w, v) / len2 : 0.0;\n",
    "  float at = clamp(t, 0.0, 1.0);\n",
    "  bool square = item.arc.w == 2;\n",
    "  if (square && ((e == first && t < 0.0) || (e == last && t > 1.0))) { at = t; }\n",
    "  float d = length(w - v * at) - halfW;\n",
    "  if (item.arc.w == 0 && len2 > 0.0) {\n",
    "    float len = sqrt(len2);\n",
    "    if (e == first) { d = max(d, -t * len); }\n",
    "    if (e == last) { d = max(d, (t - 1.0) * len); }\n",
    "  }\n",
    "  if (square) {\n",
    "    if (len2 > 0.0) {\n",
    "      float len = sqrt(len2);\n",
    "      if (e == first) { d = max(d, -t * len - halfW); }\n",
    "      if (e == last) { d = max(d, (t - 1.0) * len - halfW); }\n",
    "    } else {\n",
    "      d = max(abs(w.x), abs(w.y)) - halfW;\n",
    "    }\n",
    "  }\n",
    // plan-163-B: with join records the inner ends are cut butt; the corners are the
    // records.
    "  if (item.strokeStyle.y > 0 && len2 > 0.0) {\n",
    "    float len = sqrt(len2);\n",
    "    if (e != first) { d = max(d, -t * len); }\n",
    "    if (e != last) { d = max(d, (t - 1.0) * len); }\n",
    "  }\n",
    "  slack = len2 > 0.0 ? edgeSlack(p, a, b) + abs(halfW) * (1.0 / 262144.0) : 1.0e30;\n",
    "  return d;\n",
    "}\n",
    "static float2 polylineDistance(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float reach) {\n",
    "  int first = item.gradient.x;\n",
    "  int last = item.gradient.y;\n",
    "  bool joined = item.strokeStyle.y > 0;\n",
    "  float halfW = fx(item.misc.y);\n",
    "  float2 p = float2(x.x, y.x);\n",
    "  float bound = 1.0e6;\n",
    "  float low = 1.0e6;\n",
    "  for (int j = 0; j < band.y; ++j) {\n",
    "    int e = band.x < 0 ? j : edges[band.x + j];\n",
    "    if (e < first || e > last) { continue; }\n",
    "    float slack;\n",
    "    float d32 = polylineSegment32(item, edges, e, p, halfW, slack);\n",
    "    int kind = joined ? joinKind(item, edges, e) : 0;\n",
    "    if (kind != 0) {\n",
    "      float2 jb = joinBounds32(item, edges, e, kind, p);\n",
    "      bound = min(bound, jb.y);\n",
    "      low = min(low, jb.x);\n",
    "    }\n",
    "    if (slack < 0.0) { continue; }\n",
    "    bound = min(bound, d32 + slack);\n",
    "    low = min(low, d32 - slack);\n",
    "  }\n",
    "  if (low > reach) { return float2(low, 0.0); }\n",
    "  float2 half2 = dfFixed(item.misc.y, item.miscLo.x);\n",
    "  float2 best = float2(1.0e6, 0.0);\n",
    "  for (int j = 0; j < band.y; ++j) {\n",
    "    int e = band.x < 0 ? j : edges[band.x + j];\n",
    "    if (e < first || e > last) { continue; }\n",
    "    float slack;\n",
    "    float d32 = polylineSegment32(item, edges, e, p, halfW, slack);\n",
    "    bool segNear = slack >= 0.0 && d32 - slack <= bound;\n",
    "    int kind = joined ? joinKind(item, edges, e) : 0;\n",
    "    bool joinNear = kind != 0 && joinBounds32(item, edges, e, kind, p).x <= bound;\n",
    "    if (!segNear && !joinNear) { continue; }\n",
    // plan-163-B: `k` 0 the segment, 1..4 the edges of the join record at its start,
    // through one `dfSegment` call.
    "    float2 jd = float2(1.0e6, 0.0);\n",
    "    int winding = 0;\n",
    "    for (int k = segNear ? 0 : 1; k < (joinNear && kind == 3 ? 5 : 1); ++k) {\n",
    "    DfEdge g;\n",
    "    if (k == 0) {\n",
    "      g = dfEdge(edges, item.arc.z, e);\n",
    "    } else {\n",
    "      g.ax = joinCoord(item, edges, e, (k - 1) * 2);\n",
    "      g.ay = joinCoord(item, edges, e, (k - 1) * 2 + 1);\n",
    "      g.vx = dfSub(joinCoord(item, edges, e, (k & 3) * 2), g.ax);\n",
    "      g.by = joinCoord(item, edges, e, (k & 3) * 2 + 1);\n",
    "      g.vy = dfSub(g.by, g.ay);\n",
    "    }\n",
    "    float2 wx = dfSub(x, g.ax);\n",
    "    float2 wy = dfSub(y, g.ay);\n",
    "    DfSegment s = dfSegment(wx, wy, g.vx, g.vy);\n",
    "    if (k > 0) {\n",
    "      if (dfLess(s.d, jd)) { jd = s.d; }\n",
    "      winding += dfCrossing(g, x, y);\n",
    "      continue;\n",
    "    }\n",
    "    float2 d = dfSub(s.d, half2);\n",
    "    if (item.arc.w == 0 && s.len2.x > 0.0) {\n",
    "      float2 len = dfSqrt(s.len2);\n",
    "      if (e == first) { d = dfMax(d, -dfMul(s.t, len)); }\n",
    "      if (e == last) { d = dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len)); }\n",
    "    }\n",
    // plan-163-B: a square OUTER end -- to the line past the endpoint, then the end plane
    // moved out by the half-width; a lone zero-length segment is the square.
    "    if (item.arc.w == 2) {\n",
    "      if (s.len2.x > 0.0) {\n",
    "        float2 len = dfSqrt(s.len2);\n",
    "        bool before = e == first && dfLess(s.t, float2(0.0, 0.0));\n",
    "        bool after = e == last && dfLess(float2(1.0, 0.0), s.t);\n",
    "        if (before || after) {\n",
    "          d = dfSub(dfDiv(dfAbs(dfSub(dfMul(wx, g.vy), dfMul(wy, g.vx))), len), half2);\n",
    "        }\n",
    "        if (e == first) { d = dfMax(d, dfSub(-dfMul(s.t, len), half2)); }\n",
    "        if (e == last) { d = dfMax(d, dfSub(dfMul(dfSub(s.t, float2(1.0, 0.0)), len), half2)); }\n",
    "      } else {\n",
    "        d = dfSub(dfMax(dfAbs(wx), dfAbs(wy)), half2);\n",
    "      }\n",
    "    }\n",
    "    if (joined && s.len2.x > 0.0) {\n",
    "      float2 len = dfSqrt(s.len2);\n",
    "      if (e != first) { d = dfMax(d, -dfMul(s.t, len)); }\n",
    "      if (e != last) { d = dfMax(d, dfMul(dfSub(s.t, float2(1.0, 0.0)), len)); }\n",
    "    }\n",
    "    if (dfLess(d, best)) { best = d; }\n",
    "    }\n",
    "    if (joinNear) {\n",
    "      if ((winding & 1) != 0) { jd = -jd; }\n",
    "      float2 piece = dfJoinPiece(item, edges, e, kind, x, y, jd);\n",
    "      if (dfLess(piece, best)) { best = piece; }\n",
    "    }\n",
    "  }\n",
    "  return best;\n",
    "}\n",
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
    "static float dashWord(constant MfbItem &item, constant int *edges, int k) { return as_type<float>(edges[METAL_DASH_BASE + item.strokeStyle.z + k]); }\n",
    "static float dfF(float2 v) { return v.x + v.y; }\n",
    "static float dashWrap(constant MfbItem &item, constant int *edges, float x) {\n",
    "  float period = dashWord(item, edges, 2);\n",
    "  return x - floor(x / period) * period;\n",
    "}\n",
    "static bool dashOnAt(constant MfbItem &item, constant int *edges, float x) {\n",
    "  float w = dashWrap(item, edges, x);\n",
    "  int n = int(dashWord(item, edges, 1));\n",
    "  for (int j = 0; j < n; ++j) {\n",
    "    if (w < dashWord(item, edges, 3 + j)) { return (j & 1) == 0; }\n",
    "  }\n",
    "  return true;\n",
    "}\n",
    "static bool dashOnBefore(constant MfbItem &item, constant int *edges, float x) {\n",
    "  float w = dashWrap(item, edges, x);\n",
    "  if (w <= 0.0) { w = dashWord(item, edges, 2); }\n",
    "  int n = int(dashWord(item, edges, 1));\n",
    "  for (int j = 0; j < n; ++j) {\n",
    "    if (w <= dashWord(item, edges, 3 + j)) { return (j & 1) == 0; }\n",
    "  }\n",
    "  return false;\n",
    "}\n",
    // `__canvas_dashLocate` then `__canvas_dashAlongPiece`: the signed distance along the
    // outline from `u` to the dashes on the piece [s0, s1] (negative inside one), a dash
    // extended by `ext` past a TRUE end; those ends near `u` into `ends`, flagged by `has`.
    "static float dashAlong(constant MfbItem &item, constant int *edges, float u, float s0, float s1, float len_, bool closed, float ext, thread float2 &ends, thread bool2 &has) {\n",
    "  float at = min(max(u, s0), s1);\n",
    "  float phase = dashWord(item, edges, 0);\n",
    "  float period = dashWord(item, edges, 2);\n",
    "  int n = int(dashWord(item, edges, 1));\n",
    "  float w = dashWrap(item, edges, at + phase);\n",
    "  float lowW = 0.0;\n",
    "  float highW = period;\n",
    "  int j = 0;\n",
    "  bool found = false;\n",
    "  for (; j < n; ++j) {\n",
    "    float c = dashWord(item, edges, 3 + j);\n",
    "    if (w < c) { highW = c; found = true; break; }\n",
    "    lowW = c;\n",
    "  }\n",
    "  bool on = (j & 1) == 0;\n",
    "  if (!found) { on = true; lowW = period; highW = period + dashWord(item, edges, 3); }\n",
    "  float lo = at - (w - lowW);\n",
    "  float hi = at + (highW - w);\n",
    "  if (closed) {\n",
    "    bool onStart = dashOnAt(item, edges, phase);\n",
    "    bool onEnd = dashOnBefore(item, edges, len_ + phase);\n",
    "    if (lo < 0.0) { lo = on == onEnd ? -1.0e9 : 0.0; }\n",
    "    if (hi > len_) { hi = on == onStart ? 1.0e9 : len_; }\n",
    "  } else {\n",
    "    if (lo < 0.0) { lo = on ? 0.0 : -1.0e9; }\n",
    "    if (hi > len_) { hi = on ? len_ : 1.0e9; }\n",
    "  }\n",
    "  ends = float2(0.0);\n",
    "  has = bool2(false);\n",
    "  if (on) {\n",
    "    float left = s0;\n",
    "    float right = s1;\n",
    "    if (lo >= s0) { left = lo - ext; has.x = true; ends.x = lo; }\n",
    "    if (hi <= s1) { right = hi + ext; has.y = true; ends.y = hi; }\n",
    "    return max(left - u, u - right);\n",
    "  }\n",
    "  float d = 1.0e6;\n",
    "  if (lo > s0) { d = u - lo - ext; has.x = true; ends.x = lo; }\n",
    "  if (hi < s1) { d = min(d, hi - ext - u); has.y = true; ends.y = hi; }\n",
    "  return d;\n",
    "}\n",
    // The oracle's trigonometry (`__canvas_atan2`, `__canvas_sin`, `__canvas_cos`) as series in
    // `+ - * / sqrt`, not the driver's `atan`/`sin`/`cos`: Vulkan leaves `atan` at 4096 ULP and
    // `sin`/`cos` at 2^-11 absolute, which on a real driver moves a dash end on a 60 px circle
    // by a coverage step (measured on box 206's Intel UHD 630). Each is folded first, so the
    // series runs where it converges: the arctangent's ratio into [0, 1] and halved twice, the
    // sine's and cosine's angle into [-pi/2, pi/2].
    "static float dashAtan(float y, float x) {\n",
    "  float ax = abs(x);\n",
    "  float ay = abs(y);\n",
    "  if (ax == 0.0 && ay == 0.0) { return 0.0; }\n",
    "  bool swapped = ay > ax;\n",
    "  float z = swapped ? ax / ay : ay / ax;\n",
    "  z = z / (1.0 + sqrt(1.0 + z * z));\n",
    "  z = z / (1.0 + sqrt(1.0 + z * z));\n",
    "  float w = z * z;\n",
    "  float p = 1.0 / 21.0;\n",
    "  p = w * p - 1.0 / 19.0;\n",
    "  p = w * p + 1.0 / 17.0;\n",
    "  p = w * p - 1.0 / 15.0;\n",
    "  p = w * p + 1.0 / 13.0;\n",
    "  p = w * p - 1.0 / 11.0;\n",
    "  p = w * p + 1.0 / 9.0;\n",
    "  p = w * p - 1.0 / 7.0;\n",
    "  p = w * p + 1.0 / 5.0;\n",
    "  p = w * p - 1.0 / 3.0;\n",
    "  p = w * p + 1.0;\n",
    "  float a = 4.0 * z * p;\n",
    "  if (swapped) { a = PI / 2.0 - a; }\n",
    "  if (x < 0.0) { a = PI - a; }\n",
    "  if (y < 0.0) { a = 2.0 * PI - a; }\n",
    "  return a >= 2.0 * PI ? 0.0 : a;\n",
    "}\n",
    "static float dashWrapAngle(float a) { return a - 2.0 * PI * floor((a + PI) / (2.0 * PI)); }\n",
    "static float dashFold(float a) {\n",
    "  float x = dashWrapAngle(a);\n",
    "  if (x > PI / 2.0) { return PI - x; }\n",
    "  if (x < -PI / 2.0) { return -PI - x; }\n",
    "  return x;\n",
    "}\n",
    "static float dashSin(float a) {\n",
    "  float x = dashFold(a);\n",
    "  float x2 = x * x;\n",
    "  return x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))));\n",
    "}\n",
    "static float dashCos(float a) {\n",
    "  float x = dashFold(a);\n",
    "  float x2 = x * x;\n",
    "  float m = 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0 * (1.0 - x2 / 90.0))));\n",
    "  return abs(dashWrapAngle(a)) > PI / 2.0 ? -m : m;\n",
    "}\n",
    // The direction of `q` in [0, 2pi), from +X towards +Y.
    "static float dashAngle(float2 q) { return dashAtan(q.y, q.x); }\n",
    // A join record's corner piece at `w`, the pixel less its vertex V, from its points
    // relative to V (`__canvas_joinPieceDistance`).
    "static float2 joinRel(constant MfbItem &item, constant int *edges, int r, int k) {\n",
    "  return float2(dfF(dfSub(joinCoord(item, edges, r, k * 2), joinCoord(item, edges, r, 0))),\n",
    "        dfF(dfSub(joinCoord(item, edges, r, k * 2 + 1), joinCoord(item, edges, r, 1))));\n",
    "}\n",
    "static float joinPieceAt(constant MfbItem &item, constant int *edges, int r, int kind, float2 w) {\n",
    "  float2 a = joinRel(item, edges, r, 1);\n",
    "  float disc = length(w) - length(a);\n",
    "  if (kind == 1) { return disc; }\n",
    "  float2 b = joinRel(item, edges, r, 3);\n",
    "  if (kind == 2) {\n",
    "    float2 c = b - a;\n",
    "    float o = (c.x * (w.y - a.y) - c.y * (w.x - a.x)) / length(c);\n",
    "    if (c.x * (-a.y) - c.y * (-a.x) > 0.0) { o = -o; }\n",
    "    return max(disc, o);\n",
    "  }\n",
    "  float2 pts[4] = { float2(0.0), a, joinRel(item, edges, r, 2), b };\n",
    "  float d = 1.0e6;\n",
    "  bool inside = false;\n",
    "  for (int k = 0; k < 4; ++k) {\n",
    "    float2 s0 = pts[k];\n",
    "    float2 s1 = pts[(k + 1) & 3];\n",
    "    d = min(d, segmentDistance(w, s0, s1));\n",
    "    if ((s0.y > w.y) != (s1.y > w.y) && w.x < s0.x + (w.y - s0.y) / (s1.y - s0.y) * (s1.x - s0.x)) {\n",
    "      inside = !inside;\n",
    "    }\n",
    "  }\n",
    "  return min(disc, inside ? -d : d);\n",
    "}\n",
    // plan-164-C: the curve shapes' functions, generated from `shaders/curves.tpl`
    // (`runtime/canvas/curve_shader.rs`) and spliced in here when the source is built.
    "@CURVE_SHADER@\n",
    // `__canvas_dashedEdges`: a polygon's or polyline's band edges, a line's one edge, or a
    // rectangle's or picture's four sides, each band cut by the dashes on its own [s0, s1],
    // with the corner piece wherever a dash runs through a vertex. An open path with no
    // length is the dot its cap draws (`__canvas_dashDot`).
    "static float dashedEdges(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float halfW, int cap) {\n",
    "  int kind = item.misc.x;\n",
    "  bool poly = kind == 4 || kind == 10;\n",
    "  int count = poly ? band.y : (kind == 2 ? 1 : 4);\n",
    "  int data = 3 + int(dashWord(item, edges, 1));\n",
    "  float phase = dashWord(item, edges, 0);\n",
    "  float ext = cap == 2 ? halfW : 0.0;\n",
    "  float2 cx = dfFixed(item.shape.x, item.shapeLo.x);\n",
    "  float2 cy = dfFixed(item.shape.y, item.shapeLo.y);\n",
    "  float2 hx = dfFixed(item.shape.z, item.shapeLo.z);\n",
    "  float2 hy = dfFixed(item.shape.w, item.shapeLo.w);\n",
    "  if ((kind == 2 || kind == 10) && dashWord(item, edges, data + 2) == 0.0) {\n",
    "    float2 ox = cx;\n",
    "    float2 oy = cy;\n",
    "    if (kind == 10) {\n",
    "      DfEdge g0 = dfEdge(edges, item.arc.z, 0);\n",
    "      ox = g0.ax;\n",
    "      oy = g0.ay;\n",
    "    }\n",
    "    float2 w0 = float2(dfF(dfSub(x, ox)), dfF(dfSub(y, oy)));\n",
    "    if (cap == 0 || !dashOnAt(item, edges, phase)) { return 1.0e6; }\n",
    "    if (cap == 2) { return max(abs(w0.x), abs(w0.y)) - halfW; }\n",
    "    return length(w0) - halfW;\n",
    "  }\n",
    "  float best = 1.0e6;\n",
    "  for (int j = 0; j < count; ++j) {\n",
    "    int e = poly ? (band.x < 0 ? j : edges[band.x + j]) : j;\n",
    "    float2 ax;\n",
    "    float2 ay;\n",
    "    float2 v;\n",
    "    if (poly) {\n",
    "      DfEdge g = dfEdge(edges, item.arc.z, e);\n",
    "      ax = g.ax;\n",
    "      ay = g.ay;\n",
    "      v = float2(dfF(g.vx), dfF(g.vy));\n",
    "    } else if (kind == 2) {\n",
    "      ax = cx;\n",
    "      ay = cy;\n",
    "      v = float2(dfF(dfSub(hx, cx)), dfF(dfSub(hy, cy)));\n",
    "    } else {\n",
    "      int f = (e + 1) & 3;\n",
    "      ax = (e == 0 || e == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);\n",
    "      ay = (e < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);\n",
    "      float2 bx = (f == 0 || f == 3) ? dfSub(cx, hx) : dfAdd(cx, hx);\n",
    "      float2 by = (f < 2) ? dfSub(cy, hy) : dfAdd(cy, hy);\n",
    "      v = float2(dfF(dfSub(bx, ax)), dfF(dfSub(by, ay)));\n",
    "    }\n",
    "    float len2 = dot(v, v);\n",
    "    if (!(len2 > 0.0)) { continue; }\n",
    "    float2 w = float2(dfF(dfSub(x, ax)), dfF(dfSub(y, ay)));\n",
    "    float s0 = dashWord(item, edges, data + e * 3);\n",
    "    float s1 = dashWord(item, edges, data + e * 3 + 1);\n",
    "    float ring = dashWord(item, edges, data + e * 3 + 2);\n",
    "    float el = sqrt(len2);\n",
    "    float u = s0 + dot(w, v) / el;\n",
    "    float bandD = abs(w.x * v.y - w.y * v.x) / el - halfW;\n",
    "    if (max(bandD, max(s0 - u, u - s1) - halfW) < best) {\n",
    "      float2 ends;\n",
    "      bool2 has;\n",
    "      float piece = max(bandD, dashAlong(item, edges, u, s0, s1, abs(ring), ring > 0.0, ext, ends, has));\n",
    "      if (cap == 1) {\n",
    "        if (has.x) { piece = min(piece, length(w - v * ((ends.x - s0) / el)) - halfW); }\n",
    "        if (has.y) { piece = min(piece, length(w - v * ((ends.y - s0) / el)) - halfW); }\n",
    "      }\n",
    "      best = min(best, piece);\n",
    "    }\n",
    "    if (item.strokeStyle.y > 0) {\n",
    "      int jk = joinKind(item, edges, e);\n",
    "      if (jk != 0) {\n",
    "        float before = (ring > 0.0 && s0 == 0.0) ? abs(ring) : s0;\n",
    "        if (dashOnBefore(item, edges, before + phase) && dashOnAt(item, edges, s0 + phase)) {\n",
    "          float2 wv = float2(dfF(dfSub(x, joinCoord(item, edges, e, 0))), dfF(dfSub(y, joinCoord(item, edges, e, 1))));\n",
    "          best = min(best, joinPieceAt(item, edges, e, jk, wv));\n",
    "        }\n",
    "      }\n",
    "    }\n",
    "  }\n",
    "  return best;\n",
    "}\n",
    // `__canvas_rrectPosition`: a rounded rectangle's perimeter position of the outline point
    // nearest `q` (from its centre), from the top side's left end, clockwise.
    "static float rrectPosition(float2 q, float iw, float ih, float r) {\n",
    "  float qr = r * (PI / 2.0);\n",
    "  float ax = abs(q.x) - iw;\n",
    "  float ay = abs(q.y) - ih;\n",
    "  if (ax > 0.0 && ay > 0.0) {\n",
    "    if (q.x > 0.0 && q.y < 0.0) {\n",
    "      return 2.0 * iw + r * min(max(dashAngle(float2(q.x - iw, q.y + ih)) - 1.5 * PI, 0.0), PI / 2.0);\n",
    "    }\n",
    "    if (q.x > 0.0) {\n",
    "      return 2.0 * iw + qr + 2.0 * ih + r * min(dashAngle(float2(q.x - iw, q.y - ih)), PI / 2.0);\n",
    "    }\n",
    "    if (q.y > 0.0) {\n",
    "      return 4.0 * iw + 2.0 * qr + 2.0 * ih + r * min(max(dashAngle(float2(q.x + iw, q.y - ih)) - PI / 2.0, 0.0), PI / 2.0);\n",
    "    }\n",
    "    return 4.0 * iw + 3.0 * qr + 4.0 * ih + r * min(max(dashAngle(float2(q.x + iw, q.y + ih)) - PI, 0.0), PI / 2.0);\n",
    "  }\n",
    "  if (ay >= ax) {\n",
    "    if (q.y < 0.0) { return q.x + iw; }\n",
    "    return 2.0 * iw + 2.0 * qr + 2.0 * ih + (iw - q.x);\n",
    "  }\n",
    "  if (q.x > 0.0) { return 2.0 * iw + qr + (q.y + ih); }\n",
    "  return 4.0 * iw + 3.0 * qr + 2.0 * ih + (ih - q.y);\n",
    "}\n",
    // The outline point at perimeter position `s` of a curve, from its centre
    // (`__canvas_dashCurveX`).
    "static float2 dashCurvePoint(constant MfbItem &item, constant int *edges, int kind, int data, float s) {\n",
    "  if (kind == 1 || kind == 3) {\n",
    "    float r = dfF(dfFixed(item.shape.z, item.shapeLo.z));\n",
    "    float a = s / r + (kind == 3 ? dashWord(item, edges, data) : 0.0);\n",
    "    return r * float2(dashCos(a), dashSin(a));\n",
    "  }\n",
    "  if (kind == 7) {\n",
    "    float rx = dfF(dfFixed(item.shape.z, item.shapeLo.z));\n",
    "    float ry = dfF(dfFixed(item.shape.w, item.shapeLo.w));\n",
    "    float ca = dfF(dfFixed(item.ellipse.x, item.miscLo.z));\n",
    "    float sa = dfF(dfFixed(item.ellipse.y, item.miscLo.w));\n",
    "    int steps = item.strokeStyle.w - data - 1;\n",
    "    int lo = 0;\n",
    "    int hi = steps;\n",
    "    while (hi - lo > 1) {\n",
    "      int mid = (lo + hi) / 2;\n",
    "      if (dashWord(item, edges, data + mid) <= s) { lo = mid; } else { hi = mid; }\n",
    "    }\n",
    "    float t0 = dashWord(item, edges, data + lo);\n",
    "    float t1 = dashWord(item, edges, data + lo + 1);\n",
    "    float t = (float(lo) + (s - t0) / (t1 - t0)) * (2.0 * PI / float(steps));\n",
    "    float ox = rx * dashCos(t);\n",
    "    float oy = ry * dashSin(t);\n",
    "    return float2(ox * ca - oy * sa, ox * sa + oy * ca);\n",
    "  }\n",
    "  float iw = dfF(dfFixed(item.shape.z, item.shapeLo.z));\n",
    "  float ih = dfF(dfFixed(item.shape.w, item.shapeLo.w));\n",
    "  float r = dfF(dfFixed(item.misc.y, item.miscLo.x));\n",
    "  float qr = r * (PI / 2.0);\n",
    "  float right = 2.0 * iw + qr;\n",
    "  float bottomRight = right + 2.0 * ih;\n",
    "  float bottom = bottomRight + qr;\n",
    "  float bottomLeft = bottom + 2.0 * iw;\n",
    "  float left = bottomLeft + qr;\n",
    "  float topLeft = left + 2.0 * ih;\n",
    "  if (s < 2.0 * iw) { return float2(-iw + s, -ih - r); }\n",
    "  if (s < right) {\n",
    "    float a = 1.5 * PI + (s - 2.0 * iw) / r;\n",
    "    return float2(iw, -ih) + r * float2(dashCos(a), dashSin(a));\n",
    "  }\n",
    "  if (s < bottomRight) { return float2(iw + r, -ih + (s - right)); }\n",
    "  if (s < bottom) {\n",
    "    float a = (s - bottomRight) / r;\n",
    "    return float2(iw, ih) + r * float2(dashCos(a), dashSin(a));\n",
    "  }\n",
    "  if (s < bottomLeft) { return float2(iw - (s - bottom), ih + r); }\n",
    "  if (s < left) {\n",
    "    float a = PI / 2.0 + (s - bottomLeft) / r;\n",
    "    return float2(-iw, ih) + r * float2(dashCos(a), dashSin(a));\n",
    "  }\n",
    "  if (s < topLeft) { return float2(-iw - r, ih - (s - left)); }\n",
    "  float a = PI + (s - topLeft) / r;\n",
    "  return float2(-iw, -ih) + r * float2(dashCos(a), dashSin(a));\n",
    "}\n",
    // `__canvas_dashedCurve`: a circle, arc, ellipse or rounded rectangle, cut at the
    // perimeter position of the pixel's nearest outline point.
    "static float dashedCurve(constant MfbItem &item, constant int *edges, float2 x, float2 y, float halfW, int cap) {\n",
    "  int kind = item.misc.x;\n",
    "  float2 q = float2(dfF(dfSub(x, dfFixed(item.shape.x, item.shapeLo.x))),\n",
    "         dfF(dfSub(y, dfFixed(item.shape.y, item.shapeLo.y))));\n",
    "  int data = 3 + int(dashWord(item, edges, 1));\n",
    "  float bandD;\n",
    "  float u;\n",
    "  float len_;\n",
    "  bool closed = true;\n",
    "  if (kind == 1 || kind == 3) {\n",
    "    float r = dfF(dfFixed(item.shape.z, item.shapeLo.z));\n",
    "    bandD = abs(length(q) - r) - halfW;\n",
    "    float a = dashAngle(q);\n",
    "    if (kind == 1) {\n",
    "      u = r * a;\n",
    "      len_ = 2.0 * PI * r;\n",
    "    } else {\n",
    "      float start = dashWord(item, edges, data);\n",
    "      float sweep = dashWord(item, edges, data + 1);\n",
    "      float rel = a - start;\n",
    "      rel -= floor(rel / (2.0 * PI)) * (2.0 * PI);\n",
    "      if (rel > sweep && rel - sweep >= 2.0 * PI - rel) { rel -= 2.0 * PI; }\n",
    "      u = r * rel;\n",
    "      len_ = r * sweep;\n",
    "      closed = false;\n",
    "    }\n",
    "  } else if (kind == 7) {\n",
    "    float rx = dfF(dfFixed(item.shape.z, item.shapeLo.z));\n",
    "    float ry = dfF(dfFixed(item.shape.w, item.shapeLo.w));\n",
    "    float ca = dfF(dfFixed(item.ellipse.x, item.miscLo.z));\n",
    "    float sa = dfF(dfFixed(item.ellipse.y, item.miscLo.w));\n",
    "    float2 o = float2(q.x * ca + q.y * sa, q.y * ca - q.x * sa);\n",
    "    float t;\n",
    "    if (rx == ry) {\n",
    "      t = dashAngle(o);\n",
    "      bandD = abs(length(o) - rx) - halfW;\n",
    "    } else {\n",
    "      float2 a = abs(o);\n",
    "      float2 c0 = float2(1.0, 0.0);\n",
    "      float2 c1 = float2(0.0, 1.0);\n",
    "      float2 m = c0;\n",
    "      for (int k = 0; k < 24; ++k) {\n",
    "        m = normalize(c0 + c1);\n",
    "        float g = (a.x - rx * m.x) * (-rx * m.y) + (a.y - ry * m.y) * (ry * m.x);\n",
    "        if (g > 0.0) { c0 = m; } else { c1 = m; }\n",
    "      }\n",
    "      float dist = length(a - float2(rx, ry) * m);\n",
    "      if (dot(a / float2(rx, ry), a / float2(rx, ry)) < 1.0) { dist = -dist; }\n",
    "      bandD = abs(dist) - halfW;\n",
    "      t = dashAngle(float2(o.x < 0.0 ? -m.x : m.x, o.y < 0.0 ? -m.y : m.y));\n",
    "    }\n",
    "    int steps = item.strokeStyle.w - data - 1;\n",
    "    float f = t / (2.0 * PI / float(steps));\n",
    "    int k = clamp(int(f), 0, steps - 1);\n",
    "    float t0 = dashWord(item, edges, data + k);\n",
    "    float t1 = dashWord(item, edges, data + k + 1);\n",
    "    u = t0 + (t1 - t0) * (f - float(k));\n",
    "    len_ = dashWord(item, edges, data + steps);\n",
    "  } else {\n",
    "    float iw = dfF(dfFixed(item.shape.z, item.shapeLo.z));\n",
    "    float ih = dfF(dfFixed(item.shape.w, item.shapeLo.w));\n",
    "    float r = dfF(dfFixed(item.misc.y, item.miscLo.x));\n",
    "    bandD = abs(rectDistance(q, float2(0.0), float2(iw, ih)) - r) - halfW;\n",
    "    u = rrectPosition(q, iw, ih, r);\n",
    "    len_ = 4.0 * iw + 4.0 * ih + 4.0 * (r * (PI / 2.0));\n",
    "  }\n",
    "  float2 ends;\n",
    "  bool2 has;\n",
    "  float d = max(bandD, dashAlong(item, edges, u, 0.0, len_, len_, closed, cap == 2 ? halfW : 0.0, ends, has));\n",
    "  if (cap == 1) {\n",
    "    if (has.x) { d = min(d, length(q - dashCurvePoint(item, edges, kind, data, ends.x)) - halfW); }\n",
    "    if (has.y) { d = min(d, length(q - dashCurvePoint(item, edges, kind, data, ends.y)) - halfW); }\n",
    "  }\n",
    "  return d;\n",
    "}\n",
    // `__canvas_dashedStroke`: a dashed item's stroke distance at the shape-space point --
    // for a Line, Arc or PolyLine its whole distance, for every other kind its band's.
    "static float2 dashedStroke(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float halfW) {\n",
    "  int kind = item.misc.x;\n",
    "  bool rounded = kind == 0 && (item.misc.y != 0 || item.miscLo.x != 0);\n",
    "  bool curve = kind == 1 || kind == 3 || kind == 7 || rounded;\n",
    "  return float2(curve ? dashedCurve(item, edges, x, y, halfW, item.arc.w) : dashedEdges(item, edges, band, x, y, halfW, item.arc.w), 0.0);\n",
    "}\n",
    "static float2 sampleDistance(constant MfbItem &item, constant int *edges, int2 band, float2 x, float2 y, float reach, bool clip) {\n",
    "  int kind = item.misc.x;\n",
    // plan-163-C: a path clip's edges under its own rule, through the polygon's walk -- one
    // `edgeDistance` call site, for the shader compilers' sake.
    "  if (clip || kind == 4) {\n",
    "    return edgeDistance(edges, clip ? item.clipPath.x : item.arc.z, band.y, band.x, x, y, reach,\n",
    "                        clip ? (item.clipPath.z >> 2) : item.arc.y);\n",
    "  }\n",
    // plan-164-C: a CurvePolygon's signed distance, or a PolyCurve's stroke -- dashed or
    // not -- from its pieces. A PolyCurve is stroke-as-fill, its half-width the radius.
    "  if (kind == 11 || kind == 12) {\n",
    "    return float2(curveWalk(item, edges, band, x, y, kind == 12 ? 0 : (item.strokeStyle.w > 0 ? 3 : 1),\n",
    "                            dfF(dfFixed(item.misc.y, item.miscLo.x)), item.arc.w), 0.0);\n",
    "  }\n",
    // plan-163-B: a dashed Line, Arc or PolyLine is its dashed stroke.
    "  if (item.strokeStyle.w > 0 && (kind == 2 || kind == 3 || kind == 10)) {\n",
    "    return dashedStroke(item, edges, band, x, y, dfF(dfFixed(item.misc.y, item.miscLo.x)));\n",
    "  }\n",
    "  if (kind == 10) { return polylineDistance(item, edges, band, x, y, reach); }\n",
    "  float v = farBound(item, float2(x.x, y.x));\n",
    "  if (abs(v) > reach) { return float2(v, 0.0); }\n",
    "  return dfShapeDistance(item, x, y);\n",
    "}\n",
    "static float2 dfMapX(constant MfbItem &item, float2 x, float2 y) {\n",
    "  return dfAdd(dfAdd(dfMulF(x, as_type<float>(item.xform0.x)), dfMulF(y, as_type<float>(item.xform0.z))),\n",
    "               float2(as_type<float>(item.xform1.x), 0.0));\n",
    "}\n",
    "static float2 dfMapY(constant MfbItem &item, float2 x, float2 y) {\n",
    "  return dfAdd(dfAdd(dfMulF(x, as_type<float>(item.xform0.y)), dfMulF(y, as_type<float>(item.xform0.w))),\n",
    "               float2(as_type<float>(item.xform1.y), 0.0));\n",
    "}\n",
    // The double-float shape-space point: the surface point with the group offset -- its
    // 16.16 word plus, since bug-735, its residual -- taken off exactly.
    "static float4 dfShapePoint(float2 surface, int4 offset) {\n",
    "  return float4(dfSub(float2(surface.x, 0.0), dfFixed(offset.x, offset.z)), dfSub(float2(surface.y, 0.0), dfFixed(offset.y, offset.w)));\n",
    "}\n",
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
    "static int clipRect(constant MfbItem &item, float2 x, float2 y) {\n",
    "  if (!fixedLess(item.clip.x, item.clipLo.x, item.clip.z, item.clipLo.z)\n",
    "      || !fixedLess(item.clip.y, item.clipLo.y, item.clip.w, item.clipLo.w)) { return 0; }\n",
    "  float2 p = float2(x.x, y.x);\n",
    "  float2 lo = float2(fx(item.clip.x), fx(item.clip.y));\n",
    "  float2 hi = float2(fx(item.clip.z), fx(item.clip.w));\n",
    "  float d = rectDistance(p, (lo + hi) * 0.5, (hi - lo) * 0.5);\n",
    "  float slack = edgeSlack(p, lo, hi);\n",
    "  if (d - slack > 0.5) { return 0; }\n",
    "  if (d + slack < -0.5) { return 255; }\n",
    "  float2 x0 = dfFixed(item.clip.x, item.clipLo.x);\n",
    "  float2 y0 = dfFixed(item.clip.y, item.clipLo.y);\n",
    "  float2 x1 = dfFixed(item.clip.z, item.clipLo.z);\n",
    "  float2 y1 = dfFixed(item.clip.w, item.clipLo.w);\n",
    "  float2 dx = dfSub(x, dfMulF(dfAdd(x0, x1), 0.5));\n",
    "  float2 dy = dfSub(y, dfMulF(dfAdd(y0, y1), 0.5));\n",
    "  return dfCoverage(dfRectDistance(dx, dy, dfMulF(dfSub(x1, x0), 0.5), dfMulF(dfSub(y1, y0), 0.5)));\n",
    "}\n",
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
    // which is Lipschitz only to its own ~1e-4 px. An arc is not Lipschitz at all (out
    // of sweep it jumps to 1e6), so a transformed arc takes no shortcut; neither does
    // any neighbour sample, whose exact value the gradient needs. The clip's centre
    // sample takes the same shortcut at `0.5 * max(F, 1)` (bug-995, argued at
    // `clipReach` in `shapeCoverage` below).
    // plan-172-H: a rectangle clip's signed distance in double-float at the item's own
    // point. `clipRect` above answers coverage and is right while the rectangle is
    // axis-aligned on the surface; under a group matrix it is not, so its distance is
    // sampled like a path clip's and divided by its own gradient
    // (`__canvas_clipRectSurface`). A rectangle with no area encloses nothing.
    "static float2 clipRectDistance(constant MfbItem &item, float2 x, float2 y) {\n",
    "  if (!fixedLess(item.clip.x, item.clipLo.x, item.clip.z, item.clipLo.z)\n",
    "      || !fixedLess(item.clip.y, item.clipLo.y, item.clip.w, item.clipLo.w)) { return float2(1.0e6, 0.0); }\n",
    "  float2 x0 = dfFixed(item.clip.x, item.clipLo.x);\n",
    "  float2 y0 = dfFixed(item.clip.y, item.clipLo.y);\n",
    "  float2 x1 = dfFixed(item.clip.z, item.clipLo.z);\n",
    "  float2 y1 = dfFixed(item.clip.w, item.clipLo.w);\n",
    "  float2 dx = dfSub(x, dfMulF(dfAdd(x0, x1), 0.5));\n",
    "  float2 dy = dfSub(y, dfMulF(dfAdd(y0, y1), 0.5));\n",
    "  return dfRectDistance(dx, dy, dfMulF(dfSub(x1, x0), 0.5), dfMulF(dfSub(y1, y0), 0.5));\n",
    "}\n",
    // plan-172-H: `pt` arrives already through the groups' inverse linear part when
    // `grouped`, and `gi` is that part: the four neighbour samples are half a SURFACE
    // pixel away, which in the group's space is `gi` times that. An item inside a group
    // with a matrix takes the transformed path whether or not it has a transform of its
    // own -- `mapped` -- while `itemMapped` alone says whether its own inverse applies.
    "static int4 shapeCoverage(constant MfbItem &item, constant int *edges, float4 pt, float4 gi, bool grouped) {\n",
    "  float2 x = pt.xy;\n",
    "  float2 y = pt.zw;\n",
    "  bool itemMapped = hasTransform(item);\n",
    "  bool mapped = itemMapped || grouped;\n",
    "  int2 band = polygonBand(item, edges, itemMapped ? dfMapY(item, x, y).x : y.x);\n",
    "  float2 half2 = dfFixed(item.misc.z, item.miscLo.y);\n",
    "  bool stroked = dfLess(float2(0.0, 0.0), half2);\n",
    "  float reach = 0.5 + max(half2.x, 0.0);\n",
    // bug-995: the clip's reach -- the shape's without the stroke half, and for any kind
    // (a clip's distance is 1-Lipschitz whatever the item is, an arc's too).
    "  float clipReach = 0.5;\n",
    "  if (mapped) {\n",
    "    float2 c0 = itemMapped ? float2(as_type<float>(item.xform0.x), as_type<float>(item.xform0.y)) : float2(1.0, 0.0);\n",
    "    float2 c1 = itemMapped ? float2(as_type<float>(item.xform0.z), as_type<float>(item.xform0.w)) : float2(0.0, 1.0);\n",
    // The composed inverse's norm is at most the product of the two norms.
    "    float norm = sqrt(dot(c0, c0) + dot(c1, c1));\n",
    "    if (grouped) { norm = (itemMapped ? norm : 1.0) * length(gi); }\n",
    "    float frob = norm * (1.0 + 1.0 / 256.0) + 1.0 / 256.0;\n",
    "    reach = item.misc.x == 3 ? 1.0e30 : max(half2.x, 0.0) + 0.5 * max(frob, 1.0);\n",
    "    clipReach = 0.5 * max(frob, 1.0);\n",
    "  }\n",
    // plan-163-B: a polygon, rectangle or picture with join records strokes from its own
    // distance (`joinedStroke`), which a miter can reach past `half` from the outline --
    // so the centre's shape distance cannot decide the neighbours there.
    // plan-163-B: so can a dashed one, which strokes from `dashedStroke`.
    "  bool dashed = item.strokeStyle.w > 0;\n",
    // plan-164-C: nor a PolyCurve, stroke-as-fill like a PolyLine: `sampleDistance` is
    // its whole stroke.
    "  bool joined = (item.strokeStyle.y > 0 || dashed) && item.misc.x != 10 && item.misc.x != 11;\n",
    "  float strokeReach = mapped ? reach - max(half2.x, 0.0) : 0.5;\n",
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
    //   reach of the band (`emit_band_index`, `r >= 0.5 + 1.5 F` for the ITEM's inverse):
    //   the neighbours' crossings and nearest edges are in it, so what is sampled is
    //   Lipschitz too. This is the shape's premise as well, and like the shape's it
    //   does not cover a group's matrix, which the band index never sees.
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
    "  int clipMode = item.clipPath.z & 3;\n",
    "  bool clipMapped = itemMapped || grouped;\n",
    // plan-172-H: a rectangle clip is sampled too, under a group matrix.
    "  bool clipSampled = clipMode == 2 || (clipMode == 1 && grouped);\n",
    "  int2 cband = clipMode == 2 ? clipBandAt(item, edges, itemMapped ? dfMapY(item, x, y).x : y.x) : int2(-1, 0);\n",
    "  float2 d[10];\n",
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
    "  float2 cqx = itemMapped ? dfMapX(item, x, y) : x;\n",
    "  float2 cqy = itemMapped ? dfMapY(item, x, y) : y;\n",
    "  d[0] = sampleDistance(item, edges, band, cqx, cqy, reach, false);\n",
    "  if (!joined && abs(d[0].x) > reach) { mapped = false; }\n",
    "  if (mapped || clipSampled) {\n",
    "    for (int k = 1; k < 10; ++k) {\n",
    "      bool clipK = k >= 5;\n",
    "      int j = clipK ? k - 5 : k;\n",
    "      if (clipK ? (!clipSampled || j >= (clipMapped ? 5 : 1)) : !mapped) { continue; }\n",
    "      float ox = j == 1 ? 0.5 : (j == 2 ? -0.5 : 0.0);\n",
    "      float oy = j == 3 ? 0.5 : (j == 4 ? -0.5 : 0.0);\n",
    "      float2 sx = dfAdd(x, float2(grouped ? gi.x * ox + gi.z * oy : ox, 0.0));\n",
    "      float2 sy = dfAdd(y, float2(grouped ? gi.y * ox + gi.w * oy : oy, 0.0));\n",
    "      float2 qx = itemMapped ? dfMapX(item, sx, sy) : sx;\n",
    "      float2 qy = itemMapped ? dfMapY(item, sx, sy) : sy;\n",
    "      if (clipK && clipMode == 1) {\n",
    "        d[k] = clipRectDistance(item, qx, qy);\n",
    "      } else {\n",
    "        d[k] = sampleDistance(item, edges, clipK ? cband : band, qx, qy, k == 5 ? clipReach : 1.0e30, clipK);\n",
    "      }\n",
    "      if (k == 5 && abs(d[5].x) > clipReach) { clipMapped = false; }\n",
    "    }\n",
    "  }\n",
    "  float2 dRaw = d[0];\n",
    "  float2 g = float2(1.0, 0.0);\n",
    "  if (mapped) {\n",
    "    float2 gx = dfSub(d[1], d[2]);\n",
    "    float2 gy = dfSub(d[3], d[4]);\n",
    "    g = dfLength(gx, gy);\n",
    "    if (!(g.x > 0.000001)) { g = float2(1.0, 0.0); }\n",
    "  }\n",
    "  int fill = dfCoverage(mapped ? dfDiv(dRaw, g) : dRaw);\n",
    "  int stroke = 0;\n",
    "  if (stroked) {\n",
    // plan-164-C: a CurvePolygon's joined or dashed stroke is its pieces' (`curveWalk`).
    "    float2 s = (item.misc.x == 12 && joined)\n",
    "        ? float2(curveWalk(item, edges, band, cqx, cqy, dashed ? 3 : 2, dfF(half2), item.arc.w), 0.0)\n",
    "        : (dashed ? dashedStroke(item, edges, band, cqx, cqy, dfF(half2))\n",
    "        : (joined ? joinedStroke(item, edges, band, cqx, cqy, half2, strokeReach) : dfSub(dfAbs(dRaw), half2)));\n",
    "    stroke = dfCoverage(mapped ? dfDiv(s, g) : s);\n",
    "  }\n",
    "  int clipCov = 255;\n",
    "  if (clipMode == 1 && !grouped) { clipCov = clipRect(item, x, y); }\n",
    "  if (clipSampled) {\n",
    "    float2 cg = float2(1.0, 0.0);\n",
    "    if (clipMapped) {\n",
    "      cg = dfLength(dfSub(d[6], d[7]), dfSub(d[8], d[9]));\n",
    "      if (!(cg.x > 0.000001)) { cg = float2(1.0, 0.0); }\n",
    "    }\n",
    "    clipCov = dfCoverage(clipMapped ? dfDiv(d[5], cg) : d[5]);\n",
    "  }\n",
    "  return int4(fill, stroke, stroked ? 1 : 0, clipCov);\n",
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
    // plan-154-B: the stop lookup at a ramp parameter `t`, split from the axis that
    // computes `t` so a PolyLine can ask it at its path parameter instead.
    "static int4 gradientAt(float t, constant int *edges, constant MfbItem &item) {\n",
    "  int count = item.ellipse.z;\n",
    "  int base = METAL_GRADIENT_BASE + item.ellipse.w * 5;\n",
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
    // plan-163-D: the spread past the ramp's ends, the focal radial's position, the ramp
    // at the SHAPE-space point `q` (radial through its own inverse; `gradientKind` packs
    // `kind | spread << 1 | apply << 3`), and the stroke tint -- `mfb_canvas.frag`'s
    // `gradientSpread`, `focalT`, `gradientColour` and `tintRamp`, line for line.
    "static float gradientSpread(float t, int spread) {\n",
    "  if (spread == 0) { return t; }\n",
    "  float tt = clamp(t, -1.0e9, 1.0e9);\n",
    "  if (spread == 2) { return tt - floor(tt); }\n",
    "  float u = tt - 2.0 * floor(tt / 2.0);\n",
    "  return u > 1.0 ? 2.0 - u : u;\n",
    "}\n",
    "static float focalT(float2 p, float2 c, float r, float2 o, float fr) {\n",
    "  if (!(r > 0.0)) { return 0.0; }\n",
    "  float2 f = c + o;\n",
    "  float2 e = p - f;\n",
    "  float2 w = c - f;\n",
    "  float dr = r - fr;\n",
    "  float a = w.x * w.x + w.y * w.y - dr * dr;\n",
    "  float b = e.x * w.x + e.y * w.y + fr * dr;\n",
    "  float cc = e.x * e.x + e.y * e.y - fr * fr;\n",
    "  float disc = max(b * b - a * cc, 0.0);\n",
    "  return (b - sqrt(disc)) / a;\n",
    "}\n",
    "static int4 gradientColour(float2 q, constant int *edges, constant MfbItem &item) {\n",
    "  float2 gfrom = float2(fx(item.gradient.x), fx(item.gradient.y));\n",
    "  float2 gto = float2(fx(item.gradient.z), fx(item.gradient.w));\n",
    "  float2 axis = gto - gfrom;\n",
    "  float len2 = dot(axis, axis);\n",
    "  float t = 0.0;\n",
    "  if ((item.gradientKind & 1) == 1) {\n",
    "    float2 g = q;\n",
    "    if (item.gradInv1.z != 0) {\n",
    "      g = float2(as_type<float>(item.gradInv0.x) * q.x + as_type<float>(item.gradInv0.z) * q.y\n",
    "                     + as_type<float>(item.gradInv1.x),\n",
    "                 as_type<float>(item.gradInv0.y) * q.x + as_type<float>(item.gradInv0.w) * q.y\n",
    "                     + as_type<float>(item.gradInv1.y));\n",
    "    }\n",
    "    float2 o = float2(fx(item.gradFocal.x), fx(item.gradFocal.y));\n",
    "    float fr = fx(item.gradFocal.z);\n",
    "    float len = sqrt(len2);\n",
    "    if (o.x != 0.0 || o.y != 0.0 || fr != 0.0) {\n",
    "      t = focalT(g, gfrom, len, o, fr);\n",
    "    } else if (len > 0.0) {\n",
    "      t = length(g - gfrom) / len;\n",
    "    }\n",
    "  } else {\n",
    "    if (len2 > 0.0) { t = dot(q - gfrom, axis) / len2; }\n",
    "  }\n",
    "  return gradientAt(gradientSpread(t, (item.gradientKind >> 1) & 3), edges, item);\n",
    "}\n",
    "static int4 tintRamp(int4 ramp, int4 by) {\n",
    "  int4 tint = (ramp * by) / 255;\n",
    "  int4 out = ramp;\n",
    "  for (int c = 0; c < 3; ++c) {\n",
    "    int dc = (tint[c] - ramp[c]) * by.w;\n",
    "    out[c] = ramp[c] + (dc >= 0 ? (dc + 127) / 255 : -((127 - dc) / 255));\n",
    "  }\n",
    "  return out;\n",
    "}\n",
    // bug-1079: which of `n` texels holds `t`, the texels `d` apart -- `floor(t / d)`
    // corrected by at most one from the exact products, so the index does not hang on
    // the last bit of the quotient (Vulkan allows `/` 2.5 ULP and one driver takes it;
    // Apple's GPUs divide exactly, and this is the same rule so the copies stay one).
    // The GLSL's `pictureTexel`, line for line, and the oracle's picture arm.
    "static int pictureTexel(float t, float d, int n) {\n",
    "  float u = floor(t / d);\n",
    "  if ((u + 1.0) * d <= t) { u += 1.0; } else if (u * d > t) { u -= 1.0; }\n",
    "  return clamp(int(u), 0, n - 1);\n",
    "}\n",
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
    "static int4 pictureColour(constant MfbItem &item, constant int *edges, float2 q) {\n",
    "  int iw = item.misc.w;\n",
    "  int ih = item.arc.x;\n",
    "  if (iw <= 0 || ih <= 0) { return int4(0); }\n",
    "  float hw = fx(item.shape.z), hh = fx(item.shape.w);\n",
    "  float x0 = fx(item.shape.x) - hw, y0 = fx(item.shape.y) - hh;\n",
    "  float dw = hw * 2.0, dh = hh * 2.0;\n",
    "  int u = pictureTexel((q.x - x0) * float(iw), dw, iw);\n",
    "  int v = pictureTexel((q.y - y0) * float(ih), dh, ih);\n",
    "  uint word = as_type<uint>(edges[METAL_GLYPH_BASE + item.arc.z + v * iw + u]);\n",
    "  int4 tex = int4(int(word & 255u), int((word >> 8) & 255u),\n",
    "                  int((word >> 16) & 255u), int((word >> 24) & 255u));\n",
    "  int4 tint = (tex * item.fill) / 255;\n",
    "  int a = item.fill.w;\n",
    "  int4 out = tex;\n",
    "  for (int c = 0; c < 3; ++c) {\n",
    "    int dc = (tint[c] - tex[c]) * a;\n",
    "    out[c] = tex[c] + (dc >= 0 ? (dc + 127) / 255 : -((127 - dc) / 255));\n",
    "  }\n",
    "  out.w = (tex.w * (255 - item.clipPath.w) + 127) / 255;\n",
    "  return out;\n",
    "}\n",
    // The oracle quantizes coverage to a whole 0..255 and then takes an integer
    // `(colourAlpha * coverage) / 255`. Matching that here rather than blending in
    // float is not pedantry: near full coverage the sRGB encode is so steep that ONE
    // step of coverage moves a dark channel by up to 13 output steps, so a float
    // coverage that merely rounds differently produces a visible disagreement on
    // every antialiased edge. Quantizing the same way leaves only the pixels within
    // a rounding boundary of each other.
    "static float4 covered(int4 rgba, int coverage) {\n",
    "  float a = float((rgba.w * coverage) / 255) / 255.0;\n",
    "  return float4(srgbToLinear(float(rgba.x)) * a,\n",
    "                srgbToLinear(float(rgba.y)) * a,\n",
    "                srgbToLinear(float(rgba.z)) * a, a);\n",
    "}\n",
    // `items` and `edges` are the SAME buffer bound twice, at offset zero both times:
    // one view typed as blocks, one as raw words. Two bindings rather than one pointer
    // cast because the edge region is reached by a word index and the block region by a
    // struct index, and letting each keep its own element type is what stops a packing
    // mistake -- the class of bug that yields a plausible wrong picture, not a fault.
    // Glyph coverage is the fourth region of the same buffer (bug-670): a text item is
    // ONE instanced draw, so a per-glyph payload bound before it would be overwritten
    // by the next glyph's; each glyph's block names its own slice instead.
    "fragment float4 mfbFragment(VOut in [[stage_in]],\n",
    "                            constant MfbItem *items [[buffer(0)]],\n",
    "                            constant int *edges [[buffer(1)]],\n",
    "                            constant MfbDraw &draw [[buffer(3)]]) {\n",
    "  constant MfbItem &item = items[in.item];\n",
    // plan-116-H: `p` is the SHAPE-space point -- the surface point with the group's
    // offset taken back off -- and every distance, gradient, glyph lookup and (since
    // plan-163-C) the clip below uses it.
    "  float2 p = in.pos.xy - float2(fx(draw.offset.x), fx(draw.offset.y));\n",
    // plan-172-H: and, under a group matrix, through the groups' inverse linear part --
    // so `p` is the point in the GROUP's space, where an item's own placement lives, and
    // every consumer below (the item's own inverse map included) is unchanged.
    "  bool grouped = drawGrouped(draw);\n",
    "  float4 gi = drawInverse(draw);\n",
    "  if (grouped) { p = float2(gi.x * p.x + gi.z * p.y, gi.y * p.x + gi.w * p.y); }\n",
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
    "  float4 pt = dfShapePoint(in.pos.xy, draw.offset);\n",
    "  if (grouped) {\n",
    "    pt = float4(dfAdd(dfMulF(pt.xy, gi.x), dfMulF(pt.zw, gi.z)), dfAdd(dfMulF(pt.xy, gi.y), dfMulF(pt.zw, gi.w)));\n",
    "  }\n",
    "  int4 cov = shapeCoverage(item, edges, pt, gi, grouped);\n",
    "  int clipCov = cov.w;\n",
    // plan-116-C section 4.5: a transformed glyph samples its bitmap at the
    // inverse-mapped point, nearest. The index arithmetic is already whole-pixel, so
    // mapping the query point is the whole change -- the cache stays untransformed and
    // one entry serves every transform.
    "  if (item.misc.x == 6) {\n",
    "    float2 gp = hasTransform(item) ? inverseMap(item, p) : p;\n",
    // `floor`, not a cast: a cast truncates toward zero, and a transformed glyph maps
    // to NEGATIVE shape-space coordinates (ink runs up from the pen), where truncation
    // picks the texel on the wrong side. Untransformed, `gp` is a surface pixel centre
    // and always positive, so this is the same value the cast gave.
    "    int ix = int(floor(gp.x)) - item.shape.x;\n",
    "    int iy = int(floor(gp.y)) - item.shape.y;\n",
    "    int cov = (ix < 0 || iy < 0 || ix >= item.misc.w || iy >= item.arc.x)\n",
    "      ? 0 : edges[METAL_GLYPH_BASE + item.arc.z + iy * item.misc.w + ix];\n",
    "    return covered(item.fill, (cov * clipCov) / 255);\n",
    "  }\n",
    // Both coverages come from `shapeCoverage` in double-float at the double-float twin
    // of `p` -- the surface point with the group offset (word and residual) taken off,
    // exactly: a polygon since bug-687, every other kind since bug-735. The fill uses
    // the surface distance and the stroke subtracts `half` BEFORE converting, so the
    // outline scales with the shape (section 4.3).
    // plan-116-F: the gradient replaces the fill COLOUR and nothing else.
    // plan-163-D: at the shape-space point `q`, so the transform carries the ramp; and
    // with `FillAndStroke` the stroke -- a Line's or Arc's fill -- is the ramp tinted by
    // its colour.
    "  float2 q = hasTransform(item) ? inverseMap(item, p) : p;\n",
    "  int4 ramp = item.ellipse.z >= 2 ? gradientColour(q, edges, item) : item.fill;\n",
    "  bool rampStroke = item.ellipse.z >= 2 && (item.gradientKind & 8) != 0;\n",
    "  int4 fillRgba = ramp;\n",
    "  if (rampStroke && (item.misc.x == 2 || item.misc.x == 3 || item.misc.x == 11)) { fillRgba = tintRamp(ramp, item.fill); }\n",
    // plan-154-B: a PolyLine's stops are its per-point colours, and its ramp runs along
    // the path -- the parameter of the nearest point, in shape space, from the same band
    // the distance used.
    "  if (item.misc.x == 10 && item.ellipse.z >= 2) {\n",
    "    fillRgba = gradientAt(polylineScan(item, edges, polygonBand(item, edges, q.y), q).y, edges, item);\n",
    "  }\n",
    // bug-484: a picture's image replaces the fill COLOUR the same way, sampled at the
    // inverse-mapped point under a transform (plan-116-C section 4.5, as a glyph is).
    "  if (item.misc.x == 9) {\n",
    "    fillRgba = pictureColour(item, edges, q);\n",
    "  }\n",
    // plan-163-D: a fill alpha of -1 is the stroke record of a split blended item.
    "  float4 colour = item.fill.w < 0 ? float4(0.0) : covered(fillRgba, (cov.x * clipCov) / 255);\n",
    "  if (cov.z != 0) {\n",
    // plan-116-B: this stroke-over-fill composition equals the oracle's two sequential
    // blends only under `Normal`, which is why a non-`Normal` item that both fills and
    // strokes is emitted as two adjacent instances (`emit_split_or_publish`). By the
    // time such an item reaches here it is fill-only or stroke-only.
    "    float4 s = covered(rampStroke ? tintRamp(ramp, item.stroke) : item.stroke, (cov.y * clipCov) / 255);\n",
    "    colour = s + colour * (1.0 - s.w);\n",
    "  }\n",
    "  return colour;\n",
    "}\n",
);

/// The MSL source's C string data object symbol. The text is `METAL_SHADER_SOURCE`,
/// paired with this in `metal_data_objects`.
pub(super) const STR_METAL_SHADER: &str = "_mfb_macapp_metal_shader";
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
pub(super) const SEL_SET_ALLOW_GPU_OPTIMIZED_CONTENTS: (&str, &str) = (
    "_mfb_macapp_sel_setAllowGPUOptimizedContents",
    "setAllowGPUOptimizedContents:",
);
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
/// `setVertexBytes:length:atIndex:` outright, and both were *deleted* rather than kept
/// for a caller that might want them: every selector in `metal_data_objects` is a C
/// string emitted into every canvas binary and registered with the ObjC runtime at
/// startup, so an unsent one is not free.
///
/// plan-116-H re-added `setVertexBytes:` — see `SEL_SET_VERTEX_BYTES` — because the
/// group offset is now sent to the vertex stage once per draw entry. The uninstanced
/// draw stays deleted.
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
/// (`ITEM_BLOCK_SIZE`, 272 since bug-735; it was 112 when this was written, and
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
pub(super) const SEL_STATUS: (&str, &str) = ("_mfb_macapp_sel_status", "status");
/// `MTLCommandBufferStatusCompleted`: the one `-[MTLCommandBuffer status]` that says
/// the GPU ran every command of the buffer. After `waitUntilCompleted` the only other
/// value is `MTLCommandBufferStatusError` (5).
const MTL_COMMAND_BUFFER_STATUS_COMPLETED: usize = 4;
pub(super) const SEL_GET_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_getBytes",
    "getBytes:bytesPerRow:fromRegion:mipmapLevel:",
);

pub(super) const SEL_SET_FRAGMENT_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_setFragmentBytes",
    "setFragmentBytes:length:atIndex:",
);

/// The group offset, sent inline to the vertex stage once per draw entry (plan-116-H).
///
/// **Re-added here**: plan-116-A deleted this selector along with the uninstanced draw,
/// on the stated grounds that an unsent selector still costs a C string and a runtime
/// registration in every canvas binary. That reasoning stands — it is sent again now.
///
/// Missing it is not a compile error, which is the hazard **H5** names: the emitter
/// would `objc_msgSend` a selector the runtime never registered. It has to be in
/// `metal_data_objects` below as well as declared here.
///
/// Inline bytes rather than an `MTLBuffer` because the payload is sixteen bytes (the
/// 16.16 offset and, since bug-735, its residual words) that change per draw;
/// `setVertexBytes:` is the API for exactly that size.
pub(super) const SEL_SET_VERTEX_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_setVertexBytes",
    "setVertexBytes:length:atIndex:",
);

pub(crate) const CLASS_MTL_TEXTURE_DESCRIPTOR: &str = "_OBJC_CLASS_$_MTLTextureDescriptor";
pub(crate) const CLASS_MTL_RENDER_PASS_DESCRIPTOR: &str = "_OBJC_CLASS_$_MTLRenderPassDescriptor";

pub(crate) const CLASS_MTL_RENDER_PIPELINE_DESCRIPTOR: &str =
    "_OBJC_CLASS_$_MTLRenderPipelineDescriptor";

// --- bug-686 Phase 4: the direct present -------------------------------------------
//
// A GPU frame in a real window leaves through a `CAMetalLayer` drawable: the offscreen
// target is blitted into `[layer nextDrawable].texture` and presented, with no
// `getBytes:` readback, no BGRA->RGBA swizzle, no CPU surface and no `CGImage`. Every
// selector below is sent only on that path or on the main-thread layer bookkeeping it
// needs, and every one is listed in `metal_data_objects` (an unregistered selector is
// not a compile error — it is a message send to a selector the runtime never named).

/// `QuartzCore.framework` — where `CAMetalLayer` and `CATransaction` live.
pub(crate) const LIB_QUARTZCORE: &str = "QuartzCore";
pub(crate) const CLASS_CA_METAL_LAYER: &str = "_OBJC_CLASS_$_CAMetalLayer";
pub(crate) const CLASS_CA_TRANSACTION: &str = "_OBJC_CLASS_$_CATransaction";
pub(super) const SEL_NEXT_DRAWABLE: (&str, &str) = ("_mfb_macapp_sel_nextDrawable", "nextDrawable");
pub(super) const SEL_TEXTURE: (&str, &str) = ("_mfb_macapp_sel_texture", "texture");
pub(super) const SEL_BLIT_COMMAND_ENCODER: (&str, &str) =
    ("_mfb_macapp_sel_blitCommandEncoder", "blitCommandEncoder");
/// `copyFromTexture:toTexture:` (macOS 10.15) — the whole-texture copy. The drawable
/// is created at the offscreen target's exact size and format, which is the
/// precondition this form has, and it keeps the send to four register arguments
/// where the slice/level/origin/size form needs three `MTLOrigin`/`MTLSize` structs
/// and stack arguments.
pub(super) const SEL_COPY_TEXTURE: (&str, &str) = (
    "_mfb_macapp_sel_copyFromTextureToTexture",
    "copyFromTexture:toTexture:",
);
/// `-[CAMetalDrawable present]`, sent once the frame's buffer has completed.
pub(super) const SEL_PRESENT: (&str, &str) = ("_mfb_macapp_sel_present", "present");
pub(super) const SEL_SET_DEVICE: (&str, &str) = ("_mfb_macapp_sel_setDevice", "setDevice:");
pub(super) const SEL_SET_FRAMEBUFFER_ONLY: (&str, &str) =
    ("_mfb_macapp_sel_setFramebufferOnly", "setFramebufferOnly:");
pub(super) const SEL_SET_DRAWABLE_SIZE: (&str, &str) =
    ("_mfb_macapp_sel_setDrawableSize", "setDrawableSize:");
pub(super) const SEL_SET_HIDDEN: (&str, &str) = ("_mfb_macapp_sel_setHidden", "setHidden:");
pub(super) const SEL_SET_OPAQUE: (&str, &str) = ("_mfb_macapp_sel_setOpaque", "setOpaque:");
/// `-[CAMetalLayer setColorspace:]`, set to the window SCREEN's colour space.
///
/// The software blit hands the layer a DeviceRGB `CGImage`, which Core Animation shows
/// with no colour matching: the bytes go to the display as its own primaries. A
/// `CAMetalLayer` does not do the same by default, nor when given DeviceRGB — both are
/// matched as sRGB. Measured in a real window on a P3 display, a drawable of (255, 0, 0)
/// captured as (234, 51, 35) with the layer's space nil, DeviceRGB or sRGB, in either
/// `BGRA8Unorm` or `BGRA8Unorm_sRGB`, and as (255, 0, 0) — what the `CGImage` of the
/// same bytes shows — only with the screen's own space (or Display P3, which that
/// screen is). One picture on both paths means the drawable is declared in the space
/// the image path already lands in, so a scene that falls back to software mid-run
/// does not change colour.
pub(super) const SEL_SET_COLORSPACE: (&str, &str) =
    ("_mfb_macapp_sel_setColorspace", "setColorspace:");
/// `-[NSWindow screen]`, `-[NSScreen colorSpace]`, `-[NSColorSpace CGColorSpace]` —
/// the lookup `SEL_SET_COLORSPACE`'s value comes from.
pub(super) const SEL_SCREEN: (&str, &str) = ("_mfb_macapp_sel_screen", "screen");
pub(super) const SEL_COLOR_SPACE: (&str, &str) = ("_mfb_macapp_sel_colorSpace", "colorSpace");
pub(super) const SEL_CG_COLOR_SPACE: (&str, &str) =
    ("_mfb_macapp_sel_CGColorSpace", "CGColorSpace");
pub(super) const SEL_ADD_SUBLAYER: (&str, &str) = ("_mfb_macapp_sel_addSublayer", "addSublayer:");
/// `-[CALayer setFrame:]`, a `CGRect` by value — four doubles, so `d0`..`d3`.
pub(super) const SEL_SET_LAYER_FRAME: (&str, &str) = ("_mfb_macapp_sel_setFrame", "setFrame:");
pub(super) const SEL_BEGIN: (&str, &str) = ("_mfb_macapp_sel_begin", "begin");
pub(super) const SEL_SET_DISABLE_ACTIONS: (&str, &str) =
    ("_mfb_macapp_sel_setDisableActions", "setDisableActions:");
/// The delegate method the graphics thread hops to for every layer property change.
pub(super) const SEL_MFB_METAL_LAYER: (&str, &str) =
    ("_mfb_macapp_sel_mfbMetalLayer", "mfbMetalLayer:");

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
    let saves: [(Reg, usize); 6] = [
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
    build_nsstring_from_cstring(&mut asm, abi::LOCAL[3], STR_METAL_SHADER);
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
    assert_eq!(
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
/// bug-686 Phase 4: the graphics-thread hop that asks the main thread to configure,
/// size or show the window's `CAMetalLayer`, and the main-thread IMP that does it.
pub(super) const METAL_LAYER_SYNC_SYMBOL: &str = "_mfb_macapp_metal_layer_sync";
pub(super) const METAL_LAYER_APPLY_SYMBOL: &str = "_mfb_macapp_metal_layer_apply";

/// `MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget`.
const MTL_TEXTURE_USAGE: &str = "5";
/// `MTLStorageModeShared` — one allocation both the GPU renders into and the CPU
/// reads back from. On Apple Silicon there is no separate device memory to copy
/// across, so a `Managed` texture would add a blit-and-synchronize for nothing.
const MTL_STORAGE_MODE_SHARED: &str = "0";
/// `allowGPUOptimizedContents = NO`: the render target is stored linearly, not in the
/// GPU's lossless-compressed layout. Every headless frame is read back through
/// `getBytes:`, and a compressed texture is decompressed ON THE CPU inside that call
/// (`AGX::Texture::processCompressedRegion2D` under `sample`): 1.2 ms of a 10,000-quad
/// frame at 900x640, growing with how busy the picture is, where a linear copy is a
/// memcpy (bug-686). The pixels are the same bits either way.
const MTL_GPU_OPTIMIZED_CONTENTS: &str = "0";
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
const DRAW_FRAME: usize = OFF_DRAW_PAIR + 32;
const OFF_REGION: usize = 0;
const OFF_LR: usize = 64;
const OFF_SAVES: usize = 72;
const OFF_SURFACE: usize = 136;
const OFF_WIDTH: usize = 144;
const OFF_HEIGHT: usize = 152;
const OFF_POOL: usize = 160;
const OFF_ITEM: usize = 192;
const OFF_TEXTURE: usize = OFF_ITEM + ITEM_BLOCK_SIZE;
/// The glyph cache's two payload pointers, and the per-glyph loop's state.
///
/// On the stack rather than in `LOCAL` registers because the glyph loop makes two
/// `objc_msgSend` calls per glyph and the low `LOCAL`s are the objc temporaries.
const OFF_GLYPH_META: usize = OFF_TEXTURE + 8;
const OFF_GLYPH_COV: usize = OFF_TEXTURE + 16;
const OFF_GLYPH_INDEX: usize = OFF_TEXTURE + 24;
const OFF_GLYPH_COUNT: usize = OFF_TEXTURE + 32;
const OFF_GLYPH_HEADER: usize = OFF_TEXTURE + 40;
const OFF_GLYPH_W: usize = OFF_TEXTURE + 48;
const OFF_GLYPH_H: usize = OFF_TEXTURE + 56;
const OFF_GLYPH_X: usize = OFF_TEXTURE + 64;
const OFF_GLYPH_Y: usize = OFF_TEXTURE + 72;
/// The pointer handed straight to `setFragmentBytes:` — into the coverage cache
/// itself. Metal copies at record time, so the bitmap needs no staging buffer of its
/// own; the cache's bytes for one glyph are already contiguous.
const OFF_GLYPH_SRC: usize = OFF_TEXTURE + 80;
/// `[frameBuffer contents]`, loaded once per frame from the graphics state.
///
/// Parked rather than kept in a `LOCAL`: every item makes at least one
/// `objc_msgSend`, and the low `LOCAL`s are the objc temporaries.
const OFF_CONTENTS: usize = OFF_TEXTURE + 88;
/// The frame's item-buffer cursor — one block per drawn QUAD, so a shape takes one and
/// a glyph run takes one per glyph — and the base of the instanced run currently being
/// accumulated.
///
/// `OFF_RUN_COUNT` used to sit beside these, holding the `cursor - base` the flush
/// computed. plan-116-H deleted it with `emit_run_flush`: the instance count now comes
/// from the draw list, not from the cursor. `OFF_TEXTURE + 112` is free.
const OFF_ITEM_CURSOR: usize = OFF_TEXTURE + 96;
const OFF_RUN_START: usize = OFF_TEXTURE + 104;
/// The frame's running edge cursor, in edges. Each polygon appends here and records
/// where it started in its own item block — exactly what the Vulkan emitter has always
/// done, and what Metal could not do while its edges rode a per-item payload.
const OFF_EDGE_CURSOR: usize = OFF_TEXTURE + 120;
/// Where the glyph currently being drawn published its block, parked so the draw's
/// `baseInstance:` is staged from memory rather than from a register the staging of an
/// earlier argument would have overwritten.
const OFF_GLYPH_INSTANCE: usize = OFF_TEXTURE + 128;
/// The blend mode currently bound, this item's, and the `strokeHalf` parked across the
/// two-instance split (plan-116-B).
///
/// `strokeHalf` is on the stack rather than in a register for a reason that cost a
/// debugging round on the Vulkan side: `emit_item_publish` uses the low `SCRATCH`
/// registers, so a value saved across it comes back as a mapped address — and as a
/// stroke width that reads like an enormous band the oracle never drew.
const OFF_BOUND_MODE: usize = OFF_TEXTURE + 136;
const OFF_ITEM_MODE: usize = OFF_TEXTURE + 144;
const OFF_SAVED_STROKE: usize = OFF_TEXTURE + 152;
/// The frame's gradient-stop cursor, in STOPS — the third region's twin of
/// `OFF_EDGE_CURSOR` (plan-116-F).
const OFF_GRAD_CURSOR: usize = OFF_TEXTURE + 160;
/// The frame's glyph-sample cursor — the glyph region's twin of `OFF_EDGE_CURSOR`
/// (bug-670).
const OFF_GLYPH_CURSOR: usize = OFF_TEXTURE + 216;
/// The frame's dash-region cursor, in words (plan-163-B), in the free slot between
/// `OFF_DRAW_COUNT` and `OFF_GLYPH_CURSOR`.
const OFF_DASH_CURSOR: usize = OFF_TEXTURE + 208;
/// How many records this frame's picture table holds (bug-686,
/// `METAL_PICTURE_TABLE_BYTES`), reset with the cursors; and the index slot a picture
/// lookup missed at, parked across the upload that decides whether to record it.
const OFF_PIC_COUNT: usize = OFF_TEXTURE + 224;
const OFF_PIC_SLOT: usize = OFF_TEXTURE + 232;
/// The band region's cursor, in words (bug-686) — reset per frame like the others —
/// and `emit_band_index`'s working state for one polygon: its first and past-the-last
/// 16.16 edge in the frame buffer, its lowest edge y (16.16), the band reach `2r`
/// (16.16), the band height (16.16), the band count, and the table's address.
///
/// On the stack because the builder's loops need more values than the nine usable
/// `SCRATCH` registers hold at once.
const OFF_BAND_CURSOR: usize = OFF_TEXTURE + 240;
const OFF_BAND_EDGES: usize = OFF_TEXTURE + 248;
const OFF_BAND_END: usize = OFF_TEXTURE + 256;
const OFF_BAND_YMIN: usize = OFF_TEXTURE + 264;
const OFF_BAND_REACH: usize = OFF_TEXTURE + 272;
const OFF_BAND_HEIGHT: usize = OFF_TEXTURE + 280;
const OFF_BAND_COUNT: usize = OFF_TEXTURE + 288;
const OFF_BAND_TABLE: usize = OFF_TEXTURE + 296;
/// bug-686 Phase 4: the drawable a direct present copies into — autoreleased, so it
/// lives until the frame's pool pops — and the answer the frame returns in `x0`
/// (1 = the frame is finished: presented, or read back into the surface; 0 = it was
/// not drawn and the caller must take another path).
const OFF_DRAWABLE: usize = OFF_TEXTURE + 304;
const OFF_PRESENTED: usize = OFF_TEXTURE + 312;

/// plan-116-H Phase 3: the draw list, and the walk over it.
///
/// `OFF_DRAWS` holds the **payload** pointer and `OFF_DRAW_ENTRIES` the entry count —
/// both derived here from the collection pointer the seam now passes, rather than from
/// two arguments, because the argument bank is eight wide and the eighth is the draw
/// list itself (`func_metal_draw.rs`).
///
/// `OFF_DRAW_PAIR` is the per-draw payload laid out contiguously -- the two 16.16 offset
/// words, then (bug-735) their two residual words, then (plan-172-H) the four float32
/// bit patterns of the groups' inverse linear part, which are zero for a draw its groups
/// only translate -- so `setVertexBytes:` and `setFragmentBytes:` can point at
/// thirty-two bytes of frame rather than at a collection another thread may be writing.
/// It is the MSL's `MfbDraw`.
/// Where the group offset is bound in each stage, matching the `[[buffer(N)]]`
/// attributes in `METAL_SHADER_SOURCE`.
///
/// The vertex stage has only `items` at 0, so the offset takes 1. The fragment stage
/// has items and edges at 0 and 1 and takes 3 — index 2 held the per-glyph bitmap until
/// bug-670 moved glyphs into the frame buffer, and is left unused rather than renumbering
/// a binding the tests pin. They differ,
/// and `the_metal_shader_binds_the_draw_offset_where_the_emitter_sends_it` is what
/// keeps each equal to its shader — sending to the wrong index binds nothing and the
/// stage reads whatever was there, which is a wrong picture rather than a fault.
const METAL_DRAW_OFFSET_VERTEX_INDEX: &str = "1";
const METAL_DRAW_OFFSET_FRAGMENT_INDEX: &str = "3";

const OFF_DRAWS: usize = OFF_TEXTURE + 168;
const OFF_DRAW_ENTRIES: usize = OFF_TEXTURE + 176;
const OFF_DRAW_INDEX: usize = OFF_TEXTURE + 184;
const OFF_DRAW_BASE: usize = OFF_TEXTURE + 192;
const OFF_DRAW_COUNT: usize = OFF_TEXTURE + 200;
/// Past `OFF_PRESENTED`: sixteen bytes since bug-735 and thirty-two since plan-172-H, so
/// it outgrew the eight-byte slot at `OFF_TEXTURE + 208` it had between `OFF_DRAW_COUNT`
/// and `OFF_GLYPH_CURSOR`, which is free. It is the frame's last slot.
const OFF_DRAW_PAIR: usize = OFF_TEXTURE + 320;

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
/// **Unless the surface pointer is NULL** (bug-686 Phase 4, `canvas::metalPresentScene`).
/// Then the frame is rendered into the same offscreen target, blitted on the GPU into
/// the window `CAMetalLayer`'s next drawable and presented — no readback, no swizzle, no
/// CPU surface, no `CGImage`. A NULL payload is unambiguous: `canvas::metalDrawScene`
/// passes `block + COLLECTION_HEADER_SIZE`, which is never zero.
///
/// Returns 1 in `x0` when the frame is finished — presented, or read back — and 0 when
/// it was not drawn: no pipeline, no texture, a command buffer that did not reach
/// `MTLCommandBufferStatusCompleted` (bug-1024), and for a direct present no window
/// layer (headless) or no drawable. A 0 from a direct present is the caller's cue to
/// render the frame down the readback path instead; a 0 from the readback path
/// (`canvas::metalDrawScene` answers FALSE) is its cue to draw the frame in software.
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
    // bug-686 Phase 4: the direct present's branches.
    let init = format!("{METAL_DRAW_SYMBOL}_init");
    let render_pass = format!("{METAL_DRAW_SYMBOL}_render_pass");
    let sync_layer = format!("{METAL_DRAW_SYMBOL}_sync_layer");
    let layer_synced = format!("{METAL_DRAW_SYMBOL}_layer_synced");
    let submit = format!("{METAL_DRAW_SYMBOL}_submit");
    let direct_failed = format!("{METAL_DRAW_SYMBOL}_direct_failed");
    let readback = format!("{METAL_DRAW_SYMBOL}_readback");
    let show_done = format!("{METAL_DRAW_SYMBOL}_show_done");

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
                                                                  // plan-116-H: slots 4 and 7 arrive as **collection pointers**, not payloads — the
                                                                  // seam bought back one argument register that way so the draw list could have the
                                                                  // eighth (`func_metal_draw.rs`). Both are unpacked here, and both are read out of
                                                                  // the argument bank BEFORE anything else is staged into it.
                                                                  //
                                                                  // The count still comes off the collection header, so a caller cannot pass a length
                                                                  // that disagrees with the list it passed; it is simply read one level further in
                                                                  // than it used to be.
    asm.push(abi::load_u64(
        abi::LOCAL[5],
        abi::mfb_arg(4),
        COLLECTION_OFFSET_COUNT,
    )); // offset count
    asm.push(abi::add_immediate(
        abi::LOCAL[4],
        abi::mfb_arg(4),
        COLLECTION_HEADER_SIZE,
    )); // offsets payload
        // The draw list: payload, then entries. An entry is `CANVAS_DRAW_ENTRY_WORDS`
        // words, so the entry count is the element count shifted — the same arithmetic the
        // Vulkan emitter does, from the same constant, because a disagreement between the
        // two would be a backend that walks a different number of draws than the list has
        // (plan-116-H **H13**).
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::mfb_arg(7),
        COLLECTION_OFFSET_COUNT,
    ));
    asm.push(abi::shift_right_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        CANVAS_DRAW_ENTRY_COUNT_SHIFT as u8,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAW_ENTRIES,
    ));
    asm.push(abi::add_immediate(
        abi::SCRATCH[0],
        abi::mfb_arg(7),
        COLLECTION_HEADER_SIZE,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAWS,
    ));
    for (argument, slot) in [(5usize, OFF_GLYPH_META), (6, OFF_GLYPH_COV)] {
        asm.push(abi::store_u64(
            abi::mfb_arg(argument),
            abi::stack_pointer(),
            slot,
        ));
    }

    // bug-686 Phase 4: a NULL surface asks for a direct present. With no window layer
    // — headless, where the main thread never built a canvas view — there is nothing
    // to present to, so answer 0 before touching Metal at all: the caller then takes
    // the readback path, which is exactly the one every headless test compares.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&init));
    asm.local_address(abi::SCRATCH[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        GRAPHICS_OFFSET_MTL_LAYER,
    ));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&restore));
    asm.push(abi::label(&init));

    // The pipeline, built on first use. A failure here leaves the surface exactly as
    // `canvas::newSurface` made it, which is the cleared frame — not garbage — and
    // returns the init's 0, so a direct present that could not build falls back too.
    asm.call_internal(METAL_INIT_SYMBOL);
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&restore));

    asm.call_external("_objc_autoreleasePoolPush", LIB_OBJC);
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_POOL,
    ));
    // Nothing is finished until the submit says so: every early exit below (a
    // texture that would not allocate, a layer that could not be configured, no
    // drawable) reports 0.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for slot in [OFF_PRESENTED, OFF_DRAWABLE] {
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
    }

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
        (
            SEL_SET_ALLOW_GPU_OPTIMIZED_CONTENTS.0,
            MTL_GPU_OPTIMIZED_CONTENTS,
        ),
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

    // --- bug-686 Phase 4: the drawable, taken BEFORE anything is recorded ----------
    // Asking first is what makes "no drawable" cost nothing: the frame reports 0 and
    // the caller renders it down the readback path instead, rather than this frame
    // being rendered, found undeliverable and rendered again.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&render_pass));
    // The layer must be configured (device, format, `framebufferOnly = NO`) and its
    // drawable must be this frame's size, or the whole-texture copy below — which
    // requires identical dimensions — is invalid. Both are layer PROPERTY changes, so
    // they are made by the main thread (`emit_metal_layer_apply`), and only when one
    // of them is actually out of date: a steady-size window hops zero times a frame.
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_READY,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&sync_layer));
    for (slot, parked) in [
        (GRAPHICS_OFFSET_MTL_DRAWABLE_W, OFF_WIDTH),
        (GRAPHICS_OFFSET_MTL_DRAWABLE_H, OFF_HEIGHT),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::LOCAL[0], slot));
        asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        asm.push(abi::branch_ne(&sync_layer));
    }
    asm.push(abi::branch(&layer_synced));
    asm.push(abi::label(&sync_layer));
    for (slot, parked) in [
        (GRAPHICS_OFFSET_MTL_DRAWABLE_W, OFF_WIDTH),
        (GRAPHICS_OFFSET_MTL_DRAWABLE_H, OFF_HEIGHT),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::LOCAL[0], slot));
    }
    // Size only — `0` leaves the layer's visibility alone. It is shown after the
    // first frame is actually presented into it, below, so it never appears holding
    // nothing (or a stale drawable) over the software frame beneath.
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    asm.call_internal(METAL_LAYER_SYNC_SYMBOL);
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(&release_pool));
    // The apply ran on the main thread; if it found no layer it configured nothing.
    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_READY,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&release_pool));
    asm.push(abi::label(&layer_synced));
    // drawable = [layer nextDrawable] — autoreleased, so it lives until the pool pops.
    // nil (no drawable within the layer's timeout) is a real outcome and is not a
    // dropped frame: report 0 and the caller reads this frame back instead.
    asm.load_selector(SEL_NEXT_DRAWABLE.0);
    asm.local_address(abi::c_arg(0), GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::c_arg(0),
        GRAPHICS_OFFSET_MTL_LAYER,
    ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_DRAWABLE,
    ));
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(&release_pool));

    // --- the render pass ---------------------------------------------------------
    asm.push(abi::label(&render_pass));
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
        // bug-670: the glyph region is per frame, like the edge and gradient ones.
        OFF_GLYPH_CURSOR,
        // plan-163-B: and the dash region.
        OFF_DASH_CURSOR,
        // bug-686: so is the picture table. Its index is never cleared -- a record
        // count of 0 makes every slot read as empty (`METAL_PICTURE_TABLE_BYTES`).
        OFF_PIC_COUNT,
        // bug-686: and the band region.
        OFF_BAND_CURSOR,
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

        // plan-116-H: no flush here. The draw list already ends a run wherever the
        // blend mode changes, so this site only binds the pipeline the following blocks
        // will be published under; the draws are issued by `emit_draw_list_pass` once
        // every block is in the buffer.
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
    emit_item_block(&mut asm, "shape");
    emit_edge_buffer(&mut asm);
    // plan-163-B: after the item's own edges, so its join records follow them; then the
    // dash words, after the join copy has zeroed their block words.
    emit_join_buffer(&mut asm);
    emit_dash_buffer(&mut asm);
    // plan-163-C: the clip's words and a path clip's edges, after the join records.
    emit_clip_buffer(&mut asm);
    emit_gradient_buffer(&mut asm);
    // bug-484: after the edge buffer, which zeroes `arc.z` for every non-polygon, and
    // before the split, so both records of a blended stroked picture name its texels.
    emit_picture_buffer(&mut asm);
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
    // plan-116-H: no flush here either. Ordering used to be kept by issuing the
    // accumulated run before the glyphs; it is now kept by the draw list being in scene
    // order and the pass walking it in that order. A `Text` item is its own entry, so
    // nothing merges across it.
    emit_glyph_publish(&mut asm);
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
    // plan-116-H: every block is published by the walk above, so the pass issues every
    // draw — one instanced call per entry, each with its own pipeline and its own group
    // offset. This replaces the trailing flush and the two removed above.
    emit_draw_list_pass(&mut asm);

    asm.load_selector(SEL_END_ENCODING.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // --- bug-686 Phase 4: copy into the drawable and present it ------------------
    // The frame was rendered into the offscreen target exactly as the readback path
    // renders it; the only difference is where it goes. A GPU blit into the drawable's
    // texture keeps "one pipeline, one target" true — nothing about the render depends
    // on whether it is presented or read back — and needs the drawable's texture to
    // be a blit destination, which is why the layer is `framebufferOnly = NO`.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&submit));
    // target = [drawable texture]
    asm.load_selector(SEL_TEXTURE.0);
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_DRAWABLE,
    ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(&direct_failed));
    // blit = [commandBuffer blitCommandEncoder] — into `LOCAL[6]`, whose render encoder
    // has already ended.
    asm.load_selector(SEL_BLIT_COMMAND_ENCODER.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[7]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[6], abi::c_return(0)));
    asm.push(abi::compare_immediate(abi::LOCAL[6], "0"));
    asm.push(abi::branch_eq(&direct_failed));
    // [blit copyFromTexture:offscreen toTexture:target]; [blit endEncoding]
    asm.load_selector(SEL_COPY_TEXTURE.0);
    asm.push(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_TEXTURE,
    ));
    asm.push(abi::move_register(abi::c_arg(3), abi::LOCAL[0]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_END_ENCODING.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch(&submit));

    // A direct present that could not reach its drawable (no texture, no blit
    // encoder). The render is already encoded, so the buffer is still committed and
    // waited for — below, with every other frame — but nothing is presented: clearing
    // the drawable slot is what makes the submit report 0, so the caller reads the
    // frame back instead.
    asm.push(abi::label(&direct_failed));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAWABLE,
    ));

    // --- submit and wait ---------------------------------------------------------
    // Waiting is kept on the direct path too: the frame buffer and the offscreen target
    // are single-buffered, so the next frame may not write either while this one reads.
    asm.push(abi::label(&submit));
    for selector in [SEL_COMMIT.0, SEL_WAIT_UNTIL_COMPLETED.0] {
        asm.load_selector(selector);
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[7]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }
    // The wait returning is not the GPU having drawn (bug-1024): a buffer can end in
    // `MTLCommandBufferStatusError` — the GPU was restarted under it, or it timed out —
    // with its target written in part or not at all. Such a frame is not read back and
    // not presented. Leaving here before either path stores `OFF_PRESENTED` is what
    // makes the submit report 0, exactly as `direct_failed` does, so the caller draws
    // the frame in software.
    asm.load_selector(SEL_STATUS.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[7]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate(
        abi::c_return(0),
        &MTL_COMMAND_BUFFER_STATUS_COMPLETED.to_string(),
    ));
    asm.push(abi::branch_ne(&release_pool));
    // The frame is finished: read back, below — or, for a direct present, presented,
    // unless `direct_failed` cleared the drawable to say the copy never happened.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&readback));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAWABLE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&release_pool));
    // [drawable present] — AFTER the wait, not `presentDrawable:` on the buffer. The
    // copy is finished, so this presents at the next refresh; scheduling it on the
    // buffer instead ties the buffer's COMPLETION to the presentation, and the wait
    // above then blocks for it. Measured in a 1512x855 window on a 120 Hz display:
    // `presentDrawable:` spent 97% of the graphics thread in `waitUntilCompleted`
    // (`sample`) and held the loop to ~82 frames a second.
    asm.load_selector(SEL_PRESENT.0);
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_DRAWABLE,
    ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_PRESENTED,
    ));
    asm.push(abi::branch(&release_pool));
    asm.push(abi::label(&readback));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_PRESENTED,
    ));

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

    // bug-686 Phase 4: the first frame presented into a hidden layer shows it. Asked
    // only while it is hidden, so a window that stays on the GPU hops once, not per
    // frame; a software or readback frame hides it again (`emit_canvas_blit_apply_helper`).
    // After the pool pops, because the hop waits on the main thread and nothing here
    // needs the frame's autoreleased objects any more.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_SURFACE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&show_done));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_PRESENTED,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&show_done));
    asm.local_address(abi::SCRATCH[1], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        GRAPHICS_OFFSET_MTL_LAYER_SHOWN,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&show_done));
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "1"));
    asm.call_internal(METAL_LAYER_SYNC_SYMBOL);
    asm.push(abi::label(&show_done));
    asm.push(abi::load_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_PRESENTED,
    ));

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

/// bug-686 Phase 4 (graphics thread): `int _mfb_macapp_metal_layer_sync(int show)`.
///
/// Hops to the main thread — `mfbMetalLayer:` on the app delegate, `waitUntilDone:YES`
/// — so it configures the window's `CAMetalLayer`, sets its drawable size to the
/// `…_MTL_DRAWABLE_W`/`_H` the caller published, and, when `show` is non-zero, makes it
/// visible. Every one of those is a layer property, and layer properties belong to the
/// main thread (`.ai/canvas-threading.md` §1): the graphics thread only ever asks for
/// the next drawable and presents it.
///
/// Returns 1 once the main thread has run the apply, and 0 when there is no delegate to
/// hop to — headless, where there is no run loop to drain the perform and waiting would
/// deadlock. The delegate is read as the plain pointer the bootstrap published, for the
/// reason `emit_canvas_blit_helper` gives: `NSApp` is main-thread-only.
///
/// Waiting is what makes the drawable size safe to rely on: the frame asks for
/// `nextDrawable` only after this returns, so the drawable is already the new size.
pub(super) fn emit_metal_layer_sync() -> CodeFunction {
    let mut asm = Asm::new(METAL_LAYER_SYNC_SYMBOL);
    let frame = 32;
    let done = format!("{METAL_LAYER_SYNC_SYMBOL}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0))); // show

    asm.local_address(abi::LOCAL[0], DELEGATE_GLOBAL_SYM);
    asm.push(abi::load_u64(abi::LOCAL[0], abi::LOCAL[0], 0));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(&done));

    // Publish the visibility request where the apply reads it, before the hop.
    asm.local_address(abi::SCRATCH[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::store_u64(
        abi::LOCAL[1],
        abi::SCRATCH[0],
        GRAPHICS_OFFSET_MTL_LAYER_SHOW,
    ));

    // [delegate performSelectorOnMainThread:@selector(mfbMetalLayer:)
    //                            withObject:nil waitUntilDone:YES]
    asm.load_selector(SEL_MFB_METAL_LAYER.0);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(1))); // mfbMetalLayer: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[1]));
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "1"));

    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.metal.layerSync".to_string(),
        symbol: METAL_LAYER_SYNC_SYMBOL.to_string(),
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

/// bug-686 Phase 4 (main thread): the `mfbMetalLayer:` IMP.
///
/// Applies what [`emit_metal_layer_sync`] asked for to the layer the canvas view
/// build published in `…_MTL_LAYER`:
///
/// * once, the configuration a blit destination needs — the renderer's own device,
///   the render target's pixel format (`BGRA8Unorm_sRGB`, so the copy is a plain
///   copy), `framebufferOnly = NO` (a framebuffer-only texture cannot be a blit
///   destination), `opaque = YES` (every pixel is written, so the compositor need
///   not blend what is under it);
/// * the colour space — the window screen's, re-read on every apply (`SEL_SET_COLORSPACE`);
/// * the drawable size — the SURFACE size, which is in points. The readback path hands
///   the layer a `CGImage` of exactly that many pixels, so a drawable of the same size
///   shows exactly the same picture; Retina backing scale is not this change's;
/// * visibility, only ever turned ON here. It is turned off by the software blit's
///   apply, the other writer of `…_LAYER_SHOWN`, and both run on this thread.
///
/// All of it inside a `CATransaction` with actions disabled: `hidden` is animatable,
/// and an implicit fade over a frame is a frame showing the wrong picture.
pub(super) fn emit_metal_layer_apply() -> CodeFunction {
    let mut asm = Asm::new(METAL_LAYER_APPLY_SYMBOL);
    let frame = 32;
    let done = format!("{METAL_LAYER_APPLY_SYMBOL}_done");
    let configured = format!("{METAL_LAYER_APPLY_SYMBOL}_configured");
    let commit = format!("{METAL_LAYER_APPLY_SYMBOL}_commit");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));

    asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::LOCAL[1],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER,
    ));
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));

    emit_transaction_begin(&mut asm);

    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_READY,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&configured));
    // [layer setDevice:device] — the device the graphics thread's init built. It is
    // published before any frame can ask for this, because the frame asks only after
    // `_mfb_macapp_metal_init` has succeeded.
    asm.load_selector(SEL_SET_DEVICE.0);
    asm.push(abi::load_u64(
        abi::c_arg(2),
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_DEVICE,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    for (setter, value) in [
        (SEL_SET_PIXEL_FORMAT.0, MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB),
        (SEL_SET_FRAMEBUFFER_ONLY.0, "0"),
        (SEL_SET_OPAQUE.0, "1"),
    ] {
        asm.load_selector(setter);
        asm.push(abi::move_immediate(abi::c_arg(2), "Integer", value));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_READY,
    ));
    asm.push(abi::label(&configured));

    // [layer setColorspace:[[[window screen] colorSpace] CGColorSpace]] — every apply,
    // not once, so a hop after the window changed screens picks the new one up. See
    // `SEL_SET_COLORSPACE` for why it is the SCREEN's space. Any nil on the way (no
    // window yet, a window off every screen) leaves the layer's space as it was.
    // Each object is parked in the frame's spare slot at 24 across the next selector
    // lookup, which is a C call that clobbers every argument register.
    let no_screen_space = format!("{METAL_LAYER_APPLY_SYMBOL}_no_screen_space");
    asm.external_data(abi::c_arg(0), CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.push(abi::store_u64(abi::c_arg(0), abi::stack_pointer(), 24));
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 24));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::c_arg(1), WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    for selector in [SEL_SCREEN.0, SEL_COLOR_SPACE.0, SEL_CG_COLOR_SPACE.0] {
        asm.push(abi::compare_immediate(abi::c_return(0), "0"));
        asm.push(abi::branch_eq(&no_screen_space));
        asm.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), 24));
        asm.load_selector(selector);
        asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 24));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(&no_screen_space));
    asm.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), 24));
    asm.load_selector(SEL_SET_COLORSPACE.0);
    asm.push(abi::load_u64(abi::c_arg(2), abi::stack_pointer(), 24));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&no_screen_space));

    // [layer setDrawableSize:CGSizeMake(w, h)] — a two-double HFA, so `d0`/`d1`. Staged
    // AFTER the selector lookup: `sel_registerName` is a C call and may use the FP
    // argument registers as scratch.
    asm.load_selector(SEL_SET_DRAWABLE_SIZE.0);
    for (slot, fp) in [
        (GRAPHICS_OFFSET_MTL_DRAWABLE_W, abi::FP_SCRATCH[0]),
        (GRAPHICS_OFFSET_MTL_DRAWABLE_H, abi::FP_SCRATCH[1]),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::LOCAL[0], slot));
        asm.push(abi::signed_convert_to_float_d(fp, abi::SCRATCH[0]));
    }
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Show, if asked and not already shown.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_SHOW,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&commit));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_SHOWN,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ne(&commit));
    asm.load_selector(SEL_SET_HIDDEN.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[0],
        GRAPHICS_OFFSET_MTL_LAYER_SHOWN,
    ));

    asm.push(abi::label(&commit));
    emit_transaction_commit(&mut asm);

    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.metal.layerApply".to_string(),
        symbol: METAL_LAYER_APPLY_SYMBOL.to_string(),
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

/// `[CATransaction begin]; [CATransaction setDisableActions:YES]` — the bracket every
/// main-thread change to the Metal sublayer is made in (bug-686 Phase 4).
///
/// A standalone sublayer, unlike a view's own backing layer, animates its property
/// changes implicitly: without this, showing it would fade it in over a quarter of a
/// second and a resize would slide it to its new frame. Touches only the argument
/// registers, so a caller's callee-saved values survive it.
pub(super) fn emit_transaction_begin(asm: &mut Asm) {
    asm.load_selector(SEL_BEGIN.0);
    asm.external_data(abi::c_arg(0), CLASS_CA_TRANSACTION, LIB_QUARTZCORE);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_SET_DISABLE_ACTIONS.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
    asm.external_data(abi::c_arg(0), CLASS_CA_TRANSACTION, LIB_QUARTZCORE);
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// `[CATransaction commit]`, closing [`emit_transaction_begin`].
pub(super) fn emit_transaction_commit(asm: &mut Asm) {
    asm.load_selector(SEL_COMMIT.0);
    asm.external_data(abi::c_arg(0), CLASS_CA_TRANSACTION, LIB_QUARTZCORE);
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// Fill the parameter block at `sp + OFF_ITEM` from the geometry header whose
/// address is in `SCRATCH[0]`, and the edge buffer at `sp + OFF_EDGES` from its tail.
///
/// The quad is the header's **bounds**, not the shape's own extent: the bounds carry
/// the stroke's reach and the rim the software rasteriser gives itself so its coverage
/// ramp has pixels to run over -- a unit, which untransformed is a pixel, or half the
/// inverse's Frobenius norm of the item's units and a pixel under a transform
/// (`__canvas_boundsHeader`, bug-991) -- and the SDF fragment stage needs exactly the
/// same margin.
/// (Phase 1 used the exact extent because a flat fill has no ramp to make room for.)
///
/// Positions narrow to 16.16 fixed point, colours cross as the whole 0–255 values the
/// header already stores. Nothing here rounds a colour, so a fill is exact.
/// One quad per glyph, for a `__CANVAS_GEO_TEXT` item.
///
/// The Metal twin of `emit_glyph_publish` in `runtime/canvas/vulkan.rs`, and now the
/// same design (bug-670): each glyph's bitmap is copied into the frame buffer's glyph
/// region at `OFF_GLYPH_CURSOR`, one sample a word, and the cursor goes into the
/// glyph's block at `ITEM_ARC_EDGE_BASE`, where the shader finds it as `arc.z`. It used
/// to bind the bitmap with a per-glyph `setFragmentBytes:`, which was right while each
/// glyph was its own draw; once plan-116-H made the run ONE instanced draw, every glyph
/// read whichever bitmap was bound last.
///
/// The item block is built once for the run — fill, stroke and surface are the same for
/// every glyph in it — and then edited per glyph.
fn emit_glyph_publish(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_glyph_head");
    let done = format!("{METAL_DRAW_SYMBOL}_glyph_done");
    let next = format!("{METAL_DRAW_SYMBOL}_glyph_next");
    let blank = format!("{METAL_DRAW_SYMBOL}_glyph_blank");
    let publish = format!("{METAL_DRAW_SYMBOL}_glyph_publish");

    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    emit_item_block(asm, "glyph");

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
    // built: it draws nothing rather than indexing the metadata out of range. It still
    // publishes its (blank) block -- see `blank` below.
    asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
    asm.push(abi::branch_lt(&blank));

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

    // An empty bitmap — a space, or a glyph with no contours — has nothing to copy and
    // draws nothing, but still publishes its (blank) block — see `blank` below.
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_GLYPH_W,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[5], "0"));
    asm.push(abi::branch_le(&blank));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_GLYPH_H,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "0"));
    asm.push(abi::branch_le(&blank));
    // samples = w * h, and the frame's remaining room for them. The predicate has
    // already declined a frame that does not fit, so this bound is the emitter refusing
    // to write past its buffer if the two ever disagree — not a policy of its own.
    // Skipping rather than truncating: a clipped glyph is a different glyph.
    asm.push(abi::multiply_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        OFF_GLYPH_CURSOR,
    ));
    // Only SCRATCH[0..8] here: on AArch64 indices 10 and up realise to x20..x28, the
    // callee-saved registers this function keeps its loop state in, and 9 is x18,
    // which Apple reserves. SCRATCH[0..5] are free at this point — everything after
    // the copy reloads what it needs from the stack.
    asm.push(abi::add_registers(
        abi::SCRATCH[8],
        abi::SCRATCH[7],
        abi::SCRATCH[6],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &METAL_MAX_FRAME_GLYPH_SAMPLES.to_string(),
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[8], abi::SCRATCH[0]));
    asm.push(abi::branch_gt(&blank));

    // --- copy the bitmap into the frame buffer's glyph region --------------------------
    // dst = contents + (GLYPH_BASE + cursor) * 4, src = the cached bitmap; one byte of
    // coverage widens to one 32-bit word, which is what `edges` indexes.
    {
        let copy_head = format!("{METAL_DRAW_SYMBOL}_glyph_copy_head");
        let copy_done = format!("{METAL_DRAW_SYMBOL}_glyph_copy_done");
        let (src, dst, i, byte, at) = (
            abi::SCRATCH[0],
            abi::SCRATCH[1],
            abi::SCRATCH[2],
            abi::SCRATCH[3],
            abi::SCRATCH[4],
        );
        asm.push(abi::load_u64(src, abi::stack_pointer(), OFF_GLYPH_SRC));
        asm.push(abi::load_u64(dst, abi::stack_pointer(), OFF_CONTENTS));
        asm.push(abi::move_immediate(
            at,
            "Integer",
            &METAL_GLYPH_BASE_WORDS.to_string(),
        ));
        asm.push(abi::add_registers(at, at, abi::SCRATCH[7]));
        asm.push(abi::shift_left_immediate(at, at, 2));
        asm.push(abi::add_registers(dst, dst, at));
        asm.push(abi::move_immediate(i, "Integer", "0"));
        asm.push(abi::label(&copy_head));
        asm.push(abi::compare_registers(i, abi::SCRATCH[6]));
        asm.push(abi::branch_ge(&copy_done));
        asm.push(abi::add_registers(at, src, i));
        asm.push(abi::load_u8(byte, at, 0));
        asm.push(abi::shift_left_immediate(at, i, 2));
        asm.push(abi::add_registers(at, dst, at));
        asm.push(abi::store_u32(byte, at, 0));
        asm.push(abi::add_immediate(i, i, 1));
        asm.push(abi::branch(&copy_head));
        asm.push(abi::label(&copy_done));
        // The block names its slice; then the cursor moves past it.
        asm.push(abi::store_u32(
            abi::SCRATCH[7],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
        ));
        asm.push(abi::add_registers(
            abi::SCRATCH[7],
            abi::SCRATCH[7],
            abi::SCRATCH[6],
        ));
        asm.push(abi::store_u64(
            abi::SCRATCH[7],
            abi::stack_pointer(),
            OFF_GLYPH_CURSOR,
        ));
    }

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
    asm.push(abi::label(&publish));
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

    // No per-glyph payload: the bitmap is already in the frame buffer's glyph region,
    // named by this block's `arc.z` (bug-670).

    // plan-116-H: no draw here any more. A glyph run is still N quads, one block each,
    // but the draw that issues them is now the draw list's — a `Text` item is its own
    // entry with `count` = its glyph count, so the pass below issues them as one
    // instanced call at the group's offset. Leaving the draw here as well drew every
    // glyph twice, the second time with the offset undefined for that command.
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

    // A glyph that draws nothing — a space, an evicted cache entry, a bitmap past the
    // frame's glyph region — still publishes ONE block, with width and height 0 so the
    // shader's bounds test answers zero coverage for every fragment (bug-484).
    //
    // Skipping the publish instead desynchronised the item buffer from the draw list:
    // `__canvas_blockInstances` counts a text run as its full glyph count, so every
    // draw entry after a run containing a space named blocks one position too far on —
    // the next items drew under their neighbours' pipelines and group offsets, and the
    // last entry read a block that was never written. The quad is left as it is (the
    // previous glyph's box, or the run's hull under a transform): a transparent source
    // leaves the destination unchanged under all four blend pipelines, and zeroing the
    // quad would strand the hull a following transformed glyph relies on.
    asm.push(abi::label(&blank));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 12,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    asm.push(abi::branch(&publish));
    asm.push(abi::label(&done));
}

/// Build the item block at `OFF_ITEM` from the geometry header in `SCRATCH[0]`. `tag`
/// names this call site's labels: the draw function builds a block for a shape and, in
/// the glyph loop, one per glyph.
fn emit_item_block(asm: &mut Asm, tag: &str) {
    let header = abi::SCRATCH[0];
    let scale = abi::FP_SCRATCH[0];
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[1]));

    // The 16.16 fields no distance reads: the bounds and the gradient axis.
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

    // bug-735: every field a distance reads -- the shape parameters, the radius and
    // stroke half-width, an arc's sweep endpoints, an ellipse's rotation and the clip --
    // as its 16.16 word and, in the block's residual `ivec4`s, its residual. The twin of
    // the loop in `runtime/canvas/vulkan.rs`, from the same table.
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[1],
    ));
    for (slot, word, residual) in ITEM_RESIDUAL_FIELDS {
        for instruction in
            emit_item_residual_field(header, slot, OFF_ITEM + word, OFF_ITEM + residual)
        {
            asm.push(instruction);
        }
    }
    for instruction in emit_item_stroke_sign(
        header,
        OFF_ITEM,
        &format!("{METAL_DRAW_SYMBOL}_{tag}_stroke_positive"),
        &format!("{METAL_DRAW_SYMBOL}_{tag}_stroke_done"),
    ) {
        asm.push(instruction);
    }
    // bug-735: `arc.x` = 0 and an arc's `reflex` flag in `arc.y`, decided in f64.
    for instruction in emit_item_arc_flags(
        header,
        OFF_ITEM,
        &format!("{METAL_DRAW_SYMBOL}_{tag}_reflex_set"),
        &format!("{METAL_DRAW_SYMBOL}_{tag}_reflex_done"),
    ) {
        asm.push(instruction);
    }
    // plan-163-A: a polygon's fill rule over that word, which only an arc reads.
    for instruction in emit_item_polygon_rule(
        header,
        OFF_ITEM,
        &format!("{METAL_DRAW_SYMBOL}_{tag}_rule_done"),
    ) {
        asm.push(instruction);
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

    // misc = { kind, radius, strokeHalf, edgeCount }: the two whole numbers here, the
    // two 16.16 ones with their residuals above.
    for (index, slot) in [(0usize, HEADER_KIND), (3, HEADER_AUX0)] {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        asm.push(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
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

    // plan-163-D: a radial gradient's own inverse -- six float32 bit patterns and the
    // flag, header slots 57-63 -- word for word into `gradInv0`/`gradInv1`, the
    // transform's layout; and its focal offset and radius narrowed to 16.16 (`gradFocal`).
    for index in 0..7 {
        asm.push(abi::load_double(
            abi::FP_SCRATCH[1],
            header,
            (HEADER_GRADIENT_INV + index) * 8,
        ));
        asm.push(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_GRADIENT_INV + index * 4,
        ));
    }
    debug_assert_eq!(HEADER_GRADIENT_INV + 6, HEADER_GRADIENT_HAS_INV);
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[1],
    ));
    for index in 0..3 {
        asm.push(abi::load_double(
            abi::FP_SCRATCH[1],
            header,
            (HEADER_GRADIENT_FOCAL_X + index) * 8,
        ));
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_GRADIENT_FOCAL + index * 4,
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
    // plan-163-D: packed with the spread and the stroke apply, `kind + 2 * spread + 8 *
    // apply` (`ITEM_SURFACE_GRADIENT_KIND`), every term a small whole number.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_KIND * 8,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[2],
        header,
        HEADER_GRADIENT_SPREAD * 8,
    ));
    asm.push(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    asm.push(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[2],
        header,
        HEADER_GRADIENT_APPLY * 8,
    ));
    for _ in 0..3 {
        asm.push(abi::float_add_d(
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
        ));
    }
    asm.push(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
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
/// `METAL_MAX_FRAME_ITEMS`, so the two cannot disagree about what "full" means.
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
        &METAL_MAX_FRAME_ITEMS.to_string(),
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

    // bug-550: a `const` assertion, not a `debug_assert_eq!`. Both operands are
    // compile-time constants, so this is decidable when the compiler ITSELF is
    // built and costs nothing at run time — strictly better than an assertion that
    // ran nowhere because CI builds `--release`.
    const _: () = assert!(
        ITEM_BLOCK_SIZE % 8 == 0,
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
    let strokes = format!("{METAL_DRAW_SYMBOL}_publish_strokes");

    // Split only when the mode is not `Normal`...
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM_MODE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&single));
    // ...and the item actually strokes: `strokeHalf` > 0, which since bug-735 is its
    // 16.16 word > 0 or, for a word of 0, its residual > 0 -- `emit_item_stroke_sign`
    // makes that exactly the header's f64 `half > 0.0`, the test
    // `__canvas_blockInstances` counts two records on. Before it this read the word
    // alone, so a blended filled item stroked thinner than 2^-16 px was counted as two
    // records and published as one, and every later draw named the wrong blocks.
    //
    // SIGNED, for the reason the Vulkan twin records (plan-116-H **H15**):
    // `__canvas_strokeHalf` reports "does not stroke" as **-1.0**, which is
    // `0xFFFF0000` in 16.16, and `load_u32` zero-extends -- so without the `sxtw` the
    // 64-bit compare sees 4294901760 and every blended fill-only item is published as
    // TWO records. The comment below already claims such an item takes the single path.
    //
    // It was latent while the draw came from `emit_run_flush`, whose instance count was
    // `cursor - run_start` -- whatever had actually been published, so the spurious
    // record was drawn and painted nothing. Phase 3 took this backend's counts from
    // `__canvas_sceneDraws`, so it is latent no longer: one extra record here shifts
    // every later draw base and the scene loses its tail.
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 8,
    ));
    asm.push(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_gt(&strokes));
    asm.push(abi::branch_lt(&single));
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC_LO + 4,
    ));
    asm.push(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_le(&single));
    asm.push(abi::label(&strokes));
    // ...and actually fills. A `Line` or an `Arc` arrives with its stroke colour
    // already moved into the fill slots and `strokeHalf` negative
    // (`__canvas_strokeAsFill`), so it is fill-only and takes the single path.
    //
    // plan-163-D: "fills" is a visible fill alpha, OR a gradient (its ramp is the fill
    // colour whatever the fill alpha), OR a picture (its image is, and since plan-163-C
    // its fill alpha is only the tint's strength). Testing the fill alpha alone sent a
    // blended stroked gradient or picture with a transparent fill down the single path,
    // composing stroke over fill in the shader under a mode where that is wrong.
    let fills = format!("{METAL_DRAW_SYMBOL}_publish_fills");
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_FILL + 12,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_gt(&fills));
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "2"));
    asm.push(abi::branch_ge(&fills));
    asm.push(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC,
    ));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[0],
        &GEO_KIND_PICTURE.to_string(),
    ));
    asm.push(abi::branch_ne(&single));
    asm.push(abi::label(&fills));

    // Record one: the fill, with the stroke switched off. `strokeHalf` is parked on the
    // STACK — `emit_item_publish` owns the low scratch registers, so a register saved
    // across it comes back holding a mapped address. Off is -1.0 (`0xFFFF0000`), the
    // value `__canvas_strokeHalf` reports for a paint that does not stroke: since
    // bug-735 the shader strokes on the word PLUS its residual, and a zero word under a
    // positive residual would still stroke.
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
    // -1.0 in 16.16, spelled unsigned: the immediate is never negative, and the store
    // keeps the low 32 bits.
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        "4294901760",
    ));
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
    // plan-163-D: marked OFF -- alpha -1, which every shader reads as "no fill in this
    // record" -- rather than made transparent: a gradient's or a picture's colour does
    // not come from the fill alpha, so a zero there left record two drawing the ramp or
    // the image a second time under the blend. The stroke still reads the ramp
    // (`FillAndStroke`). Spelled unsigned; the store keeps the low 32 bits.
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        "4294967295",
    ));
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

/// One instanced draw per entry of `__canvas_sceneDraws`, after every block is
/// published (plan-116-H Phase 3 — the Metal twin of `emit_draw_list_pass` in
/// `runtime/canvas/vulkan.rs`).
///
/// This replaces `emit_run_flush`, which ended a run at a blend-mode change, at a glyph
/// run and at the end of the scene, and took its instance count from the emitter's own
/// cursor (`cursor - run_start`). The list now says where every run starts, how long it
/// is, which pipeline it wants and — the reason this letter exists — what offset to
/// draw it at.
///
/// Taking the count from the list rather than from the cursor is what makes a
/// disagreement between the two visible: while the count was `cursor - run_start` it
/// was true by construction, so a publish that wrote one record too many painted
/// nothing and went unnoticed. plan-116-H **H15** is exactly that bug, found on the
/// Vulkan side after this same conversion; the signed `strokeHalf` read in
/// `emit_split_or_publish` above is its fix, and it is a fix this backend needed too.
///
/// The offset is sent to BOTH stages. The vertex stage moves the quad and the fragment
/// stage moves the query point back, so a gradient or a glyph — both sampled per
/// fragment from `in.pos.xy` — lands where the group put it. Sending it to the vertex
/// stage alone draws the shape in the right place with the wrong ramp inside it.
fn emit_draw_list_pass(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_drawlist_head");
    let done = format!("{METAL_DRAW_SYMBOL}_drawlist_done");

    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAW_INDEX,
    ));

    asm.push(abi::label(&head));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAW_INDEX,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_DRAW_ENTRIES,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_ge(&done));

    // entry = draws + index * (CANVAS_DRAW_ENTRY_WORDS * 8). A shift, so the width has
    // to stay a power of two; `the_draw_entry_width_agrees_with_the_emitter` is what
    // keeps this equal to what `__canvas_pushOneDraw` appends.
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        CANVAS_DRAW_ENTRY_SHIFT as u8,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_DRAWS,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));

    // Every field is parked on the STACK before the first `load_selector`, which calls
    // through `sel_registerName` and clobbers the whole scratch bank — the same reason
    // `emit_run_flush` sourced its arguments from memory. An `Integer` here is 64 bits,
    // so these are `load_u64` and not `load_u32`: reading them 32 bits wide takes the
    // low half of `base` for `base` and the low half of `count` for `dx`.
    asm.push(abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_DRAW_BASE,
    ));
    asm.push(abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[0], 8));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_DRAW_COUNT,
    ));
    // The two 16.16 offsets and (bug-735) their residual words, then (plan-172-H) the
    // inverse linear part, written as 32-bit words side by side so the thirty-two bytes
    // at `OFF_DRAW_PAIR` are exactly the `MfbDraw` both shader stages declare. Entry
    // words 2, 3, 5, 6 and 8-11 (`__canvas_pushOneDraw`); the last four are zero in an
    // entry with no matrix, which is how the shader knows.
    for (word, at) in [
        (2usize, 0usize),
        (3, 4),
        (5, 8),
        (6, 12),
        (8, 16),
        (9, 20),
        (10, 24),
        (11, 28),
    ] {
        asm.push(abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[0], word * 8));
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_DRAW_PAIR + at,
        ));
    }

    // This entry's pipeline, from the blend mode the run shares. The publish walk binds
    // as it goes, but every draw now happens after that walk has finished, so a binding
    // left there would apply the LAST item's mode to the whole frame.
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        CANVAS_DRAW_ENTRY_MODE,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        3,
    ));
    asm.local_address(abi::SCRATCH[2], GRAPHICS_STATE_SYMBOL);
    asm.push(abi::add_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        GRAPHICS_OFFSET_MTL_PIPELINE_MODES,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
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

    // The offset, to both stages. `setVertexBytes:` is the selector plan-116-A deleted
    // and plan-116-H re-added (**H5**); missing it from `metal_data_objects` is not a
    // compile error, it is a message to an unregistered selector.
    for (selector, index) in [
        (SEL_SET_VERTEX_BYTES.0, METAL_DRAW_OFFSET_VERTEX_INDEX),
        (SEL_SET_FRAGMENT_BYTES.0, METAL_DRAW_OFFSET_FRAGMENT_INDEX),
    ] {
        asm.load_selector(selector);
        asm.push(abi::add_immediate(
            abi::c_arg(2),
            abi::stack_pointer(),
            OFF_DRAW_PAIR,
        ));
        asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "32"));
        asm.push(abi::move_immediate(abi::c_arg(4), "Integer", index));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

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
        OFF_DRAW_COUNT,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(6),
        abi::stack_pointer(),
        OFF_DRAW_BASE,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAW_INDEX,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_DRAW_INDEX,
    ));
    asm.push(abi::branch(&head));
    asm.push(abi::label(&done));
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
        &METAL_MAX_FRAME_GRADIENT_STOPS.to_string(),
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

/// plan-163-C: the clip's item-block words -- `clipPath` = (first clip edge, clip edge
/// count, mode | rule << 2, a picture's transparency step T) and `clipBand`'s band count
/// 0 (the shader walks every clip edge) -- and a path clip's edges, copied from the
/// record's tail (`slot1 − stops·5 − edges·5`, right before the stops) into the frame's
/// edge region after the item's own edges and join records, as 16.16 words with their
/// residuals, exactly as a polygon's edges are (`emit_edge_buffer`). The frame-sum edge cap
/// counts them, so the over-cap arm is unreachable; it leaves the edge count 0, which the
/// shader reads as a clip enclosing nothing -- hidden, never drawn unclipped.
fn emit_clip_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_clip_head");
    let done = format!("{METAL_DRAW_SYMBOL}_clip_done");
    let copy = format!("{METAL_DRAW_SYMBOL}_clip_copy");
    let banded = format!("{METAL_DRAW_SYMBOL}_clip_banded");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[6];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];
    let clip = OFF_ITEM + ITEM_OFFSET_CLIP_PATH;
    let band = OFF_ITEM + ITEM_OFFSET_CLIP_BAND;

    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    for word in [0usize, 4, 8, 12] {
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            band + word,
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            clip + word,
        ));
    }
    asm.push(abi::load_u64(
        header,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    // mode | rule << 2, and T.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_MODE * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_RULE * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        2,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[5],
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_MODE,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_TRANSPARENCY * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_PICTURE_T,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[5], "2"));
    asm.push(abi::branch_ne(&done));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(count, "0"));
    asm.push(abi::branch_le(&done));
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    asm.push(abi::add_registers(abi::SCRATCH[7], abi::SCRATCH[5], count));
    asm.push(abi::move_immediate(
        abi::SCRATCH[8],
        "Integer",
        &METAL_MAX_FRAME_EDGES.to_string(),
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[7], abi::SCRATCH[8]));
    asm.push(abi::branch_le(&copy));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&copy));
    asm.push(abi::store_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    asm.push(abi::store_u32(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_BASE,
    ));
    asm.push(abi::store_u32(
        count,
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_COUNT,
    ));
    // target = contents + METAL_EDGE_BASE_WORDS * 4 + base * 16, through registers (the
    // offset is past an `add`'s immediate); the residuals a fixed distance past it.
    asm.push(abi::shift_left_immediate(target, abi::SCRATCH[5], 4));
    asm.push(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        OFF_CONTENTS,
    ));
    asm.push(abi::add_registers(target, target, abi::SCRATCH[7]));
    asm.push(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &(METAL_EDGE_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(target, target, abi::SCRATCH[7]));
    asm.push(abi::move_immediate(
        residual_target,
        "Integer",
        &((METAL_EDGE_RESIDUAL_BASE_WORDS - METAL_EDGE_BASE_WORDS) * 4).to_string(),
    ));
    asm.push(abi::add_registers(residual_target, residual_target, target));
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[5],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[5]));
    // source = header + (slot1 - stops * 5 - count * 5) * 8.
    asm.push(abi::load_u64(
        header,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::load_double(abi::FP_SCRATCH[1], header, 8));
    asm.push(abi::float_convert_to_signed_x(source, abi::FP_SCRATCH[1]));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::add_registers(abi::SCRATCH[7], abi::SCRATCH[7], count));
    asm.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "5"));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    asm.push(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    asm.push(abi::shift_left_immediate(source, source, 3));
    asm.push(abi::add_registers(source, header, source));
    asm.push(abi::move_immediate(index, "Integer", "0"));
    asm.push(abi::label(&head));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&banded));
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
        asm.push(abi::store_u32(abi::SCRATCH[5], target, 0));
        asm.push(abi::add_immediate(target, target, 4));
        asm.push(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[5],
        ));
        asm.push(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[5], residual_target, 0));
        asm.push(abi::add_immediate(residual_target, residual_target, 4));
    }
    asm.push(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&head));
    // The clip's edges are in the region; index them by band.
    asm.push(abi::label(&banded));
    emit_band_index(asm, CLIP_BANDS);
    asm.push(abi::label(&done));
}

/// plan-163-B: copy this item's dash words into the frame buffer's dash region, and name
/// the slice in its item block — `strokeStyle.z` the first word, `strokeStyle.w` the word
/// count (`emit_join_buffer` zeroed both, which the shader reads as a solid stroke).
///
/// The words are float32 bit patterns: the phase (header slot `HEADER_DASH_PHASE`), then
/// the record's dash tail — the entry count, the period, each entry's end, and the kind's
/// perimeter data (`helper_dashes.rs`) — found at `slot1 − stops·5 − dashes`, right before
/// the gradient stops. Floats rather than 16.16: the pattern's lengths and an outline's
/// perimeter positions are compared, never added to a coordinate, and fp32 holds a
/// thousand-pixel perimeter to 6e-5 px. A frame whose words would pass the region's cap
/// is declined by the predicate, so the over-cap arm is unreachable and leaves the stroke
/// solid rather than write past the region.
fn emit_dash_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_dash_head");
    let done = format!("{METAL_DRAW_SYMBOL}_dash_done");
    let copy = format!("{METAL_DRAW_SYMBOL}_dash_copy");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let cursor = abi::SCRATCH[5];
    let target = abi::SCRATCH[6];
    let style = OFF_ITEM + ITEM_OFFSET_STROKE_STYLE;

    asm.push(abi::load_u64(
        header,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_DASH_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(count, "0"));
    asm.push(abi::branch_le(&done));
    // The slice: the phase word, then the tail's `count` words.
    asm.push(abi::load_u64(cursor, abi::stack_pointer(), OFF_DASH_CURSOR));
    asm.push(abi::add_registers(abi::SCRATCH[1], cursor, count));
    asm.push(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &METAL_MAX_FRAME_DASH_WORDS.to_string(),
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[7]));
    asm.push(abi::branch_le(&copy));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&copy));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_DASH_CURSOR,
    ));
    asm.push(abi::store_u32(
        cursor,
        abi::stack_pointer(),
        style + ITEM_STROKE_DASH_BASE,
    ));
    asm.push(abi::add_immediate(abi::SCRATCH[7], count, 1));
    asm.push(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        style + ITEM_STROKE_DASH_COUNT,
    ));
    // target = contents + METAL_DASH_BASE_WORDS * 4 + cursor * 4, through registers for
    // `emit_edge_buffer`'s reason (the offset is past an `add`'s immediate).
    asm.push(abi::shift_left_immediate(target, cursor, 2));
    asm.push(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        OFF_CONTENTS,
    ));
    asm.push(abi::add_registers(target, target, abi::SCRATCH[7]));
    asm.push(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &(METAL_DASH_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(target, target, abi::SCRATCH[7]));
    // The phase.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_DASH_PHASE * 8,
    ));
    asm.push(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[7], target, 0));
    asm.push(abi::add_immediate(target, target, 4));
    // source = header + (slot1 - stops * 5 - count) * 8.
    asm.push(abi::load_double(abi::FP_SCRATCH[1], header, 8));
    asm.push(abi::float_convert_to_signed_x(source, abi::FP_SCRATCH[1]));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "5"));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    asm.push(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    // plan-163-C: the path clip's edges sit between the dashes and the stops.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    asm.push(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    asm.push(abi::subtract_registers(source, source, count));
    asm.push(abi::shift_left_immediate(source, source, 3));
    asm.push(abi::add_registers(source, header, source));
    asm.push(abi::move_immediate(index, "Integer", "0"));
    asm.push(abi::label(&head));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&done));
    asm.push(abi::load_double(abi::FP_SCRATCH[1], source, 0));
    asm.push(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[7], target, 0));
    asm.push(abi::add_immediate(target, target, 4));
    asm.push(abi::add_immediate(source, source, 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&head));
    asm.push(abi::label(&done));
}

/// Copy a picture's texels into the frame buffer's **glyph region**, and name the slice
/// in its item block the way a glyph's does (bug-484): width in `misc.w`, height in
/// `arc.x` (`ITEM_ARC_GLYPH_HEIGHT`), base in `arc.z` (`ITEM_ARC_EDGE_BASE`). Every
/// other kind falls straight through.
///
/// The pixel block is the image's `List OF Byte`, whose address the header carries
/// split across two slots (`HEADER_PICTURE_SHADOW_HI/LO`) because a whole 48-bit
/// address does not survive the header hash. It is RGBA8, row-major, top row first,
/// so each texel's four bytes are already the little-endian word
/// `r | g << 8 | b << 16 | a << 24` the shader unpacks — the copy is verbatim, one word
/// a texel. No block is ever freed, so reading it during the frame is safe.
///
/// A picture is ONE quad and does not end the instanced run; it only shares the glyph
/// region's cursor. Since bug-686 each distinct pixel block is copied once per frame:
/// `emit_picture_lookup` finds a block already uploaded this frame and reuses its
/// texels, and `emit_picture_record` records each new upload. Runs **after**
/// `emit_edge_buffer`, which zeroes `arc.z` for every non-polygon, and after
/// `emit_item_block`, which writes `arc.x` as a 16.16 angle.
///
/// When the image cannot be uploaded — a destroyed image (zero size or no block), a
/// block shorter than `iw * ih * 4`, or a frame whose samples would pass
/// `METAL_MAX_FRAME_GLYPH_SAMPLES` (unreachable: the predicate counts picture texels
/// against that cap and declines such a frame) — `misc.w` is left 0, which the shader
/// reads as a transparent fill. The stroke still draws, as the oracle's does over a
/// transparent `shadowTexel`. Never a write past the region.
///
/// No `objc_msgSend` here, and only `SCRATCH[0..8]`: indices 10 and up are the loop
/// state's callee-saved registers and 9 is Apple's reserved x18.
fn emit_picture_buffer(asm: &mut Asm) {
    let done = format!("{METAL_DRAW_SYMBOL}_picture_done");
    let copy_head = format!("{METAL_DRAW_SYMBOL}_picture_copy_head");
    let header = abi::SCRATCH[0];
    let kind = abi::SCRATCH[1];
    let width = abi::SCRATCH[2];
    let height = abi::SCRATCH[3];
    let samples = abi::SCRATCH[4];
    let block = abi::SCRATCH[5];
    let temp = abi::SCRATCH[6];
    let cursor = abi::SCRATCH[7];
    let end = abi::SCRATCH[8];

    asm.push(abi::load_u64(
        header,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_KIND * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(kind, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(kind, GEO_KIND_PICTURE));
    asm.push(abi::branch_ne(&done));

    for (register, slot) in [(width, HEADER_AUX0), (height, HEADER_AUX1)] {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        asm.push(abi::float_convert_to_signed_x(register, abi::FP_SCRATCH[1]));
    }
    // The not-uploaded defaults: width 0 (transparent), the real height, base 0.
    asm.push(abi::move_immediate(temp, "Integer", "0"));
    asm.push(abi::store_u32(
        temp,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 12,
    ));
    asm.push(abi::store_u32(
        temp,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    asm.push(abi::store_u32(
        height,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    asm.push(abi::compare_immediate(width, "0"));
    asm.push(abi::branch_le(&done));
    asm.push(abi::compare_immediate(height, "0"));
    asm.push(abi::branch_le(&done));
    asm.push(abi::multiply_registers(samples, width, height));

    // block = (hi << PICTURE_SHADOW_SPLIT_BITS) | lo
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_SHADOW_HI * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(block, abi::FP_SCRATCH[1]));
    const _: () = assert!(
        PICTURE_SHADOW_SPLIT_BITS < 64,
        "the split is a 64-bit shift amount"
    );
    asm.push(abi::shift_left_immediate(
        block,
        block,
        PICTURE_SHADOW_SPLIT_BITS as u8,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_SHADOW_LO * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(temp, abi::FP_SCRATCH[1]));
    asm.push(abi::or_registers(block, block, temp));
    asm.push(abi::compare_immediate(block, "0"));
    asm.push(abi::branch_eq(&done));
    // The block must hold every texel the width and height promise — the bound
    // `canvas::shadowTexel` applies per read, applied once.
    asm.push(abi::load_u64(temp, block, COLLECTION_OFFSET_COUNT));
    asm.push(abi::shift_left_immediate(end, samples, 2));
    asm.push(abi::compare_registers(end, temp));
    asm.push(abi::branch_gt(&done));

    // bug-686: has this frame already uploaded this block? Then point the item at those
    // texels and copy nothing. `height` is dead from here (stored above) and serves as
    // the record pointer; `header`, `kind`, `temp`, `cursor` and `end` are free until the
    // fit check. No `compare_immediate` against a wide constant in this stretch: its
    // encoding borrows x16/x17, which are `cursor` and `end`.
    emit_picture_lookup(asm, &done);

    // Would this picture's texels fit the frame's glyph region?
    asm.push(abi::load_u64(
        cursor,
        abi::stack_pointer(),
        OFF_GLYPH_CURSOR,
    ));
    asm.push(abi::add_registers(end, cursor, samples));
    asm.push(abi::move_immediate(
        temp,
        "Integer",
        &METAL_MAX_FRAME_GLYPH_SAMPLES.to_string(),
    ));
    asm.push(abi::compare_registers(end, temp));
    asm.push(abi::branch_gt(&done));

    // The block names its slice; then the cursor moves past it.
    asm.push(abi::store_u64(end, abi::stack_pointer(), OFF_GLYPH_CURSOR));
    asm.push(abi::store_u32(
        cursor,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    asm.push(abi::store_u32(
        width,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 12,
    ));
    // bug-686: record (block, base) so later items naming this block reuse the texels.
    emit_picture_record(asm);

    // dst = contents + (GLYPH_BASE + cursor) * 4, src = block + COLLECTION_HEADER_SIZE.
    // `header` and `kind` are dead from here and serve as the copy's registers.
    let (dst, word) = (header, kind);
    asm.push(abi::move_immediate(
        dst,
        "Integer",
        &METAL_GLYPH_BASE_WORDS.to_string(),
    ));
    asm.push(abi::add_registers(dst, dst, cursor));
    asm.push(abi::shift_left_immediate(dst, dst, 2));
    asm.push(abi::load_u64(temp, abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::add_registers(dst, temp, dst));
    asm.push(abi::add_immediate(block, block, COLLECTION_HEADER_SIZE));
    asm.push(abi::label(&copy_head));
    asm.push(abi::compare_immediate(samples, "0"));
    asm.push(abi::branch_le(&done));
    asm.push(abi::load_u32(word, block, 0));
    asm.push(abi::store_u32(word, dst, 0));
    asm.push(abi::add_immediate(block, block, 4));
    asm.push(abi::add_immediate(dst, dst, 4));
    asm.push(abi::subtract_immediate(samples, samples, 1));
    asm.push(abi::branch(&copy_head));
    asm.push(abi::label(&done));
}

/// The picture table's multiplicative hash constant (xorshift*'s), odd so the multiply
/// is a bijection on the address bits it keeps.
const PICTURE_HASH_MULTIPLIER: &str = "2685821657736338717";

/// Look the picture's pixel block up in this frame's picture table (bug-686). On a hit,
/// name the uploaded texels in the item block (base in `arc.z`, width in `misc.w`) and
/// branch to `done`; on a miss, park the empty index slot the probe ended at in
/// `OFF_PIC_SLOT` and fall through to the upload.
///
/// Live on entry: `block` (`SCRATCH[5]`), `width` (`SCRATCH[2]`), `samples`
/// (`SCRATCH[4]`). Uses `SCRATCH[0,1,3,6,7,8]`, all dead at the call site.
///
/// A slot is live only when its record number is below this frame's count and that
/// record names the slot back — the sparse-set test `METAL_PICTURE_TABLE_BYTES`
/// describes — so the index never needs clearing.
fn emit_picture_lookup(asm: &mut Asm, done: &str) {
    let probe = format!("{METAL_DRAW_SYMBOL}_picture_probe");
    let live = format!("{METAL_DRAW_SYMBOL}_picture_live");
    let miss = format!("{METAL_DRAW_SYMBOL}_picture_miss");
    let hit = format!("{METAL_DRAW_SYMBOL}_picture_hit");
    let (index, record_no, record, records, slot, count) = (
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        abi::SCRATCH[6],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    );
    let (width, block) = (abi::SCRATCH[2], abi::SCRATCH[5]);
    const INDEX_BITS: u32 = METAL_PICTURE_INDEX_SLOTS.trailing_zeros();

    // records = contents + RECORDS_OFFSET; index = contents + INDEX_OFFSET.
    asm.push(abi::load_u64(records, abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::move_immediate(
        index,
        "Integer",
        &METAL_PICTURE_INDEX_OFFSET.to_string(),
    ));
    asm.push(abi::add_registers(index, records, index));
    asm.push(abi::move_immediate(
        record_no,
        "Integer",
        &METAL_PICTURE_RECORDS_OFFSET.to_string(),
    ));
    asm.push(abi::add_registers(records, records, record_no));
    // slot = ((block >> 4) * K) >> (64 - INDEX_BITS). Blocks are 16-byte aligned arena
    // allocations, so the low four bits carry nothing.
    asm.push(abi::shift_right_immediate(slot, block, 4));
    asm.push(abi::move_immediate(
        record_no,
        "Integer",
        PICTURE_HASH_MULTIPLIER,
    ));
    asm.push(abi::multiply_registers(slot, slot, record_no));
    asm.push(abi::shift_right_immediate(
        slot,
        slot,
        (64 - INDEX_BITS) as u8,
    ));
    asm.push(abi::load_u64(count, abi::stack_pointer(), OFF_PIC_COUNT));

    asm.push(abi::label(&probe));
    asm.push(abi::shift_left_immediate(record_no, slot, 2));
    asm.push(abi::add_registers(record_no, index, record_no));
    asm.push(abi::load_u32(record_no, record_no, 0));
    // Unsigned: a slot holding garbage from before this buffer's first frame is a large
    // number, and it must read as "not below the count" rather than as negative.
    asm.push(abi::compare_registers(record_no, count));
    asm.push(abi::branch_lo(&live));
    asm.push(abi::branch(&miss));
    asm.push(abi::label(&live));
    const _: () = assert!(
        PICTURE_RECORD_BYTES == 16,
        "records are addressed by a shift of 4"
    );
    asm.push(abi::shift_left_immediate(record, record_no, 4));
    asm.push(abi::add_registers(record, records, record));
    asm.push(abi::load_u32(record_no, record, 12));
    asm.push(abi::compare_registers(record_no, slot));
    asm.push(abi::branch_ne(&miss));
    asm.push(abi::load_u64(record_no, record, 0));
    asm.push(abi::compare_registers(record_no, block));
    asm.push(abi::branch_eq(&hit));
    // Another block's live slot: the next one, wrapping.
    asm.push(abi::add_immediate(slot, slot, 1));
    asm.push(abi::move_immediate(
        record_no,
        "Integer",
        &(METAL_PICTURE_INDEX_SLOTS - 1).to_string(),
    ));
    asm.push(abi::and_registers(slot, slot, record_no));
    asm.push(abi::branch(&probe));

    asm.push(abi::label(&hit));
    asm.push(abi::load_u32(record_no, record, 8));
    asm.push(abi::store_u32(
        record_no,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    asm.push(abi::store_u32(
        width,
        abi::stack_pointer(),
        OFF_ITEM + ITEM_OFFSET_MISC + 12,
    ));
    asm.push(abi::branch(done));

    asm.push(abi::label(&miss));
    asm.push(abi::store_u64(slot, abi::stack_pointer(), OFF_PIC_SLOT));
}

/// Record the picture just given its slice of the glyph region — `block` in
/// `SCRATCH[5]`, its base in `cursor` (`SCRATCH[7]`) — at the index slot
/// `emit_picture_lookup` parked (bug-686).
///
/// Uses `SCRATCH[0,1,6]` only: `cursor` and `block` are read after this by the copy.
/// The count check is unreachable — one record per picture item, and the predicate
/// already caps items at `METAL_PICTURE_RECORDS` — and is kept because the alternative
/// is a write past the table; an unrecorded picture is still uploaded, only not shared.
fn emit_picture_record(asm: &mut Asm) {
    let full = format!("{METAL_DRAW_SYMBOL}_picture_table_full");
    let (count, at, temp) = (abi::SCRATCH[0], abi::SCRATCH[1], abi::SCRATCH[6]);
    let (block, cursor) = (abi::SCRATCH[5], abi::SCRATCH[7]);

    asm.push(abi::load_u64(count, abi::stack_pointer(), OFF_PIC_COUNT));
    asm.push(abi::move_immediate(
        temp,
        "Integer",
        &METAL_PICTURE_RECORDS.to_string(),
    ));
    asm.push(abi::compare_registers(count, temp));
    asm.push(abi::branch_ge(&full));

    // records[count] = (block, cursor, slot)
    asm.push(abi::load_u64(temp, abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::shift_left_immediate(at, count, 4));
    asm.push(abi::add_registers(at, temp, at));
    asm.push(abi::move_immediate(
        temp,
        "Integer",
        &METAL_PICTURE_RECORDS_OFFSET.to_string(),
    ));
    asm.push(abi::add_registers(at, at, temp));
    asm.push(abi::store_u64(block, at, 0));
    asm.push(abi::store_u32(cursor, at, 8));
    asm.push(abi::load_u64(temp, abi::stack_pointer(), OFF_PIC_SLOT));
    asm.push(abi::store_u32(temp, at, 12));
    // index[slot] = count -- written AFTER the record, so the slot is never live while
    // the record it names is stale.
    asm.push(abi::shift_left_immediate(temp, temp, 2));
    asm.push(abi::load_u64(at, abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::add_registers(at, at, temp));
    asm.push(abi::move_immediate(
        temp,
        "Integer",
        &METAL_PICTURE_INDEX_OFFSET.to_string(),
    ));
    asm.push(abi::add_registers(at, at, temp));
    asm.push(abi::store_u32(count, at, 0));
    asm.push(abi::add_immediate(count, count, 1));
    asm.push(abi::store_u64(count, abi::stack_pointer(), OFF_PIC_COUNT));
    asm.push(abi::label(&full));
}

/// plan-163-B: the item's join records into the frame buffer's edge region after its own
/// edges, two edge slots per record -- `emit_join_upload` in `runtime/canvas/vulkan.rs`,
/// line for line: the records' points as 16.16 words with their residuals, found from
/// the record's end (`header + (slot1 - stops * 5 - records * 8) * 8`), their first slot
/// and count in `strokeStyle`, and 0 there (and in the dash words) for an item with none.
fn emit_join_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_join_head");
    let done = format!("{METAL_DRAW_SYMBOL}_join_done");
    let none = format!("{METAL_DRAW_SYMBOL}_join_none");
    let copy = format!("{METAL_DRAW_SYMBOL}_join_copy");
    let records = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let slots = abi::SCRATCH[6];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];
    let style = OFF_ITEM + ITEM_OFFSET_STROKE_STYLE;

    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in [ITEM_STROKE_DASH_BASE, ITEM_STROKE_DASH_COUNT] {
        asm.push(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            style + word,
        ));
    }
    asm.push(abi::load_u64(
        source,
        abi::stack_pointer(),
        OFF_GLYPH_HEADER,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_JOIN_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(records, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(records, "0"));
    asm.push(abi::branch_le(&none));
    asm.push(abi::shift_left_immediate(slots, records, 1));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    asm.push(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], slots));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[1],
        &METAL_MAX_FRAME_EDGES.to_string(),
    ));
    asm.push(abi::branch_le(&copy));
    // Unreachable: `__canvas_metalRenderable` counts two edges per record and declines
    // a frame whose total does not fit. Kept because the alternative is a write past the
    // buffer; dropping the records draws the corners round, not a wrong shape.
    asm.push(abi::branch(&none));

    asm.push(abi::label(&copy));
    asm.push(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        style + ITEM_STROKE_JOIN_BASE,
    ));
    asm.push(abi::store_u32(
        records,
        abi::stack_pointer(),
        style + ITEM_STROKE_JOIN_COUNT,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    // target = contents + METAL_EDGE_BASE_WORDS * 4 + base * 16, through registers for
    // `emit_edge_buffer`'s reason (the offset is past an `add`'s immediate).
    asm.push(abi::shift_left_immediate(target, abi::SCRATCH[0], 4));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_CONTENTS,
    ));
    asm.push(abi::add_registers(target, target, abi::SCRATCH[0]));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &(METAL_EDGE_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(target, target, abi::SCRATCH[0]));
    asm.push(abi::move_immediate(
        residual_target,
        "Integer",
        &((METAL_EDGE_RESIDUAL_BASE_WORDS - METAL_EDGE_BASE_WORDS) * 4).to_string(),
    ));
    asm.push(abi::add_registers(residual_target, residual_target, target));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[0],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[0]));
    // source = header + (slot1 - stops * 5 - dashes - records * 8) * 8: the dash tail
    // (plan-163-B) sits between the records and the stops.
    asm.push(abi::load_double(abi::FP_SCRATCH[1], source, 8));
    asm.push(abi::float_convert_to_signed_x(index, abi::FP_SCRATCH[1]));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_GRADIENT_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[7], "Integer", "5"));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    asm.push(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    asm.push(abi::shift_left_immediate(abi::SCRATCH[0], records, 3));
    asm.push(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_DASH_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    // plan-163-C: and the path clip's edges, five floats each, after the dashes.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[7], "Integer", "5"));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    asm.push(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    asm.push(abi::shift_left_immediate(index, index, 3));
    asm.push(abi::add_registers(source, source, index));
    asm.push(abi::move_immediate(index, "Integer", "0"));

    asm.push(abi::label(&head));
    asm.push(abi::compare_registers(index, slots));
    asm.push(abi::branch_ge(&done));
    for word in 0..4usize {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], source, word * 8));
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[0], target, 0));
        asm.push(abi::add_immediate(target, target, 4));
        asm.push(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[0],
        ));
        asm.push(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
        asm.push(abi::add_immediate(residual_target, residual_target, 4));
    }
    asm.push(abi::add_immediate(source, source, 32));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&head));

    asm.push(abi::label(&none));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in [ITEM_STROKE_JOIN_BASE, ITEM_STROKE_JOIN_COUNT] {
        asm.push(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            style + word,
        ));
    }
    asm.push(abi::label(&done));
}

fn emit_edge_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_edge_head");
    let done = format!("{METAL_DRAW_SYMBOL}_edge_done");
    let empty = format!("{METAL_DRAW_SYMBOL}_edge_empty");
    let convert = format!("{METAL_DRAW_SYMBOL}_edge_convert");
    let band = format!("{METAL_DRAW_SYMBOL}_edge_band");
    let polyline = format!("{METAL_DRAW_SYMBOL}_edge_polyline");
    let has_pieces = format!("{METAL_DRAW_SYMBOL}_edge_pieces");
    let chords = format!("{METAL_DRAW_SYMBOL}_edge_chords");
    let controls_init = format!("{METAL_DRAW_SYMBOL}_edge_controls_init");
    let controls = format!("{METAL_DRAW_SYMBOL}_edge_controls");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];

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
    let has_edges = format!("{METAL_DRAW_SYMBOL}_edge_has");
    asm.push(abi::compare_immediate(abi::SCRATCH[5], GEO_KIND_POLYGON));
    asm.push(abi::branch_eq(&has_edges));
    // plan-154-B: a PolyLine's segments are edges in the polygon layout, one per
    // segment and without a wrap edge, so they take the same upload and band index.
    asm.push(abi::compare_immediate(abi::SCRATCH[5], GEO_KIND_POLYLINE));
    asm.push(abi::branch_eq(&polyline));
    // plan-164-C: a curve shape's pieces take the same region, TWO slots a piece, and
    // the same band index (below).
    asm.push(abi::compare_immediate(abi::SCRATCH[5], GEO_KIND_POLYCURVE));
    asm.push(abi::branch_eq(&has_pieces));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[5],
        GEO_KIND_CURVEPOLYGON,
    ));
    asm.push(abi::branch_ne(&empty));
    asm.push(abi::label(&has_pieces));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_AUX0 * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    asm.push(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        OFF_EDGE_CURSOR,
    ));
    asm.push(abi::add_registers(abi::SCRATCH[6], abi::SCRATCH[5], count));
    asm.push(abi::add_registers(abi::SCRATCH[6], abi::SCRATCH[6], count));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[6],
        &METAL_MAX_FRAME_EDGES.to_string(),
    ));
    asm.push(abi::branch_le(&convert));
    asm.push(abi::branch(&empty));
    asm.push(abi::label(&polyline));
    // Its first and last live segments, as whole numbers over the 16.16 axis
    // `emit_item_block` wrote into `gradient` (`HEADER_POLYLINE_FIRST`). Here rather than
    // in `emit_item_block` because that runs for glyphs too and this does not; a
    // polyline is never a glyph.
    for (index, slot) in [(0usize, HEADER_POLYLINE_FIRST), (1, HEADER_POLYLINE_LAST)] {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        asm.push(abi::float_convert_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(
            abi::SCRATCH[5],
            abi::stack_pointer(),
            OFF_ITEM + ITEM_OFFSET_GRADIENT + index * 4,
        ));
    }
    asm.push(abi::label(&has_edges));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_AUX0 * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    // bug-686: no per-polygon cap. There was one (256 edges) that wrote NO edges for a
    // longer polygon, so it drew nothing; nothing in the transport needs it since
    // plan-116-A moved the edges into this region.
    //
    // Would this polygon's edges fit the frame's region? Unreachable —
    // `__canvas_metalRenderable` declines a frame whose polygons sum past
    // `METAL_MAX_FRAME_EDGES`, so the whole scene went to software. Kept because the
    // alternative to declining is a write past the buffer.
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

    // Not a polygon, or past the frame's region: draw no edges and leave the base at
    // zero. Clamping would render a *different polygon*.
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

    // bug-687: each word's residual goes to the same edge's slot in the residual
    // region, a fixed distance past its 16.16 word (`METAL_EDGE_RESIDUAL_BASE_WORDS`).
    // Through a register for the same reason as the region offset above.
    asm.push(abi::move_immediate(
        residual_target,
        "Integer",
        &((METAL_EDGE_RESIDUAL_BASE_WORDS - METAL_EDGE_BASE_WORDS) * 4).to_string(),
    ));
    asm.push(abi::add_registers(
        residual_target,
        residual_target,
        abi::SCRATCH[6],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(
        residual_scale,
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

    // plan-164-C: a curve shape's pieces (`x0, y0, x1, y1, x2, y2, w1` each, seven
    // doubles) go in as two runs of `count` slots. First each piece's CHORD, `x0, y0, x2,
    // y2` -- an edge in every respect the band index cares about, since a piece is
    // y-monotone and its two ends are its whole y-range, so `emit_band_index` indexes
    // pieces exactly as it indexes edges. Then each piece's control point and weight,
    // `x1, y1, w1, 0`, in the `count` slots after: piece `e`'s is slot `base + count +
    // e`. The weight takes the same 16.16 word and residual a coordinate does.
    //
    // The kind is read again here because every scratch register the dispatch above
    // held it in has been used since.
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_KIND * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[5], GEO_KIND_POLYCURVE));
    asm.push(abi::branch_eq(&chords));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[5],
        GEO_KIND_CURVEPOLYGON,
    ));
    asm.push(abi::branch_ne(&head));

    // One double of the record as its 16.16 word and its residual, each cursor advanced.
    let word = |asm: &mut Asm, slot: usize| {
        asm.push(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
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
        asm.push(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[5],
        ));
        asm.push(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[5], residual_target, 0));
        asm.push(abi::add_immediate(residual_target, residual_target, 4));
    };
    asm.push(abi::label(&chords));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&controls_init));
    for slot in [0usize, 1, 4, 5] {
        word(asm, slot);
    }
    asm.push(abi::add_immediate(source, source, CURVE_PIECE_SLOTS * 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&chords));
    asm.push(abi::label(&controls_init));
    asm.push(abi::add_immediate(source, header, HEADER_SLOTS * 8));
    asm.push(abi::move_immediate(index, "Integer", "0"));
    asm.push(abi::label(&controls));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&band));
    for slot in [2usize, 3] {
        word(asm, slot);
    }
    // The weight as its fp32 BITS, not a 16.16 word: the shader's arithmetic on it is
    // single precision either way, and a weight has no 32767 to stay under.
    asm.push(abi::load_double(abi::FP_SCRATCH[1], source, 6 * 8));
    asm.push(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::float_move_x_from_d(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[5], abi::SCRATCH[6], 0));
    asm.push(abi::move_immediate(abi::SCRATCH[5], "Integer", "0"));
    asm.push(abi::store_u32(abi::SCRATCH[5], abi::SCRATCH[6], 4));
    asm.push(abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 8));
    asm.push(abi::store_u32(abi::SCRATCH[5], residual_target, 0));
    asm.push(abi::store_u32(abi::SCRATCH[5], residual_target, 4));
    asm.push(abi::add_immediate(residual_target, residual_target, 8));
    asm.push(abi::add_immediate(source, source, CURVE_PIECE_SLOTS * 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&controls));

    asm.push(abi::label(&head));
    asm.push(abi::compare_registers(index, count));
    asm.push(abi::branch_ge(&band));
    // out[0..1] = (x0, y0); out[2..3] = (x0 + dx, y0 + dy), each as its 16.16 word and,
    // in the residual region, `round((v * 65536 - word) * 2^31)` (bug-687,
    // `EDGE_RESIDUAL_SCALE`). `v * 65536 - word` is exact: the two are within half a
    // step of each other.
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
        asm.push(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[5],
        ));
        asm.push(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        asm.push(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        asm.push(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u32(abi::SCRATCH[5], residual_target, 0));
        asm.push(abi::add_immediate(residual_target, residual_target, 4));
    }
    asm.push(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    asm.push(abi::add_immediate(index, index, 1));
    asm.push(abi::branch(&head));

    // bug-686: the polygon's edges are in the region; index them by band.
    asm.push(abi::label(&band));
    emit_band_index(asm, ITEM_BANDS);
    asm.push(abi::label(&done));
}

/// Which edge set a band index is built for: the item's own edges, its header in
/// `arcCaps`, or (plan-163-C) a path clip's, its header in `clipBand`. A clip's reach has
/// no stroke in it -- its coverage clamps at `|d| >= 0.5` -- and the transform terms
/// apply as they do to the shape, whose five samples the clip's five mirror.
#[derive(Clone, Copy)]
struct BandSet {
    count: usize,
    base: usize,
    header: usize,
    clip: bool,
    tag: &'static str,
}

const ITEM_BANDS: BandSet = BandSet {
    count: OFF_ITEM + ITEM_OFFSET_MISC + 12,
    base: OFF_ITEM + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    header: OFF_ITEM + ITEM_OFFSET_ARC_CAPS,
    clip: false,
    tag: "band",
};

const CLIP_BANDS: BandSet = BandSet {
    count: OFF_ITEM + ITEM_OFFSET_CLIP_PATH + ITEM_CLIP_PATH_COUNT,
    base: OFF_ITEM + ITEM_OFFSET_CLIP_PATH + ITEM_CLIP_PATH_BASE,
    header: OFF_ITEM + ITEM_OFFSET_CLIP_BAND,
    clip: true,
    tag: "clipband",
};

/// Build this polygon's band index in the frame buffer's band region and write its
/// header into the item block's `arcCaps` (bug-686; layout at `METAL_BAND_BASE_WORDS`) --
/// or a path clip's into `clipBand` (`set`).
///
/// Runs after `emit_edge_buffer` has converted the polygon's edges, and works on those
/// 16.16 words — exactly what the shader reads — rather than on the header's doubles,
/// so the band arithmetic is integer and a rounding difference between the two cannot
/// put an edge in the wrong band.
///
/// ## Why the result is bit-identical to the full loop
///
/// An edge is put in every band its `[ylo − r, yhi + r]` meets, so a band holds (a)
/// every edge whose y-span contains any row of the band — the only edges the even-odd
/// crossing test counts, so `inside` is exact — and (b) every edge within `r` of any
/// point of the band — so `best` is exact whenever the true distance is at most `r`,
/// and otherwise both loops' `best` exceed `r`, where coverage has clamped:
///
/// * **Untransformed**, `d` is the surface distance. A fill clamps to 0 or 255 once
///   `|d| ≥ 0.5`, and a stroke to 0 once `|d| − half ≥ 0.5`, so `r = half + 0.5`, with
///   `half = max(strokeHalf, 0)` (a non-stroking paint reports −1).
/// * **Transformed**, the shader evaluates five shape-space points — the centre `q` and
///   `q ± M·(0.5, 0)`, `q ± M·(0, 0.5)`, M the inverse matrix — all against `q`'s band
///   (`shapeCoverage`). With `F = ‖M‖_F`, each neighbour is within `F/2` of `q`,
///   and the scale `g` the shader divides by is at most `F` (the distance is 1-Lipschitz)
///   or the fallback 1.0. So: if `q` is within `r − F` of the nearest edge, every
///   neighbour's nearest edge is within `r` of `q` and all five values are exact; if not,
///   both loops' `|dRaw| > r − F`, and `r = half + 0.5 + 1.5·F` makes that past the
///   clamp for any `g ≤ F` and for `g = 1`. Crossings are exact at the neighbours too,
///   since `r ≥ F/2`, which also keeps the band's own distance function continuous across
///   the neighbourhood, so its difference quotients stay at most `F` as well. The builder
///   uses `2·F`, the margin rounded up.
///
/// Plus a **1-unit guard** in shape space, for the float error of the shader's band
/// lookup against these integer bands. `band.swift` (bug-686 section G) measured 0
/// differing pixels for fills, strokes, spikes and a self-intersecting star with it.
///
/// ## Sizing
///
/// Bands are 2 px tall untransformed, `√2·F` (about two surface pixels) transformed, and
/// taller when the span needs more than `min(edges, BAND_MAX)` bands. If the list
/// would pass `BAND_LIST_PER_EDGE` entries per edge — a shape whose edges each span
/// most rows, like a star of long spikes — the height doubles until it fits; at one band
/// the list is every edge once. So one polygon never needs more than
/// `BAND_WORDS_PER_EDGE` words per edge, which the region is sized for; the
/// region-full branch is unreachable and kept because the alternative is a write past
/// the buffer. It, a polygon under `BAND_MIN_EDGES`, and a transform whose
/// matrix is not finite write band count 0: the shader loops over every edge. Never a
/// decline, never a truncation.
///
/// No calls; `SCRATCH[0..8]` only (9 is Apple's x18, 10+ the loop state's callee-saved
/// registers). `compare_immediate` is used only against values that fit 12 bits: a
/// wider one borrows x16/x17, which are `SCRATCH[7]`/`SCRATCH[8]`.
fn emit_band_index(asm: &mut Asm, set: BandSet) {
    let tag = set.tag;
    let none = format!("{METAL_DRAW_SYMBOL}_{tag}_none");
    let done = format!("{METAL_DRAW_SYMBOL}_{tag}_done");
    let s = abi::SCRATCH;
    let f = abi::FP_SCRATCH;
    let label = |name: &str| format!("{METAL_DRAW_SYMBOL}_{tag}_{name}");
    let block = |field: usize| set.header + field;

    // --- which polygons: E >= BAND_MIN_EDGES --------------------------------
    asm.push(abi::load_u32(s[2], abi::stack_pointer(), set.count));
    asm.push(abi::move_immediate(
        s[3],
        "Integer",
        &BAND_MIN_EDGES.to_string(),
    ));
    asm.push(abi::compare_registers(s[2], s[3]));
    asm.push(abi::branch_lt(&none));

    // --- the edges: P = contents + EDGE_BASE*4 + base*16, END = P + E*16 ------------
    asm.push(abi::load_u32(s[0], abi::stack_pointer(), set.base));
    asm.push(abi::shift_left_immediate(s[0], s[0], 4));
    asm.push(abi::load_u64(s[1], abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::add_registers(s[0], s[0], s[1]));
    asm.push(abi::move_immediate(
        s[1],
        "Integer",
        &(METAL_EDGE_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(s[0], s[0], s[1]));
    asm.push(abi::store_u64(s[0], abi::stack_pointer(), OFF_BAND_EDGES));
    asm.push(abi::shift_left_immediate(s[1], s[2], 4));
    asm.push(abi::add_registers(s[1], s[0], s[1]));
    asm.push(abi::store_u64(s[1], abi::stack_pointer(), OFF_BAND_END));

    // --- ymin (s2) and ymax (s3) over both endpoints of every edge ----------------
    asm.push(abi::load_u32(s[2], s[0], 4));
    asm.push(abi::sign_extend_word(s[2], s[2]));
    asm.push(abi::move_register(s[3], s[2]));
    asm.push(abi::label(&label("measure_head")));
    asm.push(abi::compare_registers(s[0], s[1]));
    asm.push(abi::branch_ge(&label("measure_done")));
    for (n, offset) in [(0, 4usize), (1, 12)] {
        let not_lower = label(&format!("measure_not_lower_{n}"));
        let not_higher = label(&format!("measure_not_higher_{n}"));
        asm.push(abi::load_u32(s[4], s[0], offset));
        asm.push(abi::sign_extend_word(s[4], s[4]));
        asm.push(abi::compare_registers(s[4], s[2]));
        asm.push(abi::branch_ge(&not_lower));
        asm.push(abi::move_register(s[2], s[4]));
        asm.push(abi::label(&not_lower));
        asm.push(abi::compare_registers(s[4], s[3]));
        asm.push(abi::branch_le(&not_higher));
        asm.push(abi::move_register(s[3], s[4]));
        asm.push(abi::label(&not_higher));
    }
    asm.push(abi::add_immediate(s[0], s[0], 16));
    asm.push(abi::branch(&label("measure_head")));
    asm.push(abi::label(&label("measure_done")));
    asm.push(abi::store_u64(s[2], abi::stack_pointer(), OFF_BAND_YMIN));
    // s3 = the edges' own y extent; s2 stays ymin through the reach computation.
    asm.push(abi::subtract_registers(s[3], s[3], s[2]));

    // --- the reach r (f1) and the minimum band height (f4), in shape units --------
    asm.push(abi::load_u64(s[0], abi::stack_pointer(), OFF_GLYPH_HEADER));
    asm.push(abi::load_double(f[1], s[0], HEADER_STROKE_HALF * 8));
    // plan-163-C: a clip's coverage clamps at `|d| >= 0.5` -- no stroke reaches past it --
    // so its half-width is 0 (and its own cap and join reach do not apply).
    let stroke_reach_done = label("stroke_reach_done");
    if set.clip {
        asm.push(abi::move_immediate(s[4], "Integer", "0"));
        asm.push(abi::signed_convert_to_float_d(f[1], s[4]));
        asm.push(abi::branch(&stroke_reach_done));
    }
    // plan-154-B: a PolyLine is stroke-as-fill, so its half-width is the RADIUS slot and
    // slot 7 is -1. A polygon never writes the radius slot (it stays the blank header's
    // 0), so taking the larger of the two leaves every polygon's reach as it was.
    asm.push(abi::load_double(f[2], s[0], HEADER_RADIUS * 8));
    asm.push(abi::float_max_d(f[1], f[1], f[2]));
    asm.push(abi::move_immediate(s[4], "Integer", "0"));
    asm.push(abi::signed_convert_to_float_d(f[2], s[4]));
    // `fmaxnm`: a NaN half-width reads as 0 rather than poisoning the reach.
    asm.push(abi::float_max_d(f[1], f[1], f[2]));
    // plan-163-B: a Square cap reaches `half * sqrt(2)` past its endpoint.
    let square_reach = label("square_reach");
    for instruction in emit_square_cap_reach(s[0], f[1], s[4], f[2], &square_reach) {
        asm.push(instruction);
    }
    // plan-163-B: and to a join record's reach (0 for an item with none), so a miter's
    // point in a row's band is reached through the edge it sits at the start of.
    asm.push(abi::load_double(f[2], s[0], HEADER_JOIN_REACH * 8));
    asm.push(abi::float_max_d(f[1], f[1], f[2]));
    asm.push(abi::label(&stroke_reach_done));
    asm.push(abi::move_immediate(s[4], "Integer", "3"));
    asm.push(abi::signed_convert_to_float_d(f[2], s[4]));
    asm.push(abi::move_immediate(s[4], "Integer", "2"));
    asm.push(abi::signed_convert_to_float_d(f[3], s[4]));
    asm.push(abi::float_divide_d(f[2], f[2], f[3]));
    // r = half + 0.5 (the clamp) + 1 (the guard)
    asm.push(abi::float_add_d(f[1], f[1], f[2]));
    asm.push(abi::float_move_d_from_d(f[4], f[3]));
    let untransformed = label("untransformed");
    asm.push(abi::load_double(f[5], s[0], HEADER_HAS_TRANSFORM * 8));
    asm.push(abi::float_convert_to_signed_x(s[4], f[5]));
    asm.push(abi::compare_immediate(s[4], "0"));
    asm.push(abi::branch_eq(&untransformed));
    // F = ||M||_F from the four matrix terms, which the header carries as float32 BIT
    // PATTERNS (`__canvas_float32Bits`). This assembler has no single->double convert,
    // so the bits are rebuilt as a double by hand: exponent rebiased by 1023 - 127 =
    // 896, mantissa shifted up 52 - 23 = 29. The sign is dropped (the term is squared);
    // a zero or subnormal term counts as 0, under 1.2e-38 short of its true size, which
    // the 1-unit guard dwarfs; an infinite or NaN one gives the polygon no bands.
    asm.push(abi::move_immediate(s[4], "Integer", "0"));
    asm.push(abi::signed_convert_to_float_d(f[6], s[4]));
    for slot in [
        HEADER_TRANSFORM_IA,
        HEADER_TRANSFORM_IB,
        HEADER_TRANSFORM_IC,
        HEADER_TRANSFORM_ID,
    ] {
        let zero_term = label(&format!("zero_term_{slot}"));
        asm.push(abi::load_double(f[7], s[0], slot * 8));
        asm.push(abi::float_convert_to_signed_x(s[4], f[7]));
        asm.push(abi::shift_right_immediate(s[5], s[4], 23));
        asm.push(abi::move_immediate(s[6], "Integer", "255"));
        asm.push(abi::and_registers(s[5], s[5], s[6]));
        asm.push(abi::compare_registers(s[5], s[6]));
        asm.push(abi::branch_eq(&none));
        asm.push(abi::compare_immediate(s[5], "0"));
        asm.push(abi::branch_eq(&zero_term));
        asm.push(abi::add_immediate(s[5], s[5], 896));
        asm.push(abi::shift_left_immediate(s[5], s[5], 52));
        asm.push(abi::move_immediate(s[6], "Integer", "8388607"));
        asm.push(abi::and_registers(s[7], s[4], s[6]));
        asm.push(abi::shift_left_immediate(s[7], s[7], 29));
        asm.push(abi::or_registers(s[5], s[5], s[7]));
        asm.push(abi::float_move_d_from_x(f[7], s[5]));
        asm.push(abi::float_multiply_d(f[7], f[7], f[7]));
        asm.push(abi::float_add_d(f[6], f[6], f[7]));
        asm.push(abi::label(&zero_term));
    }
    asm.push(abi::float_sqrt_d(f[6], f[6]));
    // r += 2F; the minimum height is sqrt(2)*F, about two surface pixels.
    asm.push(abi::float_add_d(f[7], f[6], f[6]));
    asm.push(abi::float_add_d(f[1], f[1], f[7]));
    asm.push(abi::float_sqrt_d(f[7], f[3]));
    asm.push(abi::float_multiply_d(f[4], f[6], f[7]));
    asm.push(abi::label(&untransformed));

    // --- to 16.16, rounding UP: s4 = r, s5 = the minimum height ---------------------
    asm.push(abi::move_immediate(s[4], "Integer", FIXED_POINT_SCALE));
    asm.push(abi::signed_convert_to_float_d(f[7], s[4]));
    asm.push(abi::float_multiply_d(f[1], f[1], f[7]));
    asm.push(abi::float_ceil_to_signed_x(s[4], f[1]));
    asm.push(abi::float_multiply_d(f[4], f[4], f[7]));
    asm.push(abi::float_ceil_to_signed_x(s[5], f[4]));
    // A reach or height past 2^30 (16,384 units) is a degenerate transform; `fcvtps`
    // saturates an infinity to i64::MAX, which this catches too.
    asm.push(abi::move_immediate(
        s[6],
        "Integer",
        &(1u64 << 30).to_string(),
    ));
    asm.push(abi::compare_registers(s[4], s[6]));
    asm.push(abi::branch_gt(&none));
    asm.push(abi::compare_registers(s[5], s[6]));
    asm.push(abi::branch_gt(&none));
    let height_positive = label("height_positive");
    asm.push(abi::compare_immediate(s[5], "1"));
    asm.push(abi::branch_ge(&height_positive));
    asm.push(abi::move_immediate(s[5], "Integer", "1"));
    asm.push(abi::label(&height_positive));

    // --- span = extent + 2r (s3); top = ymin - r must be an int32, and so must the
    // bottom, because the shader reads `top` as a 16.16 `int` ----------------------
    asm.push(abi::shift_left_immediate(s[4], s[4], 1));
    asm.push(abi::store_u64(s[4], abi::stack_pointer(), OFF_BAND_REACH));
    asm.push(abi::add_registers(s[3], s[3], s[4]));
    asm.push(abi::shift_right_immediate(s[6], s[4], 1));
    asm.push(abi::subtract_registers(s[7], s[2], s[6]));
    // The encoder parses an immediate as u64, so i32::MIN goes in as its 64-bit two's
    // complement.
    asm.push(abi::move_immediate(
        s[8],
        "Integer",
        &(i64::from(i32::MIN) as u64).to_string(),
    ));
    asm.push(abi::compare_registers(s[7], s[8]));
    asm.push(abi::branch_lt(&none));
    asm.push(abi::add_registers(s[8], s[7], s[3]));
    asm.push(abi::move_immediate(s[6], "Integer", &i32::MAX.to_string()));
    asm.push(abi::compare_registers(s[8], s[6]));
    asm.push(abi::branch_gt(&none));

    // --- height = max(minimum, ceil(span / (min(E, BAND_MAX) - 1))) -----------
    // At most `min(E, BAND_MAX)` bands: span / height <= that - 1.
    let bands_capped = label("bands_capped");
    asm.push(abi::load_u32(s[6], abi::stack_pointer(), set.count));
    asm.push(abi::move_immediate(s[7], "Integer", &BAND_MAX.to_string()));
    asm.push(abi::compare_registers(s[6], s[7]));
    asm.push(abi::branch_le(&bands_capped));
    asm.push(abi::move_register(s[6], s[7]));
    asm.push(abi::label(&bands_capped));
    const _: () = assert!(BAND_MIN_EDGES >= 2 && BAND_MAX >= 2);
    asm.push(abi::subtract_immediate(s[6], s[6], 1));
    asm.push(abi::add_registers(s[7], s[3], s[6]));
    asm.push(abi::subtract_immediate(s[7], s[7], 1));
    asm.push(abi::unsigned_divide_registers(s[7], s[7], s[6]));
    let height_chosen = label("height_chosen");
    asm.push(abi::compare_registers(s[7], s[5]));
    asm.push(abi::branch_ge(&height_chosen));
    asm.push(abi::move_register(s[7], s[5]));
    asm.push(abi::label(&height_chosen));
    asm.push(abi::store_u64(s[7], abi::stack_pointer(), OFF_BAND_HEIGHT));

    // --- size the list; double the height until it is within its budget -----------
    // s3 (span) is live through this loop.
    asm.push(abi::label(&label("fit_head")));
    asm.push(abi::load_u64(s[4], abi::stack_pointer(), OFF_BAND_HEIGHT));
    asm.push(abi::unsigned_divide_registers(s[6], s[3], s[4]));
    asm.push(abi::add_immediate(s[6], s[6], 1));
    asm.push(abi::store_u64(s[6], abi::stack_pointer(), OFF_BAND_COUNT));
    asm.push(abi::move_immediate(s[2], "Integer", "0"));
    asm.push(abi::load_u64(s[0], abi::stack_pointer(), OFF_BAND_EDGES));
    asm.push(abi::load_u64(s[1], abi::stack_pointer(), OFF_BAND_END));
    asm.push(abi::label(&label("size_head")));
    asm.push(abi::compare_registers(s[0], s[1]));
    asm.push(abi::branch_ge(&label("size_done")));
    emit_band_range(asm, tag, "size");
    asm.push(abi::subtract_registers(s[5], s[5], s[4]));
    asm.push(abi::add_immediate(s[5], s[5], 1));
    asm.push(abi::add_registers(s[2], s[2], s[5]));
    asm.push(abi::add_immediate(s[0], s[0], 16));
    asm.push(abi::branch(&label("size_head")));
    asm.push(abi::label(&label("size_done")));
    asm.push(abi::load_u32(s[4], abi::stack_pointer(), set.count));
    asm.push(abi::move_immediate(
        s[5],
        "Integer",
        &BAND_LIST_PER_EDGE.to_string(),
    ));
    asm.push(abi::multiply_registers(s[4], s[4], s[5]));
    asm.push(abi::compare_registers(s[2], s[4]));
    asm.push(abi::branch_le(&label("fits")));
    asm.push(abi::load_u64(s[4], abi::stack_pointer(), OFF_BAND_HEIGHT));
    asm.push(abi::shift_left_immediate(s[4], s[4], 1));
    asm.push(abi::store_u64(s[4], abi::stack_pointer(), OFF_BAND_HEIGHT));
    asm.push(abi::branch(&label("fit_head")));
    asm.push(abi::label(&label("fits")));

    // --- claim 2*bands + list words of the region ---------------------------------
    asm.push(abi::load_u64(s[6], abi::stack_pointer(), OFF_BAND_COUNT));
    asm.push(abi::shift_left_immediate(s[4], s[6], 1));
    asm.push(abi::add_registers(s[4], s[4], s[2]));
    asm.push(abi::load_u64(s[5], abi::stack_pointer(), OFF_BAND_CURSOR));
    asm.push(abi::add_registers(s[4], s[4], s[5]));
    asm.push(abi::move_immediate(
        s[7],
        "Integer",
        &METAL_MAX_FRAME_BAND_WORDS.to_string(),
    ));
    asm.push(abi::compare_registers(s[4], s[7]));
    asm.push(abi::branch_gt(&none));
    asm.push(abi::store_u64(s[4], abi::stack_pointer(), OFF_BAND_CURSOR));

    // --- the header: top, height, count, table offset -----------------------------
    asm.push(abi::store_u32(
        s[5],
        abi::stack_pointer(),
        block(ITEM_BAND_START),
    ));
    asm.push(abi::store_u32(
        s[6],
        abi::stack_pointer(),
        block(ITEM_BAND_COUNT),
    ));
    asm.push(abi::load_u64(s[4], abi::stack_pointer(), OFF_BAND_HEIGHT));
    asm.push(abi::store_u32(
        s[4],
        abi::stack_pointer(),
        block(ITEM_BAND_HEIGHT),
    ));
    asm.push(abi::load_u64(s[4], abi::stack_pointer(), OFF_BAND_REACH));
    asm.push(abi::shift_right_immediate(s[4], s[4], 1));
    asm.push(abi::load_u64(s[7], abi::stack_pointer(), OFF_BAND_YMIN));
    asm.push(abi::subtract_registers(s[7], s[7], s[4]));
    asm.push(abi::store_u32(
        s[7],
        abi::stack_pointer(),
        block(ITEM_BAND_TOP),
    ));

    // --- the table's address: contents + BAND_BASE*4 + start*4 ---------------------
    asm.push(abi::shift_left_immediate(s[5], s[5], 2));
    asm.push(abi::load_u64(s[4], abi::stack_pointer(), OFF_CONTENTS));
    asm.push(abi::add_registers(s[5], s[5], s[4]));
    asm.push(abi::move_immediate(
        s[4],
        "Integer",
        &(METAL_BAND_BASE_WORDS * 4).to_string(),
    ));
    asm.push(abi::add_registers(s[5], s[5], s[4]));
    asm.push(abi::store_u64(s[5], abi::stack_pointer(), OFF_BAND_TABLE));

    // --- counts to zero: table[2k + 1] = 0 (s5 table, s6 bands) --------------------
    asm.push(abi::move_immediate(s[4], "Integer", "0"));
    asm.push(abi::move_immediate(s[7], "Integer", "0"));
    asm.push(abi::label(&label("zero_head")));
    asm.push(abi::compare_registers(s[4], s[6]));
    asm.push(abi::branch_ge(&label("zero_done")));
    asm.push(abi::shift_left_immediate(s[8], s[4], 3));
    asm.push(abi::add_registers(s[8], s[8], s[5]));
    asm.push(abi::store_u32(s[7], s[8], 4));
    asm.push(abi::add_immediate(s[4], s[4], 1));
    asm.push(abi::branch(&label("zero_head")));
    asm.push(abi::label(&label("zero_done")));

    // --- count each band's edges -----------------------------------------------------
    asm.push(abi::load_u64(s[0], abi::stack_pointer(), OFF_BAND_EDGES));
    asm.push(abi::load_u64(s[1], abi::stack_pointer(), OFF_BAND_END));
    asm.push(abi::load_u64(s[2], abi::stack_pointer(), OFF_BAND_TABLE));
    asm.push(abi::label(&label("count_head")));
    asm.push(abi::compare_registers(s[0], s[1]));
    asm.push(abi::branch_ge(&label("count_done")));
    emit_band_range(asm, tag, "count");
    asm.push(abi::label(&label("count_band")));
    asm.push(abi::compare_registers(s[4], s[5]));
    asm.push(abi::branch_gt(&label("count_next")));
    asm.push(abi::shift_left_immediate(s[6], s[4], 3));
    asm.push(abi::add_registers(s[6], s[6], s[2]));
    asm.push(abi::load_u32(s[7], s[6], 4));
    asm.push(abi::add_immediate(s[7], s[7], 1));
    asm.push(abi::store_u32(s[7], s[6], 4));
    asm.push(abi::add_immediate(s[4], s[4], 1));
    asm.push(abi::branch(&label("count_band")));
    asm.push(abi::label(&label("count_next")));
    asm.push(abi::add_immediate(s[0], s[0], 16));
    asm.push(abi::branch(&label("count_head")));
    asm.push(abi::label(&label("count_done")));

    // --- each band's first list entry: table[2k] = sum of the counts before it -----
    asm.push(abi::load_u64(s[6], abi::stack_pointer(), OFF_BAND_COUNT));
    asm.push(abi::move_immediate(s[4], "Integer", "0"));
    asm.push(abi::move_immediate(s[7], "Integer", "0"));
    asm.push(abi::label(&label("start_head")));
    asm.push(abi::compare_registers(s[4], s[6]));
    asm.push(abi::branch_ge(&label("start_done")));
    asm.push(abi::shift_left_immediate(s[8], s[4], 3));
    asm.push(abi::add_registers(s[8], s[8], s[2]));
    asm.push(abi::store_u32(s[7], s[8], 0));
    asm.push(abi::load_u32(s[5], s[8], 4));
    asm.push(abi::add_registers(s[7], s[7], s[5]));
    asm.push(abi::add_immediate(s[4], s[4], 1));
    asm.push(abi::branch(&label("start_head")));
    asm.push(abi::label(&label("start_done")));

    // --- the list: each edge's number into each of its bands, using table[2k] as the
    // band's fill cursor (s8 = the list, s3 = the edge number) ------------------------
    asm.push(abi::shift_left_immediate(s[8], s[6], 3));
    asm.push(abi::add_registers(s[8], s[8], s[2]));
    asm.push(abi::load_u64(s[0], abi::stack_pointer(), OFF_BAND_EDGES));
    asm.push(abi::load_u64(s[1], abi::stack_pointer(), OFF_BAND_END));
    asm.push(abi::move_immediate(s[3], "Integer", "0"));
    asm.push(abi::label(&label("fill_head")));
    asm.push(abi::compare_registers(s[0], s[1]));
    asm.push(abi::branch_ge(&label("fill_done")));
    emit_band_range(asm, tag, "fill");
    asm.push(abi::label(&label("fill_band")));
    asm.push(abi::compare_registers(s[4], s[5]));
    asm.push(abi::branch_gt(&label("fill_next")));
    asm.push(abi::shift_left_immediate(s[6], s[4], 3));
    asm.push(abi::add_registers(s[6], s[6], s[2]));
    asm.push(abi::load_u32(s[7], s[6], 0));
    asm.push(abi::add_immediate(s[7], s[7], 1));
    asm.push(abi::store_u32(s[7], s[6], 0));
    asm.push(abi::subtract_immediate(s[7], s[7], 1));
    asm.push(abi::shift_left_immediate(s[7], s[7], 2));
    asm.push(abi::add_registers(s[7], s[7], s[8]));
    asm.push(abi::store_u32(s[3], s[7], 0));
    asm.push(abi::add_immediate(s[4], s[4], 1));
    asm.push(abi::branch(&label("fill_band")));
    asm.push(abi::label(&label("fill_next")));
    asm.push(abi::add_immediate(s[0], s[0], 16));
    asm.push(abi::add_immediate(s[3], s[3], 1));
    asm.push(abi::branch(&label("fill_head")));
    asm.push(abi::label(&label("fill_done")));

    // --- the fill cursors ran to each band's end; take the counts back off ---------
    asm.push(abi::load_u64(s[6], abi::stack_pointer(), OFF_BAND_COUNT));
    asm.push(abi::move_immediate(s[4], "Integer", "0"));
    asm.push(abi::label(&label("rewind_head")));
    asm.push(abi::compare_registers(s[4], s[6]));
    asm.push(abi::branch_ge(&done));
    asm.push(abi::shift_left_immediate(s[8], s[4], 3));
    asm.push(abi::add_registers(s[8], s[8], s[2]));
    asm.push(abi::load_u32(s[5], s[8], 0));
    asm.push(abi::load_u32(s[7], s[8], 4));
    asm.push(abi::subtract_registers(s[5], s[5], s[7]));
    asm.push(abi::store_u32(s[5], s[8], 0));
    asm.push(abi::add_immediate(s[4], s[4], 1));
    asm.push(abi::branch(&label("rewind_head")));

    // --- no bands: the shader loops over every edge -------------------------------
    asm.push(abi::label(&none));
    asm.push(abi::move_immediate(s[4], "Integer", "0"));
    for field in [
        ITEM_BAND_TOP,
        ITEM_BAND_HEIGHT,
        ITEM_BAND_COUNT,
        ITEM_BAND_START,
    ] {
        asm.push(abi::store_u32(s[4], abi::stack_pointer(), block(field)));
    }
    asm.push(abi::label(&done));
}

/// The bands the 16.16 edge at `SCRATCH[0]` goes in: first in `SCRATCH[4]`, last in
/// `SCRATCH[5]` (inclusive), from the working state `emit_band_index` parked. Uses
/// `SCRATCH[6]` as a temporary.
///
/// With `top = ymin − r`: first = ⌊(ylo − r − top) / h⌋ = ⌊(ylo − ymin) / h⌋ and last =
/// ⌊(yhi + r − top) / h⌋ = ⌊(yhi − ymin + 2r) / h⌋. Both numerators are non-negative
/// (every y is at least ymin), so the unsigned divide is exact, and the last is at most
/// ⌊span / h⌋ = bands − 1.
fn emit_band_range(asm: &mut Asm, tag: &str, site: &str) {
    let s = abi::SCRATCH;
    let ordered = format!("{METAL_DRAW_SYMBOL}_{tag}_{site}_ordered");
    asm.push(abi::load_u32(s[4], s[0], 4));
    asm.push(abi::sign_extend_word(s[4], s[4]));
    asm.push(abi::load_u32(s[5], s[0], 12));
    asm.push(abi::sign_extend_word(s[5], s[5]));
    asm.push(abi::compare_registers(s[4], s[5]));
    asm.push(abi::branch_le(&ordered));
    asm.push(abi::move_register(s[6], s[4]));
    asm.push(abi::move_register(s[4], s[5]));
    asm.push(abi::move_register(s[5], s[6]));
    asm.push(abi::label(&ordered));
    asm.push(abi::load_u64(s[6], abi::stack_pointer(), OFF_BAND_YMIN));
    asm.push(abi::subtract_registers(s[4], s[4], s[6]));
    asm.push(abi::subtract_registers(s[5], s[5], s[6]));
    asm.push(abi::load_u64(s[6], abi::stack_pointer(), OFF_BAND_REACH));
    asm.push(abi::add_registers(s[5], s[5], s[6]));
    asm.push(abi::load_u64(s[6], abi::stack_pointer(), OFF_BAND_HEIGHT));
    asm.push(abi::unsigned_divide_registers(s[4], s[4], s[6]));
    asm.push(abi::unsigned_divide_registers(s[5], s[5], s[6]));
}

/// The C strings this module's sends need, for the reconcile data-object list.
pub(super) fn metal_data_objects() -> Vec<(&'static str, &'static str)> {
    let shader: &'static str = METAL_SHADER_SOURCE.as_str();
    vec![
        (STR_METAL_SHADER, shader),
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
        SEL_SET_ALLOW_GPU_OPTIMIZED_CONTENTS,
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
        SEL_STATUS,
        SEL_GET_BYTES,
        SEL_SET_FRAGMENT_BYTES,
        SEL_SET_VERTEX_BYTES,
        SEL_DRAW_PRIMITIVES_INSTANCED,
        SEL_NEW_BUFFER,
        SEL_CONTENTS,
        SEL_SET_VERTEX_BUFFER,
        SEL_SET_FRAGMENT_BUFFER,
        // bug-686 Phase 4: the direct present and its main-thread layer bookkeeping.
        SEL_NEXT_DRAWABLE,
        SEL_TEXTURE,
        SEL_BLIT_COMMAND_ENCODER,
        SEL_COPY_TEXTURE,
        SEL_PRESENT,
        SEL_SET_DEVICE,
        SEL_SET_FRAMEBUFFER_ONLY,
        SEL_SET_DRAWABLE_SIZE,
        SEL_SET_HIDDEN,
        SEL_SET_OPAQUE,
        SEL_SET_COLORSPACE,
        SEL_SCREEN,
        SEL_COLOR_SPACE,
        SEL_CG_COLOR_SPACE,
        SEL_ADD_SUBLAYER,
        SEL_SET_LAYER_FRAME,
        SEL_BEGIN,
        SEL_SET_DISABLE_ACTIONS,
        SEL_MFB_METAL_LAYER,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::operand::Operand;
    use crate::codegen::runtime::canvas::METAL_PICTURE_TABLE_BYTES;

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

    /// The index of the instruction that loads `selector`'s address for a send.
    fn selector_load(func: &CodeFunction, selector: &str) -> usize {
        let loads: Vec<usize> = func
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| i.op == CodeOp::Adrp && i.get("symbol").as_deref() == Some(selector))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(loads.len(), 1, "{selector} is sent once, got {loads:?}");
        loads[0]
    }

    /// bug-1024: the command buffer's `status` is read after the wait and before the
    /// frame is used, on BOTH ways out — the readback and the direct present.
    ///
    /// `waitUntilCompleted` returns when the buffer has finished, not when it has
    /// succeeded. A buffer that ended in `MTLCommandBufferStatusError` leaves its
    /// tile-based target written in part or not at all, so reading it back, or
    /// presenting it, shows a frame the GPU did not draw and reports it as drawn.
    #[test]
    fn the_buffer_status_is_read_after_the_wait_and_before_the_frame_is_used() {
        let func = emit_metal_draw();
        let wait = selector_load(&func, SEL_WAIT_UNTIL_COMPLETED.0);
        let status = selector_load(&func, SEL_STATUS.0);
        let read_back = selector_load(&func, SEL_GET_BYTES.0);
        let present = selector_load(&func, SEL_PRESENT.0);
        assert!(
            wait < status && status < read_back && status < present,
            "the order must be waitUntilCompleted ({wait}), status ({status}), then \
             getBytes ({read_back}) or present ({present})"
        );
        assert!(
            metal_data_objects().contains(&SEL_STATUS),
            "the `status` selector's name must be among the module's data objects"
        );
    }

    /// A buffer that did not complete makes the submit answer 0: the branch that
    /// follows the `status` send leaves for the pool release when the answer is not
    /// `MTLCommandBufferStatusCompleted`, and nothing has stored "presented" by then.
    ///
    /// That is the answer `direct_failed` gives, by the same means: the answer is the
    /// `OFF_PRESENTED` slot, zeroed at entry and set to 1 only on the two paths below
    /// this branch. So the caller declines the frame and software draws it.
    #[test]
    fn a_buffer_that_did_not_complete_reports_the_frame_not_drawn() {
        let func = emit_metal_draw();
        let ins = &func.instructions;
        let status = selector_load(&func, SEL_STATUS.0);
        let send = (status..ins.len())
            .find(|at| ins[*at].get("target").as_deref() == Some("_objc_msgSend"))
            .expect("the status selector is sent");
        let answer = render_field("lhs", abi::c_return(0));
        assert_eq!(ins[send + 1].op, CodeOp::CmpImm, "the status is compared");
        assert_eq!(ins[send + 1].get("lhs"), Some(answer));
        assert_eq!(
            ins[send + 1].get("rhs"),
            Some(MTL_COMMAND_BUFFER_STATUS_COMPLETED.to_string()),
            "the status is compared with MTLCommandBufferStatusCompleted"
        );
        assert_eq!(ins[send + 2].op, CodeOp::BranchNe, "and branched on");
        assert_eq!(
            ins[send + 2].get("target"),
            Some(format!("{METAL_DRAW_SYMBOL}_release_pool")),
            "a buffer that did not complete skips the readback and the present"
        );

        // "Presented" is stored as 1 only after that branch, and as 0 only before it.
        let presented = OFF_PRESENTED.to_string();
        let stores: Vec<usize> = (0..ins.len())
            .filter(|at| {
                ins[*at].op == CodeOp::StrU64
                    && ins[*at].get("offset").as_deref() == Some(presented.as_str())
                    && ins[*at].get("base") == Some(render_field("base", abi::stack_pointer()))
            })
            .collect();
        let after: Vec<usize> = stores.iter().copied().filter(|at| *at > send).collect();
        assert_eq!(
            (stores.len(), after.len()),
            (3, 2),
            "one store clears the answer at entry and two set it, one for the present \
             and one for the readback, both after the status branch: {stores:?}"
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
            (OFF_EDGE_CURSOR, 8, "edgeCursor"),
            (OFF_GRAD_CURSOR, 8, "gradCursor"),
            (OFF_GLYPH_INSTANCE, 8, "glyphInstance"),
            (OFF_BOUND_MODE, 8, "boundMode"),
            (OFF_ITEM_MODE, 8, "itemMode"),
            (OFF_SAVED_STROKE, 8, "savedStroke"),
            (OFF_DRAWS, 8, "draws"),
            (OFF_DRAW_ENTRIES, 8, "drawEntries"),
            (OFF_DRAW_INDEX, 8, "drawIndex"),
            (OFF_DRAW_BASE, 8, "drawBase"),
            (OFF_DRAW_COUNT, 8, "drawCount"),
            (OFF_DRAW_PAIR, 32, "drawPair"),
            (OFF_GLYPH_CURSOR, 8, "glyphCursor"),
            (OFF_DASH_CURSOR, 8, "dashCursor"),
            (OFF_PIC_COUNT, 8, "picCount"),
            (OFF_PIC_SLOT, 8, "picSlot"),
            (OFF_BAND_CURSOR, 8, "bandCursor"),
            (OFF_BAND_EDGES, 8, "bandEdges"),
            (OFF_BAND_END, 8, "bandEnd"),
            (OFF_BAND_YMIN, 8, "bandYmin"),
            (OFF_BAND_REACH, 8, "bandReach"),
            (OFF_BAND_HEIGHT, 8, "bandHeight"),
            (OFF_BAND_COUNT, 8, "bandCount"),
            (OFF_BAND_TABLE, 8, "bandTable"),
            (OFF_DRAWABLE, 8, "drawable"),
            (OFF_PRESENTED, 8, "presented"),
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

    /// The draw offset is bound where the shader declares it, in both stages.
    ///
    /// The emitter sends `setVertexBytes:...atIndex:` and `setFragmentBytes:...atIndex:`
    /// with two *different* indices, because the fragment stage has items and edges at
    /// 0 and 1 (index 2, the old per-glyph bitmap, is unused since bug-670) while the
    /// vertex stage has only items at 0.
    /// The shader is a string the Rust compiler never parses, so nothing else checks
    /// that the two agree — and a mismatch does not fault: the stage reads whatever was
    /// bound at that index, which is a wrong picture reported as success.
    #[test]
    fn the_metal_shader_binds_the_draw_offset_where_the_emitter_sends_it() {
        for (index, stage) in [
            (METAL_DRAW_OFFSET_VERTEX_INDEX, "vertex"),
            (METAL_DRAW_OFFSET_FRAGMENT_INDEX, "fragment"),
        ] {
            // `MfbDraw` since plan-172-H: the offset's two 16.16 words and their residual
            // words (an `int4` since bug-735), then the inverse linear part.
            let declaration = format!("constant MfbDraw &draw [[buffer({index})]]");
            assert!(
                METAL_SHADER_SOURCE.contains(&declaration),
                "the emitter sends the draw offset to buffer index {index} for the \
                 {stage} stage, but METAL_SHADER_SOURCE has no `{declaration}`",
            );
        }
        // And exactly one declaration per stage: a second one at another index would
        // satisfy the loop above while the emitter still fed only one of them.
        assert_eq!(
            METAL_SHADER_SOURCE
                .matches("MfbDraw &draw [[buffer(")
                .count(),
            2,
            "expected exactly one draw-offset binding in each of the two stages",
        );
    }

    /// The fragment stage evaluates everything — the clip included — in shape space.
    ///
    /// `p` is the surface point with the group offset removed; `in.pos.xy` is the
    /// surface point itself. Since plan-163-C the clip is in the item's own coordinates
    /// and travels with its group (reversing plan-116-H §4.2's surface clip, which this
    /// test pinned until then), so it comes out of `shapeCoverage` with the distance —
    /// and the distance, gradient and glyph lookups must all use `p`, because each is
    /// evaluated per fragment and would otherwise be sampled at the wrong point inside a
    /// correctly positioned quad.
    #[test]
    fn the_metal_fragment_clips_and_samples_in_shape_space() {
        let body = METAL_SHADER_SOURCE
            .split("fragment float4 mfbFragment")
            .nth(1)
            .expect("the fragment entry point is in the shader");
        assert!(
            body.contains("int clipCov = cov.w;"),
            "the clip must come out of `shapeCoverage`, at the SHAPE-space point — \
             evaluating it at the surface point leaves it behind when the group moves",
        );
        // plan-163-D: the ramp is in the item's own space, so it takes `q`, the
        // inverse-mapped twin of `p` -- still the shape-space point, never `in.pos.xy`.
        for (call, what) in [
            (
                "float2 q = hasTransform(item) ? inverseMap(item, p) : p;",
                "the ramp's point",
            ),
            ("gradientColour(q, edges, item)", "the gradient ramp"),
            ("inverseMap(item, p) : p", "the glyph sample"),
        ] {
            assert!(
                body.contains(call),
                "{what} must be evaluated at the SHAPE-space point `p`; sampling it at \
                 `in.pos.xy` draws a correctly placed shape with the wrong contents",
            );
        }
        // bug-687 (polygons), bug-735 (every kind): the shape distance is taken at the
        // double-float twin of `p`, which has to be built from the surface point itself
        // (`p` has already rounded to fp32). It is the third and last legitimate mention,
        // and it is held to the same rule: the group offset -- word and residual -- comes
        // off.
        assert!(
            body.contains("dfShapePoint(in.pos.xy, draw.offset)"),
            "the shape's double-float point must be the surface point minus the draw \
             offset, like `p`",
        );
        assert!(
            METAL_SHADER_SOURCE.contains(
                "return float4(dfSub(float2(surface.x, 0.0), dfFixed(offset.x, offset.z)), \
                 dfSub(float2(surface.y, 0.0), dfFixed(offset.y, offset.w)));"
            ),
            "`dfShapePoint` must subtract the draw offset from the surface point",
        );
        // plan-164-C: the curve shapes are drawn from `curveWalk`, which takes the
        // double-float shape point `sampleDistance` was given and a piece's coordinates
        // RELATIVE to it (`curvePiece`). The generated block is spliced in ahead of the
        // fragment entry point, so it is checked on its own: nothing in it may name the
        // surface position.
        let curves = curve_shader(ShaderLanguage::Msl);
        assert!(
            !curves.contains("in.pos") && !curves.contains("gl_FragCoord"),
            "a curve function reads the surface position; it must take the shape-space \
             point it is handed",
        );
        assert!(
            curves.contains("k.p0 = float2(dfF(dfSub(curveCoord(item, edges, s, 0), x)), dfF(dfSub(curveCoord(item, edges, s, 1), y)));"),
            "a piece's points must be taken relative to the shape-space point, in \
             double-float, before anything is done in single precision",
        );
        for call in [
            "curveWalk(item, edges, band, x, y, kind == 12 ? 0 : (item.strokeStyle.w > 0 ? 3 : 1),",
            "curveWalk(item, edges, band, cqx, cqy, dashed ? 3 : 2, dfF(half2), item.arc.w)",
        ] {
            assert!(
                METAL_SHADER_SOURCE.contains(call),
                "a curve shape's distance must be asked for at the shape-space point: {call}",
            );
        }
        assert_eq!(
            body.matches("in.pos.xy").count(),
            2,
            "only `p`'s own definition and its double-float twin may mention \
             `in.pos.xy` — another use is a consumer that was not moved into shape space",
        );
    }

    /// The MSL's `METAL_EDGE_BASE` is `METAL_EDGE_BASE_WORDS`.
    ///
    /// Since bug-686 the bases are formatted into `METAL_SHADER_SOURCE` from the layout
    /// constants, so they cannot be spelled twice; this reads the formatted text back,
    /// which is what still catches a base generated from the WRONG constant, and it
    /// checks the region chain the constants form. A disagreement would not fail
    /// anywhere: every polygon
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
        //
        // bug-686 put the band index between the edges and the gradient stops.
        assert!(
            METAL_SHADER_SOURCE.contains(&format!(
                "constant int METAL_BAND_BASE = {METAL_BAND_BASE_WORDS};"
            )),
            "the MSL declares a band-region base that is not METAL_BAND_BASE_WORDS \
             ({METAL_BAND_BASE_WORDS}); every banded polygon would read another region \
             as its edge lists"
        );
        // bug-687 put the edge residuals between the edges and the band index.
        assert!(
            METAL_SHADER_SOURCE.contains(&format!(
                "constant int METAL_EDGE_RESIDUAL_BASE = {METAL_EDGE_RESIDUAL_BASE_WORDS};"
            )),
            "the MSL declares an edge-residual base that is not \
             METAL_EDGE_RESIDUAL_BASE_WORDS ({METAL_EDGE_RESIDUAL_BASE_WORDS}); every \
             polygon vertex would be corrected by another region's words"
        );
        assert_eq!(
            METAL_EDGE_RESIDUAL_BASE_WORDS * 4,
            METAL_EDGE_BASE_WORDS * 4 + METAL_MAX_FRAME_EDGES * 16,
            "the edge-residual region must start where the edge region ends"
        );
        assert_eq!(
            METAL_BAND_BASE_WORDS * 4,
            METAL_EDGE_RESIDUAL_BASE_WORDS * 4 + METAL_MAX_FRAME_EDGES * 16,
            "the band region must start where the edge-residual region ends"
        );
        assert_eq!(
            METAL_GRADIENT_BASE_WORDS * 4,
            METAL_BAND_BASE_WORDS * 4 + METAL_MAX_FRAME_BAND_WORDS * 4,
            "the gradient region must start where the band region ends"
        );
        assert!(
            METAL_SHADER_SOURCE.contains(&format!(
                "constant int METAL_GLYPH_BASE = {METAL_GLYPH_BASE_WORDS};"
            )),
            "the MSL declares a glyph-region base that is not METAL_GLYPH_BASE_WORDS \
             ({METAL_GLYPH_BASE_WORDS}); every glyph would read its coverage from the \
             wrong offset of a buffer that is entirely valid memory"
        );
        // plan-163-B put the dash words between the gradient stops and the glyphs.
        assert!(
            METAL_SHADER_SOURCE.contains(&format!(
                "constant int METAL_DASH_BASE = {METAL_DASH_BASE_WORDS};"
            )),
            "the MSL declares a dash-region base that is not METAL_DASH_BASE_WORDS \
             ({METAL_DASH_BASE_WORDS}); every dashed stroke would read its pattern from \
             another region"
        );
        assert_eq!(
            METAL_DASH_BASE_WORDS * 4,
            METAL_GRADIENT_BASE_WORDS * 4
                + METAL_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4,
            "the dash region must start where the gradient region ends"
        );
        // bug-670 added the glyph region; the chain grows by one link.
        assert_eq!(
            METAL_GLYPH_BASE_WORDS * 4,
            METAL_DASH_BASE_WORDS * 4 + METAL_MAX_FRAME_DASH_WORDS * 4,
            "the glyph region must start where the dash region ends"
        );
        // bug-686 added the fifth, the picture table, which the shader never reads.
        assert_eq!(
            METAL_PICTURE_RECORDS_OFFSET,
            METAL_GLYPH_BASE_WORDS * 4 + METAL_MAX_FRAME_GLYPH_SAMPLES * 4,
            "the picture table must start where the glyph region ends"
        );
        assert_eq!(
            METAL_PICTURE_INDEX_OFFSET,
            METAL_PICTURE_RECORDS_OFFSET + METAL_PICTURE_RECORDS * PICTURE_RECORD_BYTES,
            "the picture index must start where the picture records end"
        );
        assert_eq!(
            METAL_BUFFER_BYTES,
            METAL_PICTURE_INDEX_OFFSET + METAL_PICTURE_INDEX_SLOTS * 4,
            "the buffer must be exactly its seven regions, with nothing past the last"
        );
        assert_eq!(
            METAL_PICTURE_TABLE_BYTES,
            METAL_BUFFER_BYTES - METAL_PICTURE_RECORDS_OFFSET,
            "the picture table is the last region"
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
