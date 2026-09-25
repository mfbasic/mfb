# bug-687: Metal's fp32 coverage flips one quantisation step against the f64 oracle, and near black that is past `GPU_DEFAULT`

Last updated: 2026-09-24
Effort: medium (1-3d)
Severity: MEDIUM
Class: Correctness (GPU/oracle agreement)

Status: Open
Regression Test: none yet — see Phases

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

Recommended: 1.

## Phases

### Phase 1: the contract

- [ ] Decide option 1 vs 2; if 1, implement the quantum-aware comparison in
      `tests/common/canvas_image.rs` with the four-point proof, and add the {251/97} and
      {301/97} stars as oracle rows that must pass.

Commit: —
