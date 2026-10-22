#version 450

// plan-98-F: the Vulkan twin of plan-98-E's MSL vertex shader.
//
// The parameter block is byte-identical to the Metal one, so the CPU-side emitter
// that fills it (`emit_item_block`) feeds both backends unchanged: six ivec4s at
// offsets 0,16,32,48,64,80 and the surface size at 96. It is declared as a seventh
// ivec4 rather than an ivec2 so the two languages cannot disagree about trailing
// padding.
//
// plan-116-A: the block arrives in a **storage buffer indexed by instance**, not in a
// push constant. A push constant is per-*draw*, so it could describe only one item —
// which forced one draw per item and pinned the block under Vulkan's guaranteed
// 128-byte range. Both properties had to go: the block is what every later letter of
// plan-116 widens. `ItemBlock`'s std430 array stride is exactly the 112 bytes the
// block already is (every member is an `ivec4`, which std430 aligns and sizes at 16),
// and `the_item_block_matches_the_std430_stride` in `vulkan/tests/mod.rs` pins that agreement.

struct ItemBlock {
    ivec4 quad;    //  0: bounds minX, minY, maxX, maxY (float32 bits)
    ivec4 shape;   // 16: p0..p3 (float32 bits)
    ivec4 fill;    // 32: RGBA 0..255
    ivec4 stroke;  // 48: RGBA 0..255
    ivec4 misc;    // 64: kind, radius, strokeHalf (float32 bits), edgeCount
    ivec4 arc;     // 80: glyph/picture height, arc reflex flag (bug-735) / polygon fill
                   //   rule (plan-163-A), edgeBase, capStyle
    ivec4 surface; //  96: width, height, blendMode, gradient kind (0 linear, 1 radial)
    ivec4 clip;    // 112: the clip rectangle x0,y0,x1,y1 (float32 bits); zero-area = unclipped
    ivec4 xform0;  // 128: inverse transform ia,ib,ic,id as float32 BITS
    ivec4 xform1;  // 144: itx, ity (float32 bits), hasTransform (0 or 1); then
                   //      (bug-1083) how much of the quad's x sides is rim, float32 bits
    ivec4 arcCaps; // 160: an arc's sweep endpoints startX,startY,endX,endY (float32 bits)
    ivec4 ellipse; // 176: an ellipse's rotation cos, sin (float32 bits); then a gradient's
                   //      stop count and first-stop index
    ivec4 gradient; // 192: a gradient's axis startX,startY,endX,endY (float32 bits)
    // plan-163-B: join records' first edge slot and count, dash pattern's first entry
    // and count. Declared here as well as in the fragment stage because THIS declaration
    // sets the array stride (304); the vertex stage reads none of them.
    ivec4 strokeStyle; // 208
    ivec4 clipBand;    // 224, plan-163-C
    ivec4 clipPath;    // 240
    // plan-163-D: a radial gradient's inverse (float32 bits) and flag, and its focal
    // offset and radius (float32 bits). The vertex stage reads none of them, except
    // (bug-1083) `gradInv1.w`: how much of the quad's y sides is rim, float32 bits.
    ivec4 gradInv0;    // 256
    ivec4 gradInv1;    // 272
    ivec4 gradFocal;   // 288
};

layout(std430, set = 0, binding = 1) readonly buffer Items {
    ItemBlock blocks[];
} itemBuf;

// The fragment stage needs the same record this one read. Passing the *index* rather
// than the block costs one flat varying instead of twenty-eight, and it is the index
// the fragment stage would have had to be given anyway — `gl_InstanceIndex` is not
// available there.
layout(location = 0) flat out int vItem;

// plan-116-H: the group translation for THIS draw, as float32 bits.
//
// Per-draw rather than per-item, which is the whole point: a group referenced twice is
// one set of item blocks and two draws differing only by this. Both stages declare it —
// the fragment stage subtracts it before evaluating the distance field — because a
// range no stage consumes is a layout the validation layers reject, which is why
// plan-116-A deleted the range rather than leaving it empty (H4).
//
// `zw` are reserved and zero.
//
// plan-172-H: `inverse` is the linear part of the INVERSE of the enclosing groups'
// composed affine, as four float32 bit patterns — `ia`, `ib`, `ic`, `id`, so a point
// maps as `(ia * x + ic * y, ib * x + id * y)` — and all-zero bits when the groups only
// translate, the convention every `canvas::Transform` has. 32 bytes, both stages.
layout(push_constant) uniform Draw {
    ivec4 offset;
    ivec4 inverse;
} draw;

// A word as the float32 it holds: every position reaches the GPU as float32 bits.
float fx(int v) { return intBitsToFloat(v); }

void main() {
    // `gl_InstanceIndex` includes `firstInstance`, so a run of consecutive items drawn
    // as one instanced `vkCmdDraw(cmd, 4, count, 0, base)` reads blocks `base ..
    // base+count-1` with no other index arithmetic. (Metal's `[[instance_id]]` does
    // NOT include the base instance, which is why that backend offsets the *binding*
    // instead — the one place the two emitters genuinely differ.)
    vItem = gl_InstanceIndex;
    ItemBlock item = itemBuf.blocks[vItem];

    // Four corners of the item's bounds, expanded from gl_VertexIndex — no vertex
    // buffer, exactly as on Metal.
    // plan-116-H: the group offset moves the QUAD, and the fragment stage moves the
    // query point the other way. That is the same split the software renderer makes
    // (plan-116-G §4.5) — bounds by +offset, distance evaluated at p - offset — so one
    // rule covers all three renderers rather than each inventing its own.
    //
    // No clamp to the surface here. The quad may extend past it, and the rasteriser
    // discards what falls outside clip space; the fragment stage's own bounds and clip
    // tests do the rest. A clamp would additionally be wrong: it would shrink the quad
    // that the fragment stage still expects to span the item's full extent.
    vec2 corner = vec2(fx((gl_VertexIndex & 1) == 0 ? item.quad.x : item.quad.z),
                       fx((gl_VertexIndex & 2) == 0 ? item.quad.y : item.quad.w))
                + vec2(fx(draw.offset.x), fx(draw.offset.y));
    // plan-172-H: under a group matrix the quad is the item's box carried through the
    // FORWARD linear part — the inverse of the 2x2 the draw carries — which makes it a
    // parallelogram, and four mapped corners of a triangle strip are exactly that.
    //
    // bug-1083: the box carried is not the block's as it stands. Its rim — `xform1.w`
    // on the x sides, `gradInv1.w` on the y sides, zero for a box that is not an extent
    // and a rim — was sized for the item drawn with no group, and a pixel takes
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
    // instead). Metal's `mfbVertex`, line for line.
    if ((draw.inverse.x | draw.inverse.y | draw.inverse.z | draw.inverse.w) != 0) {
        vec4 gi = vec4(intBitsToFloat(draw.inverse.x), intBitsToFloat(draw.inverse.y),
                       intBitsToFloat(draw.inverse.z), intBitsToFloat(draw.inverse.w));
        float pad = length(gi);
        vec2 rim = vec2(fx(item.xform1.w), fx(item.gradInv1.w));
        vec2 grow = rim * pad - rim + pad;
        vec2 local = vec2(fx((gl_VertexIndex & 1) == 0 ? item.quad.x : item.quad.z) + ((gl_VertexIndex & 1) == 0 ? -grow.x : grow.x),
                          fx((gl_VertexIndex & 2) == 0 ? item.quad.y : item.quad.w) + ((gl_VertexIndex & 2) == 0 ? -grow.y : grow.y));
        float det = gi.x * gi.w - gi.y * gi.z;
        corner = vec2((gi.w * local.x - gi.z * local.y) / det, (gi.x * local.y - gi.y * local.x) / det)
                 + vec2(fx(draw.offset.x), fx(draw.offset.y));
    }
    // Vulkan clip space is Y-down already, unlike Metal's Y-up — so this is the one
    // line that differs between the two shaders, and it differs by *not* flipping.
    gl_Position = vec4(corner.x / float(item.surface.x) * 2.0 - 1.0,
                       corner.y / float(item.surface.y) * 2.0 - 1.0, 0.0, 1.0);
}
