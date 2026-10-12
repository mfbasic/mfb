# bug-1079: on Intel's Vulkan driver a picture takes the wrong texel where a texel boundary is a pixel centre

Last updated: 2026-10-12
Effort: small in code (one expression in each GPU shader), medium to land (SPIR-V
regeneration, and proof on Metal and three Vulkan drivers)
Severity: LOW — one row or column of a picture, one texel off, on one driver
Class: Other

Status: Fixed (bs23-agent-b)
Regression Test: the `texeledge` row of `scripts/test-canvas-gpu-rows.sh` (`tests/canvas/scenes/picture_texel_edge.mfb`); `rt_canvas_metal::texel_boundaries_on_pixel_centres_match_the_software_oracle_on_metal`

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Rule: `mfb spec app canvas` "Rendering conventions": a picture's texel is chosen with "`floor` and `+ - * /` only, so every backend picks the same texel". Vulkan allows `/` 2.5 ULP, so that sentence holds only if the index does not depend on how the driver divides (`.ai/canvas-threading.md`, the dash words: the same driver's `atan`/`sin`/`cos`, answered there with the oracle's own series).
- Place: the one index expression `floor((q - x0) * iw / dw)` in `pictureColour`, three copies: `mfb_canvas.frag`, `metal.rs` (MSL) and the oracle `helper_items.rs:DRAW_GEOMETRY`.
- Fix: make the index exact whatever the quotient's last bit is: `u = floor(t / dw)` with `t = (q - x0) * iw`, corrected by one from the exact products (`(u + 1) * dw <= t`, `u * dw > t`); the same expression in all three, so they stay one rule. Nothing is keyed on a driver or a vendor.

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

## Re-verified 2026-10-12: still reproduces, same pixels (HEAD `555130a25`)

The repro as a row of a scratch copy of `scripts/test-canvas-gpu-rows.sh`
(`--target windows-x86_64 --rows texeledge,pictureshapes`, box 206):

```
ok: texeledge: drawn on Vulkan (gpuFrames=1)
FAIL: texeledge: Vulkan disagrees with the software oracle: worst=220 differing=0.0278% first-beyond-tolerance=(460, 262, '286f00ff', '2828dcff')
ok: pictureshapes: Vulkan matches the software oracle (ok worst=1 differing=1.2231%, 168562 lit)
```

The failure line is byte for byte the one below. `pictureColour` is unchanged in
`mfb_canvas.frag` and in `metal.rs`. Metal and lavapipe were not re-run.

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

## Resolution (bs23-agent-b, 2026-10-12)

The index is `u = floor(t / d)` with `t = (q - x0) * iw`, then `u + 1` when
`(u + 1) * d <= t`, else `u - 1` when `u * d > t`, then the clamp: `pictureTexel` in
`mfb_canvas.frag` and in the MSL (`metal.rs`), and the same three steps written out in the
picture arm of `helper_items.rs:DRAW_GEOMETRY`, for x and for y. Nothing is keyed on a
driver. The quotient is within one of the answer whatever its last bits are, and `*` is
correctly rounded on every backend, so the comparison settles it.

The regression scene `tests/canvas/scenes/picture_texel_edge.mfb` is the repro's picture,
the same picture on its side (a column boundary on a pixel centre), and six 4 by 4 images
over squares 14, 22, 26, 38, 46 and 58 px wide (six more divisors, x and y).

Proof (all on 2026-10-12):

- RED, box 206, compiler at main, `scripts/test-canvas-gpu-rows.sh /tmp/main-dbg-target/release/mfb --target windows-x86_64 --rows texeledge`:
  `FAIL: texeledge: Vulkan disagrees with the software oracle: worst=220 differing=0.1132% first-beyond-tolerance=(43, 40, '6f2800ff', '2828dcff')`
  (652 pixels: rows and columns of the small squares, row 262 of the wide picture, column
  127 of the tall one).
- GREEN, box 206, the fixed compiler, `--rows texeledge,pictureshapes,pictureauto,curvepictures`:
  `ok: texeledge: Vulkan matches the software oracle (ok worst=0 differing=0.0000%, 43480 lit)`;
  the other three rows pass with the figures they had.
- Lavapipe (box 2226, `--target linux-aarch64`, the same four rows, `--keep`): the software
  frame and the GPU frame of every row are byte-identical between the compiler at main and
  the fixed one (`cmp` of the eight `.rgba` files).
- Metal and software, locally: `picture_texel_edge`, `picture_shapes_on_black` and
  `curve_pictures_on_black` dumped with `MFB_CANVAS_GPU=1` and `=0` from both compilers are
  byte-identical (`cmp`, six pairs). `cargo test --test rt_canvas_metal` 61 passed,
  `rt_canvas_golden` 26 passed, `rt_canvas_picture` 16 passed, `rt_canvas_picture_fit` 9
  passed, `rt_canvas_curves` 40 passed.
- Boxes 2227 and 2228: not run (boxes down).

A software or Metal pixel could move only where the correctly rounded quotient is a whole
number and the exact one is not (`t` within half an ULP below `k * d` with `k * d` itself
not representable): there the old index was the texel the pixel centre is not in. No golden
and no scene measured has such a pixel.

### Sweep: other places where a driver-rounded quotient selects a discrete outcome

`mfb_canvas.frag` (the MSL has the same lines). Only `pictureColour` was changed.

| site | what the quotient selects | verdict |
|---|---|---|
| `pictureColour` u and v | the texel | FIXED (this bug) |
| `polygonBand`, `clipBandAt`: `floor((y - top) / height)` | the band whose edge list is walked | safe: `emit_band_index` builds each band with a 1-unit guard "for the float error of the shader's band lookup" on top of the reach, so the neighbouring band holds every edge that can matter at the boundary |
| `gradientAt`: `fx(stop) >= t`, `t` from `dot(q - from, axis) / len2` or `length(g - from) / len` or `focalT`'s `/ a` | the pair of stops | EXPOSED at a hard stop (two stops at one offset) lying exactly on a pixel centre: `t` a last bit above the offset takes the colour past the stop, a whole row or column. Not changed. A soft ramp is continuous there |
| `gradientAt`: `int((t - lo) / (hi - lo) * 4096.0)` | the 1/4096 ramp step | bounded: one step of 4096 is under one byte of colour, inside `GPU_DEFAULT` |
| `gradientSpread`, Repeat: `tt - floor(tt)` | which end of the ramp a whole-number `t` maps to | EXPOSED: `t` exactly a whole number on a pixel centre is 0 (first stop) in the oracle and can be 0.99999994 (last stop) on the driver; visible when the first and last stops differ. Not changed |
| `gradientSpread`, Reflect: `floor(tt / 2.0)` | the fold | safe: a division by 2 is exact under any rounding, and the fold is continuous at both ends |
| `dashWrap`: `x - floor(x / period) * period` | where in the pattern a position is | safe by construction: a `w` that comes out `period` instead of 0 falls out of `dashOnAt`'s loop to "on" and takes `dashAlong`'s `!found` arm, which is the next period's first dash, the same interval as `w = 0`; a `w` a last bit negative gives a dash end that far away, a distance |
| arc dashes: `rel -= floor(rel / (2 pi)) * (2 pi)` | which turn the angle is on | safe: the next line folds a point nearer the start back below zero, so `rel` near 0 and near `2 pi` give the same position; only a full-turn arc keeps a seam, at an angle fp32 does not produce |
| `dashWrapAngle`, `dashFold` | the series' fold | safe: `sin` and `cos` are continuous across it |
| `pieceArc`: `floor(t * 16.0)`; `pieceAtArc`: `(s - c0) / (c1 - c0)` | the arc-length table row | safe: no division selects the row (a product and a binary search), and the interpolation is continuous across rows |
| `glyphCoverage`: `floor(gp.x)`, `floor(gp.y)` | the glyph texel | safe from `/`: `gp` is products and sums only. (fp32 against the oracle's fp64 at a transformed glyph is plan-116-C's known limit, not this.) |
| `dfCoverage`: `floor(x.x)` | the coverage step | safe: double-float products and sums, no division |
| `dfDiv` (`precise` quotients) | double-float coordinates | safe: each quotient is corrected by its own remainder, the same idea as this fix |
| `gradientChannel`: `lin / 65535.0 * 12.92`, `pow` | the byte nearest in linear light | safe: already corrected by the two midpoint comparisons either side |
| `srgbTable`: `floor(srgbToLinear(i) * 65535.0 + 0.5)` (`pow`, `/ 12.92`, `/ 1.055`) | a 16-bit table entry | bounded: one unit of 65535, far under a byte |
| curve dash inside test: `w.x < s0.x + (w.y - s0.y) / (s1.y - s0.y) * (s1.x - s0.x)`; ellipse `dot(a / r, a / r) < 1.0` | the sign of a distance | bounded: the sign can only differ where the distance is zero |
| every `dot(w, v) / len2`, `/ length(c)`, `/ el`, `/ r` feeding a distance, a path parameter or an angle | a continuous value | bounded: a last-bit change moves coverage by at most the step the double-float paths already settle |
| integer `/ 255`, `/ 2` (tint, `covered`, binary search) | exact integer division | safe |

## Golden impact

No pixel golden moves (`rt_canvas_golden`, `rt_canvas_metal`, `rt_canvas_picture*`,
`rt_canvas_curves` pass unchanged). `scripts/artifact-gate.sh target/release/mfb all`
reports 9 diffs in 3 fixtures, all from this change:

- `syntax/app/app-mouse-surface` (`.ir`, `.macos-aarch64.app.nir`, `.nplan`, and the four
  `.app.ncode`): `__canvas_drawGeometry` grew (the two corrections in its picture arm);
  every later builtin source line number in the `.ir` moves by the 19 added lines; the
  macOS code also carries the MSL string (`pictureTexel`), the Linux and Windows code the
  regenerated `mfb_canvas.frag.spv`.
- `syntax/app/app-window-surface` (`.macos-aarch64.app.ncode` only): the embedded MSL.
- `syntax/resources/loop-body-resource-reference-valid` (`.ir`): the `"line"` of the canvas
  package's globals, which follow `helper_items.rs` in the builtin source, moves by 19.
