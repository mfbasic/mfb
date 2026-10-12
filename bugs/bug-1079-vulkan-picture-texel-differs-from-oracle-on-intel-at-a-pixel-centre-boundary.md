# bug-1079: on Intel's Vulkan driver a picture takes the wrong texel where a texel boundary is a pixel centre

Last updated: 2026-10-11
Effort: small in code (one expression in each GPU shader), medium to land (SPIR-V
regeneration, and proof on Metal and three Vulkan drivers)
Severity: LOW — one row or column of a picture, one texel off, on one driver
Class: Other

Status: Open
Regression Test: none

A `canvas::Picture` drawn on box 206's GPU (Windows 11, Intel UHD 630) shows the
neighbouring texel along every pixel row or column whose CENTRE is exactly a texel
boundary. The software renderer, Metal, and lavapipe (box 2226) agree with each other
there; the Intel driver differs.

**The single correct behavior a fix produces:** the repro below renders on box 206's
GPU to the frame the software renderer draws — 0 pixels beyond `Tolerance::GPU_DEFAULT`
— without changing a pixel on Metal or lavapipe.

References:

- `src/codegen/runtime/canvas/shaders/mfb_canvas.frag:pictureColour` — the Vulkan
  sampler: `floor((q.y - y0) * float(ih) / dh)` in float32.
- `src/target/macos_aarch64/app/metal.rs:pictureColour` (in the MSL source) — the same
  expression, which Apple's GPUs evaluate exactly.
- `src/codegen/builtins/canvas/helper_items.rs:DRAW_GEOMETRY` — the oracle, the same
  expression in float64.
- `mfb spec app canvas`, "Rendering conventions": "`floor` and `+ - * /` only, so every
  backend picks the same texel". That holds only where the division is exact, or
  correctly rounded.
- `.ai/canvas-threading.md`, on the dash words: the same driver takes Vulkan's licence
  on `atan`, `sin` and `cos`, which is why the dash shaders use the oracle's series
  instead (`dashAtan`). This is the division's turn.
- Found by plan-172-L's `pictureshapes` row of `scripts/test-canvas-gpu-rows.sh`, whose
  first scene put two such boundaries on pixel centres (an ellipse 110 px tall over a
  4-row image). The scene was changed to avoid them; this bug is what it avoided.

## Repro

`bugs/repro/bug-1079/texel_edge.mfb`: one plain picture, an 8 by 4 image stretched over
160 by 110 pixels at (460, 235). Rows are 27.5 px tall, so the boundary between image
rows 0 and 1 is y = 262.5 — the centre of pixel row 262, where
`(262.5 - 235) * 4 / 110` is exactly 1.

Run it as a render row of `scripts/test-canvas-gpu-rows.sh` on box 206 (copy it into
`tests/canvas/scenes/`, add the row `texeledge|texel_edge|render` to the script's table,
and pass `--target windows-x86_64 --box 22 --rows texeledge`):

```
ok: texeledge: drawn on Vulkan (gpuFrames=1)
FAIL: texeledge: Vulkan disagrees with the software oracle: worst=220 differing=0.0278% first-beyond-tolerance=(460, 262, '286f00ff', '2828dcff')
```

160 pixels — the whole of row 262, x 460 to 619 — and nothing else. The oracle has image
row 1 there (green 111) and the GPU row 0 (green 40, blue 220).

The same run with the compiler built at `88005fb60`, before plan-172-L, and the same
scene in that revision's record syntax, fails with the same line: the bug is not that
letter's.

## Cause, as far as it is known

`27.5 * 4 = 110` and `110 / 110 = 1` are both exact in float32, so a correctly rounded
division answers 1 and `floor` picks row 1. Row 0 means the quotient came out below 1:
the driver is not dividing — most likely multiplying by a reciprocal, `110 * (1 / 110)`,
which is `0.99999994`. Vulkan's precision rules allow that (2.5 ULP for `/`). Not
confirmed by reading the driver's output; the arithmetic is the evidence.

Row 317, where `(317.5 - 235) * 4 / 110` is exactly 3, is right on the same run, which
fits: `330 * (1 / 110)` rounds up to 3.

## First steps

1. Make the index exact whatever the division does: take `u = floor(t / dw)` with
   `t = (q - x0) * iw`, then correct it by one — `u + 1` when `(u + 1) * dw <= t`,
   `u - 1` when `u * dw > t`. The products are exact wherever the boundary case arises
   (small integers times a destination size), so the comparison is.
2. The same expression in the MSL (`metal.rs`) and in the oracle
   (`helper_items.rs`), so the three stay one rule, and `scripts/regen-spirv.sh` for the
   GLSL (it needs a box with `glslang-tools`; its default, 2228, was down on
   2026-10-11).
3. Prove: the repro on box 206; `scripts/test-canvas-gpu-rows.sh` on 2226, 2227 and
   206; `cargo test --test rt_canvas_metal`; and the picture goldens unchanged.
4. Add the repro as a row of `scripts/test-canvas-gpu-rows.sh`.
