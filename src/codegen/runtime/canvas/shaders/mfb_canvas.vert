#version 450

// plan-98-F: the Vulkan twin of plan-98-E's MSL vertex shader.
//
// The parameter block is byte-identical to the Metal one, so the CPU-side emitter
// that fills it (`emit_item_block`) feeds both backends unchanged: six ivec4s at
// offsets 0,16,32,48,64,80 and the surface size at 96. It is declared as a seventh
// ivec4 rather than an ivec2 so the two languages cannot disagree about trailing
// padding.
//
// 112 bytes fits inside Vulkan's guaranteed 128-byte push-constant range, so Phase 1
// needs no descriptor sets and no buffers at all.

layout(push_constant) uniform Item {
    ivec4 quad;    //  0: bounds minX, minY, maxX, maxY (16.16 px)
    ivec4 shape;   // 16: p0..p3 (16.16 px)
    ivec4 fill;    // 32: RGBA 0..255
    ivec4 stroke;  // 48: RGBA 0..255
    ivec4 misc;    // 64: kind, radius (16.16), strokeHalf (16.16), edgeCount
    ivec4 arc;     // 80: startAngle, endAngle (16.16 rad), unused, unused
    ivec4 surface; // 96: width, height, unused, unused (whole px)
} item;

const float FIXED = 65536.0;

float fx(int v) { return float(v) / FIXED; }

void main() {
    // Four corners of the item's bounds, expanded from gl_VertexIndex — no vertex
    // buffer, exactly as on Metal.
    vec2 corner = vec2(fx((gl_VertexIndex & 1) == 0 ? item.quad.x : item.quad.z),
                       fx((gl_VertexIndex & 2) == 0 ? item.quad.y : item.quad.w));
    // Vulkan clip space is Y-down already, unlike Metal's Y-up — so this is the one
    // line that differs between the two shaders, and it differs by *not* flipping.
    gl_Position = vec4(corner.x / float(item.surface.x) * 2.0 - 1.0,
                       corner.y / float(item.surface.y) * 2.0 - 1.0, 0.0, 1.0);
}
