#!/usr/bin/env python3
"""Write the curve shapes' GLSL into mfb_canvas.frag from curves.tpl.

The Metal shader takes the same template at run time (`curve_shader.rs`); Vulkan
takes checked-in SPIR-V, so the GLSL has to be in the file glslang compiles. Run
this after editing `src/codegen/runtime/canvas/shaders/curves.tpl`, then
`scripts/regen-spirv.sh`. `the_fragment_shader_holds_the_generated_curve_functions`
(`cargo test --bin mfb curve_shader`) fails if the copy is stale.

Run from the repository root:  python3 tools/canvas-curve-shader/generate.py
"""
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SHADERS = ROOT / "src/codegen/runtime/canvas/shaders"
BEGIN = "// @CURVES-BEGIN generated from curves.tpl by tools/canvas-curve-shader/generate.py"
END = "// @CURVES-END"

# `curve_shader.rs:curve_coord`, GLSL.
CURVE_COORD = (
    "vec2 curveCoord(int slot, int w) {\n"
    "  int i = slot * 4 + w;\n"
    "  return edgeCoord(i, i + EDGE_RESIDUAL_BASE);\n"
    "}\n"
    "float curveWeight(int slot) {\n"
    "  return intBitsToFloat(edges.values[slot * 4 + 2]);\n"
    "}\n"
)


def glsl(template: str) -> str:
    body = template
    for marker, text in (("@S ", ""), ("@D ", ""), ("@C ", ""), ("@E[", "edges.values[")):
        body = body.replace(marker, text)
    return CURVE_COORD + body


def main() -> int:
    template = (SHADERS / "curves.tpl").read_text()
    frag_path = SHADERS / "mfb_canvas.frag"
    frag = frag_path.read_text()
    if BEGIN not in frag or END not in frag:
        print(f"generate.py: {frag_path} has no curve block markers", file=sys.stderr)
        return 1
    start = frag.index(BEGIN) + len(BEGIN) + 1
    end = frag.index(END, start)
    updated = frag[:start] + glsl(template) + frag[end:]
    if updated == frag:
        print("generate.py: mfb_canvas.frag is current")
        return 0
    frag_path.write_text(updated)
    print("generate.py: mfb_canvas.frag updated -- now run scripts/regen-spirv.sh")
    return 0


if __name__ == "__main__":
    sys.exit(main())
