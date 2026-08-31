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
//!   on write, exactly where the software path's `__CANVAS_SRGB` table does it;
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
    GRAPHICS_OFFSET_MTL_DEVICE, GRAPHICS_OFFSET_MTL_PIPELINE, GRAPHICS_OFFSET_MTL_QUEUE,
    GRAPHICS_OFFSET_MTL_READY, GRAPHICS_OFFSET_MTL_TEXTURE, GRAPHICS_OFFSET_MTL_TEX_HEIGHT,
    GRAPHICS_OFFSET_MTL_TEX_WIDTH, GRAPHICS_STATE_SYMBOL,
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
    "struct MfbItem {\n",
    "  int4 quad;\n",    // bounds minX, minY, maxX, maxY (16.16 px)
    "  int4 shape;\n",   // p0..p3 (16.16 px)
    "  int4 fill;\n",    // RGBA 0..255
    "  int4 stroke;\n",  // RGBA 0..255
    "  int4 misc;\n",    // kind, radius (16.16), strokeHalf (16.16), edgeCount
    "  int4 arc;\n",     // startAngle, endAngle (16.16 rad), unused, unused
    "  int2 surface;\n", // width, height (px)
    "};\n",
    "struct VOut { float4 pos [[position]]; };\n",
    "static float fx(int v) { return float(v) / FIXED; }\n",
    "vertex VOut mfbVertex(uint vid [[vertex_id]],\n",
    "                      constant MfbItem &item [[buffer(0)]]) {\n",
    "  float2 corner = float2(fx((vid & 1) == 0 ? item.quad.x : item.quad.z),\n",
    "                         fx((vid & 2) == 0 ? item.quad.y : item.quad.w));\n",
    "  VOut o;\n",
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
    "static bool arcInSweep(float2 d, float2 s, float2 e, bool reflex) {\n",
    "  bool afterStart = s.x * d.y - s.y * d.x >= 0.0;\n",
    "  bool beforeEnd  = e.x * d.y - e.y * d.x <= 0.0;\n",
    "  return reflex ? (afterStart || beforeEnd) : (afterStart && beforeEnd);\n",
    "}\n",
    "static float edgeDistance(constant int *edges, int count, float2 p) {\n",
    "  float best = 1.0e6;\n",
    "  bool inside = false;\n",
    "  for (int e = 0; e < count; ++e) {\n",
    "    int i = e * 4;\n",
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
    "static float geoDistance(constant MfbItem &item, constant int *edges, float2 p) {\n",
    "  float radius = fx(item.misc.y);\n",
    "  float2 c = float2(fx(item.shape.x), fx(item.shape.y));\n",
    "  if (item.misc.x == 0) {\n",
    "    return rectDistance(p, c, float2(fx(item.shape.z), fx(item.shape.w))) - radius;\n",
    "  }\n",
    "  if (item.misc.x == 1) { return length(p - c) - fx(item.shape.z) - radius; }\n",
    "  if (item.misc.x == 2) {\n",
    "    return segmentDistance(p, c, float2(fx(item.shape.z), fx(item.shape.w))) - radius;\n",
    "  }\n",
    "  if (item.misc.x == 3) {\n",
    "    float2 d = p - c;\n",
    "    float a0 = fx(item.arc.x), a1 = fx(item.arc.y);\n",
    "    float2 s = float2(cos(a0), sin(a0));\n",
    "    float2 e = float2(cos(a1), sin(a1));\n",
    "    if (!arcInSweep(d, s, e, (a1 - a0) > PI)) { return 1.0e6; }\n",
    "    return abs(length(d) - fx(item.shape.z)) - radius;\n",
    "  }\n",
    "  return edgeDistance(edges, item.misc.w, p);\n",
    "}\n",
    "static float srgbToLinear(float c) {\n",
    "  c = c / 255.0;\n",
    "  return c <= 0.04045 ? (c / 12.92) : pow((c + 0.055) / 1.055, 2.4);\n",
    "}\n",
    "static float4 premultiplied(int4 rgba, float distance) {\n",
    // The oracle quantizes coverage to a whole 0..255 and then takes an integer
    // `(colourAlpha * coverage) / 255`. Matching that here rather than blending in
    // float is not pedantry: near full coverage the sRGB encode is so steep that ONE
    // step of coverage moves a dark channel by up to 13 output steps, so a float
    // coverage that merely rounds differently produces a visible disagreement on
    // every antialiased edge. Quantizing the same way leaves only the pixels within
    // a rounding boundary of each other.
    "  int coverage = int(clamp(0.5 - distance, 0.0, 1.0) * 255.0 + 0.5);\n",
    "  float a = float((rgba.w * coverage) / 255) / 255.0;\n",
    "  return float4(srgbToLinear(float(rgba.x)) * a,\n",
    "                srgbToLinear(float(rgba.y)) * a,\n",
    "                srgbToLinear(float(rgba.z)) * a, a);\n",
    "}\n",
    "fragment float4 mfbFragment(VOut in [[stage_in]],\n",
    "                            constant MfbItem &item [[buffer(0)]],\n",
    "                            constant int *edges [[buffer(1)]]) {\n",
    "  float d = geoDistance(item, edges, in.pos.xy);\n",
    "  float4 colour = premultiplied(item.fill, d);\n",
    "  float halfWidth = fx(item.misc.z);\n",
    "  if (halfWidth > 0.0) {\n",
    "    float4 s = premultiplied(item.stroke, abs(d) - halfWidth);\n",
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
pub(super) const SEL_SET_VERTEX_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_setVertexBytes",
    "setVertexBytes:length:atIndex:",
);
pub(super) const SEL_DRAW_PRIMITIVES: (&str, &str) = (
    "_mfb_macapp_sel_drawPrimitives",
    "drawPrimitives:vertexStart:vertexCount:",
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
/// is the same transform the software path's `__CANVAS_SRGB` table applies on the
/// way out — so the two agree by construction rather than by a matching pair of
/// hand-written conversions. It is also a `CAMetalLayer`-supported format, so the
/// offscreen target this pipeline is proved against and the on-screen drawable it
/// eventually presents to use *one* pipeline, not two that could drift.
pub(super) const MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB: &str = "81";
/// `MTLBlendFactorOne` — premultiplied source.
const MTL_BLEND_FACTOR_ONE: &str = "1";
/// `MTLBlendFactorOneMinusSourceAlpha`.
const MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: &str = "5";

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
    let saves: [(&str, usize); 5] = [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
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
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0))); // attachment

    // The colour chain, all on that attachment: sRGB target, premultiplied `over`.
    for (setter, value) in [
        (SEL_SET_PIXEL_FORMAT.0, MTL_PIXEL_FORMAT_BGRA8UNORM_SRGB),
        (SEL_SET_BLENDING_ENABLED.0, "1"),
        (SEL_SET_SRC_RGB_FACTOR.0, MTL_BLEND_FACTOR_ONE),
        (SEL_SET_SRC_ALPHA_FACTOR.0, MTL_BLEND_FACTOR_ONE),
        (
            SEL_SET_DST_RGB_FACTOR.0,
            MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        ),
        (
            SEL_SET_DST_ALPHA_FACTOR.0,
            MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        ),
    ] {
        asm.load_selector(setter);
        asm.push(abi::move_immediate(abi::c_arg(2), "Integer", value));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
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

    // Publish device, queue and pipeline for the frame path, and record success.
    // The pipeline is stored last: a frame that races this sees a non-zero pipeline
    // only once the device and queue it needs are already there.
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0)));
    asm.local_address(abi::c_arg(1), GRAPHICS_STATE_SYMBOL);
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
const MTL_STORE_ACTION_STORE: &str = "1";
/// `MTLPrimitiveTypeTriangleStrip` — four vertices, two triangles, no index buffer.
///
/// 4, not 3: the enum runs Point, Line, LineStrip, Triangle, TriangleStrip, so 3 is
/// the triangle *list*. A list with four vertices is not an error — it draws one
/// triangle and ignores the fourth vertex, which renders exactly half the quad and
/// looks like a geometry bug rather than an enum one.
const MTL_PRIMITIVE_TRIANGLE_STRIP: &str = "4";

/// 16.16 fixed point: the scale the shader divides the quad by.
const FIXED_POINT_SCALE: &str = "65536";

/// The per-item parameter block. Six `int4`s and an `int2`, 112 bytes.
///
/// Every member is an `int4` so the emitter's offsets and MSL's own packing cannot
/// disagree. A mixed struct would put the burden of predicting MSL's alignment rules
/// on this file, and a wrong prediction is not a compile error — it is a scene that
/// draws with its fields shifted. The trailing `int2` is safe because it is last.
const ITEM_BLOCK_SIZE: usize = 112;
const ITEM_OFFSET_QUAD: usize = 0;
const ITEM_OFFSET_SHAPE: usize = 16;
const ITEM_OFFSET_FILL: usize = 32;
const ITEM_OFFSET_STROKE: usize = 48;
const ITEM_OFFSET_MISC: usize = 64;
const ITEM_OFFSET_ARC: usize = 80;
const ITEM_OFFSET_SURFACE: usize = 96;

/// Geometry-header slots this renderer reads (`__canvas_headerFor`'s fixed layout).
const HEADER_KIND: usize = 0;
const HEADER_SHAPE: usize = 2;
const HEADER_RADIUS: usize = 6;
const HEADER_STROKE_HALF: usize = 7;
const HEADER_FILL_R: usize = 8;
const HEADER_STROKE_R: usize = 12;
const HEADER_BOUNDS: usize = 16;
/// Slot 20 is the polygon's edge count *and* the arc's start angle — the header
/// reuses it per kind, and so does the item block. Writing both unconditionally is
/// cheaper than branching and can never be wrong: the shader reads only the one its
/// `kind` selects.
const HEADER_AUX0: usize = 20;
const HEADER_AUX1: usize = 21;
/// The fixed header length, in slots — where a polygon's edge tail begins.
const HEADER_SLOTS: usize = 22;
/// Doubles per cached edge: `x0, y0, dx, dy, invLenSq`.
const EDGE_SLOTS: usize = 5;
/// The most edges one polygon may carry on the GPU path.
///
/// `setFragmentBytes:length:atIndex:` is capped at 4 KB, and each edge crosses as
/// four 16.16 ints. `__canvas_metalRenderable` declines a polygon past this rather
/// than truncating it, because a truncated polygon renders as a *different shape*
/// and would read as a geometry bug.
const MAX_EDGES: usize = 256;

// The frame. `OFF_REGION` holds the 48-byte `MTLRegion` that
// `getBytes:bytesPerRow:fromRegion:mipmapLevel:` takes by value in C. AAPCS64 rule
// B.4 turns a composite argument larger than 16 bytes into a **pointer to a
// caller-allocated copy** before register assignment ever happens, so the region is
// passed as an address in the next argument register — not spilled to an outgoing
// stack area. Getting that wrong is not a subtle mismatch: the callee dereferences
// whatever is in that register, and a zero there faults inside
// `-[IOGPUMetalTexture getBytes:…]` with none of our frames in the trace.
const DRAW_FRAME: usize = 320 + MAX_EDGES * 16;
const OFF_REGION: usize = 0;
const OFF_LR: usize = 64;
const OFF_SAVES: usize = 72;
const OFF_SURFACE: usize = 136;
const OFF_WIDTH: usize = 144;
const OFF_HEIGHT: usize = 152;
const OFF_POOL: usize = 160;
const OFF_ITEM: usize = 192;
const OFF_TEXTURE: usize = 304;
/// The edge count, parked because `load_selector` calls through `sel_registerName`
/// and every scratch register is caller-saved across it.
const OFF_EDGE_COUNT: usize = 312;
/// The polygon edge buffer: `MAX_EDGES` edges of four 16.16 ints.
const OFF_EDGES: usize = 320;

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

    // --- one quad per item -------------------------------------------------------
    asm.push(abi::move_immediate(abi::LOCAL[2], "Integer", "0"));
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

    emit_edge_buffer(&mut asm);
    asm.push(abi::store_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_EDGE_COUNT,
    ));
    emit_item_block(&mut asm);

    // The block goes to both stages: the vertex shader needs the quad and the surface
    // size, the fragment shader needs everything else.
    for setter in [SEL_SET_VERTEX_BYTES.0, SEL_SET_FRAGMENT_BYTES.0] {
        asm.load_selector(setter);
        asm.push(abi::add_immediate(
            abi::c_arg(2),
            abi::stack_pointer(),
            OFF_ITEM,
        ));
        asm.push(abi::move_immediate(
            abi::c_arg(3),
            "Integer",
            &ITEM_BLOCK_SIZE.to_string(),
        ));
        asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }

    let empty_edges = format!("{METAL_DRAW_SYMBOL}_empty_edges");
    // The edge buffer, always bound even when empty: `setFragmentBytes:length:` will
    // not take a zero length, and an unbound buffer the shader declares is a
    // validation failure at draw time rather than a nil the shader can test.
    asm.load_selector(SEL_SET_FRAGMENT_BYTES.0);
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_EDGE_COUNT,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "16"));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::move_register(abi::c_arg(3), abi::SCRATCH[1]));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&empty_edges));
    asm.push(abi::move_register(abi::c_arg(3), abi::SCRATCH[0]));
    asm.push(abi::label(&empty_edges));
    asm.push(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_EDGES,
    ));
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "1"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.load_selector(SEL_DRAW_PRIMITIVES.0);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        MTL_PRIMITIVE_TRIANGLE_STRIP,
    ));
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "4"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[6]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::add_immediate(abi::LOCAL[2], abi::LOCAL[2], 1));
    asm.push(abi::branch(&item_head));
    asm.push(abi::label(&item_done));

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
}

/// Convert a polygon's cached edge tail into the shader's edge buffer at
/// `sp + OFF_EDGES`, and leave the edge count in `SCRATCH[2]`.
///
/// `SCRATCH[0]` holds the geometry header. The cache stores each edge as
/// `x0, y0, dx, dy, invLenSq`; the buffer carries the two **endpoints** instead, and
/// the shader recomputes the edge vector. That is not lost work: the cache keeps
/// `invLenSq` to keep a reciprocal off the software path's per-pixel loop, the GPU
/// has the divide for free, and `invLenSq` is the one header quantity 16.16 fixed
/// point represents badly — a 100-px edge gives 1e-4, which is 6 in 16.16.
///
/// A non-polygon leaves the count at zero and reads nothing: slot 20 is the arc's
/// start angle for an arc, so walking a tail that is not there would read the *next*
/// item's header as edge coordinates.
fn emit_edge_buffer(asm: &mut Asm) {
    let head = format!("{METAL_DRAW_SYMBOL}_edge_head");
    let done = format!("{METAL_DRAW_SYMBOL}_edge_done");
    let convert = format!("{METAL_DRAW_SYMBOL}_edge_convert");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let scale = abi::FP_SCRATCH[0];

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
    asm.push(abi::compare_immediate(abi::SCRATCH[5], "4")); // __CANVAS_GEO_POLYGON
    asm.push(abi::branch_ne(&done));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_AUX0 * 8,
    ));
    asm.push(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    asm.push(abi::compare_immediate(count, &MAX_EDGES.to_string()));
    asm.push(abi::branch_le(&convert));
    // Declined upstream by `__canvas_metalRenderable`; clamping here would render a
    // *different polygon*, so refuse to draw any of it instead.
    asm.push(abi::move_immediate(count, "Integer", "0"));
    asm.push(abi::branch(&done));

    asm.push(abi::label(&convert));
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    asm.push(abi::signed_convert_to_float_d(scale, abi::SCRATCH[5]));
    asm.push(abi::add_immediate(source, header, HEADER_SLOTS * 8));
    asm.push(abi::add_immediate(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        OFF_EDGES,
    ));
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
        SEL_COMMAND_BUFFER,
        SEL_RENDER_COMMAND_ENCODER,
        SEL_SET_RENDER_PIPELINE_STATE,
        SEL_SET_VERTEX_BYTES,
        SEL_DRAW_PRIMITIVES,
        SEL_END_ENCODING,
        SEL_COMMIT,
        SEL_WAIT_UNTIL_COMPLETED,
        SEL_GET_BYTES,
        SEL_SET_FRAGMENT_BYTES,
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
             would skip the encode the software oracle applies through __CANVAS_SRGB"
        );
    }

    fn render_field(name: &'static str, operand: impl Into<Operand>) -> String {
        CodeInstruction::new("mov")
            .field(name, operand)
            .get(name)
            .expect("the field was just set")
    }
}
