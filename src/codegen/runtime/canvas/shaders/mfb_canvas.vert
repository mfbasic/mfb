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
// and `the_item_block_matches_the_std430_stride` in `vulkan.rs` pins that agreement.

struct ItemBlock {
    ivec4 quad;    //  0: bounds minX, minY, maxX, maxY (16.16 px)
    ivec4 shape;   // 16: p0..p3 (16.16 px)
    ivec4 fill;    // 32: RGBA 0..255
    ivec4 stroke;  // 48: RGBA 0..255
    ivec4 misc;    // 64: kind, radius (16.16), strokeHalf (16.16), edgeCount
    ivec4 arc;     // 80: startAngle, endAngle (16.16 rad), edgeBase, capStyle
    ivec4 surface; //  96: width, height, blendMode, unused
    ivec4 clip;    // 112: the clip rectangle x0,y0,x1,y1 (16.16 px); zero-area = unclipped
    ivec4 xform0;  // 128: inverse transform ia,ib,ic,id as float32 BITS
    ivec4 xform1;  // 144: itx, ity (float32 bits), hasTransform (0 or 1), unused
    ivec4 arcCaps; // 160: an arc's sweep endpoints startX,startY,endX,endY (16.16 px)
    ivec4 ellipse; // 176: an ellipse's rotation cos, sin (16.16); then a gradient's
                   //      stop count and first-stop index
    ivec4 gradient; // 192: a gradient's axis startX,startY,endX,endY (16.16 px)
};

layout(std430, set = 0, binding = 1) readonly buffer Items {
    ItemBlock blocks[];
} itemBuf;

// The fragment stage needs the same record this one read. Passing the *index* rather
// than the block costs one flat varying instead of twenty-eight, and it is the index
// the fragment stage would have had to be given anyway — `gl_InstanceIndex` is not
// available there.
layout(location = 0) flat out int vItem;

// plan-116-H: the group translation for THIS draw, in 16.16.
//
// Per-draw rather than per-item, which is the whole point: a group referenced twice is
// one set of item blocks and two draws differing only by this. Both stages declare it —
// the fragment stage subtracts it before evaluating the distance field — because a
// range no stage consumes is a layout the validation layers reject, which is why
// plan-116-A deleted the range rather than leaving it empty (H4).
layout(push_constant) uniform Draw {
    ivec2 offset;
} draw;

const float FIXED = 65536.0;

float fx(int v) { return float(v) / FIXED; }

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
    // Vulkan clip space is Y-down already, unlike Metal's Y-up — so this is the one
    // line that differs between the two shaders, and it differs by *not* flipping.
    gl_Position = vec4(corner.x / float(item.surface.x) * 2.0 - 1.0,
                       corner.y / float(item.surface.y) * 2.0 - 1.0, 0.0, 1.0);
}
