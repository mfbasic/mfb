# bug-995: A `Paint.clip` on a transformed item evaluates the whole clip five times on every pixel, with no early-out, so the canvas-demo menu renders at 273 ms a frame on Vulkan

Last updated: 2026-10-12
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: planned `tools/canvas-bench/bench.sh <mfb> clips` (new scene: N clipped rectangles, with and without a `Paint.transform`); pixel guards stay `scripts/test-canvas-gpu-rows.sh` row `pathclips` and `rt_canvas_metal` (`path_clips_on_black.mfb`)

When an item has a path clip (`Paint.clip`) **and** a `Paint.transform`, every GPU pixel of
the item computes the clip's signed distance at five points (the pixel centre and its four
±0.5 px neighbours, all inverse-mapped), and all five run with an unbounded reach, so the
cheap "far from every edge" exit of `edgeDistance` never fires and each sample also runs
the double-float refinement loop. An untransformed clip computes one sample with reach 0.5
and skips almost all of that work on interior and exterior pixels. The item's *own* shape
distance has the matching early-out (`mapped = false` once `|d0| > reach`, so the four
neighbours are skipped); the clip has none.

Measured on box 206 (Intel UHD 630, real Vulkan, `--debug` builds, headless with
`MFB_CANVAS_GPU=1`, 900×640): 150 clipped 100×100 px rectangles cost **272 ms a frame**
under a scale-4 transform against **43 ms** for the same transformed rectangles without a
clip and **36 ms** for the same clip untransformed. The canvas-demo menu, whose only
transformed clipped items are the Grove card's SVG sigil (`menu.mfb:emblem` case 7,
`clipped(it, localRing(clip, viewOf(it)))`), drops from **273 ms to 101 ms a frame** when
just those items lose their clip. Metal runs the same shader logic line for line and pays
2× (10.7 → 22 ms on the same probe, Apple M-series), so this is not Vulkan-only. Only the
cost is affected; the pixels are correct.

**The single correct behavior a fix produces:** a pixel whose clip distance at the centre
sample is provably past the clip's antialiasing reach (the same bound the shape uses,
`0.5 · max(frob, 1)` in shape units) takes its clip coverage from that one sample (0 or
255) without evaluating the four neighbours, and that centre sample uses that reach (not
`1.0e30`) so `edgeDistance`'s fp32 pass can answer it. Every pixel's clip coverage stays
identical to today's, so goldens and the GPU-vs-oracle rows do not move. A transformed
clipped item then costs within a small factor of an untransformed one.

**The report this replaces was wrong about the cause.** It said `emit_clip_upload`
(`src/codegen/runtime/canvas/vulkan.rs`) writes a band count of 0, so the Vulkan shader
walks every clip edge. It does not: since `58bbb5a66` (plan-163-C Phase 3) the upload ends
in `emit_band_index(builder, bands, CLIP_BANDS)` and the shader reads it through
`clipBandAt`, the same as Metal's. A clip under `BAND_MIN_EDGES` (32,
`src/codegen/runtime/canvas/mod.rs`) gets band count 0 by design, like a polygon. Measured
(table below), untransformed clips are cheap at 28 and at 64 points. The uncommitted
demo edit that cut the menu ring from 64 to 28 points to "save per-edge cost" moves the
ring under `BAND_MIN_EDGES`, so it is no longer banded. It is cheaper in the menu only
because the expensive transformed samples walk fewer edges.

References:

- `mfb spec app canvas` (`src/docs/spec/app/06_canvas.md`): GPU parity and the band index
- `planning/completed/plan-163-C-clip-path-transparency.md` §4 / Correction 4: why a path clip is divided by its own gradient (the five clip samples)
- bug-991 (a transformed item's bounds pad scales with magnification): related cost on transformed items, but NOT this cause (the transformed no-clip controls below are fast)
- Found while profiling the canvas-demo menu on box 206 (5 fps windowed, ~213 ms render phase)

## Re-verified 2026-10-12: still reproduces on Metal; a group matrix now takes the same path (HEAD `555130a25`)

`bugs/repro/bug-995/clips.mfb` (arguments: clip points, scale, frames; 0 means none):
150 clipped 100×100 px squares, `mfb build --app --debug`, headless with
`MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS`, 40 frames, all on the GPU
(`gpuFrames=40`); ms/frame = `phaseRenderMs / gpuFrames`:

| scene | Metal ms/frame |
| --- | --- |
| no clip, untransformed | 5.6 |
| 64-pt clip, untransformed | 5.9 |
| 28-pt clip, untransformed | 8.5 |
| no clip, scale 4 | 8.6 |
| **64-pt clip, scale 4** | **17.9** |
| **28-pt clip, scale 4** | **23.8** |
| no clip, scale 0.1 | 7.5 |
| **64-pt clip, scale 0.1** | **12.9** |

The same 2× on Metal as before. Box 206 (Vulkan) was not re-measured.

Two things changed in the shader since this was written, and both widen the bug:

- `clipMapped` is now `itemMapped || grouped` (plan-172-H) in both
  `mfb_canvas.frag:shapeCoverage` and `metal.rs:METAL_SHADER_BODY`. An untransformed
  clipped item inside a `canvas::Group` that has a matrix therefore takes the five
  unbounded clip samples too. Not measured.
- `clipSampled` is `clipMode == 2 || (clipMode == 1 && grouped)`: under a group matrix a
  *rectangle* clip is sampled five times as well, through `clipRectDistance`, which is a
  closed form and not an edge walk, so it is expected to be cheap. Not measured.

The Fix Design's `clipReach` must use the composed norm the shape's `reach` now uses
(`norm * length(gi)` when grouped), not the item's inverse alone.

The API name has moved: the transform is the item's `transform` field since plan-172-C,
not `Paint.transform`. `Paint.clip` is unchanged.

## Failing Reproduction

Probe: `canvas::present` in a 10 s loop of 150 `Rectangle`s, 100×100 px, alpha fill,
optionally with `clip := [ring(…, P points)]` in the item's own coordinates, optionally
under `transform := Transform[a := s, d := s, tx := cx, ty := cy]` (the item and clip
drawn in units of `1/s`). Generator: `/tmp/clipprobe/gen2.sh DIR ITEMS P R S SECS`
(scratch; inline it into the bench scene in Phase 1).

```
mfb build --app --debug --target windows-x86_64 DIR
# box 206, a CRLF .bat:
set MFB_WINAPP_HEADLESS=1
set MFB_CANVAS_GPU=1
set MFB_CANVAS_STATS=%USERPROFILE%\clipq\stats.txt
clipprobe.exe
# run each exe TWICE and read the second run: the first pays bug-996's ~25 s pipeline compile
# ms/frame = phaseRenderMs / gpuFrames on the last stats line
```

| scene (150 items, 100×100 px) | Vulkan, box 206 ms/frame | Metal, M-series ms/frame |
| --- | --- | --- |
| no clip, untransformed | 47.5 | 6.6 |
| 28-pt clip, untransformed (unbanded) | 52.7 | — |
| 64-pt clip, untransformed (banded) | 35.9 | 7.7 |
| no clip, scale 0.1 | 30.7 | — |
| no clip, scale 4 | 43.2 | 10.7 |
| **64-pt clip, scale 0.1** | **146** | — |
| **28-pt clip, scale 0.1** | **305** | — |
| **64-pt clip, scale 4** | **272** | **22.1** |

canvas-demo menu, same harness (`MFB_CANVAS_STATS`, stdout redirected, 20 s, frames 30 → end):

| menu variant | ms/frame |
| --- | --- |
| no clips at all | 93 |
| 28-pt ring (worktree edit) | 167 |
| 64-pt ring (HEAD `02cef1888`) | 273 |
| 64-pt ring, sigil (transformed) items unclipped | 101 |
| 28-pt ring, sigil items unclipped | 95.5 |

- Observed: a transformed clipped item costs 3–8× the same item unclipped or untransformed-clipped.
- Expected: within a small factor of the untransformed clip (interior/exterior pixels decided by one sample).

The ~93 ms the menu costs with no clips at all is NOT this bug and is not root-caused here.
It was measured in a debug build, and `tools/canvas-bench/README.md` puts the debug-build
tax at about 35%.

## Root Cause

`src/codegen/runtime/canvas/shaders/mfb_canvas.frag:shapeCoverage`: the ten-sample loop
calls

```
d[k] = sampleDistance(clipK ? cband : band, qx, qy,
    k == 0 ? reach : ((k == 5 && !clipMapped) ? 0.5 : 1.0e30), clipK);
if (k == 0 && !joined && abs(d[0].x) > reach) { mapped = false; }
```

* Samples 5..9 are the clip. When `clipMapped`, all five (`j >= (clipMapped ? 5 : 1)`)
  run, and every one passes reach `1.0e30`.
* In `edgeDistance`, `distant = low > reach` is then never true, so the second loop
  (double-float `dfSegmentDistance` over the candidate edges) runs on every pixel, for
  all five samples.
* The shape's own neighbours are dropped by `mapped = false` once `|d0| > reach`. The clip
  has no equivalent, so `d[6..9]` are computed even when `d[5]` already puts the pixel far
  inside or outside the clip, where `dfCoverage(d5 / cg)` clamps to 255 or 0 anyway.

The Metal shader is the same text (`src/target/macos_aarch64/app/metal.rs:METAL_SHADER_BODY`,
the `clipMapped` lines), which is why Metal also doubles. The oracle
(`src/codegen/builtins/canvas/helper_items.rs:__canvas_clipPathSurface`) always evaluates
the four neighbours on a transformed item too. That sets the software renderer's cost but
does not change its result: the oracle's `d / g` is what the skip must reproduce.

Why the contrast cases are immune: untransformed clips evaluate only `k == 5` with reach
0.5, so `edgeDistance` exits after its fp32 pass on every pixel farther than ~0.5 px from
an edge. Transformed items without a clip have `clipMode != 2`, so samples 5..9 are skipped.

## Goal

- On box 206, the 64-pt / scale-4 probe row is ≤ 2× its no-clip row (43 ms), down from 272 ms.
- The canvas-demo menu (64-pt ring) is within 15% of its "sigil unclipped" variant.
- `pathclips` GPU row and `rt_canvas_metal` path-clip rows: 0 new differing pixels.

### Non-goals (must NOT change)

- Clip coverage of any pixel: no golden and no oracle comparison may move. The skip is
  valid only where the clamp makes it exact.
- The oracle's arithmetic. It may take the same exact shortcut, but its output is the contract.
- The band index (`emit_band_index`, `CLIP_BANDS`, `BAND_MIN_EDGES`). It works, and is not the cause.
- Do NOT "fix" this by dropping the clip's own gradient (dividing by the shape's
  gradient instead). plan-163-C Correction 4 shows that is wrong under a shear.

## Blast Radius

`grep -rn clipMapped src` → two sites, plus the oracle:

- `src/codegen/runtime/canvas/shaders/mfb_canvas.frag:shapeCoverage`: fixed by this bug (Vulkan).
- `src/target/macos_aarch64/app/metal.rs:METAL_SHADER_BODY` (the `shapeCoverage` twin): fixed by this bug (Metal, kept line for line).
- `src/codegen/builtins/canvas/helper_items.rs:__canvas_clipPathSurface` / `__canvas_glyphClip`: the software oracle. It has the same always-five cost. In scope only as an optional exact shortcut. Correctness is unaffected.
- Glyph clips (`glyphCoverage` path): audit whether the glyph arm evaluates the clip the same way. Write the verdict in Phase 1.

## Fix Design

Mirror the shape's early-out for the clip. Compute `clipReach = 0.5 * max(frob, 1)` (the
same Frobenius bound on the inverse matrix the shape's `reach` uses, so `|d5| > clipReach`
implies `|d5 / cg| ≥ 0.5` for every gradient the neighbours could produce). Pass it as
sample 5's reach when `clipMapped`. If `abs(d[5].x) > clipReach`, skip samples 6..9 and
set `clipCov` straight from the sign (0 outside, 255 inside). Before relying on it, prove
the bound in a comment, as `emit_band_index`'s doc does for the band reach, and pin it with
an argued test. The risk is all in that bound: if it is wrong, a boundary pixel's coverage
silently changes. That is why the pixel rows are the acceptance gate.

Rejected: raising `BAND_MIN_EDGES` or banding harder. The untransformed rows show banding
already works, and the cost is in the sample count and the reach.

## Phases

### Phase 1 — failing measurement + audit (no behavior change)

- [ ] Add a `clips` subcommand to `tools/canvas-bench/bench.sh` (`gen/clips.sh`: the probe
      above, items × points × scale), and record the table above on box 206 at HEAD.
- [ ] Add a correctness row with a transformed clipped item whose clip boundary runs through
      many pixels at scales 0.1, 1 and 4 (extend `tests/canvas/scenes/path_clips_on_black.mfb`
      or add a sibling scene) so the skip's exactness is pinned before it lands.
- [ ] Audit the glyph clip arm; write its verdict here.

Acceptance: bench reproduces the 3–8× gap. The new pixel row passes at HEAD on Metal and Vulkan.
Commit: —

### Phase 2 — the fix

- [ ] `mfb_canvas.frag:shapeCoverage`: clip reach for sample 5 when mapped; skip 6..9 past it; regenerate SPIR-V (`scripts/regen-spirv.sh`).
- [ ] `metal.rs:METAL_SHADER_BODY`: the same change, line for line.
- [ ] Spec: note the clip's early-out next to the shape's in `src/docs/spec/app/06_canvas.md`.

Acceptance: Phase 1 pixel rows unchanged; bench meets the Goal numbers on box 206.
Commit: —

### Phase 3 — full validation

- [ ] Full suite; `scripts/test-canvas-gpu-rows.sh` per target; `rt_canvas_metal`.
- [ ] Re-run the menu table on box 206 and a windowed menu run (`schtasks /it`) for fps.

Acceptance: full suite green, no golden delta, menu numbers recorded here.
Commit: —

## Validation Plan

- Regression test(s): `tools/canvas-bench` `clips` scene (perf); the transformed-clip pixel rows (exactness).
- Runtime proof: canvas-demo menu on box 206, windowed, `MFB_CANVAS_STATS` render phase before/after.
- Doc sync: `mfb spec app canvas` clip paragraph.
- Full suite: `cargo test`, `scripts/test-canvas-gpu-rows.sh`.

## Open Decisions

- Apply the same exact shortcut to the oracle (`__canvas_clipPathSurface`)? Recommended: yes, as a separate commit, since it is the software renderer's cost too. The alternative is leaving the oracle as the plain reference.

## Summary

The risk is the reach bound. With it proven, the change is two mirrored shader edits and an
optional oracle shortcut. The band index, the upload, and every pixel stay as they are.
