# bug-995: A `Paint.clip` on a transformed item evaluates the whole clip five times on every pixel, with no early-out, so the canvas-demo menu renders at 273 ms a frame on Vulkan

Last updated: 2026-10-12
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (Performance)

Status: Fixed (bs23-agent-b)
Regression Test: `tools/canvas-bench/bench.sh <mfb> clips` (cost); the `xformclips` row of `scripts/test-canvas-gpu-rows.sh` and `rt_canvas_metal::transformed_path_clips_match_the_software_oracle_on_metal` (`tests/canvas/scenes/transformed_clips_on_black.mfb`, exactness), beside `pathclips` and `groupaffine`

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Rule: the shape's own early-out in `shapeCoverage` (bound argued in `metal.rs:METAL_SHADER_BODY`): a centre sample farther than `0.5 * max(frob, 1)` from the outline decides the pixel, so the four neighbours are skipped and the sample may answer from `edgeDistance`'s fp32 pass. A path clip is a second distance field through the same composed inverse, so the same bound holds for it.
- Place: the one sampling loop in `mfb_canvas.frag:shapeCoverage` and its line-for-line Metal twin exempt the clip: `k == 5` gets reach `1.0e30` when `clipMapped`, and nothing clears `clipMapped` after it.
- Fix: sample 5 takes the clip's reach (the composed norm the shape uses, without the stroke half) and, past it, `clipMapped` goes false exactly as `mapped` does. The `(k == 5 && !clipMapped) ? 0.5 : 1.0e30` special case goes. Exactness: `|d5| > 0.5 * max(frob, 1)` and `cg <= frob` give `|d5 / cg| > 0.5`, where `dfCoverage` clamps; prove it in the comment and pin it with pixel rows at scales 0.1, 1 and 4.

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

## Resolution (bs23-agent-b, 2026-10-12)

`shapeCoverage` (`mfb_canvas.frag` and the MSL in `metal.rs`, line for line): `clipReach`
is `0.5` untransformed and `0.5 * max(frob, 1)` when `mapped`, `frob` being the composed
norm the shape's `reach` is built from (`norm * length(gi)` when `grouped`). Sample 5 is
asked with `clipReach`; `if (k == 5 && abs(d[5].x) > clipReach) { clipMapped = false; }`
then skips samples 6..9 and the final `dfCoverage(d[5])` clamps by the sign. The
`(k == 5 && !clipMapped) ? 0.5 : 1.0e30` special case is gone. A rectangle clip under a
group matrix (`clipMode == 1 && grouped`) goes through the same line. The bound is argued
in the comment at `clipReach` in `metal.rs`.

The oracle takes the same shortcut in its own commit: `__canvas_clipReach` and the early
return in `__canvas_clipPathSurface` and `__canvas_clipRectSurface` (`helper_items.rs`).

### Phase 1 verdicts

- **Glyph clip arm.** No separate arm on the GPUs: `main` calls `shapeCoverage` for every
  kind and a glyph (kind 6) multiplies `glyphCoverage` by its `.w`, so a glyph's clip is
  samples 5..9 of the one loop and takes the early-out with it. In the oracle
  `__canvas_glyphClip` calls `__canvas_clipPathSurface` (or `__canvas_clipRectSurface` under
  a group matrix) with `__canvas_hasTransform`, which is TRUE under a group matrix too, so
  it takes the oracle shortcut the same way. Nothing glyph-specific to change.
- **Pixel scene.** `tests/canvas/scenes/transformed_clips_on_black.mfb`: a 40-edge star
  clip (banded) and a 24-edge ring clip (unbanded) on squares under transforms that scale
  by 0.1, turn 30 degrees at scale 1, scale by 4, and shear; an untransformed star-clipped
  square in a group turned and scaled by 1.5, and in a group at half size; an untransformed
  square with a rectangle clip in a sheared group; a scale-4 clipped square in a turned
  group.

### Exactness

Frames before the change (the commit before it) and after, same device, `cmp`:

| device | scenes | result |
|---|---|---|
| Metal, local (`MFB_CANVAS_GPU=1` dump) | `transformed_clips_on_black`, `path_clips_on_black`, `group_affine_on_black` | byte-identical, 3 of 3 |
| lavapipe, box 2226 (`scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64 --rows xformclips,pathclips,groupaffine --keep <dir>`) | the three GPU frames | byte-identical, 3 of 3 |
| Intel UHD 630, box 206 (`--target windows-x86_64`, same rows) | the three GPU frames | byte-identical, 3 of 3 |
| software (oracle shortcut), local (`MFB_CANVAS_GPU=0` dump) | those three, `picture_shapes_on_black`, `curve_pictures_on_black` | byte-identical, 5 of 5 |

Each row also passes against the software oracle: `xformclips` `worst=1 differing=0.5429%`
on box 206 and `worst=1 differing=0.6068%` on lavapipe, before and after.
`cargo test --test rt_canvas_metal` 62 passed; `rt_canvas_golden` 26 passed;
`rt_canvas_rasteriser` 106 passed, 2 ignored; `rt_canvas_curves` 40 passed;
`rt_canvas_picture` 16 passed; `rt_canvas_damage` 7 passed.

### Timings (indicative: other builds ran beside these)

`bash tools/canvas-bench/bench.sh <mfb> clips` (150 squares, 40 frames, `--debug`,
ms a frame after the first). Box 206 is
`bash tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host test@192.168.1.206 clips`.

| row | Metal before | Metal after | box 206 before | box 206 after |
|---|---|---|---|---|
| no clip, untransformed | 7.0 | 6.5 | 55.3 | 51.0 |
| 64-pt clip, untransformed | 6.3 | 10.5 | 72.4 | 74.3 |
| 28-pt clip, untransformed | 9.0 | 8.1 | 95.2 | 99.0 |
| no clip, item scale 4 | 9.1 | 8.8 | 53.9 | 55.1 |
| **64-pt clip, item scale 4** | **18.5** | **12.1** | **273.1** | **109.3** |
| **28-pt clip, item scale 4** | **24.4** | **12.9** | **387.8** | **133.1** |
| no clip, item scale 0.1 | 16.2 | 10.4 | 50.1 | 48.4 |
| **64-pt clip, item scale 0.1** | **21.4** | **12.8** | **163.7** | **102.4** |
| **28-pt clip, item scale 0.1** | **25.0** | **13.7** | **335.8** | **105.6** |
| no clip, group scale 4 | 9.1 | 10.3 | 53.5 | 53.9 |
| **64-pt clip, group scale 4** | **18.7** | **11.3** | **274.4** | **106.2** |
| **28-pt clip, group scale 4** | **26.2** | **19.1** | **383.2** | **130.5** |

Local load average (`uptime`) was 21 to 28 during "Metal before" and 9 to 25 during "Metal
after"; the box 206 columns ran at local load 7 to 13 (the box itself is shared with other
agents). Before is the compiler at main, after is this fix.

Goal 1 (box 206, 64-pt at scale 4 at most twice its no-clip row): 109.3 against 55.1, a
ratio of 1.98. It is met, narrowly, on indicative numbers; the coordinator re-measures.
What is left is the five-sample band itself: at scale 4, `frob` is 0.36, so `clipReach`
is the fallback's 0.5 shape units, which is 2 surface pixels either side of the clip's
edge (bug-991's secondary 1, the `max(F, 1)` term, is the same bound).

Goal 2 (the canvas-demo menu within 15% of its "sigil unclipped" variant): not measured as
written. `examples/canvas-demo/src/menu.mfb` no longer has the variants the table names
(`grep -n localRing examples/canvas-demo/src/menu.mfb` finds nothing). The unmodified menu
in a real window on Metal, `--debug`, 20 s, frames 30 to the end: 9.8 ms a frame before
(658 frames, load 54) and 8.6 after (1,766 frames, load 26). The load difference is
larger than the effect, so this says only that nothing got slower. Box 206 windowed:
not run.

Software (the oracle shortcut), `tools/canvas-bench/gen/clips.sh` with `MFB_CANVAS_GPU=0`,
150 squares, 6 frames, load 17 to 22: 64-pt at scale 4 11,070 to 4,338 ms a frame; 28-pt
at scale 4 4,759 to 3,297; 64-pt at scale 0.1 10,303 to 3,011; 64-pt in a group at scale
4 10,603 to 4,980.

## Golden impact

No pixel golden moves. `scripts/artifact-gate.sh target/release/mfb all` reports the same
9 diffs in the same 3 fixtures as after bug-1079 (the goldens are still main's):

- `syntax/app/app-mouse-surface`: the MSL string (`shapeCoverage`) in the macOS code, the
  regenerated `mfb_canvas.frag.spv` in the Linux and Windows code, and from the oracle
  commit the new `__canvas_clipReach` and the grown `__canvas_clipPathSurface` and
  `__canvas_clipRectSurface` (`.ir`, `.nir`, `.nplan`, all four `.ncode`), with every
  later builtin source line number.
- `syntax/app/app-window-surface` (`.macos-aarch64.app.ncode`): the embedded MSL.
- `syntax/resources/loop-body-resource-reference-valid` (`.ir`): the `"line"` of the canvas
  globals after `helper_items.rs`, which grew by 43 lines in the oracle commit.
