# bug-687: Metal's fp32 coverage flips one quantisation step against the f64 oracle, and near black that is past `GPU_DEFAULT`

Last updated: 2026-09-25
Effort: medium (1-3d)
Severity: MEDIUM
Class: Correctness (GPU/oracle agreement)

Status: Open — **option 2 chosen (owner, 2026-09-25)**: emulate more precision in the
shader; the comparison instrument and `GPU_DEFAULT` stay as they are. **STOPPED at the
Phase 1 model gate (2026-09-25): option 2 as designed does not reach `GPU_DEFAULT` on the
{301/97} row** — the residual is the 16.16 narrowing of the vertices, not shader
arithmetic (see "Phase 1 results"). **V5 chosen (owner, 2026-09-25)**: carry each
vertex at double-float precision and evaluate the polygon distance and the inverse
transform in double-float in both shaders — Phase 2 below.
Regression Test: `tests/canvas/rt_canvas_metal.rs` (the four `*over_black*` rows) and the
`star251x97`, `star301x97`, `star301x2`, `polygonsblack` rows of
`scripts/test-canvas-gpu-rows.sh` — RED at HEAD, see Phase 1 results

Found while fixing bug-686. A polygon drawn on Metal differs from the software oracle by
more than `Tolerance::GPU_DEFAULT` allows (`max_channel_delta: 2`) on a handful of dark,
antialiased edge pixels. It is **pre-existing**: it reproduces at `42cea38df`, before any
bug-686 change, on a polygon under the old 256-edge cap, and it does not depend on the
edge count, the band index, or any cap.

## Failing Reproduction

A single self-intersecting star polygon, filled `rgb(250, 250, 120)` on the default black
surface, rendered on software and on Metal (`MFB_CANVAS_SYNC=1`, `MFB_CANVAS_DUMP`,
`--debug`), then pixel-compared (`/tmp/bug686-spike/run_star.sh <mfb>`):

| star, radius 180 | build | differing px | max channel delta | on Metal |
| --- | --- | --- | --- | --- |
| {251/97} (long chords) | `42cea38df` (HEAD before bug-686) | 3,567 | **4** | yes |
| {251/97} | bug-686 integration branch | 3,567 | **4** (byte-identical to HEAD) | yes |
| {301/97} | bug-686 branch | 3,790 (in `LARGE_POLYGONS`) | **9** | yes |
| {301/2} (short edges) | bug-686 branch | 103 | **3** | yes |
| {301/5} | bug-686 branch | 98 | 1 | yes |

- Expected: every pixel within 2 channel steps (the documented GPU contract).
- Observed: isolated dark pixels off by 3-9, and which scenes trip it depends on where
  edges happen to fall, not on anything about the scene's structure.

### The same flip on Vulkan (found by bug-688)

It is not Metal's: Vulkan's GLSL evaluates the same fp32 distance, and Mesa's lavapipe
(box 2226, aarch64) trips it on `tests/canvas/scenes/large_polygons.mfb` as it stood
before bug-688 gave that scene a grey ground (`git show 662910824:tests/canvas/rt_canvas_metal.rs`,
`LARGE_POLYGONS`; that scene drew on Vulkan at the old caps too, 6,401 edges):

| scene | backend | differing px | max channel delta | first pixel |
| --- | --- | --- | --- | --- |
| `LARGE_POLYGONS` over black | Vulkan (lavapipe), `main` and bug-688 | 0.19% | **4** | (736, 52): GPU `1d0418`, software `21051c` |
| the same over a `rgb(128,128,128)` ground | Vulkan (lavapipe), bug-688 | 0.26% | 1 | — |

(736, 52) is on the rim of the rotated ring `d`, `rgb(230, 80, 200)` over black: software
coverage 5/255 encodes to 33, the GPU's 4/255 to 29 — one coverage step, the mechanism
below. bug-688 put its oracle scenes on a mid-grey ground (`large_polygons.mfb`,
`huge_polygon.mfb`, `forty_thousand_opaque_edges.mfb`) so they measure what they are
about; the near-black rows this bug needs are still its own to add.

## Root Cause (measured by the bug-686 caps agent with a C model of both paths)

The shader evaluates the signed distance in fp32; the oracle in f64. Coverage is then
quantised to 0..255 identically on both (plan-98-E made the shader match the oracle's
quantisation). Where the exact distance lands within ~3e-5 px of a quantisation-step
boundary, the two round to adjacent steps. Near black the sRGB encode is steep enough
that one coverage step moves a channel by up to 9-13 output levels (plan-98-E measured
13 over the oracle's table). Long edges put more pixels near a boundary, so a scene with
more edge length trips it more often; at {301/97} (~40x the edge length of a ring) it
is four pixels at delta 3-9 at, e.g., (530, 562): software coverage 2, Metal 1.

Computing the shader's edge vectors from exact integer (16.16) differences cut the
mismatches from 145 to 51 in the model, and the dark ones from 3 to 2 — not to zero. No
finite-precision change reaches zero: two implementations of different precision always
disagree at some boundary.

## Goal

A Metal frame agrees with the software oracle under a contract that an fp32 GPU can
actually meet, for every scene, so an oracle test passes or fails for the right reason
rather than for where an edge happens to land.

## Fix Design (options — decide before implementing)

1. **Make the comparison quantum-aware.** Allow a pixel beyond `max_channel_delta` when
   the two colours are exactly one coverage step apart for that pixel's paint and
   background. This states the real contract (one step of coverage, anywhere), but it
   changes a shared test instrument, so it needs the never-edit-a-test proof: the four
   rows above are the proof that the current bound is unmeetable by construction.
2. **Emulate more precision in the shader** for the distance (double-float arithmetic on
   the edge loop). Would shrink the window but, per the root cause, not close it, and it
   costs every polygon pixel.
3. **Quantise coverage in both paths at a coarser grid** — changes the oracle's output
   and every golden; rejected unless 1 is rejected too.

~~Recommended: 1.~~ **Decided 2026-09-25 (owner): option 2.** The oracle comparison
(`tests/common/canvas_image.rs`, `Tolerance::GPU_DEFAULT`) is not edited. The fix is in
the shaders: Metal (`src/codegen/runtime/canvas/metal.rs`, the embedded MSL) and Vulkan
(`src/codegen/runtime/canvas/shaders/mfb_canvas.frag`, rebuilt to
`mfb_canvas.frag.spv`), since bug-688 showed lavapipe trips the same flip.

**Revised 2026-09-25 (owner): V5.** Phase 1's model showed option 2 as first written
(exact 16.16 edge vectors + double-float distance) cannot pass {301/97}: the residual is
the CPU's 16.16 narrowing of the vertices. The owner chose V5 from the Phase 1 table:
upload each vertex coordinate at double-float precision (two 32-bit words per
coordinate) and compute the polygon's distance, crossing test, coverage rounding and
inverse transform in double-float in both shaders. The quantisation
`int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` is unchanged; `tests/common/canvas_image.rs`
and every tolerance stay as they are. (The MSL lives in
`src/target/macos_aarch64/app/metal.rs`, not `src/codegen/runtime/canvas/metal.rs`.)

**Known risk, stated before implementing.** The root cause above says no finite-precision
change reaches zero, and the C model's exact-integer edge vectors cut mismatches from 145
to 51, not 0. Option 2 is therefore measured, not assumed: if the extended-precision
distance still leaves pixels past `GPU_DEFAULT` on the Phase 1 rows, stop and report the
residual table to the owner. Do **not** fall back to option 1 or widen the tolerance
without a new owner decision.

## Phases

### Phase 1: the contract

- [x] Decide option 1 vs 2 — **option 2** (owner, 2026-09-25).
- [x] Add the {251/97}, {301/97} and {301/2} stars over black, and `LARGE_POLYGONS`
      over black, as oracle rows on Metal and Vulkan that must pass at `GPU_DEFAULT`.
      Confirm they fail at HEAD (the tables above). Scenes
      `tests/canvas/scenes/star_{251_97,301_97,301_2}_on_black.mfb` and
      `large_polygons_on_black.mfb`; RED on Vulkan everywhere, RED on Metal for the three
      stars (the large-polygon row passes on Metal at HEAD) — see "Phase 1 results".
- [x] Extend the C model of both paths with the double-float (two-fp32) distance and
      the exact 16.16 integer edge vectors; report the mismatch count and worst dark
      delta on every row before touching a shader. If the model does not reach
      `GPU_DEFAULT` on every row, stop here and report (see "Known risk").
      **It does not: {301/97} keeps 2 pixels at delta 4. Stopped here.**

#### Phase 1 results (2026-09-25, worktree `worktree-B-687` at `4ef538bb9`)

**RED at HEAD.** `cargo build --release`, then per backend:

| row | Metal (local, `cargo test --release --test rt_canvas_metal -- over_black`) | Vulkan lavapipe, 2226 linux-aarch64 | Vulkan lavapipe, 2230 windows-x86_64 |
| --- | --- | --- | --- |
| {251/97} over black | **FAIL** 3,567 px, max delta 4 | **FAIL** worst 4, 1.36%, first (373, 261) | **FAIL** worst 4, 1.36%, first (373, 261) |
| {301/97} over black | **FAIL** 3,394 px, max delta 4 | **FAIL** worst 4, 1.39%, first (370, 183) | **FAIL** worst 4, 1.39%, first (370, 183) |
| {301/2} over black | **FAIL** 103 px, max delta 3 | **FAIL** worst 3, 0.03%, first (270, 318) | **FAIL** worst 3, 0.03%, first (270, 318) |
| `LARGE_POLYGONS` over black | ok (worst 1, 0.08%) | **FAIL** worst 4, 0.19%, first (736, 52) — bug-688's pixel | **FAIL** worst 4, 0.19%, first (736, 52) |

Vulkan: `scripts/test-canvas-gpu-rows.sh target/release/mfb --target linux-aarch64
--rows star251x97,star301x97,star301x2,polygonsblack` (and `--target windows-x86_64`).
Every row drew on the GPU (`gpuFrames=1`) with `geoVerifyMismatches=0`. The Metal
large-polygon row passing is measured, not a harness gap: the same frame on Metal is
`ok worst=1 differing=0.0825%` through `scripts/rgba_compare.py`; Metal's arithmetic
misses the one boundary pixel the fp32 model predicts (below), lavapipe's hits it.

**The model** is `/tmp/b687-model/model.c` (`cc -O2 -ffp-contract=off -o model model.c
-lm; ./model star:N:K:450:320 [sw.rgba [gpu.rgba]]`, `./model large ...`). It rebuilds each
scene's vertices, runs the oracle's `__canvas_edgeDistance` / `__canvas_coverage` /
`__canvas_blendChannel` in f64 and each GPU variant, and composites over black. It is
checked against the real frames before it is believed:

- the oracle model reproduces the software dump **bit for bit on every row** (0 pixels
  differ, all four scenes);
- the fp32 model (V1) predicts the Metal pixels beyond tolerance: {251/97} 4 of Metal's 6
  (the other 2 are flips the device's own sRGB encode pushes from 2 to 3), {301/97} 2 of
  2, {301/2} 2 of 2.

Variants: **V1** the shader today (`fx()` of 16.16 → fp32 segment distance); **V2**
exact integer 16.16 edge vectors (`B - A`, `P - A` in integers), fp32 after;
**V3 = the Phase 2 design**: V2's exact vectors, distance, projection, crossing and
coverage rounding in double-float (two fp32, Dekker products, no FMA needed), inverse
transform in double-float; **V4** f64 arithmetic on the 16.16-rounded vertices — the
ceiling of *any* shader arithmetic, since the shader only ever sees 16.16; **V5**
double-float arithmetic on vertices *carried* in double-float (no 16.16 narrowing).
Coverage mismatches / pixels past delta 2 / worst channel delta, against the oracle:

| row | V1 (today) | V2 | **V3 (Phase 2 design)** | V4 (ceiling) | V5 (df vertices) |
| --- | --- | --- | --- | --- | --- |
| {251/97} | 135 / 4 / 4 | 66 / 1 / 12 | **50 / 0 / 2** | 50 / 0 / 2 | 0 / 0 / 0 |
| {301/97} | 121 / 2 / 4 | 51 / 2 / 4 | **42 / 2 / 4** | 42 / 2 / 4 | 0 / 0 / 0 |
| {301/2} | 3 / 2 / 3 | 2 / 0 / 1 | **2 / 0 / 1** | 2 / 0 / 1 | 0 / 0 / 0 |
| `LARGE_POLYGONS` | 11 / 1 / 4 | 4 / 1 / 4 | **3 / 0 / 1** | 3 / 0 / 1 | 0 / 0 / 0 |

V3's {251/97} "0 past 2, worst 2" does not survive the device either: re-scored through
the colour Metal actually writes for each coverage (a map taken from the HEAD Metal frame),
V3 leaves 2 pixels at delta 3 on {251/97} as well as the 2 at delta 4 on {301/97}; V5 leaves
0 on every row (worst 1).

**Why V3 stops short: the input, not the arithmetic.** V3 equals V4 on every row, pixel
for pixel (the model's `V3 vs V4 coverage: 0 pixels differ` line, all four scenes) —
double-float already computes the exact distance *of the 16.16 polygon*. The
16.16 polygon is not the oracle's polygon: `emit_edge_buffer` rounds each vertex to
1/65536 px (`float_round_to_signed_x`), up to 7.6e-6 px per coordinate. Both {301/97}
pixels (`LIST=1 ./model star:301:97:450:320`):

| pixel | oracle d | cov | V3 d (16.16 polygon) | cov | gap from d to the step boundary |
| --- | --- | --- | --- | --- | --- |
| (370, 183) | 0.482351704 | 5 | 0.482353770 | 4 | 1.24e-6 px |
| (370, 456) | 0.482351704 | 5 | 0.482353770 | 4 | 1.24e-6 px |

(mirror images across the star's axis). The narrowing moves the distance 2.07e-6 px; the
boundary is 1.24e-6 px away. No shader-side precision closes that, which is what the
"Known risk" paragraph anticipated. The root cause's 145 → 51 (a {301/97} star at a
different centre, inside bug-686's scene) has the same shape as this table's V1 → V2
(121 → 51); the new fact is that V2's residual is almost all input rounding: V2 → V4
removes only 9 of 51.

**What would reach it (for the owner, not implemented).** V5 — vertices carried at
double-float precision — gives 0 mismatches on all four rows. That is a larger change than
option 2 as written: the edge region would carry each coordinate as two words (e.g. the
16.16 value plus a second 32-bit residual, since the emitters have no double → single
convert), doubling the edge region (`METAL_MAX_FRAME_EDGES`, Vulkan's region chain, the
band index offsets, the buffer size), and both shaders would evaluate the distance and the
inverse transform in double-float. It still is not a proof of zero for every scene: V5
only moves the input error from ~1e-5 px to ~1e-12 px. Options 1 and 3 are unchanged.

### Phase 2: the shaders

- [ ] Metal MSL and Vulkan GLSL (V5, owner 2026-09-25): each vertex coordinate carried
      as its 16.16 word plus a residual word, both backends' edge upload, region layout
      and buffer size to match; the polygon distance, crossing test, coverage rounding
      and inverse transform in double-float; the quantisation
      `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` unchanged. Rebuild the SPIR-V.
- [ ] Measure the per-pixel cost on the bug-686 rows (`scripts/test-canvas-gpu-rows.sh`,
      `tools/canvas-bench`); record it here. On lavapipe the numbers describe a CPU, not a
      GPU (bug-688) — say so next to them.
- [ ] Phase 1 rows pass on Metal (local) and Vulkan (2226, 2230); every existing GPU row
      still passes.

Commit: —
