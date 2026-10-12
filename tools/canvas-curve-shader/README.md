# canvas-curve-shader

Writes the curve shapes' fragment-shader functions into
`src/codegen/runtime/canvas/shaders/mfb_canvas.frag`.

`canvas::PolyCurve` and `canvas::CurvePolygon` are drawn per pixel by the same
functions on Metal and on Vulkan: the fill's crossing test, the nearest point of a
piece, the cap cut, and the walk over a band's pieces. They are written once, in
`src/codegen/runtime/canvas/shaders/curves.tpl`, and turned into each shading language
by four markers (the table is in `src/codegen/runtime/canvas/curve_shader.rs`).

- **Metal** compiles its shader at run time, so the compiler expands the template
  itself when it builds the source. Nothing to run.
- **Vulkan** takes checked-in SPIR-V, compiled from the `.frag` file, so the GLSL
  expansion has to be in that file. This tool writes it between the
  `// @CURVES-BEGIN` and `// @CURVES-END` lines.

## After editing `curves.tpl`

```
python3 tools/canvas-curve-shader/generate.py     # rewrite the block in mfb_canvas.frag
scripts/regen-spirv.sh                            # recompile mfb_canvas.frag.spv
cargo test --bin mfb curve_shader                 # the copy matches the template
```

`the_fragment_shader_holds_the_generated_curve_functions` fails when the `.frag` copy
is not what the template expands to, so a template edit that skipped this tool cannot
land.
