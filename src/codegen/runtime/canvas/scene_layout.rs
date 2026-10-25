//! plan-176-V: the scene layout both GPU backends read — the per-item block (`ITEM_*` and its
//! emitters), the `HEADER_*` words, `GEO_KIND_*`, glyph and tile slots, and the GPU buffer
//! sizing. The shared imports and `GraphicsScratch` live in `runtime/canvas/mod.rs`.

use super::*;

/// The per-item parameter block both GPU backends push to their shaders.
///
/// **One contract, two emitters.** The block is byte-identical between the Metal
/// pipeline (an MSL `constant MfbItem&`) and the Vulkan one (a GLSL push-constant
/// block) — glslang's reflection reports exactly these offsets and this size — so
/// the shaders agree by construction rather than by two hand-maintained layouts
/// staying in step. Each backend emits the *stores* in its own IR flavour; the
/// *layout* is here, once.
///
/// Seventeen `ivec4`s, one of them holding the surface size. Every member is an `ivec4`
/// so the two shading languages' packing rules cannot disagree.
///
/// **The bound is the item buffer, not a push-constant range** (plan-116-A). Until
/// then the block rode `vkCmdPushConstants` / `setVertexBytes:` and 112 was chosen to
/// fit Vulkan's *guaranteed* 128-byte range — a hard ceiling, and the only guaranteed
/// one, so widening the block past it would have made the feature set depend on the
/// device. It now travels in `…_VULKAN_ITEM_BUFFER` (Metal:
/// `…_MTL_ITEM_BUFFER`), one record per drawn quad indexed by instance, so the only
/// limit left is capacity: `VULKAN_MAX_FRAME_ITEMS` records must fit
/// `VULKAN_ITEM_BUFFER_BYTES`, which is defined *from* this constant and therefore
/// cannot fall out of step.
///
/// What still constrains the value is **agreement between the two shading languages**,
/// and that is gated rather than trusted: glslang's reflection for the GLSL
/// `ItemBlock` must report `topLevelArrayStride` equal to this size with one member per
/// 16 bytes (`glslangValidator -V -q mfb_canvas.vert`; 208 with members at
/// 0/16/.../192 when plan-116-F added the gradient axis `ivec4`, 272 with the four
/// residual `ivec4`s at 208/224/240/256 from bug-735 until the shaders went to fp32, which
/// took them out again; plan-163-B's `strokeStyle` at 208; plan-163-C's `clipBand` at 224
/// and `clipPath` at 240; plan-163-D's `gradInv0`/`gradInv1` at 256/272 and `gradFocal`
/// at 288), matching the
/// `ITEM_OFFSET_*` constants below one for one. `the_item_block_matches_the_std430_stride`
/// in `vulkan/tests/mod.rs` pins it. Widening the block means keeping every member `ivec4`-sized
/// so std430's stride stays equal to the size, then re-running that reflection.
pub(crate) const ITEM_BLOCK_SIZE: usize = 304;
/// Bounds `minX, minY, maxX, maxY`, as float32 bit patterns (`emit_f32_bits`).
pub(crate) const ITEM_OFFSET_QUAD: usize = 0;
/// The shape parameters `p0..p3`, float32 bits.
pub(crate) const ITEM_OFFSET_SHAPE: usize = 16;
/// Fill and stroke `RGBA`, whole 0–255 values.
pub(crate) const ITEM_OFFSET_FILL: usize = 32;
pub(crate) const ITEM_OFFSET_STROKE: usize = 48;
/// `kind`, `radius` (float32 bits), `strokeHalf` (float32 bits), `edgeCount`.
pub(crate) const ITEM_OFFSET_MISC: usize = 64;
/// A glyph's or picture's bitmap height (`ITEM_ARC_GLYPH_HEIGHT`, 0 otherwise), an
/// arc's `reflex` flag (`ITEM_ARC_REFLEX`), then the polygon's first-edge index into the
/// frame's edge region, then the cap tag. The first two words held the arc's 16.16
/// angles until bug-735.
///
/// The words are per-kind the same way `HEADER_AUX0` is: an arc never reads the
/// edge base and a polygon never reads the flag, so one `ivec4` carries both rather
/// than each taking its own. plan-163-A gives a polygon's copy of the flag word its
/// fill rule instead ([`emit_item_polygon_rule`]). Both backends carry the edge base since plan-116-A: each
/// polygon takes a slice of one buffer that serves the whole frame, and this is where
/// the offset travels.
pub(crate) const ITEM_OFFSET_ARC: usize = 80;
/// The word inside `ITEM_OFFSET_ARC` holding the polygon's first-edge index.
///
/// For a glyph (`GEO_KIND_TEXT`) the same word holds the first-*sample* index into the
/// buffer's glyph region, for exactly the same reason: Vulkan records one buffer for the
/// frame, so a per-draw offset is the only way each glyph can see its own bitmap.
pub(crate) const ITEM_ARC_EDGE_BASE: usize = 8;
/// The word inside `ITEM_OFFSET_ARC` holding a glyph's bitmap height.
///
/// A glyph reads neither arc angle, so `arc.x` carries the height beside `misc.w`'s
/// width — the same per-kind reuse the arc/polygon pair already makes of this block.
pub(crate) const ITEM_ARC_GLYPH_HEIGHT: usize = 0;
/// The word inside `ITEM_OFFSET_ARC` holding the `CapStyle` tag, 0 or 1 (plan-116-D).
///
/// The block's last free word, and the right one: only `Line` and `Arc` read it, and
/// they are the two kinds this block already serves — a polygon reads
/// `ITEM_ARC_EDGE_BASE`, a glyph reads `ITEM_ARC_GLYPH_HEIGHT`, and none of the three
/// uses overlaps another because no item is two kinds at once. Taking a word here is
/// what keeps `ITEM_BLOCK_SIZE` at 160 rather than growing every item in every scene
/// to carry one bit for two primitives.
pub(crate) const ITEM_ARC_CAP: usize = 12;
/// The surface's width and height, in whole pixels, then the blend mode.
pub(crate) const ITEM_OFFSET_SURFACE: usize = 96;
/// The word inside `ITEM_OFFSET_SURFACE` holding the `BlendMode` tag, 0..3
/// (plan-116-B).
///
/// It lands here rather than in a new `ivec4` because plan-116-A's audit found this
/// word free — the surface size needs two of the four, and the block is declared as a
/// full `ivec4` on both sides precisely so trailing padding cannot differ between the
/// languages. Using the space costs nothing and keeps the block one `ivec4` narrower.
pub(crate) const ITEM_SURFACE_BLEND: usize = 8;
/// The item's clip rectangle, resolved to `x0, y0, x1, y1` as float32 bits
/// (plan-116-B).
///
/// The `ivec4` that took the block from 112 to 128 bytes — which the push-constant
/// transport could not have afforded, since 128 is the whole guaranteed range and the
/// pipeline layout would have had nothing left. plan-116-A moved the block into a
/// buffer for exactly this.
///
/// A zero-area rectangle means **unclipped** and is recognised by `x0 >= x1 || y0 >=
/// y1`, the same test the geometry header uses, so the two agree by construction.
pub(crate) const ITEM_OFFSET_CLIP: usize = 112;
/// The inverted transform, as **raw IEEE-754 `float32` bits** in two `ivec4`s
/// (plan-116-C): `ia, ib, ic, id` at 128 and `itx, ity, hasTransform, unused` at 144.
///
/// Float32 like every other geometric field in the block, but narrowed in MFBASIC
/// (`__canvas_float32Bits`) rather than by the emitter: the header slots already hold
/// the bit patterns, so the emitter copies them as whole numbers. The shaders decode
/// with `intBitsToFloat` / `as_type<float>`.
///
/// `hasTransform` is a whole 0 or 1, not a float bit pattern: it is compared, never
/// arithmetic.
pub(crate) const ITEM_OFFSET_TRANSFORM: usize = 128;
/// An arc's two sweep endpoints as float32 bits — `startX, startY, endX, endY`
/// (plan-116-D).
///
/// The eleventh `ivec4`, and the one member of the block that had to grow it: the
/// per-kind `arc` block's last free word went to the cap tag, and four coordinates do
/// not fit in the two words left elsewhere. 160 → 176 bytes, which stays a multiple of
/// 16 so std430's array stride still equals the size —
/// `the_item_block_matches_the_std430_stride` is what pins that.
///
/// Every arc reads them since bug-735: a `Round` cap's two discs, and for every cap the
/// sweep test's direction vectors, which are the endpoints less the centre -- a positive
/// multiple of the oracle's Taylor `(cos, sin)`, so the shaders need no trigonometry at
/// all. Every item pays 16 bytes of buffer for it, which is the trade: the alternative
/// is a per-pixel `sin`/`cos` pair in three renderers, and the deterministic series the
/// oracle requires is far more expensive than a fetch.
pub(crate) const ITEM_OFFSET_ARC_CAPS: usize = 160;
/// A **polygon**'s band-index header in the same `ivec4` (bug-686 on Metal, bug-688 on
/// Vulkan): `bandTop`, `bandHeight` (float32 bits, shape space), the band count (0 = loop over
/// every edge) and its table's word offset in the band region (`METAL_BAND_BASE_WORDS`,
/// `VULKAN_BAND_BASE_WORDS`).
///
/// The per-kind sharing `ITEM_OFFSET_ARC` already does: only an arc reads its caps and
/// only a polygon reads its bands, and no item is both. Each backend's `emit_band_index`
/// writes these after `emit_item_block`'s generic arc-caps store, for polygons only; the
/// shaders read them only when `misc.x` is the polygon kind.
pub(crate) const ITEM_BAND_TOP: usize = 0;
pub(crate) const ITEM_BAND_HEIGHT: usize = 4;
pub(crate) const ITEM_BAND_COUNT: usize = 8;
pub(crate) const ITEM_BAND_START: usize = 12;
/// bug-1085: a band header's fifth word, outside its `ivec4`: `F_built`, the norm its
/// reach was built for, as float32 bits. That is `max(F, sqrt 2)`, `F` the
/// Frobenius norm of the item's own inverse (0 untransformed) -- the only inverse the
/// builder can know, since a block is built once a pass and may be drawn under several
/// group matrices -- floored at the identity's norm, so an identity, rotation or
/// magnifying group keeps the band. The shaders take the band only for a pixel whose
/// composed norm `N` the reach covers, `0.75 N <= F_built` (the builder adds `2 F_built`
/// where the five-sample argument needs `1.5 N`), and otherwise walk every edge: only a
/// group that shrinks past the floor does.
///
/// The polygon's is `ellipse.x`, an ellipse's rotation cosine, which only an ellipse
/// reads, and no ellipse has a band index; a path clip's is `gradFocal.w`, which
/// nothing else writes, because a clip can be on an ellipse. Written by each backend's
/// `emit_band_index` with the rest of the header (0 with no bands), after
/// `emit_item_block` wrote the rotation, and read only beside a band count above 0.
pub(crate) const ITEM_BAND_NORM: usize = ITEM_OFFSET_ELLIPSE;
pub(crate) const ITEM_CLIP_BAND_NORM: usize = ITEM_OFFSET_GRADIENT_FOCAL + 12;
/// An ellipse's rotation, as `cos, sin` in float32 bits, then the gradient's stop count
/// and first-stop index (plan-116-E, plan-116-F).
///
/// The twelfth `ivec4`. Two words would have fit in an existing block's spare, but
/// std430 rounds the array stride to a multiple of 16 anyway, so a half-used `ivec4`
/// costs exactly what a full one does and keeps every member `ivec4`-sized — which is
/// the property `the_item_block_matches_the_std430_stride` exists to hold.
///
/// The CPU evaluates the deterministic Taylor pair; this is only how the answer
/// travels. A banded polygon's block carries its band norm in the cosine's word
/// (`ITEM_BAND_NORM`).
pub(crate) const ITEM_OFFSET_ELLIPSE: usize = 176;
/// A gradient's axis as float32 bits — `startPoint.x, startPoint.y, endPoint.x, endPoint.y`
/// (plan-116-F).
///
/// The thirteenth `ivec4`, and the block had to grow for it: the `ellipse` block's two
/// spare words went to the stop count and base, and four coordinates do not fit in the
/// one word left elsewhere. The gradient's *kind* does fit there, and takes it —
/// `ITEM_SURFACE_GRADIENT_KIND`.
///
/// Growing the block moves Metal's hand-assigned frame and `METAL_EDGE_BASE` with it;
/// `.ai/canvas-threading.md` lists the five things that move and which three are
/// silent.
pub(crate) const ITEM_OFFSET_GRADIENT: usize = 192;
/// plan-163-B: the stroke's style words -- the first edge slot of the item's join
/// records in the frame's edge region and how many records there are (each takes two
/// slots), then the first entry of its dash pattern in the frame's dash region and how
/// many entries. All four are 0 for an item that draws no join record and no dash.
pub(crate) const ITEM_OFFSET_STROKE_STYLE: usize = 208;
pub(crate) const ITEM_STROKE_JOIN_BASE: usize = 0;
pub(crate) const ITEM_STROKE_JOIN_COUNT: usize = 4;
pub(crate) const ITEM_STROKE_DASH_BASE: usize = 8;
pub(crate) const ITEM_STROKE_DASH_COUNT: usize = 12;
/// plan-163-C: a path clip's band header -- top, height, band count, table offset, the
/// shape of `arcCaps`'s polygon band header -- for the clip's own edges; band count 0
/// walks every clip edge.
pub(crate) const ITEM_OFFSET_CLIP_BAND: usize = 224;
/// plan-163-C: the clip's words -- its first edge in the frame's edge region and its edge
/// count (a path clip), its mode (0 none, 1 rectangle, 2 path) with its FillRule ordinal
/// shifted left 2, and a picture's transparency step T.
pub(crate) const ITEM_OFFSET_CLIP_PATH: usize = 240;
pub(crate) const ITEM_CLIP_PATH_BASE: usize = 0;
pub(crate) const ITEM_CLIP_PATH_COUNT: usize = 4;
pub(crate) const ITEM_CLIP_PATH_MODE: usize = 8;
pub(crate) const ITEM_CLIP_PATH_PICTURE_T: usize = 12;
/// plan-163-D: a radial gradient's own inverse transform -- `ia, ib, ic, id` then `itx,
/// ity` as float32 BIT PATTERNS and its flag (0 or 1), the `xform0`/`xform1` layout --
/// copied word for word from header slots 57-63 into `gradInv0` and `gradInv1`.
pub(crate) const ITEM_OFFSET_GRADIENT_INV: usize = 256;
/// plan-163-D: a radial gradient's focal offset from its centre and focal radius, float32
/// bits (`gradFocal`), already clamped by the builder.
pub(crate) const ITEM_OFFSET_GRADIENT_FOCAL: usize = 288;
/// The word inside `ITEM_OFFSET_ARC` holding an arc's sweep TURN (bug-735, widened by
/// bug-1088): 0 when `endAngle - startAngle <= PI`, 1 when it is past `PI` (reflex), 2
/// when it is a whole turn or more (`>= 2 PI`, the whole ring, which two sweep vectors
/// cannot tell from a zero sweep), decided by the emitter in the oracle's own f64
/// (`__canvas_arcTurn`), so the sweep's choice of wedge cannot differ from the oracle's. It replaced the 16.16 end angle, which the shaders turned
/// into a sweep vector with GPU `cos`/`sin` -- transcendentals that do not match the
/// oracle's deterministic Taylor pair. The sweep vectors now come from the arc's cap
/// endpoints (`arcCaps` minus the centre, a positive multiple of `(cos, sin)`), so no
/// angle reaches either shader and `arc.x` is written 0 for every kind but a glyph or
/// picture, which put their bitmap height there.
///
/// For a polygon (kind 4) the same word is its `FillRule` ordinal, 0 EvenOdd or 1
/// NonZero (plan-163-A, [`emit_item_polygon_rule`]): only an arc reads it as the flag.
pub(crate) const ITEM_ARC_REFLEX: usize = 4;

/// Every item-block field a distance reads, as `(header slot, word offset)` -- offsets
/// into the block: the shape parameters, an arc's sweep endpoints, the clip, the radius,
/// the stroke half-width and an ellipse's rotation. Both emitters narrow each one to
/// float32 with [`emit_item_float_field`], so the Metal and Vulkan blocks carry the same
/// words by construction.
pub(crate) const ITEM_FLOAT_FIELDS: [(usize, usize); 16] = [
    (HEADER_SHAPE, ITEM_OFFSET_SHAPE),
    (HEADER_SHAPE + 1, ITEM_OFFSET_SHAPE + 4),
    (HEADER_SHAPE + 2, ITEM_OFFSET_SHAPE + 8),
    (HEADER_SHAPE + 3, ITEM_OFFSET_SHAPE + 12),
    (HEADER_CAP_START_X, ITEM_OFFSET_ARC_CAPS),
    (HEADER_CAP_START_X + 1, ITEM_OFFSET_ARC_CAPS + 4),
    (HEADER_CAP_END_X, ITEM_OFFSET_ARC_CAPS + 8),
    (HEADER_CAP_END_X + 1, ITEM_OFFSET_ARC_CAPS + 12),
    (HEADER_CLIP_X0, ITEM_OFFSET_CLIP),
    (HEADER_CLIP_Y0, ITEM_OFFSET_CLIP + 4),
    (HEADER_CLIP_X1, ITEM_OFFSET_CLIP + 8),
    (HEADER_CLIP_Y1, ITEM_OFFSET_CLIP + 12),
    (HEADER_RADIUS, ITEM_OFFSET_MISC + 4),
    (HEADER_STROKE_HALF, ITEM_OFFSET_MISC + 8),
    (HEADER_ELLIPSE_COS, ITEM_OFFSET_ELLIPSE),
    (HEADER_ELLIPSE_SIN, ITEM_OFFSET_ELLIPSE + 4),
];

/// `PI` (`3.141592653589793`, the constant `__canvas_arcTurn` compares an arc's sweep
/// against) as its IEEE-754 double bit pattern.
const PI_F64_BITS: &str = "4614256656552045848";
/// `2 PI` (`6.283185307179586`, `__canvas_arcTurn`'s whole turn), likewise (bug-1088).
const TWO_PI_F64_BITS: &str = "4618760256179416344";

/// Narrow the double in `fp` to binary32 (nearest, ties to even) and leave its bit
/// pattern in the low 32 bits of `bits`, ready for a `store_u32`. This is how every
/// geometric value reaches either GPU: the shaders read it back with `intBitsToFloat` /
/// `as_type<float>`. `fp` is clobbered. Plain instructions, so the Metal `Asm` and the
/// Vulkan `CodeBuilder` emit the same ones.
pub(crate) fn emit_f32_bits(bits: abi::Reg, fp: abi::Reg) -> Vec<CodeInstruction> {
    vec![
        abi::float_convert_s_from_d(fp, fp),
        abi::float_move_x_from_d(bits, fp),
    ]
}

/// 65536: the scale of the 16.16 integers the band builders (`emit_band_index`, both
/// backends) do their band arithmetic in. Nothing the GPU reads is 16.16: positions
/// reach the shaders as float32 (`emit_f32_bits`), and the builders convert at the edges
/// of their work (`emit_f32_word_to_fixed`, `emit_fixed_to_f32_bits`).
pub(crate) const FIXED_POINT_SCALE: &str = "65536";

/// `0xFFFF_FFFF_0000_0000`: OR'd into a register holding a float32 bit pattern in its low
/// half before it moves to an FP register, so a RISC-V `fcvt.d.s` reads a NaN-boxed
/// single (AArch64 and x86-64 read only the low half and ignore it).
const NAN_BOX_HIGH: &str = "18446744069414584320";

/// The band builders' bridge from the float32 words the GPU reads to the 16.16 integers
/// they do their band arithmetic in: load the float32 at `base + offset`, widen it
/// (exact), and leave `round(v * 65536)` in `dst`. The rounding moves a value by at most
/// 2^-17 of a unit, well inside the builders' 1-unit guard. Clobbers `tmp`, `fp` and
/// `fp_scale`.
pub(crate) fn emit_f32_word_to_fixed(
    dst: abi::Reg,
    base: abi::Reg,
    offset: usize,
    tmp: abi::Reg,
    fp: abi::Reg,
    fp_scale: abi::Reg,
) -> Vec<CodeInstruction> {
    vec![
        abi::load_u32(dst, base, offset),
        abi::move_immediate(tmp, "Integer", NAN_BOX_HIGH),
        abi::or_registers(dst, dst, tmp),
        abi::float_move_d_from_x(fp, dst),
        abi::float_convert_d_from_s(fp, fp),
        abi::move_immediate(tmp, "Integer", FIXED_POINT_SCALE),
        abi::signed_convert_to_float_d(fp_scale, tmp),
        abi::float_multiply_d(fp, fp, fp_scale),
        abi::float_round_to_signed_x(dst, fp),
    ]
}

/// The way back: the 16.16 integer in `src` as float32 bits in `bits` (`src / 65536` is
/// exact in a double, then narrowed like every other value). `src` may be `bits`.
/// Clobbers `tmp`, `fp` and `fp_scale`.
pub(crate) fn emit_fixed_to_f32_bits(
    bits: abi::Reg,
    src: abi::Reg,
    tmp: abi::Reg,
    fp: abi::Reg,
    fp_scale: abi::Reg,
) -> Vec<CodeInstruction> {
    let mut out = vec![
        abi::signed_convert_to_float_d(fp, src),
        abi::move_immediate(tmp, "Integer", FIXED_POINT_SCALE),
        abi::signed_convert_to_float_d(fp_scale, tmp),
        abi::float_divide_d(fp, fp, fp_scale),
    ];
    out.extend(emit_f32_bits(bits, fp));
    out
}

/// Narrow one header slot to float32 at `block + word_at` (an offset from the stack
/// pointer) -- one field of the item block. Clobbers `FP_SCRATCH[1]` and `SCRATCH[1]`.
pub(crate) fn emit_item_float_field(
    header: abi::Reg,
    slot: usize,
    word_at: usize,
) -> Vec<CodeInstruction> {
    let mut out = vec![abi::load_double(abi::FP_SCRATCH[1], header, slot * 8)];
    out.extend(emit_f32_bits(abi::SCRATCH[1], abi::FP_SCRATCH[1]));
    out.push(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        word_at,
    ));
    out
}

/// bug-735: make the stroke half-width's word (`misc.z`) positive exactly when the
/// header's f64 half-width is -- the test the software rasteriser strokes on
/// (`half > 0.0`), and the one the shaders (`misc.z > 0`) and the emitters' split test
/// (`__canvas_blockInstances`' twin) take on the word as a SIGNED integer: a float32's
/// bits are positive exactly when it is above +0.
///
/// `__canvas_strokeHalf` answers -1 or `strokeWidth / 2`, so only a half-width under the
/// smallest float32 (~1.4e-45 px) can narrow to +0; this makes its bits 1, the smallest
/// positive float32. Comparing the bits rather than the value is what keeps that
/// stroking on a GPU that flushes denormals to zero. Without it the shaders and the
/// emitters' split test would read it as "does not stroke" while the oracle strokes it
/// and the draw list counts two records for it. `positive` and `done` are two fresh
/// labels; clobbers `FP_SCRATCH[1..=2]` and `SCRATCH[1]`. Runs after the word is
/// written.
pub(crate) fn emit_item_stroke_sign(
    header: abi::Reg,
    block_at: usize,
    positive: &str,
    done: &str,
) -> Vec<CodeInstruction> {
    let word_at = block_at + ITEM_OFFSET_MISC + 8;
    vec![
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_STROKE_HALF * 8),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "0"),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[2], abi::SCRATCH[1]),
        abi::float_compare_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::branch_gt(positive),
        abi::branch(done),
        abi::label(positive),
        abi::load_u32(abi::SCRATCH[1], abi::stack_pointer(), word_at),
        abi::compare_immediate(abi::SCRATCH[1], "0"),
        abi::branch_ne(done),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "1"),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), word_at),
        abi::label(done),
    ]
}

/// bug-735: the item block's `arc.x` (0) and `arc.y` (`ITEM_ARC_REFLEX`) at
/// `block_at` from the stack pointer: the sweep's turn, the oracle's own `__canvas_arcTurn`
/// in f64 -- 2 for `endAngle - startAngle >= 2 PI`, 1 for `> PI`, else 0 (bug-1088) -- for
/// every kind; only an arc reads it. `set` and `done` are two fresh labels (and `set`
/// with `_whole` appended a third). Clobbers `FP_SCRATCH[1..=2]` and `SCRATCH[1]`.
pub(crate) fn emit_item_arc_flags(
    header: abi::Reg,
    block_at: usize,
    set: &str,
    done: &str,
) -> Vec<CodeInstruction> {
    let reflex_at = block_at + ITEM_OFFSET_ARC + ITEM_ARC_REFLEX;
    let whole = format!("{set}_whole");
    vec![
        abi::move_immediate(abi::SCRATCH[1], "Integer", "0"),
        abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            block_at + ITEM_OFFSET_ARC,
        ),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), reflex_at),
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_AUX1 * 8),
        abi::load_double(abi::FP_SCRATCH[2], header, HEADER_AUX0 * 8),
        abi::float_subtract_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::move_immediate(abi::SCRATCH[1], "Integer", TWO_PI_F64_BITS),
        abi::float_move_d_from_x(abi::FP_SCRATCH[2], abi::SCRATCH[1]),
        // `2 PI <= sweep` as `b.ls` (a NaN falls through): float `>=` has no portable
        // branch after `float_compare_d`.
        abi::float_compare_d(abi::FP_SCRATCH[2], abi::FP_SCRATCH[1]),
        abi::branch_ls(&whole),
        abi::move_immediate(abi::SCRATCH[1], "Integer", PI_F64_BITS),
        abi::float_move_d_from_x(abi::FP_SCRATCH[2], abi::SCRATCH[1]),
        abi::float_compare_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::branch_gt(set),
        abi::branch(done),
        abi::label(&whole),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "2"),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), reflex_at),
        abi::branch(done),
        abi::label(set),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "1"),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), reflex_at),
        abi::label(done),
    ]
}
/// `std::f64::consts::SQRT_2` as its IEEE-754 bit pattern, for `float_move_d_from_x`.
const SQRT_2_F64_BITS: u64 = 0x3FF6_A09E_667F_3BCD;

/// plan-163-B: widen a band reach's half-width in `half` to `half * sqrt(2)` when the
/// item's cap tag (`HEADER_CAP`) is Square (2): a square end's corners lie a diagonal
/// half-width from its endpoint, so an edge that far from a row can still cover a pixel
/// on it. Any other tag leaves `half` unchanged, so every other item's band index is
/// what it was. `skip` is a fresh label; clobbers `int_scratch` and `fp_scratch`.
pub(crate) fn emit_square_cap_reach(
    header: abi::Reg,
    half: abi::Reg,
    int_scratch: abi::Reg,
    fp_scratch: abi::Reg,
    skip: &str,
) -> Vec<CodeInstruction> {
    vec![
        abi::load_double(fp_scratch, header, HEADER_CAP * 8),
        abi::float_convert_to_signed_x(int_scratch, fp_scratch),
        abi::compare_immediate(int_scratch, "2"),
        abi::branch_ne(skip),
        abi::move_immediate(int_scratch, "Integer", &SQRT_2_F64_BITS.to_string()),
        abi::float_move_d_from_x(fp_scratch, int_scratch),
        abi::float_multiply_d(half, half, fp_scratch),
        abi::label(skip),
    ]
}

/// plan-163-A: a polygon's or a curve polygon's `FillRule` ordinal
/// (`HEADER_POLYGON_RULE`, 0 EvenOdd, 1 NonZero) into `arc.y` (`ITEM_ARC_REFLEX`) of the item block at `block_at` from the
/// stack pointer. Only an arc reads `arc.y` as its reflex flag, so a polygon's copy of
/// the word is free; call this AFTER [`emit_item_arc_flags`], which writes the flag for
/// every kind. Any other kind is skipped and keeps the flag. Stroked text is a polygon
/// with slot 21 at 0, so it reads EvenOdd. `skip` is a fresh label. Clobbers
/// `FP_SCRATCH[1]` and `SCRATCH[1]`.
pub(crate) fn emit_item_polygon_rule(
    header: abi::Reg,
    block_at: usize,
    skip: &str,
) -> Vec<CodeInstruction> {
    // plan-164-C: a `CurvePolygon` keeps its rule in the same slot and reads it from the
    // same word (`curveWalk`).
    let has_rule = format!("{skip}_has");
    vec![
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_KIND * 8),
        abi::float_convert_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]),
        abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_POLYGON),
        abi::branch_eq(&has_rule),
        abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_CURVEPOLYGON),
        abi::branch_ne(skip),
        abi::label(&has_rule),
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_POLYGON_RULE * 8),
        abi::float_convert_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]),
        abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            block_at + ITEM_OFFSET_ARC + ITEM_ARC_REFLEX,
        ),
        abi::label(skip),
    ]
}
/// The gradient's kind (0 linear, 1 radial) in `surface.w`, which was the block's last
/// spare word -- since plan-163-D packed with the spread and the stroke apply as `kind |
/// spread << 1 | apply << 3` (spread 0 Pad, 1 Reflect, 2 Repeat), which the shaders
/// unpack with `& 1`, `>> 1 & 3` and `& 8`.
pub(crate) const ITEM_SURFACE_GRADIENT_KIND: usize = 12;
/// plan-163-D: `__CANVAS_GEO_GRADIENT_SPREAD`, `_APPLY`, the radial inverse's first word
/// (`_INV`, six float32 bit patterns) and its flag (`_HAS_INV`), and the focal offset's x
/// (`_FOCALX`, then y and the focal radius).
pub(crate) const HEADER_GRADIENT_SPREAD: usize = 55;
pub(crate) const HEADER_GRADIENT_APPLY: usize = 56;
pub(crate) const HEADER_GRADIENT_INV: usize = 57;
pub(crate) const HEADER_GRADIENT_HAS_INV: usize = 63;
pub(crate) const HEADER_GRADIENT_FOCAL_X: usize = 64;
/// The gradient's axis in the geometry header — `startPoint.x` at 43, running to
/// `endPoint.y` at 46 — and its kind at 42.
pub(crate) const HEADER_GRADIENT_KIND: usize = 42;
pub(crate) const HEADER_GRADIENT_FROM_X: usize = 43;
/// An ellipse's rotation in the geometry header, as its cosine then its sine
/// (plan-116-E). Evaluated once per ellipse by `__canvas_ellipseHeader` with the
/// deterministic Taylor pair, so all three renderers read the same two numbers.
/// The gradient's stop count in the geometry header, 0 when there is none
/// (plan-116-F). The header also carries the kind and the two points at 42–46; the
/// emitters need only the count, because the shader reads the rest out of the block.
pub(crate) const HEADER_GRADIENT_COUNT: usize = 41;
/// The gradient's stop count and first-stop index, in the item block's `ellipse`
/// `ivec4` (plan-116-F).
///
/// Words 2 and 3 of that block were spare: an ellipse reads only `cos` and `sin` from
/// it, and no item is an ellipse *and* something else. Reusing them is the same
/// per-kind sharing `ITEM_OFFSET_ARC` already does three ways (edge base, glyph
/// height, cap), and it keeps `ITEM_BLOCK_SIZE` at 192 — which matters more than it
/// looks, because growing the block shifts Metal's whole hand-assigned frame and
/// `METAL_EDGE_BASE` with it (`.ai/canvas-threading.md`).
pub(crate) const ITEM_ELLIPSE_GRADIENT_COUNT: usize = 8;
pub(crate) const ITEM_ELLIPSE_GRADIENT_BASE: usize = 12;
pub(crate) const HEADER_ELLIPSE_COS: usize = 39;
pub(crate) const HEADER_ELLIPSE_SIN: usize = 40;
/// bug-1083: the two words of the item block that carry the box's rim (`HEADER_RIM`),
/// float32 bits like the box itself: the x sides' in `xform1.w` and the y sides' in
/// `gradInv1.w`. Only the vertex stage reads them, and only under a group matrix.
///
/// They are the fourth word of each inverse's second `ivec4`, which nothing wrote: an
/// inverse is six floats and a flag. Taking them is what keeps `ITEM_BLOCK_SIZE` where
/// it is, and with it the fragment stage's SPIR-V and both backends' frame layouts: the
/// vertex stage is the only shader text this bug changes. A fifth pair of the per-kind
/// kind of sharing `ITEM_OFFSET_ARC` does would not do, since every kind has a box.
pub(crate) const ITEM_RIM_WORDS: [usize; 2] =
    [ITEM_OFFSET_TRANSFORM + 28, ITEM_OFFSET_GRADIENT_INV + 28];
// The `hasTransform` flag is the seventh word from `ITEM_OFFSET_TRANSFORM`, written
// positionally by the same loop that writes the six terms — a named offset for it
// would be a second way to say where it lives, and one of the two would eventually be
// wrong. The shaders read it as `xform1.z`, which the struct comment records.

/// `__CANVAS_GEO_POLYGON` — the one geometry kind whose payload does not fit in the
/// item block, so both backends have to test for it by hand.
///
/// The rest of the dispatch happens inside the shaders, which read `misc.x`; this is
/// here because the *emitters* branch on it too, to decide whether to build an edge
/// payload at all. Spelled once so a renumbering in `helper_geometry/`'s
/// `__CANVAS_GEO_POLYGON` cannot leave two backends disagreeing with the source of
/// truth in different ways.
pub(crate) const GEO_KIND_POLYGON: &str = "4";

/// `__CANVAS_GEO_TEXT` — a glyph run, the other kind whose payload does not fit in the
/// item block, and the only one that is not *one* draw.
///
/// A run's tail is `(cacheEntry, penX, penY)` per glyph, and each glyph is its own quad
/// with its own coverage bitmap, so a text item becomes N draws rather than one. Both
/// emitters therefore branch on this before they build an item block at all.
pub(crate) const GEO_KIND_TEXT: &str = "6";

/// A `canvas::Group` node's kind (plan-116-G).
///
/// A group draws nothing itself, so this is not a shape the renderers dispatch on —
/// it is what lets a group node be told apart from `GEO_KIND_NONE`, which means "an
/// item with no geometry". Spelled here and once in MFBASIC as `__CANVAS_GEO_GROUP`,
/// with `the_geo_layout_constants_match_their_rust_counterparts` as the only thing
/// relating the two.
///
/// `#[cfg(test)]` because that guard is genuinely its only consumer right now: no
/// emitter writes kind 8 yet, since a Phase 2 group node still carries an empty
/// header. That is not a placeholder — the pin is the whole reason the Rust spelling
/// exists at this point — and it is self-correcting: the phase that teaches an emitter
/// about groups cannot compile until the attribute comes off.
#[cfg(test)]
pub(crate) const GEO_KIND_GROUP: &str = "8";
/// `__CANVAS_GEO_PICTURE` — a `canvas::Picture` (bug-484).
///
/// To the shaders a rectangle whose fill colour is sampled from an image; to the
/// emitters the third kind with a payload, after the polygon's edges and the glyph's
/// coverage. Its texels ride the frame buffer's **glyph region**, one packed RGBA word
/// per texel (`r | g << 8 | b << 16 | a << 24`, the shadow's own byte order), and the
/// block names its slice exactly as a glyph's does: width in `misc.w`, height in
/// `arc.x` (`ITEM_ARC_GLYPH_HEIGHT`), base in `arc.z` (`ITEM_ARC_EDGE_BASE`). Sharing
/// the glyph region rather than adding a fourth means sharing its cursor and its frame
/// cap, and the predicates count a picture's texels against that cap.
pub(crate) const GEO_KIND_PICTURE: &str = "9";
/// `__CANVAS_GEO_POLYLINE` — a `canvas::PolyLine` (plan-154-A).
///
/// Its tail is the polygon edge layout (without the wrap edge) followed by its per-point
/// colours as gradient stops, so an emitter that uploads it needs the polygon's edge path
/// and the gradient's stop path, and nothing new.
///
/// Spelled here and once in MFBASIC; `the_geo_layout_constants_match_their_rust_counterparts`
/// holds the two equal.
pub(crate) const GEO_KIND_POLYLINE: &str = "10";
/// `__CANVAS_GEO_POLYCURVE` and `__CANVAS_GEO_CURVEPOLYGON` — a `canvas::PolyCurve` and
/// a `canvas::CurvePolygon` (plan-164-A). Two kinds over one tail of rational-quadratic
/// pieces, as `POLYLINE` and `POLYGON` are two kinds over one edge layout: the kind says
/// whether there is an interior.
///
/// Spelled here and once in MFBASIC;
/// `the_geo_layout_constants_match_their_rust_counterparts` holds the two equal.
pub(crate) const GEO_KIND_POLYCURVE: &str = "11";
pub(crate) const GEO_KIND_CURVEPOLYGON: &str = "12";
/// `__CANVAS_GEO_TILE` — a source rectangle of a `canvas::Image` drawn into a
/// destination rectangle (plan-173).
///
/// Like `Picture`, its texels ride the glyph region. Its four source-rectangle values
/// follow the fixed geometry header and are copied into the otherwise-unused
/// `ITEM_OFFSET_GRADIENT` words as float32 bits for both GPU shaders.
pub(crate) const GEO_KIND_TILE: &str = "13";
/// The doubles one piece of a curve record's tail occupies: `x0, y0, x1, y1, x2, y2,
/// w1` (`__CANVAS_CURVE_PIECE` in `builtins/canvas/helper_curves/pieces.rs`, which
/// `canvas_curve_piece_width_is_pinned` holds equal to this).
///
/// On both GPUs a piece is TWO slots of the frame's edge region (plan-164-C): its
/// chord `x0, y0, x2, y2` among the item's first `count` slots, where the band index
/// reads it as the edge it is in y; and `x1, y1, w1, 0` in the `count` slots after.
pub(crate) const CURVE_PIECE_SLOTS: usize = 7;
/// A polyline's first and last segments with length, in the header's gradient axis
/// slots (`__CANVAS_GEO_POLYLINE_FIRST` / `_LAST`), which a polyline does not otherwise
/// use.
///
/// The emitters copy them into the item block's `gradient.x`/`.y` as **whole numbers**,
/// not the float32 bits the axis is written as: the shaders compare them with segment
/// indices, which are integers. The butt planes cut only these
/// two segments, each on its outer end.
pub(crate) const HEADER_POLYLINE_FIRST: usize = 43;
pub(crate) const HEADER_POLYLINE_LAST: usize = 44;
/// The header slots holding a picture's pixel-block address, high and low 24 bits.
///
/// Split because every header slot is hashed through `__canvas_hashFloat`, which
/// scales by 65536 before `toInt` — a whole 48-bit address overflows it. The emitters
/// rebuild the address as `hi << PICTURE_SHADOW_SPLIT_BITS | lo`; the block is a
/// `List OF Byte`, so the texels start `COLLECTION_HEADER_SIZE` bytes in.
pub(crate) const HEADER_PICTURE_SHADOW_HI: usize = 35;
pub(crate) const HEADER_PICTURE_SHADOW_LO: usize = 36;
pub(crate) const PICTURE_SHADOW_SPLIT_BITS: usize = 24;
/// Floats per glyph in a `__CANVAS_GEO_TEXT` tail: `cacheEntry, penX, penY`.
pub(crate) const GLYPH_RUN_SLOTS: usize = 3;
/// Integers per `__CANVAS_GLYPH_META` entry: `x0, y0, w, h, covStart`.
///
/// `x0`/`y0` are the bitmap's offset from the pen, which is what lets the same bitmap
/// serve the same glyph wherever it lands; `covStart` indexes `__CANVAS_GLYPH_COV`.
pub(crate) const GLYPH_META_SLOTS: usize = 5;
pub(crate) const GLYPH_META_X0: usize = 0;
pub(crate) const GLYPH_META_Y0: usize = 1;
pub(crate) const GLYPH_META_W: usize = 2;
pub(crate) const GLYPH_META_H: usize = 3;
pub(crate) const GLYPH_META_START: usize = 4;

/// Slots of `__canvas_headerFor`'s fixed 51-float geometry header that the GPU
/// backends read. The software rasteriser indexes the same layout.
pub(crate) const HEADER_KIND: usize = 0;
pub(crate) const HEADER_SHAPE: usize = 2;
pub(crate) const HEADER_RADIUS: usize = 6;
pub(crate) const HEADER_STROKE_HALF: usize = 7;
pub(crate) const HEADER_FILL_R: usize = 8;
pub(crate) const HEADER_STROKE_R: usize = 12;
pub(crate) const HEADER_BOUNDS: usize = 16;
/// Slot 20 is the polygon's edge count *and* the arc's start angle — the header
/// reuses it per kind, and so does the item block. Writing both unconditionally is
/// cheaper than branching and can never be wrong: a shader reads only the one its
/// `kind` selects.
pub(crate) const HEADER_AUX0: usize = 20;
pub(crate) const HEADER_AUX1: usize = 21;
/// A polygon's `canvas::FillRule` ordinal (0 EvenOdd, 1 NonZero) — `HEADER_AUX1`
/// under its polygon meaning (plan-163-A, `__CANVAS_GEO_POLYGON_RULE`). Stroked text
/// is a polygon too and leaves it 0.
pub(crate) const HEADER_POLYGON_RULE: usize = HEADER_AUX1;
/// The item's clip rectangle, **resolved** to `x0, y0, x1, y1` in surface pixels
/// (plan-116-B).
///
/// Resolved rather than `x/y/w/h` so neither the rasteriser nor either shader repeats
/// the addition — the clip is tested once per covered pixel on the boundary and once
/// per item everywhere else, and `x + w` is not a term worth recomputing there.
///
/// A zero-area clip means **no clipping**, which is what an unset `Paint.clip` reads
/// as (`canvas::Paint.clip`'s description). It is stored as four zeros and recognised
/// by `x0 >= x1 OR y0 >= y1` — a test that also catches a caller passing a negative
/// extent, which `Bounds` cannot forbid.
pub(crate) const HEADER_CLIP_X0: usize = 22;
pub(crate) const HEADER_CLIP_Y0: usize = 23;
pub(crate) const HEADER_CLIP_X1: usize = 24;
pub(crate) const HEADER_CLIP_Y1: usize = 25;
/// The item's transform, **already inverted**, as `ia, ib, ic, id, itx, ity`
/// (plan-116-C).
///
/// Applied as `x' = ia*x + ic*y + itx`, `y' = ib*x + id*y + ity` — the same convention
/// `canvas::Transform` documents for the forward matrix.
///
/// Inverted **once, on the CPU, at header-build time**, because the renderers need
/// `T⁻¹` and nothing else: a shape is drawn by evaluating its distance field at the
/// inverse-mapped query point. Inverting per pixel, or per frame in each of three
/// renderers, would be the same arithmetic done between 1 and 10^6 times more often —
/// and would be three places for the all-zero-means-identity rule to live.
/// `__canvas_invertTransform` is that single place.
pub(crate) const HEADER_TRANSFORM_IA: usize = 27;
pub(crate) const HEADER_TRANSFORM_IB: usize = 28;
pub(crate) const HEADER_TRANSFORM_IC: usize = 29;
pub(crate) const HEADER_TRANSFORM_ID: usize = 30;
pub(crate) const HEADER_TRANSFORM_ITX: usize = 31;
pub(crate) const HEADER_TRANSFORM_ITY: usize = 32;
/// 1 when the item carries a transform that is not the identity, 0 otherwise
/// (plan-116-C).
///
/// A flag rather than six compares, because the per-pixel gate is what an *untransformed*
/// item pays — and that is every item in every scene written before this letter. It is
/// also what gates the two extra distance evaluations the gradient correction needs
/// (`ITEM_OFFSET_TRANSFORM`), which are the real cost.
pub(crate) const HEADER_HAS_TRANSFORM: usize = 33;
/// The item's `BlendMode`, as the enum tag 0..3 (plan-116-B).
///
/// `BlendMode.Normal` is 0, so an unset `Paint.blend` is the source-over behaviour
/// every scene had before this field was read — the zero value is the no-op, the same
/// rule the rest of `Paint` follows.
pub(crate) const HEADER_BLEND: usize = 26;
/// The item's `CapStyle`, as the enum tag 0..1 (plan-116-D).
///
/// Only `Line` and `Arc` write it; every other kind leaves it at the blank header's
/// zero. That is safe rather than merely unread — a kind with no ends never reaches a
/// cap branch in any of the three renderers.
///
/// **The zero is not "today's behaviour" for both variants**, which is the thing to
/// know here. A `Line` was round before this letter (`__canvas_segmentDistance` clamps
/// `t` to `0..1`) and an `Arc` was butt (the sweep test cuts it along a radius), so
/// preserving today's rendering meant splitting the existing sites: `Line` → `Round`,
/// `Arc` → `Butt`. `Butt` is still the enum's zero, because that is how the feature was
/// specified and a defaulted `cap` cannot arise — MFBASIC named construction requires
/// every field.
pub(crate) const HEADER_CAP: usize = 34;
/// An arc's two sweep endpoints in surface pixels, `startX, startY, endX, endY`
/// (plan-116-D).
///
/// Written by every arc and, on the GPU since bug-735, read by every arc (the sweep
/// test's direction vectors; the software rasteriser reads them for a `Round` cap
/// only), because a per-shape constant computed once at header-build time is the cheap
/// half of this letter: a cap disc centred on the endpoint would otherwise need a `sin`/`cos` pair
/// **per pixel** in each of three renderers, and `__canvas_cos`/`__canvas_sin` are the
/// deterministic Taylor series the oracle needs rather than libm. The arc header
/// already calls them for its sweep vectors, so this costs two multiply-adds each.
pub(crate) const HEADER_CAP_START_X: usize = 35;
pub(crate) const HEADER_CAP_END_X: usize = 37;
/// The fixed header length in slots — where a polygon's edge tail begins.
///
/// Spelled again in MFBASIC as `__CANVAS_GEO_HEADER` (`helper_geometry/mod.rs`), with no
/// compiler between the two; `the_geo_layout_constants_match_their_rust_counterparts`
/// is what keeps them equal. Changing this without changing that one makes a polygon's
/// first edge coordinate read as a header field.
pub(crate) const HEADER_SLOTS: usize = 69;
pub(crate) const TILE_SOURCE_X: usize = HEADER_SLOTS;
pub(crate) const TILE_SOURCE_Y: usize = HEADER_SLOTS + 1;
pub(crate) const TILE_SOURCE_W: usize = HEADER_SLOTS + 2;
pub(crate) const TILE_SOURCE_H: usize = HEADER_SLOTS + 3;
/// bug-1083: `__CANVAS_GEO_RIM` -- how much of the box in `HEADER_BOUNDS` is rim, on its
/// x sides and (the slot after) on its y sides, in the units the box is in. Zero for a
/// box that is not an extent and a rim (a glyph run, a picture, an arc).
pub(crate) const HEADER_RIM: usize = 67;
/// plan-163-B: `__CANVAS_GEO_JOIN_COUNT` -- a stroke's join records in the tail, one
/// per outline edge -- and `__CANVAS_GEO_JOIN_REACH`, how far a record reaches from its
/// vertex, which a polygon's or polyline's band index widens to.
pub(crate) const HEADER_JOIN_COUNT: usize = 47;
pub(crate) const HEADER_JOIN_REACH: usize = 48;
/// plan-163-B: `__CANVAS_GEO_DASH_COUNT` -- the dash tail's floats, which sit between
/// the join records and the gradient stops -- and `__CANVAS_GEO_DASH_PHASE`, the dash
/// offset wrapped into one period, which the emitters upload as the first dash word.
pub(crate) const HEADER_DASH_COUNT: usize = 49;
pub(crate) const HEADER_DASH_PHASE: usize = 50;
/// plan-163-C: `__CANVAS_GEO_CLIP_EDGE_COUNT` -- a path clip's edges, five floats each,
/// which sit between the dash tail and the gradient stops.
pub(crate) const HEADER_CLIP_EDGE_COUNT: usize = 52;
/// plan-163-C: `__CANVAS_GEO_CLIP_MODE`, `__CANVAS_GEO_CLIP_RULE` and
/// `__CANVAS_GEO_PICTURE_TRANSPARENCY`, copied into the item block's `clipPath`.
pub(crate) const HEADER_CLIP_MODE: usize = 51;
pub(crate) const HEADER_CLIP_RULE: usize = 53;
pub(crate) const HEADER_PICTURE_TRANSPARENCY: usize = 54;
/// Doubles per cached polygon edge: `x0, y0, dx, dy, invLenSq`.
pub(crate) const EDGE_SLOTS: usize = 5;
/// The most edges one **pass** may carry on the Vulkan path — the size of the frame
/// buffer's edge region (bug-765: a frame past it is drawn as several passes).
///
/// Vulkan has no per-item limit — the edges live in a storage buffer, not in the
/// push constants — so the real constraint is the buffer, and it is a whole-buffer
/// one: every polygon of a pass takes a slice. `__canvas_passNext` sums the blocks'
/// edges (`__canvas_blockCost`) against this and ends the pass before the total
/// passes it, and a block past it alone sends the frame to software, so the emitter's
/// own bound check is unreachable.
///
/// 262,144 since bug-688 (it was 16,384, which `examples/weather`'s coastline alone
/// exceeds): Metal's number since bug-686, and 4 MiB of region. A large polygon no
/// longer costs every edge per covered pixel — the band index (`VULKAN_BAND_BASE_WORDS`)
/// has the shader walk only its own band's edges.
pub(crate) const VULKAN_MAX_FRAME_EDGES: usize = 262_144;
/// Four float32 words per edge — the two endpoints.
pub(crate) const VULKAN_EDGE_BYTES: usize = VULKAN_MAX_FRAME_EDGES * 16;

/// The most **item blocks** one Vulkan frame may carry — the capacity of the item
/// buffer.
///
/// A *drawn quad*, not a scene item: every non-text item takes one block, and a glyph
/// run takes one per glyph, because each glyph is its own quad with its own block
/// (`GEO_KIND_TEXT`). So the count both predicates sum is "quads", and that is the
/// number this bounds.
///
/// It is the size of the frame BUFFER's item region, and so of one PASS (bug-765):
/// `__canvas_passNext` sums a frame's quads against it and ends a pass before the
/// total passes it. Only a block of more quads than this by itself (a `Text` of more
/// glyphs) sends the whole frame to software, for the reason `VULKAN_MAX_FRAME_EDGES`
/// gives: a truncated scene is a *different scene*, and software is the oracle, so
/// declining is never worse than drawing.
///
/// 65,536 since bug-688, Metal's `METAL_MAX_FRAME_ITEMS`: 4,096 (the old shared
/// `CANVAS_MAX_FRAME_ITEMS`) sent `examples/weather` (about 5,700 blocks), a 5,000-quad
/// scene and a terminal screen of text to software. 13 MiB of item buffer at the
/// current `ITEM_BLOCK_SIZE`, host-visible and mapped for the renderer's life.
pub(crate) const VULKAN_MAX_FRAME_ITEMS: usize = 65536;

/// The most **item blocks** one Metal frame may carry — `VULKAN_MAX_FRAME_ITEMS`'s
/// Metal twin (bug-686; the Vulkan cap was the shared `CANVAS_MAX_FRAME_ITEMS` until bug-688).
///
/// 4096 quads sent ordinary scenes to software: a terminal screen of text is one quad
/// per glyph, `examples/weather` publishes about 5,700 blocks a frame, and the 1,000-item
/// scenes bug-686 measured were a quarter of the way there. 65,536 blocks is 13 MiB of
/// item buffer at the current `ITEM_BLOCK_SIZE`, and every frame region after it moves
/// with it — which is why the shader's region bases are generated from these constants
/// rather than spelled in the MSL (`metal_shader_source`).
///
/// A frame past it is drawn as several passes (bug-765, `__canvas_passNext`); the
/// MFBASIC copy is generated from this constant (`helper_render/mod.rs`, `RENDER_CAPS`).
pub(crate) const METAL_MAX_FRAME_ITEMS: usize = 65536;

/// How many 64-bit words one entry of the GPU draw list occupies.
///
/// The draw list is built in MFBASIC (`__canvas_pushOneDraw`) and walked by the native
/// emitters, so this width exists on both sides of a boundary the Rust compiler cannot
/// see across -- the MFBASIC side is a `&str`. It is a power of two because the emitter
/// addresses an entry with a shift rather than a multiply.
///
/// When the two sides disagreed (plan-116-H13: the emitter at eight words, the builtin
/// still appending four) the emitter strode 64 bytes through a 32-byte array. It then
/// read every *other* entry, and took the following entry's `base` as a blend mode --
/// which indexes the pipeline table out of range and hands Vulkan a junk `VkPipeline`.
/// That does not fail as a wrong picture: it SIGSEGVs inside the driver's JIT-compiled
/// code, with no MFBASIC frame anywhere in the backtrace.
///
/// plan-172-G: sixteen words. The first five are what an entry always was — block base
/// and count, the translation as float32 bits, the blend mode — and words 5–6, which
/// held the translation's 16.16 residuals until the shaders went to fp32, are reserved
/// and zero, so a translate-only entry is its first five words followed by zeros. Word 7 says
/// whether the entry carries a matrix, and words 8–11 are that matrix: the linear part
/// of the INVERSE of the enclosing groups' composed affine, as float32 bit patterns.
/// Word 12 is the pass word ([`CANVAS_DRAW_PASS_WORD`]); words 13–15 are reserved and
/// zero.
pub(crate) const CANVAS_DRAW_ENTRY_WORDS: usize = 16;

/// bug-765: the word of a draw list's FIRST entry that says where the list stands in
/// its frame, for a frame drawn as several passes (`__canvas_passNext`). An emitter is
/// handed one pass's blocks and entries exactly as it is handed a whole frame's, and
/// this word is the only thing that tells the two apart:
///
/// * [`CANVAS_DRAW_PASS_CONTINUES`] — a pass of this frame was drawn before this one.
///   The target holds it, so it is LOADED, not cleared, and the image a direct present
///   draws into is the one the first pass acquired.
/// * [`CANVAS_DRAW_PASS_MORE`] — a pass follows. This one is submitted and waited for
///   (the frame buffer is single-buffered, and the next pass refills it), but the frame
///   is neither presented nor read back.
///
/// Zero is a frame of one pass, which is every list `__canvas_pushOneDraw` and
/// `canvas::sceneDrawsFlat` lay out: they write 0 here, so such a frame is drawn by the
/// instructions that drew it before there were passes.
///
/// It rides the list because the list is what makes a pass a pass, and because the
/// seam's argument bank is full (`func_metal_draw.rs`). It is its own word for the
/// reason the blend mode is: a flag packed into the high bits of `count` survives
/// exactly until someone draws that many instances.
pub(crate) const CANVAS_DRAW_PASS_WORD: usize = 12;
pub(crate) const CANVAS_DRAW_PASS_CONTINUES: usize = 1;
pub(crate) const CANVAS_DRAW_PASS_MORE: usize = 2;

/// `log2(CANVAS_DRAW_ENTRY_WORDS * 8)` -- entry index to byte offset.
pub(crate) const CANVAS_DRAW_ENTRY_SHIFT: u32 = CANVAS_DRAW_ENTRY_WORDS.trailing_zeros() + 3;

/// `log2(CANVAS_DRAW_ENTRY_WORDS)` -- element count to entry count.
pub(crate) const CANVAS_DRAW_ENTRY_COUNT_SHIFT: u32 = CANVAS_DRAW_ENTRY_WORDS.trailing_zeros();

/// Byte offset of the blend mode within an entry (word 4).
pub(crate) const CANVAS_DRAW_ENTRY_MODE: usize = 32;
/// Vulkan's item buffer in bytes — one `ITEM_BLOCK_SIZE` record per quad, for
/// `VULKAN_MAX_FRAME_ITEMS` quads. A buffer of its own (binding 1), unlike Metal's item
/// region, which heads its one frame buffer.
pub(crate) const VULKAN_ITEM_BUFFER_BYTES: usize = VULKAN_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE;
/// Metal's item region in bytes — one `ITEM_BLOCK_SIZE` record per quad, for
/// `METAL_MAX_FRAME_ITEMS` quads. The first region of the Metal frame buffer.
pub(crate) const METAL_ITEM_BUFFER_BYTES: usize = METAL_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE;

/// The most edges one **frame** may carry on the Metal path, mirroring
/// `VULKAN_MAX_FRAME_EDGES` (plan-116-A).
///
/// Metal's edges used to ride a per-item `setFragmentBytes:` payload, which is copied
/// into the command buffer at record time — so the cap was per *item* (256 edges)
/// and there was no frame total at all. An instanced draw cannot rebind that payload
/// between instances, so the edges moved into a region of the frame buffer, exactly
/// where Vulkan has always kept them, and the cap became a frame total to match.
///
/// A Metal scene whose polygon edges sum past this is declined to software. It was
/// 16384 until bug-686, which `examples/weather`'s coastline alone exceeds; 262,144 edges
/// is 4 MiB of region, and a 40,000-edge frame measured within max channel delta 1 of
/// the software oracle (bug-686 spike C). Software is the oracle, so a declined frame
/// is at least as correct — truncating instead would draw a *different shape*.
///
/// There is **no per-polygon cap** since bug-686. The 256-edge one was kept "by policy"
/// after plan-116-A moved the edges into this region, and it existed twice — in the
/// predicate and as a branch in `emit_edge_buffer` that drew an over-long polygon as
/// nothing; spike B drew 300-4,000-edge polygons on Metal within max channel delta 1.
pub(crate) const METAL_MAX_FRAME_EDGES: usize = 262_144;
/// Where Metal's edge region starts inside the frame buffer, in 32-bit words.
///
/// The item blocks come first, so this is simply past them. The shader adds it to each
/// polygon's `ITEM_ARC_EDGE_BASE` rather than reading a separately-offset binding,
/// which is the same shape `VULKAN_GLYPH_BASE_WORDS` already uses — and it sidesteps
/// `MTLBuffer` offset alignment entirely, since nothing is ever bound at a non-zero
/// offset. `the_metal_shader_region_bases_match_the_buffer_layout` pins the number
/// against the copy inside the MSL string.
pub(crate) const METAL_EDGE_BASE_WORDS: usize = METAL_ITEM_BUFFER_BYTES / 4;

/// A polygon's **band index** (bug-686): which of its edges a pixel at a given height
/// has to visit, so the fragment shader's `edgeDistance` stops looping over every edge
/// for every covered pixel — O(edges × pixels), 252 ms of GPU for one 64,000-edge
/// polygon (bug-686 section E).
///
/// `emit_band_index` splits the polygon's shape-space y-range into bands and puts each
/// edge in every band its `[ymin − r, ymax + r]` meets, r being the distance past which
/// an edge cannot change the pixel's coverage. The shader evaluates the SAME loop body
/// over its own band's list, so the result is bit-identical to the full loop (the
/// argument is at `emit_band_index`). Per polygon, at the word offset its block carries
/// in `ITEM_BAND_START`:
///
/// * `2 × bands` words of table — `(first, count)` per band, `first` relative to the
///   list that follows;
/// * the list — edge numbers, `0..edgeCount`, band after band.
///
/// The polygon's block carries `bandTop`, `bandHeight` (float32 bits, shape space), the band
/// count and that offset in its `arcCaps` `ivec4`, which only an arc reads
/// (`ITEM_BAND_*`). A band count
/// of 0 means "loop over every edge": what a polygon under `BAND_MIN_EDGES` gets,
/// and what any polygon gets whose index would not fit — correct, only slower. The
/// index is never truncated and a frame is never declined for it.
///
/// Budget: bands are at most the polygon's edge count, and the list at most
/// `BAND_LIST_PER_EDGE` entries per edge (a polygon over it gets taller bands), so
/// one polygon needs at most `BAND_WORDS_PER_EDGE` words per edge — and the frame's
/// edges are capped at `METAL_MAX_FRAME_EDGES` (`VULKAN_MAX_FRAME_EDGES`), so each
/// backend's band region (`METAL_MAX_FRAME_BAND_WORDS`, `VULKAN_MAX_FRAME_BAND_WORDS`)
/// always has room. Both backends build the same index (bug-688 ported it to Vulkan).
pub(crate) const BAND_LIST_PER_EDGE: usize = 6;
/// Table (two words per band, at most one band per edge) plus list, per edge.
pub(crate) const BAND_WORDS_PER_EDGE: usize = 2 + BAND_LIST_PER_EDGE;
/// The band region's size in 32-bit words.
pub(crate) const METAL_MAX_FRAME_BAND_WORDS: usize = BAND_WORDS_PER_EDGE * METAL_MAX_FRAME_EDGES;
/// The most bands one polygon is split into. Past ~1,000 the table costs more to build
/// than a thinner list saves; a 600 px polygon gets 2-px bands up to here.
pub(crate) const BAND_MAX: usize = 1024;
/// Polygons with fewer edges than this loop over all of them (band count 0): the full
/// loop over a handful of edges costs less than finding the band.
pub(crate) const BAND_MIN_EDGES: usize = 32;
/// Where Metal's band region starts, in 32-bit words — right after the edges it indexes.
pub(crate) const METAL_BAND_BASE_WORDS: usize = METAL_EDGE_BASE_WORDS + METAL_MAX_FRAME_EDGES * 4;
/// The most gradient stops one **Vulkan** frame may carry (plan-116-F; bug-688).
///
/// Two stops per quad of `VULKAN_MAX_FRAME_ITEMS`, as Metal's is per quad of its own:
/// a gradient has at least two, so a frame of nothing but simple gradient-filled shapes
/// is declined by the item cap before this one. It was 4,096 on both backends until
/// bug-686 found 2,100 two-stop gradient rectangles — an ordinary chart — declined by it.
pub(crate) const VULKAN_MAX_FRAME_GRADIENT_STOPS: usize = 2 * VULKAN_MAX_FRAME_ITEMS;
/// The most gradient stops one **Metal** frame may carry (bug-686).
///
/// Two stops per quad of `METAL_MAX_FRAME_ITEMS`: a gradient has at least two, so this
/// is the first cap at which a frame of nothing but simple gradient-filled shapes is
/// declined by the item cap rather than by this one. 2.5 MiB of region.
pub(crate) const METAL_MAX_FRAME_GRADIENT_STOPS: usize = 2 * METAL_MAX_FRAME_ITEMS;
/// Five 32-bit words a stop: offset, then the four colour channels.
pub(crate) const GRADIENT_STOP_WORDS: usize = 5;
/// Where Metal's gradient region starts, in 32-bit words — after the items, edges and
/// band index.
pub(crate) const METAL_GRADIENT_BASE_WORDS: usize =
    METAL_BAND_BASE_WORDS + METAL_MAX_FRAME_BAND_WORDS;
/// The most coverage samples one **frame**'s glyphs may carry on the Metal path — the
/// same number, and the same one-sample-per-word layout, as
/// `VULKAN_MAX_FRAME_GLYPH_SAMPLES` (bug-670).
///
/// Metal's glyph bitmaps used to ride a per-glyph `setFragmentBytes:` payload. Once
/// plan-116-H made a text item ONE instanced draw, that payload was overwritten by each
/// glyph's publish before the draw ran, and every glyph drew the run's last bitmap. A
/// frame-wide region, each glyph's slice named by the offset in its own block, is what
/// Vulkan always did, and it is the only shape an instanced run can read.
///
/// `1 << 23` since bug-686 (it was `1 << 20`, Vulkan's size): a picture's texels ride
/// this region too, and one 1024×1024 background plus anything else overflowed a
/// million words. Eight million is a 2560×1440 background with room left for tiles and
/// text, and 32 MiB of region.
pub(crate) const METAL_MAX_FRAME_GLYPH_SAMPLES: usize = 1 << 23;
/// The most dash words one **Metal** frame may carry (plan-163-B), summed over its
/// dashed items: each takes its dash tail (`__CANVAS_GEO_DASH_COUNT` floats) plus the
/// phase, as float32 bit patterns. A frame-wide region like the gradient stops', its
/// slice named in the item block's `strokeStyle.z`/`.w`. `1 << 20` holds three words for
/// every edge of `METAL_MAX_FRAME_EDGES` with room for patterns and ellipse tables
/// (257 words each); 4 MiB of region.
pub(crate) const METAL_MAX_FRAME_DASH_WORDS: usize = 1 << 20;
/// Where Metal's dash region starts, in 32-bit words — after the gradient stops.
pub(crate) const METAL_DASH_BASE_WORDS: usize =
    METAL_GRADIENT_BASE_WORDS + METAL_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS;
/// Where Metal's glyph region starts, in 32-bit words — after the dash words.
pub(crate) const METAL_GLYPH_BASE_WORDS: usize = METAL_DASH_BASE_WORDS + METAL_MAX_FRAME_DASH_WORDS;
/// The whole Metal frame buffer: item blocks, then edges (four float32 words each), then
/// the band index (bug-686), then gradient stops (five each), then dash words (one
/// float32 each, plan-163-B), then glyph coverage (one sample a word), then the picture
/// table (bug-686, `METAL_PICTURE_TABLE_BYTES`).
///
/// A sum of the regions' sizes rather than "the last base plus the last size", so
/// `the_metal_shader_region_bases_match_the_buffer_layout` compares two independent
/// computations and a region left out of the chain shows up there.
pub(crate) const METAL_BUFFER_BYTES: usize = METAL_ITEM_BUFFER_BYTES
    + METAL_MAX_FRAME_EDGES * 16
    + METAL_MAX_FRAME_BAND_WORDS * 4
    + METAL_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4
    + METAL_MAX_FRAME_DASH_WORDS * 4
    + METAL_MAX_FRAME_GLYPH_SAMPLES * 4
    + METAL_PICTURE_TABLE_BYTES;

/// The per-frame **picture table**: which image blocks this frame has already copied
/// into the glyph region, and where (bug-686). The last region of the Metal frame
/// buffer, read only by the emitter — the shader never sees it, so it has no MSL base.
///
/// `emit_picture_buffer` used to copy a picture's texels for every `Picture` item, so a
/// tilemap of 2,000 tiles drawn from two images copied 2,000 images and counted them
/// against the glyph region's cap. It now looks the image's block up here first and, on
/// a hit, points the item at the texels already uploaded; the pass walk
/// (`__canvas_passNext`) counts each distinct block once a pass to match. The table is
/// a PASS's: each pass is one call of the renderer, which starts it empty.
///
/// Two parts, a record array and an open-addressed index over it:
///
/// * `METAL_PICTURE_RECORDS` records of `PICTURE_RECORD_BYTES`:
///   `block` (u64, the pixel block's address), `base` (u32, its first texel in the
///   glyph region), `slot` (u32, the index slot that names this record);
/// * `METAL_PICTURE_INDEX_SLOTS` u32 slots, each a record number, probed linearly from
///   a multiplicative hash of the block address.
///
/// **Nothing is cleared between frames.** A slot is live only if its record number is
/// below this frame's record count AND that record's `slot` names it back — the
/// sparse-set test — so last frame's slots, or whatever the buffer held at creation,
/// read as empty without a pass over the index. Every record this frame writes is
/// written before its slot, and nothing is deleted within a frame, so every live
/// slot's probe chain is intact.
///
/// One record per quad of `METAL_MAX_FRAME_ITEMS` — a picture is one item and every
/// item at least one quad, so the predicate's own cap means the table is never full;
/// the index has twice as many slots, so a probe always reaches an empty one.
pub(crate) const METAL_PICTURE_RECORDS: usize = METAL_MAX_FRAME_ITEMS;
/// Bytes per picture record: block (8), base (4), index slot (4).
pub(crate) const PICTURE_RECORD_BYTES: usize = 16;
/// Slots in the picture index — a power of two, twice the records.
pub(crate) const METAL_PICTURE_INDEX_SLOTS: usize = 2 * METAL_PICTURE_RECORDS;
const _: () = assert!(
    METAL_PICTURE_INDEX_SLOTS.is_power_of_two(),
    "the picture index is masked, not divided"
);
/// Where the picture records start, in bytes — after the glyph region.
pub(crate) const METAL_PICTURE_RECORDS_OFFSET: usize =
    METAL_GLYPH_BASE_WORDS * 4 + METAL_MAX_FRAME_GLYPH_SAMPLES * 4;
/// Where the picture index starts, in bytes — after the records.
pub(crate) const METAL_PICTURE_INDEX_OFFSET: usize =
    METAL_PICTURE_RECORDS_OFFSET + METAL_PICTURE_RECORDS * PICTURE_RECORD_BYTES;
/// The picture table's whole size: records, then index slots.
pub(crate) const METAL_PICTURE_TABLE_BYTES: usize =
    METAL_PICTURE_RECORDS * PICTURE_RECORD_BYTES + METAL_PICTURE_INDEX_SLOTS * 4;

/// The most coverage samples one **frame**'s glyphs may carry on the Vulkan path.
///
/// Glyph bitmaps ride the same buffer as the edges, in a region after them, for the
/// reason a second buffer would have to be justified rather than assumed: it would need
/// its own allocation, its own memory-type search, its own descriptor binding and its
/// own upload, to hold data with exactly the edges' lifetime and exactly their access
/// pattern. One buffer, several regions, one binding.
///
/// One sample per 32-bit word rather than four packed per word. That wastes three
/// quarters of the region and buys a shader arm with no shifting and no masking, in the
/// only place where a packing mistake would be invisible — a wrongly unpacked coverage
/// byte still produces a glyph, just a wrong one.
///
/// `1 << 23` since bug-688 (it was `1 << 20`): a picture's texels ride this region
/// too, and a 1024×1024 background plus anything else overflowed a million words.
/// Metal's `METAL_MAX_FRAME_GLYPH_SAMPLES`, and 32 MiB of region.
pub(crate) const VULKAN_MAX_FRAME_GLYPH_SAMPLES: usize = 1 << 23;

// --- the Vulkan shared buffer (binding 0) ------------------------------------------
//
// Edges from word 0, then the band index, the gradient stops, the dash words, the
// glyph/picture texels and the picture table. The shader reaches the band, gradient,
// dash and glyph regions through BASES THAT ARE SPECIALIZATION
// CONSTANTS (bug-688): the fragment shader declares them `layout(constant_id = N) const int`, and the pipeline feeds the
// values below through `VkSpecializationInfo` (`VULKAN_SPEC_CONSTANTS`). Until then
// `GRADIENT_BASE` and `GLYPH_BASE` were literals in the checked-in GLSL, kept equal to
// these by a unit test — so every cap before them was frozen, because moving one meant
// a hand edit of the shader and a SPIR-V rebuild or a plausible wrong picture.

/// The band region's size in 32-bit words — `BAND_WORDS_PER_EDGE` per edge of
/// `VULKAN_MAX_FRAME_EDGES`, the budget `emit_band_index` holds every polygon to.
pub(crate) const VULKAN_MAX_FRAME_BAND_WORDS: usize = BAND_WORDS_PER_EDGE * VULKAN_MAX_FRAME_EDGES;
/// Where the band region starts, in 32-bit words — right after the edges it indexes.
pub(crate) const VULKAN_BAND_BASE_WORDS: usize = VULKAN_EDGE_BYTES / 4;
/// Where the gradient region starts, in 32-bit words — after the band index.
pub(crate) const VULKAN_GRADIENT_BASE_WORDS: usize =
    VULKAN_BAND_BASE_WORDS + VULKAN_MAX_FRAME_BAND_WORDS;
/// The most dash words one **Vulkan** frame may carry (plan-163-B) — Metal's
/// `METAL_MAX_FRAME_DASH_WORDS`, the same layout.
pub(crate) const VULKAN_MAX_FRAME_DASH_WORDS: usize = 1 << 20;
/// Where the dash region starts, in 32-bit words — after the gradient stops.
pub(crate) const VULKAN_DASH_BASE_WORDS: usize =
    VULKAN_GRADIENT_BASE_WORDS + VULKAN_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS;
/// Where the glyph (and picture texel) region starts, in 32-bit words — after the
/// dash words.
pub(crate) const VULKAN_GLYPH_BASE_WORDS: usize =
    VULKAN_DASH_BASE_WORDS + VULKAN_MAX_FRAME_DASH_WORDS;
/// Vulkan's per-frame picture table (bug-688) — Metal's (`METAL_PICTURE_TABLE_BYTES`)
/// with the same record and index layout, one record per quad of
/// `VULKAN_MAX_FRAME_ITEMS`. The last region of the shared buffer, read only by the
/// emitter: the shader never sees it, so it has no specialization constant.
pub(crate) const VULKAN_PICTURE_RECORDS: usize = VULKAN_MAX_FRAME_ITEMS;
/// Slots in the picture index — a power of two, twice the records.
pub(crate) const VULKAN_PICTURE_INDEX_SLOTS: usize = 2 * VULKAN_PICTURE_RECORDS;
const _: () = assert!(
    VULKAN_PICTURE_INDEX_SLOTS.is_power_of_two(),
    "the picture index is masked, not divided"
);
/// Where the picture records start, in bytes — after the glyph region.
pub(crate) const VULKAN_PICTURE_RECORDS_OFFSET: usize =
    VULKAN_GLYPH_BASE_WORDS * 4 + VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4;
/// Where the picture index starts, in bytes — after the records.
pub(crate) const VULKAN_PICTURE_INDEX_OFFSET: usize =
    VULKAN_PICTURE_RECORDS_OFFSET + VULKAN_PICTURE_RECORDS * PICTURE_RECORD_BYTES;
/// The picture table's whole size: records, then index slots.
pub(crate) const VULKAN_PICTURE_TABLE_BYTES: usize =
    VULKAN_PICTURE_RECORDS * PICTURE_RECORD_BYTES + VULKAN_PICTURE_INDEX_SLOTS * 4;
/// The whole shared buffer: edges, band index, gradient stops,
/// dash words (plan-163-B), glyph/picture texels, picture table.
///
/// A sum of the regions' sizes rather than "the last base plus the last size", so
/// `the_shared_buffer_holds_every_region` compares two independent computations and a
/// region left out of the chain shows up there.
pub(crate) const VULKAN_BUFFER_BYTES: usize = VULKAN_EDGE_BYTES
    + VULKAN_MAX_FRAME_BAND_WORDS * 4
    + VULKAN_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4
    + VULKAN_MAX_FRAME_DASH_WORDS * 4
    + VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4
    + VULKAN_PICTURE_TABLE_BYTES;
/// The fragment shader's specialization constants, as `(constant_id, value)`: the
/// region bases it reads, in words. The ids are the `layout(constant_id = N)` numbers in
/// `shaders/mfb_canvas.frag`; `the_pipeline_specializes_every_region_base` pins the two
/// lists to each other. Id 3 was the edge residual region's base until the shaders went
/// to fp32; the dash base keeps id 4 rather than renumbering.
pub(crate) const VULKAN_SPEC_CONSTANTS: [(u32, usize); 4] = [
    (0, VULKAN_GLYPH_BASE_WORDS),
    (1, VULKAN_GRADIENT_BASE_WORDS),
    (2, VULKAN_BAND_BASE_WORDS),
    (4, VULKAN_DASH_BASE_WORDS),
];
