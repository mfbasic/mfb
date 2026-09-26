# bug-687: Metal's fp32 coverage flips one quantisation step against the f64 oracle, and near black that is past `GPU_DEFAULT`

Last updated: 2026-09-25
Effort: medium (1-3d)
Severity: MEDIUM
Class: Correctness (GPU/oracle agreement)

Status: FIXED (`800a1b966`, 2026-09-25) — history: **option 2 chosen (owner, 2026-09-25)**: emulate more precision in the
shader; the comparison instrument and `GPU_DEFAULT` stay as they are. **STOPPED at the
Phase 1 model gate (2026-09-25): option 2 as designed does not reach `GPU_DEFAULT` on the
{301/97} row** — the residual is the 16.16 narrowing of the vertices, not shader
arithmetic (see "Phase 1 results"). **V5 chosen (owner, 2026-09-25)**: carry each
vertex at double-float precision and evaluate the polygon distance and the inverse
transform in double-float in both shaders. **Phase 2 done (2026-09-25)**: the four rows
pass on Metal and on Vulkan (2226, 2230), every existing GPU row still passes; see
"Phase 2 results". **Scope narrowed (owner, 2026-09-25): bug-687 lands with the
filled-polygon fix.** The other shape kinds Phase 3 measured are filed as **bug-735**:
circles, ellipses, lines, arcs, polylines, a non-grid stroke half-width, and a non-grid
group offset. Only the coordinator's full-suite gate and the `Commit:` lines remain
open. Not archived.
Regression Test: `tests/canvas/rt_canvas_metal.rs` (the six `*over_black*` rows) and the
`star251x97`, `star301x97`, `star301x2`, `polygonsblack`, `smallpolysblack`,
`rectsblack` rows of `scripts/test-canvas-gpu-rows.sh`. The first four and
`smallpolysblack` are RED before this bug's shader work and GREEN on this branch;
`rectsblack` is a guard, GREEN both sides.

Found while fixing bug-686. A polygon drawn on Metal differs from the software oracle by
more than `Tolerance::GPU_DEFAULT` allows (`max_channel_delta: 2`) on a handful of dark,
antialiased edge pixels. It is **pre-existing**: it reproduces at `42cea38df`, before any
bug-686 change, on a polygon under the old 256-edge cap, and it does not depend on the
edge count, the band index, or any cap.

## STATUS: FIXED (`800a1b966`)

Landed on `main` by merging the combined `/fix-bug` integration branch; full suite green on the integration branch at `800a1b966` (bug-652/732/733/540/699/687/734 + main `f332f61a3`): `cargo test --release --no-fail-fast` — 242 test binaries, 6198 passed, 0 failed, 24 ignored, `artifact_gate_all` ok; `scripts/artifact-gate.sh target/release/mfb all` 0 diffs / 2160 goldens after regenerating the 72 `.ncode(sum)` goldens the fixes moved (`82f1a43bb`, `800a1b966`), classified against detached `main` (0 diffs / 2160 at both `4df37a4fe` and `f332f61a3`). Scope narrowed by the owner to filled polygons; every other shape kind is bug-735 (open).

Re-verified after the bug-732/733 reclaim follow-up (`3a7232241`, goldens `596d1300e`): `cargo test --release --no-fail-fast` 242 binaries / 6200 passed / 0 failed / 24 ignored; `scripts/test-accept.sh` 1556 passed, 0 mismatches; `scripts/artifact-gate.sh all` 0 diffs / 2160.

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

~~A Metal frame agrees with the software oracle under a contract that an fp32 GPU can
actually meet, for every scene, so an oracle test passes or fails for the right reason
rather than for where an edge happens to land.~~

**Narrowed 2026-09-25 (owner): filled polygons.** A filled polygon (untransformed or
transformed) agrees with the software oracle within `GPU_DEFAULT` over black on Metal
and Vulkan, whatever its edge count and wherever its edges land. Every other shape kind,
and a polygon's non-grid stroke half-width or group offset, is **bug-735**
(`bugs/bug-735-gpu-shapes-other-than-filled-polygons-flip-coverage-near-black.md`).

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

- [x] Metal MSL and Vulkan GLSL (V5, owner 2026-09-25): each vertex coordinate carried
      as its 16.16 word plus a residual word, both backends' edge upload, region layout
      and buffer size to match; the polygon distance, crossing test, coverage rounding
      and inverse transform in double-float; the quantisation
      `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` unchanged. Rebuild the SPIR-V.
- [x] Measure the per-pixel cost on the bug-686 rows (`scripts/test-canvas-gpu-rows.sh`,
      `tools/canvas-bench`); record it here. On lavapipe the numbers describe a CPU, not a
      GPU (bug-688) — say so next to them.
- [x] Phase 1 rows pass on Metal (local) and Vulkan (2226, 2230); every existing GPU row
      still passes.

#### Phase 2 results (2026-09-25)

**What changed.**

- **Layout.** Each polygon edge's four endpoint coordinates also upload a residual word,
  `round((x * 65536 - word) * 2^31)` (`EDGE_RESIDUAL_SCALE`, units of 2^-47 px). The
  words go in a new region parallel to the 16.16 words: `METAL_EDGE_RESIDUAL_BASE_WORDS`,
  and on Vulkan `VULKAN_EDGE_RESIDUAL_BASE_WORDS` as specialization constant 3. The band
  index and every region after it move up by one edge region, and both buffers grow by
  4 MiB. The emitters are `emit_edge_buffer` (Metal) and `emit_edge_upload` (Vulkan).
  The band builders and polylines read only the 16.16 words, so they are unchanged.
  Nothing on the CPU reads the GPU edge buffer back: `MFB_CANVAS_GEO_VERIFY` checks the
  geometry records, and it reports 0 mismatches on every row below.
- **Shaders.** A polygon's (kind 4) fill and stroke coverage come from `polygonCoverage`,
  in double-float. That covers the shape-space point with the group offset taken off,
  the inverse transform and its five central-difference samples, the distance, the
  even-odd crossing (multiplied out, no division), `d / ||grad d||`, and the coverage
  rounding. The quantisation `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)` is unchanged, and
  every other kind keeps its fp32 path.
- **Metal's fast math** reassociated the double-float error terms to zero. Measured with
  a Swift/Metal probe (`/tmp/b687-probe`): 65,418 of 65,536 TwoSum error terms and every
  Dekker TwoProd term came back wrong. So each MSL helper turns reassociation off for its
  own body (`#pragma clang fp reassociate(off)`, measured: 0 wrong), and TwoProd uses
  `fma`, which MSL defines as exact. The GLSL marks its helpers `precise` and splits with
  Dekker, because GLSL does not promise a fused `fma`.
- **SPIR-V** rebuilt on 2226 (`MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh`). The vertex
  blob was restored, since its GLSL did not change.

**Why the fp32 pre-pass is exact, not a tolerance.** Evaluating every band edge in
double-float cost 2.8x the render time on bug-686's 64,000-edge polygon. So
`edgeDistance` spends double-float only where fp32 cannot decide the answer. The returned
value is still the double-float value, and nothing about the comparison or the
quantisation moves.

- **`edgeSlack(p, a, b)` is an error bound, not a margin on the answer.** It bounds how
  far one edge's fp32 distance can be from its double-float distance. The sources are
  the 16.16 word's conversion to float, the dropped residual (at most 2^-17 px), and the
  subtractions, products, division and square root. Each is a few ulps of the largest
  coordinate involved, because a segment's distance is 1-Lipschitz in its endpoints. The
  bound is 2^-18 of the six coordinates' magnitudes (64 ulps of each) plus 2^-12 px.
  Measured on the CPU with `/tmp/b687-model/slackcheck.c`, over every pixel x edge pair
  of {301/97}, {301/2}, the 4,000-edge ring, the 64,000-edge polygon (every 5th pixel)
  and a {301/97} star at (30000, 30000), with and without FMA: at most 0.8% of the bound
  is used. The crossing test's bound uses at most 0.54% of its limit, and there are 0 row
  or side decisions wrong outside the bounds.
- **Pass 1 is fp32 over the band.** It computes `bound` (the least `d32 + slack`, which
  is at least the true minimum distance) and `low` (the least `d32 - slack`, which is at
  most the true minimum). It also computes the parity of every crossing fp32 decides with
  certainty: a row test is certain unless an endpoint is within the slack of the row, and
  a side test is certain unless `|cross|` is within the slack times `|v|1 + 2|w|1`.
- **Pass 2 uses double-float for exactly two sets of edges.**
  - Distance: only edges whose `d32 - slack <= bound`. No other edge can be the nearest,
    because its true distance is at least `d32 - slack`, which exceeds `bound`, which is
    at least the true minimum.
  - Parity: only the crossings pass 1 left out as unsure.
- **Pass 2 is skipped when `low > reach`.** `reach` is 0.5 + the stroke half-width
  untransformed; under a transform there is no shortcut, because the gradient needs all
  five values. Past `reach` the fill coverage is 0 or 255 by sign alone and the stroke's
  is 0, so any value past `reach` with the right sign gives the same pixel. `low` is
  returned in the exact value's place.
- **Proved on the device**, not only argued: each frame was rendered on Metal with the
  full double-float build (every edge in double-float, `b5e8b9aae`) and with the final
  build, then `cmp`'d. They are **byte-identical** for all four new scenes, `huge_polygon`,
  `forty_thousand_opaque_edges` and `large_polygons` (`/tmp/b687-model/run_scene.sh`,
  then `cmp`).
- **One call site per double-float helper.** lavapipe (Mesa, LLVM) compiles the whole
  fragment shader at pipeline creation. An intermediate version with four double-float
  call sites per `edgeDistance`, inlined six times, did not finish compiling in 16 minutes
  on box 2226. The quads row hung at 100% CPU and was killed. The final version samples
  the five transform points in one loop and has one site for each helper; both
  test-canvas-gpu-rows rows (quads, star301x97) finished in 17 s.

**GREEN.** Final build `mfb` md5 `1ac75211c3c594122afd4469a00b03c5`:

| row | Metal (local) | Vulkan 2226 linux-aarch64 | Vulkan 2230 windows-x86_64 |
| --- | --- | --- | --- |
| {251/97} over black | ok | ok worst 1, 1.35% | ok worst 1, 1.35% |
| {301/97} over black | ok | ok worst 1, 1.38% | ok worst 1, 1.38% |
| {301/2} over black | ok | ok worst 1, 0.03% | ok worst 1, 0.03% |
| `LARGE_POLYGONS` over black | ok | ok worst 1, 0.19% | ok worst 1, 0.19% |
| every existing row | 20 ok | 8 render + 4 geo rows ok | 8 render + 3 geo rows ok; `race` fails, pre-existing (below) |

- Metal: `cargo test --release --test rt_canvas_metal` gives 24 passed, 0 failed. The
  local renders (`run_scene.sh`) are all `ok worst=1`, e.g. {301/97} at 0.57%. The
  remaining 1-level differences are the device's sRGB encode; the model predicts 0
  coverage mismatches for V5.
- Vulkan: `scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64` and
  `--target windows-x86_64`. Every row drew on the GPU with `geoVerifyMismatches=0`.
- **2230's `race` row fails, and not because of this change.** It is filed as
  **bug-734**. It fails 3 of 3 runs at `main` `4df37a4fe` (detached worktree, box alone),
  and at `8158a9524`. A 6,000-item software frame on the emulated box takes 13-18 s
  against a 15-19 s publish window, so only 2 frames are rendered; macOS renders 6-9. The
  3-frame requirement is real (the first two frames only fill the caches), so it stays.
- Unit tests: `cargo test --release --bin mfb -- metal vulkan canvas` gives 107 passed.
  That includes the extended region-chain tests, the spec-constant test and the
  shape-space test.
- `scripts/spec-census.sh --citations`: 0 misses.

**Cost.** Measured with `/tmp/b687-cost/cost.sh`: each scene is presented 200 times with
a 2-px rectangle moving so that every frame renders, under `MFB_CANVAS_SYNC=1`, in a
`--debug` build. The figure is the stats line's `phaseRenderMs / frames`, which covers
the GPU draw plus the readback and CPU encode. The table gives the minimum of 3
alternating runs in ms per frame. Baseline is `8158a9524`, the commit before the change.
The machine was shared (load 20-30); "net" subtracts the one-rectangle control:

| scene (Metal, Apple GPU) | before | after | net before → after |
| --- | --- | --- | --- |
| one rectangle (control) | 2.25 | 2.29 | — |
| {301/97} over black | 3.94 | 6.95 | 1.7 → 4.7 (2.8x) |
| {251/97} over black | 3.79 | 6.99 | 1.5 → 4.7 |
| `LARGE_POLYGONS` (grey ground) | 3.56 | 6.29 | 1.3 → 4.0 |
| `LARGE_POLYGONS` over black | 3.75 | 7.30 | 1.5 → 5.0 |
| 64,000-edge polygon (`huge_polygon`) | 10.70 | 19.49 | 8.5 → 17.2 (2.0x) |
| 40,000 opaque edges | 3.35 | 4.30 | 1.1 → 2.0 |

Before the exact pre-pass (every edge in double-float): 64,000 edges 38.2 ms, {301/97}
11.95 ms. Rotated rings cost most, because a transformed polygon evaluates all five
samples in double-float and has no `reach` shortcut.

On lavapipe the numbers describe a CPU, not a GPU (bug-688). Box 2226 aarch64,
`/tmp/b687-cost/cost_linux.sh` with 20 frames, two runs each; the box's noise is about 2x
between runs:

| scene (lavapipe, CPU) | before | after |
| --- | --- | --- |
| {301/97} over black | 163 / 82 | 637 / 230 |
| `LARGE_POLYGONS` over black | 105 / 90 | 285 / 811 |
| 64,000-edge polygon | 209 / 358 | 921 / 1155 |
| 40,000 opaque edges | 324 / 254 | 206 / 223 |

**Goldens expected to shift (not regenerated, per the task).** The `app-mouse-surface`
canvas change-sentinel's `.ncodesum` files, for all four targets
(`tests/syntax/app/app-mouse-surface/golden/app_mouse_surface.{macos-aarch64,linux-aarch64,linux-x86_64,windows-x86_64}.app.ncodesum`).
Four things move in them:
- the emitted edge upload, `emit_edge_buffer` on Metal and `emit_edge_upload` on Vulkan;
- every Metal and Vulkan emitter that folds a region base or the buffer size, since the
  band, gradient, glyph and picture offsets all moved;
- Metal's embedded MSL string;
- the Vulkan specialization table (4 entries) and the embedded fragment SPIR-V.

The `.ir`, `.nir`, `.nplan` and `.ast` files should not move, because no MFBASIC helper
changed. That is a prediction, not a measurement: `artifact-gate.sh all` was not run.

**Known limits, unchanged by this fix.** A stroke's half-width and a group's draw offset
still reach the shader as 16.16, and a `PolyLine`'s distance is still fp32. A stroke whose
half-width is not a multiple of 2^-16, or a group at a non-16.16 offset, can still flip a
step near black. Neither is in any row here.

Commit: 92649bd1f, f3e7ed29d (rows, RED), 8158a9524 (V5 decision), b5e8b9aae + 01d791117 (V5 shaders), ed51c2327 + 42731364b (spec), e8df49119, 9171a9d6c, 6ac70ee4c, fe022d6ef, ef9a9b8f2 (narrowed; bug-735 filed in a9c1e22f1)

### Landing

- [x] Guard rows (`small_polygons_on_black`, `rectangles_on_black`) on Metal and Vulkan.
- [x] bug-734 (2230 race row) fixed on this branch; bug-735 filed for the other kinds.
- [x] Full suite (`cargo test --release --no-fail-fast`, including `artifact_gate_all`)
      and the `app-mouse-surface` sentinel regenerated with proof — the coordinator's.

Commit: 800a1b966 (goldens + full suite)

### Phase 3: the other shapes (coordinator, 2026-09-25)

Probes: `/tmp/b687-limits/gen.py <kind>` builds 140 items of one kind on a 60-px grid,
so nothing overlaps. Positions and sizes are off the pixel and 16.16 grids, the fill is
`rgb(250, 250, 120)` over the default black, and strokes are 2 px unless stated. Metal is
local (`/tmp/b687-model/run_scene.sh`). Vulkan is box 2226 (`/tmp/b687-limits/vk.sh`,
`MFB_CANVAS_SYNC=1`, `gpuFrames=1` on every run). Each cell is pixels past delta 2 /
worst delta:

| kind | Metal before (`8158a9524`) | Metal after | Vulkan 2226 after |
| --- | --- | --- | --- |
| polygon (40-edge rings, filled) | 3 / 4 | **0 / 1** | **0 / 1** |
| rectangle | 0 / 2 | 0 / 2 | 0 / 2 |
| polygon, stroke width 1.0-2.9 (half-width off 16.16) | 4 / 6 | 1 / 3 | 2 / 3 |
| polygon in a group at an off-grid offset | 3 / 4 | 3 / 4 | 4 / 4 |
| polyline | 2 / 6 | 2 / 6 | 4 / 6 |
| circle | 2 / 6 | 2 / 6 | 3 / 6 |
| ellipse | 8 / 9 | 8 / 9 | 12 / 9 |
| line | 1 / 3 | 1 / 3 | 1 / 3 |
| arc | 3 / 9 | 3 / 9 | 3 / 9 |

Findings:

- **The polygon path is fixed and held.** Two new guard rows in `rt_canvas_metal.rs`
  and `test-canvas-gpu-rows.sh`:
  - `small_polygons_on_black.mfb`: RED before on Metal (3 / 4) and on 2226 (worst 4);
    GREEN after on Metal, 2226 and 2230 (worst 1).
  - `rectangles_on_black.mfb`: worst 2 everywhere, before and after.
- **Every other signed distance still flips near black.** Circle, ellipse, line, arc and
  polyline trip before and after, by 3-9 levels. So do two inputs that still reach the
  polygon path as 16.16: the stroke half-width and the group offset.
- **This is the same bug, bug-687's mechanism.** Each is an fp32 distance, from inputs
  narrowed to 16.16, against the oracle's f64.
- **It is not contained.** Fixing it in V5's style means:
  - residual words for every item-block field that feeds a distance: centres, radii,
    ellipse axes, arc endpoints, stroke half-width, clip rectangle;
  - the same for the per-draw group offset;
  - a double-float twin of every SDF, including the ellipse's 24-step bisection and the
    arc's sweep test;
  - for arcs, the angles themselves: the shader takes `cos`/`sin` of 16.16 angles, and
    GPU transcendentals do not match the oracle's.

  That is a new block layout, a new offset transport, and double-float in every shader
  arm. It is not a residual of the polygon change. **Stopped here for an owner
  decision**, as the task required.
- **Owner decision (2026-09-25):** bug-687 lands with the filled-polygon fix. The rest is
  filed as **bug-735**, with this table, the mechanism, the V5-extension cost outline and
  the probe generator inlined as its reproduction.
- The tripping kinds were not added as rows. They would be RED on every backend today;
  the probes above are the reproduction.
