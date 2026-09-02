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
    ivec4 arc;     // 80: startAngle, endAngle (16.16 rad), edgeBase, unused
    ivec4 surface; //  96: width, height, blendMode, unused
    ivec4 clip;    // 112: the clip rectangle x0,y0,x1,y1 (16.16 px); zero-area = unclipped
};

layout(std430, set = 0, binding = 1) readonly buffer Items {
    ItemBlock blocks[];
} itemBuf;

// The fragment stage needs the same record this one read. Passing the *index* rather
// than the block costs one flat varying instead of twenty-eight, and it is the index
// the fragment stage would have had to be given anyway — `gl_InstanceIndex` is not
// available there.
layout(location = 0) flat out int vItem;

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
    vec2 corner = vec2(fx((gl_VertexIndex & 1) == 0 ? item.quad.x : item.quad.z),
                       fx((gl_VertexIndex & 2) == 0 ? item.quad.y : item.quad.w));
    // Vulkan clip space is Y-down already, unlike Metal's Y-up — so this is the one
    // line that differs between the two shaders, and it differs by *not* flipping.
    gl_Position = vec4(corner.x / float(item.surface.x) * 2.0 - 1.0,
                       corner.y / float(item.surface.y) * 2.0 - 1.0, 0.0, 1.0);
}
