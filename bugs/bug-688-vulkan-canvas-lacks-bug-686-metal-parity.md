# bug-688: The Vulkan canvas backend is still at pre-bug-686 Metal: ordinary scenes fall to software, large polygons are O(edges × pixels), pictures copy per occurrence, every frame is read back

Last updated: 2026-10-12 (bug-staging-23 round 2)
Effort: huge (>3d)
Severity: HIGH
Class: Correctness / Performance

Status: V1-V4 FIXED; V5 MEASURED on real hardware (box 206) with three fixes landed; V6 FIXED on Windows (swapchain present), Linux present is bug-1082; the 10k-quad shader cost OPEN (106 to 58 ms for squares, 108 to 92 for the sweep's polygon quads; see "bug-staging-23 round 2")
Regression Test: `tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host test@192.168.1.206 shapes` (the fragment cost, box 206); `scripts/test-canvas-gpu-rows.sh` (the bug-686 rows and the geo-verify rows, per target); `scripts/test-canvas-vulkan.sh --target`; the `vulkan.rs` unit tests `the_pipeline_specializes_every_region_base` and `the_shared_buffer_holds_every_region`; `both_predicates_count_each_picture_block_once`; `rt_canvas_metal` rows for the shared scenes; V6: `present_layout_tables_are_self_consistent`, `the_present_path_is_windows_only_and_fully_named`, `the_canvas_child_window_is_shown_by_one_arm_and_hidden_by_the_others`, and the real-window run on box 206 with `tools/win-screen-capture` (manual: needs the interactive session)

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Scope (OWNER RULING 2026-10-12): macOS (Metal, this machine) and Windows box 206 (Intel UHD 630) only; Linux present is bug-1082. 10,000 moving items is a GOAL, not a hard limit (2D games at that scale).
- Rule: the early-out `shapeCoverage` already states (GLSL and its Metal twin): a pixel its centre sample decides pays ONE distance sample; the four neighbours and the five clip samples are skipped. The source spells that as `continue` inside one fixed ten-pass loop around a single `sampleDistance` call site, and Intel's compiler does not turn that into skipped work.
- Measured on box 206, 10,000 static 16 px rectangles, `--debug`, headless, render ms a frame (scratch shader variants, `/tmp/bs23/v/*.frag`): shipped 106; constant colour 14.2; item record fetched 14.9 (the 368-byte fetch is free); `farBound` only 19.6; one `sampleDistance` + `dfCoverage` 32.1; all of `shapeCoverage` with everything after it cut 105.4; **sample 0 taken out of the loop and the loop entered only under `mapped || clipSampled`: 56.0** (33.4 -> 16.7 at radius 0.6). Rectangle, circle and 4-point polygon cost the same (95 to 109), so it is not the edge walk; "the shader grew" was a guess and is not the cause. On Metal the same scene renders in 13.4 ms and the program is the limit (20.6 fps).
- Place: the sampling loop of `mfb_canvas.frag:shapeCoverage` and `metal.rs:METAL_SHADER_BODY`, line for line. Fix: make the early-out real control flow (the decided pixel runs one straight-line sample; neighbour and clip samples sit behind a branch), same arithmetic, every pixel byte-identical on the same device. The gap that remains to the one-sample figure (56 against 32) is found the same way, by variant and measurement, not by guessing.
- Cost to weigh: a second `sampleDistance` call site lengthened the cold pipeline compile on box 206 in the one run measured (bug-996 takes that off the first frame). After bug-995, which edits the same loop.

## bug-staging-23 round 2 (agent D, 2026-10-12): the centre sample is straight-line code

Base `dce8984c7`. The 10,000-quad row is still OPEN: the renderer is no longer twice
the program's cost on box 206, but it is still as large as it (numbers at the end).

**What changed.** `mfb_canvas.frag:shapeCoverage` and its twin in
`metal.rs:METAL_SHADER_BODY`, line for line: the shape's centre sample is taken before
the loop, and the loop (now samples 1 to 9) is entered only under
`mapped || clipSampled`. Same samples, same order, same arithmetic; the centre point is
`x`, `y` itself where the loop added a zero offset to it (a double-float sum of a
normalised pair and zero returns the pair). bug-995's clip early-out is unchanged
(sample 5 still takes `clipReach`, `clipMapped` still goes false past it).
`sampleDistance` now has two call sites.

**RED / GREEN.** `bash tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host
test@192.168.1.206 shapes` (new subcommand: 10,000 static items of radius 8, `--debug`,
headless, each row twice; `phaseRenderMs` a frame):

| row | base `dce8984c7` | after | one item alone |
|---|---|---|---|
| 10,000 squares | 110.3, 100.1 | 57.8, 57.9 | 8.0 |
| 10,000 circles | 109.0, 102.6 | 54.2, 54.4 | 8.4 |
| 10,000 4-point polygons | 111.0, 101.0 | 88.8, 88.8 | 8.1 |

The polygon gains least: its sample is `edgeDistance`'s two passes over the edges, which
the restructure does not touch, where a square's or a circle's is `farBound` and, near
the outline, `dfShapeDistance`. The sweep's quads are polygons.

**Frames byte-identical, before and after, on each device** (`cmp` of every frame):

| device | how | result |
|---|---|---|
| Metal and software, this machine | each render row of `scripts/test-canvas-gpu-rows.sh` and `bugs/repro/bug-991/anisotropic_halo.mfb`, `mfb build --app --debug`, `MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0|1 MFB_CANVAS_DUMP=…` with both compilers | 98 of 98 frames identical (49 scenes, software and Metal) |
| box 2226 (lavapipe) | `scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64 --keep <dir>` with both compilers | 96 of 96 identical (48 rows, software and Vulkan); every row passes |
| box 206 (Intel UHD 630) | the same with `--target windows-x86_64` | 96 of 96 identical (48 rows, software and Vulkan); every row passes |

**Variants measured on box 206** (10,000 squares, ms of render a frame, two runs; the
scratch sources are not kept). Each was built as a whole compiler with only the
fragment blob changed. Readings taken in different sessions differ by about 5 ms, so
each is beside the kept structure's reading from the same session.

| variant | reading | kept structure, nearest reading | verdict |
|---|---|---|---|
| the kept structure (centre sample out of the loop, second call site) | 57.8, 57.9 | | kept |
| one call site, the loop's bound and index decide what runs (`last = 1`, raised after the centre sample; no `continue`) | 106.6, 101.6 | 70.0, 70.4 | no gain over the base |
| one call site, the same loop with no `d[10]` (the differences taken as the samples arrive) | 178.6, 169.7 | 57.8, 57.9 | 1.7 times slower than the base |
| kept, and no `polygonBand` call (diagnostic: wrong for polygons) | 65.1, 64.9 | 63.3, 67.6 | no gain |
| kept, and `polygonBand` written with one exit | 60.4, 68.0 | 63.3, 67.6 | no gain |
| kept, and the neighbours' array declared inside the branch, gradients taken there | 71.4, 62.2 | 70.0, 70.4 | within noise |
| kept, and the mapped centre computed once under an `if` (it is computed up to three times) | 59.2, 57.5 | 56.9, 60.0 | no gain |
| kept, and the centre call given constant arguments (diagnostic) | 57.5, 58.4 | 56.9, 60.0 | no gain |
| kept, and `shapeCoverage` returning right after the centre sample, the rest still reachable (diagnostic) | 64.5, 59.6 | 70.0, 70.4 | within noise |
| the same with the rest unreachable (diagnostic) | 51.8, 50.5 | 56.9, 60.0 | 7 ms |
| the same again without the `polygonBand` call (diagnostic) | 36.7, 36.7 | 56.9, 60.0 | 14 ms here, none with the rest reachable (row 4): the costs do not add |
| one `sampleDistance` and `dfCoverage` in `main`, everything else unreachable (the Cause note's 32) | 34.9, 33.8 | 56.9, 60.0 | the floor for this arithmetic |
| the same one sample taken first, everything else still reachable (diagnostic) | 43.0, 45.3 | 56.9, 60.0 | 10 ms for code that does not run |
| kept, and `edgeDistance`'s second pass nested under an `if` instead of `continue` (4-point polygons) | 91.2, 91.1 | 88.8, 88.8 | no gain |

**What is left, and why.** The gap from the kept structure (58) to one sample (34) is
not a statement the pixel runs. The two diagnostics that run the SAME path cost 34 with
the rest of the shader unreachable and 44 with it reachable, and cutting off only the
neighbour, stroke and clip code that an unclipped square never runs took 7 ms off; no
change to a statement the pixel does run (the `polygonBand` call, the array, the centre
arguments, the map) moved the reading. So what remains is what Intel's compiler charges
every pixel for the size of the one live shader, and the only way under it with this
arithmetic is a second, smaller pipeline for the items that need less of it, which is a
fast path chosen by naming what an item is. Not done. For polygons the cost above a
square is the edge walk itself (`edgeDistance`, 31 ms for 10,000 4-point polygons), and
the one restructure tried there gained nothing.

**Cold pipeline compile on box 206** (`bugs/repro/bug-996/coldstart.mfb`, a fresh exe
name each launch, `firstPresentMs`; the second launch of each name was 68 to 74 ms):

| compiler | launch 1 | launch 2 |
|---|---|---|
| base | 36,249 | 35,041 |
| after | 70,165 | 64,685 |

The second `sampleDistance` call site doubles the cold compile. Both structures with one
call site compiled in the base's time (35.9 and 39.0 s) and neither was faster than the
base at drawing, so the faster frame is kept and the compile is bug-996's to take off the
first frame. A program's first GPU frame on this driver now waits about 67 s instead
of 35 until then.

**The sweep, box 206** (`bash tools/canvas-bench/bench.sh <mfb> --target windows-x86_64
--host test@192.168.1.206 sweep`, `--debug`, each compiler twice; fps / render ms):

| row | base | after |
|---|---|---|
| 95 x 8-pt moving | 94.0 / 10.5, 95.0 / 10.4 | 105.8 / 9.3, 106.2 / 9.3 |
| 1,000 x 8-pt moving | 31.8 / 30.4, 31.8 / 30.3 | 46.6 / 20.3, 46.8 / 20.3 |
| 1,000 x 8-pt static | 32.4 / 30.4, 32.4 / 30.3 | 48.2 / 20.4, 48.0 / 20.5 |
| 2,000 x 8-pt moving | 18.6 / 51.7, 18.4 / 51.8 | 28.8 / 32.4, 28.8 / 32.5 |
| 2,500 x 8-pt moving | 15.2 / 62.8, 15.0 / 62.8 | 24.0 / 38.8, 23.8 / 38.8 |
| 5,000 triangles moving | 12.8 / 73.0, 12.8 / 73.0 | 20.0 / 38.3, 19.8 / 38.6 |
| 10,000 quads moving | 8.4 / 107.6, 8.2 / 108.0 | 9.2 / 92.5, 9.2 / 92.7 |
| 10 x 300-pt moving | 103.8 / 9.5, 102.6 / 9.6 | 99.0 / 9.9, 98.8 / 10.0 |
| 100 x 300-pt moving | 24.0 / 24.8, 24.0 / 24.8 | 24.2 / 23.7, 24.2 / 23.7 |
| 10 x 4,000-pt moving | 30.2 / 31.7, 30.0 / 31.7 | 30.6 / 31.2, 30.6 / 31.3 |

`10 x 300-pt` reads 0.4 ms slower in both runs; every other row is faster or level.

**The same sweep on Metal** (this machine, no `--target`; each compiler twice, in the
order base, after, base, after, at load average 8 to 13 from other agents' work, so
indicative). fps / render ms:

| row | base | after |
|---|---|---|
| 95 x 8-pt moving | 414.4 / 2.4, 341.4 / 2.9 | 372.8 / 2.6, 326.8 / 3.0 |
| 1,000 x 8-pt moving | 133.6 / 4.7, 129.6 / 4.4 | 148.2 / 4.4, 117.8 / 4.4 |
| 1,000 x 8-pt static | 253.6 / 3.8, 226.4 / 4.3 | 160.6 / 4.7, 217.4 / 4.4 |
| 2,000 x 8-pt moving | 84.6 / 4.6, 59.0 / 5.5 | 78.2 / 5.3, 59.0 / 5.4 |
| 2,500 x 8-pt moving | 54.8 / 6.7, 62.0 / 5.4 | 66.0 / 4.8, 47.4 / 5.9 |
| 5,000 triangles moving | 29.6 / 9.8, 41.2 / 5.9 | 47.2 / 5.1, 32.6 / 6.7 |
| 10,000 quads moving | 21.6 / 11.1, 16.2 / 13.5 | 21.8 / 10.7, 16.2 / 12.2 |
| 10 x 300-pt moving | 415.0 / 2.4, 361.6 / 2.7 | 379.4 / 2.6, 316.2 / 3.1 |
| 100 x 300-pt moving | 78.6 / 4.0, 55.2 / 5.1 | 54.6 / 5.2, 58.4 / 5.2 |
| 10 x 4,000-pt moving | 95.4 / 5.7, 65.4 / 7.0 | 66.6 / 6.7, 72.2 / 6.7 |

Two rows read slower in both runs after than in both runs before: `1,000 x 8-pt static`
(4.7 and 4.4 against 3.8 and 4.3) and `100 x 300-pt` (5.2 and 5.2 against 4.0 and 5.1).
The spread between two runs of ONE compiler is larger than either (9.8 and 5.9, 4.0 and
5.1), so they are not shown to be the shader. `shapes` on Metal, 10,000 items, render ms, four readings each at load 36
to 67: squares base 14.8, 15.6, 13.9, 11.5 and after 12.3, 10.4, 12.3, 10.5; circles base
68.0, 12.0, 11.9, 11.6 and after 10.7, 8.4, 10.6, 10.4; polygons base 13.4, 14.5, 14.9,
12.4 and after 11.5, 10.3, 13.9, 14.9. `clips` on Metal ran twice with each compiler at
load 33 to 90; no row is slower in both runs. "Metal is not slower on any row" is
consistent with these and NOT proven by them: it needs the machine alone
(`bash tools/canvas-bench/bench.sh <mfb> sweep`, `shapes` and `clips` with each compiler
at load under 4).

**What the renderer costs now, on box 206.** 10,000 static squares: 58 ms of render a
frame, of which 8 is the frame's fixed cost (one item alone), against 34 for the same
quads with one distance sample and nothing else live in the shader, and 14 with a
constant colour (the Cause note's figure). 10,000 moving 4-point polygons (the sweep's
quad row, `--debug`): 93 and 96 ms of render a frame, while the program spends 81 ms
building each scene and 13 ms in `present` (`presents=48 workerBuildMs=3886
workerPresentMs=618` over 5 s). So the renderer is no longer the only limit on that row,
and it is still as large as the program: the acceptance ("the renderer path is not the
limit at 10,000 moving items") is not met.

### Golden impact

`scripts/artifact-gate.sh <mfb> all` lists the same nine diffs with the base compiler
(`dce8984c7`) and with this change: `app_mouse_surface` `.ir`, `.macos-aarch64.app.nir`,
`.nplan` and the four targets' `.app.ncode`, `app_window_surface.macos-aarch64.app.ncode`,
and `loop_body_resource_reference_valid.ir`. They are round 1's, not yet regenerated;
this change adds no fixture to the list. What it moves is inside fixtures already on it:

- `mfb_canvas.frag.spv` is regenerated (`MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh`;
  the vertex blob came back unchanged), so the `linux-*` and `windows-x86_64`
  `app_mouse_surface` `.app.ncode` hashes move by the embedded blob (data only).
- The `macos-aarch64` `.app.ncode` hashes move by the MSL string constant
  (`METAL_SHADER_BODY`). No function's instructions change on any target, and nothing
  moves at the `.ir`, `.nir` or `.nplan` level.
- No canvas pixel golden moves (`rt_canvas_golden`, `rt_canvas_rasteriser`,
  `rt_canvas_curves`, `rt_canvas_metal`: all passed with the change).

## Re-verified 2026-10-12: two items remain, both as recorded (HEAD `555130a25`)

What is still open is only this, and the title's four complaints are no longer the state
of the code (V1 to V4 are fixed, and V6 is fixed on Windows):

1. **The 10,000-quad row is still renderer-limited on box 206 (V5 acceptance not met).**
   `bash tools/canvas-bench/bench.sh target/debug/mfb --target windows-x86_64 --host
   test@192.168.1.206 sweep` (debug build, headless, 900×640):

   | row | fps | render ms/frame | 2026-09-27 |
   |---|---|---|---|
   | 1,000 × 8-pt moving | 32.6 | 29.5 | 24.8 / 26.3 |
   | 1,000 × 8-pt static | 33.2 | 29.8 | 34.8 / 26.3 |
   | 2,000 × 8-pt moving | 18.2 | 52.4 | 12.0 / 40.5 |
   | 2,500 × 8-pt moving | 15.6 | 60.9 | 9.0 / 47.6 |
   | 5,000 triangles moving | 13.2 | 70.7 | 6.6 / 51.5 |
   | **10,000 quads moving** | **8.6** | **103.8** | 3.6 / 115.5 |
   | 10 × 300-pt moving | 104.6 | 9.4 | 67.0 / 13.6 |
   | 100 × 300-pt moving | 24.6 | 23.9 | 6.4 / 27.5 |
   | 10 × 4,000-pt moving | 31.0 | 30.9 | 9.8 / 35.1 |

   The frame rates rose (the program side got cheaper), but the render phase did not: the
   8-pt rows now render slower than on 2026-09-27 (2,000 items: 52.4 ms against 40.5), and
   10,000 quads still take 104 ms of render a frame. The fragment shader is larger than it
   was then (`mfb_canvas.frag.spv` 298,680 bytes against 248,628 on 2026-10-06, `ls -l`);
   whether that is the cause was not measured. The sweep's first row (95 × 8-pt) is not in
   the table: it reported `fps=0.4 phaseRenderMs=18500` because it was the first launch
   of that exe name and paid bug-996's pipeline compile. The bench does not discard a
   first run.
2. **Linux present is still readback-only.** `vulkan_present.rs:has_present_path` is
   `matches!(platform.family(), PlatformFamily::Windows)`. Still blocked on a real-GPU
   Linux box (`.ai/remote_systems.md`: every Linux Vulkan driver is lavapipe) and on the
   GTK4 design decision below.

Not re-run: the correctness rows as a set (`scripts/test-canvas-gpu-rows.sh`; two rows ran
on box 206 for bug-1079 and `pictureshapes` passed), and the real-window present on box
206 (bug-1045's probe did run there: 120 of 120 frames `directFrames`).

Stale below, left as history: box 2230 is retired (owner, 2026-10-06); every Windows row
runs on box 206. The frame caps are also what stops bug-765's 100,000-dot scene on both
GPU backends (65,536 items a frame).

## STATUS (2026-09-27): V1-V4 FIXED; V5 measured, two fixes landed; V6 FIXED on Windows

**V6 — FIXED on Windows: a windowed Vulkan frame presents through a swapchain.**
`canvas::vulkanPresentScene` (`src/codegen/runtime/canvas/vulkan_present.rs`) records the
frame into an acquired swapchain image and presents it (FIFO); `__canvas_renderVulkan`
takes it under Metal's conditions (no damage mode, no dump) and falls back to the readback
path whenever it answers FALSE — headless, no WSI extensions, a surface that is not the
canvas size, no image within 100 ms, every non-Windows target. `directFrames=` counts it.
Spec: `mfb spec` app/06_canvas "On Windows a Vulkan frame in a real window goes to a
swapchain".

Measured on box 206 (debug build, 95 × 8-pt moving, fullscreen 1360×768 — the console
session's resolution, so 1080p is not reachable there; `MFB_CANVAS_STATS` over 10 s):

| path | GPU frames / 10 s | fps |
|---|---|---|
| readback (every frame declined to readback) | 441 | 44.1 |
| swapchain present | 590 (all direct) | 59.0 — the display's 59 Hz, FIFO-capped |

Pixels: a Desktop Duplication capture of the direct frame and of the readback frame have
identical colour histograms (`(0,0,0)` 1,025,080 / `(0,162,207)` 14,820 / `(0,105,135)`
760 …).

Three things the implementation had to get right, each found on the real box:
- **The swapchain presents to a child window, not the canvas window.** With it on the
  top-level window, `app::setMode(Console)` left the last canvas frame over the transcript
  (Desktop Duplication capture). The graphics thread owns the Vulkan objects and nothing
  wakes it on a mode change, so the fix is a hidden `STATIC` child the UI thread shows on
  `WM_APP_CANVAS_SHOW` (posted after each direct present, honoured only in canvas mode)
  and hides on a readback frame's `WM_APP_BLIT` and on every reconcile out of canvas mode.
- **Entering canvas mode now publishes the client size** (pre-existing Windows bug). The
  canvas stayed at its 900×640 default in the default 400×300 window (client 384×261)
  until something resized the window: those frames were clipped, and could never present
  directly because the surface was not the canvas size.
- **The surface-format search built its array address with `add_registers(sp, …)`**, which
  reads register 31 as zero, and so declined every frame (`exit=40` from a throwaway probe
  build). The base goes through `add_immediate` as everywhere else in `vulkan.rs`.

Input with the child shown: keys verified on box 206 (`SendKeys` "q"/"z" arrive as press
and release through `io::readKey` while every frame presents directly). The mouse, re-checked
after bug-758 (Windows app mouse never delivered) was fixed: with every frame presenting
directly through the child, injected moves to client (50,40) and (60,45) and a left click
arrive as `Move 50,40`, `Move 60,45`, `Down Left 60,45`, `Up Left 60,45` — the `STATIC`
child's `HTTRANSPARENT` passes them to the canvas window as intended.

Transition run (windowed → fullscreen → windowed → Console → canvas, 8 s each): 1,899 of
1,900 GPU frames direct; every phase's Desktop Duplication capture shows the scene, and
the Console phase shows the transcript. **GDI screen grabs are not evidence here** — during
the fullscreen phase they returned frozen or background-only images while every frame
presented; `tools/win-screen-capture` is the instrument (`.ai/remote_systems.md`).

**Linux present — BLOCKED on hardware.** The Fix Design's Linux half (`VK_KHR_xlib_surface`
/ `VK_KHR_wayland_surface` under GTK) needs a real Linux GPU to measure against the
readback path, as V6 requires; every reachable Linux Vulkan driver is lavapipe, where the
"GPU" is the CPU. It also needs a design decision GTK4 forces: GTK composites its own
window, so a swapchain needs an X11 child window, a Wayland subsurface, or a dmabuf
handed to GTK as a texture. Linux keeps the readback path (`has_present_path` is
Windows-only).

### Earlier status (2026-09-27): V5 measured on a real GPU, two fixes landed; V6 open

**Hardware now reachable.** Box 206 (`ssh test@192.168.1.206`, `.ai/remote_systems.md`):
Windows 11 on a real Intel UHD Graphics 630 with Intel's Vulkan 1.3 driver. The Phase 0
blocker is resolved for Windows; there is still no real-GPU Linux box.

**Instrument.** `tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host
test@192.168.1.206 <subcommand>` (Windows mode, `42c207a2e`): builds the `.exe`, runs it
headless (`MFB_WINAPP_HEADLESS=1`) through a CRLF `.bat`, copies stats/dumps back. Stats
need a `--debug` build (plan-130-E); a headless run needs `MFB_CANVAS_GPU=1` to use the GPU.

**V5 finding 1 — fixed (`42c207a2e`).** Every frame's render phase carried a fixed ~200 ms
(debug sweep: 95 items 204 ms, 10k quads 296 ms). The readback buffer took the first
`HOST_VISIBLE|HOST_COHERENT` memory type, which on this driver is type 1, write-combined
and uncached for the CPU's per-pixel reads (`vulkaninfo`: type 2 adds `HOST_CACHED`). It
now asks for `HOST_VISIBLE|HOST_COHERENT|HOST_CACHED` first and falls back to plain
coherent (`READBACK_MEMORY_CANDIDATES`; unit test
`the_readback_buffer_prefers_cached_memory`). Debug sweep after: 95 items 204 -> 13.4 ms
render (5 -> 71 fps), 1k 222 -> 26 ms (4.4 -> 24.6 fps), 10 x 300-pt 209 -> 17 ms.

**V5 finding 2 — fixed (`cc5e0ea53`).** The shared edge/band/gradient/picture buffer and
the item buffer took the same write-combined type, and the band-index builder reads back
edges it just wrote. Same candidates now: 100 x 300-pt 62.6 -> 27.5 ms, 10 x 4000-pt
81.3 -> 35.1 ms, 10 x 300-pt 17.2 -> 13.6 ms; quad rows unchanged.

Correctness on the real GPU after both: `poly 1000` and `poly 4000` maxdelta 1 (unchanged
from before the fixes); `pics 2000 32 0 0` oracle pixels=0.

**V5 numbers now (debug build, box 206, 900x640, `MFB_CANVAS_GPU=1`):**

| row | fps | render ms/frame |
|---|---|---|
| 95 x 8-pt moving | 71.4 | 13.4 |
| 1,000 x 8-pt moving | 24.8 | 26.3 |
| 1,000 x 8-pt static bulk | 34.8 | 26.3 |
| 2,000 x 8-pt moving | 12.0 | 40.5 |
| 2,500 x 8-pt moving | 9.0 | 47.6 |
| 5,000 triangles moving | 6.6 | 51.5 |
| 10,000 quads moving | 3.6 | 115.5 |
| 10 x 300-pt moving | 67.0 | 13.6 |
| 100 x 300-pt moving | 6.4 | 27.5 |
| 10 x 4,000-pt moving | 9.8 | 35.1 |

**OPEN — 10k quads is renderer-limited (V5 acceptance not met).** Same 10,000 quads at
radius 8 vs 0.6: 128 ms vs 46.8 ms render, so ~80 ms is GPU fragment work (~20 ns per
covered pixel — heavy for 4-edge coverage; bug-735's double-float coverage on this weak
iGPU is the first suspect) and ~47 ms is per-item upload plus the fixed frame cost.
`draws=0:10001:0:0:0` — one draw, so not draw-call overhead.

**V6, full-screen present (resolved above).** The 95-item scene's render phase scales with pixels:
900x640 13.2 ms, 1920x1080 38.1 ms, 2560x1440 64.6 ms (~17 ns/pixel), over the 16.6 ms
60 Hz budget at 1080p. Not yet split between the readback path (copy + CPU swizzle +
blit, which a swapchain removes) and GPU shading (which it does not). Next step: time the
readback section, then decide the swapchain per the Fix Design.

**V6 split (2026-09-27, box 206, 95-item scene, debug build, headless).** Throwaway compiler
builds with pieces removed (not committed), render ms/frame:

| build | 1920x1080 | 900x640 |
|---|---|---|
| normal (readback path) | 38.2 | 13.2 |
| no CPU swizzle | 35.0 | |
| no swizzle, no `vkCmdCopyImageToBuffer` | 32.7 | |
| ... and no `canvas::blitSurface` | 25.7 | |
| ... and no `vkQueueSubmit` (no GPU at all) | 23.3 | 7.9 |

So at 1080p the GPU itself is ~2.4 ms. The rest is CPU work the readback path does
per pixel: copy + swizzle ~5.5 ms, the Windows blit ~7 ms, and ~10 ns/pixel of surface
work (`canvas::newSurface` allocating and zeroing the frame is the largest share); the
fixed recording/upload cost is ~2 ms. A directly presented frame skips the surface, the
copy, the swizzle and the blit — the bug-686 Phase 4 finding on Metal, repeated here —
so by these numbers it lands near 5 ms at 1080p against the readback path's 38 ms, which
is over the 16.6 ms budget on its own. **Decision (per the Fix Design): build the
swapchain present.**

**Noted, not reproduced:** one `pics 2000` run took 2.6 s/frame; two reruns gave 55 ms.

### Earlier status: V1-V4 FIXED, V5/V6 BLOCKED

Landed on `worktree-B-688` and merged to `main`. What holds, with the commands behind it:

- **V1 (no ordinary scene falls to software).** Vulkan's caps are Metal's (65,536 quads,
  262,144 edges, 131,072 stops, 8M texels), generated into the predicate, and the shader's
  region bases are specialization constants (`937ddc149`).
  `scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64` (box 2226) and
  `--target windows-x86_64` (box 2230): every render row draws on Vulkan
  (`gpuFrames=1`) within `GPU_DEFAULT`: quads worst 0, 40k opaque edges worst 1 /
  1.09%, gradients 0, tilemap 0, 1080p background 0, large polygons 1 / 0.26%,
  64k polygon 1 / 0.03%, layers 1 / 0.84%. At `main` the first six were declined
  (`gpuFrames=0`).
- **V2 (band index).** `05c5ffa81`. The GPU frame with bands against the full loop on
  the same device (2226), byte-compared: identical for large_polygons, huge_polygon,
  forty_thousand_opaque_edges and a worst-case probe ({4001/1999} star, 4,000 spikes, a
  stroked 3,000-edge ring under a shearing transform).
- **V3 (pictures once per block).** `937ddc149`: the per-frame picture table and the
  distinct-block predicate count; the tilemap row (2,000 tiles, two images) draws on
  Vulkan.
- **V4 (shared native paths on x86-64 and AArch64).** `MFB_CANVAS_GEO_VERIFY=1` on every
  row and the four `rt_canvas_geo_native` programs: `geoVerifyMismatches=0` on
  linux-aarch64 (2226) and windows-x86_64 (2230), including the publish race (2226: 15
  frames on the GPU; 2230: 3 frames, software — the GPU run is too slow there to race,
  see `df4f589a7`). **Not run on linux-x86_64** (2228/2227): this fix was limited to
  boxes 2226 and 2230. x86-64 code generation is covered by 2230 (same ISA, Win64 ABI).
- **A bug found and fixed on x86-64** (`069348f8b`): `emit_state_load`'s temporary vreg
  could be coloured onto a live hand-assigned `SCRATCH` register; the band index faulted
  on Windows (`0xC0000005`). The helper now uses no temporary.
- **V5 / V6 BLOCKED.** Every reachable Vulkan driver is Mesa's CPU lavapipe
  (`.ai/remote_systems.md`), so no frame-rate or present-cost number here would describe a
  GPU. Per Open Decisions ("Hardware", recommended), no swapchain was built on a guess.
  The instrument is ready: `tools/canvas-bench/bench.sh --target linux-… --box <port>`
  (`bd49be0d0`). What remains is Phase 4, on a machine with a Vulkan GPU.

Deviations:

- **The Vulkan 40k-edge row uses `forty_thousand_opaque_edges.mfb`.** The translucent
  bug-686 scene draws on Vulkan, but lavapipe's blending drifts one step from the oracle
  per 20+ stacked blends (11.8% of the frame, every overlapped pixel off by exactly 1,
  measured per overlap depth) — blend precision, not edges.
- **`large_polygons.mfb` gained a grey ground**: over black, lavapipe tripped bug-687's
  one-coverage-step flip (evidence recorded in bug-687).
- **Band builder ported, not hoisted** (Open Decisions' alternative): hoisting would have
  re-emitted Metal's instructions, which the non-goals keep unchanged.
- **The `app-window-surface` linux-x86_64 sentinel was stale at `main`** and was corrected
  (`df722b970`).

Gates: `cargo test --release --no-fail-fast` — 227 suites, 6,032 passed, 0 failed;
`scripts/artifact-gate.sh <mfb> all` — at main 1 diff (the stale sentinel), on the branch
the 7 app-mouse-surface diffs, localized to `__canvas_vulkanRenderable` and the Vulkan
emitter and regenerated (`7668925a4`); `scripts/test-canvas-vulkan.sh --box 2226 --target
linux-aarch64` passed; `scripts/test-winapp.sh` (2230) passed; `rt_canvas_metal` 19
passed; `scripts/spec-census.sh --citations` 0 misses.

bug-686 made Metal draw ordinary scenes on the GPU and made the graphics thread fast.
The shared half landed for every backend. Vulkan (Linux and Windows, `has_vulkan_backend`,
`src/codegen/runtime/canvas/vulkan.rs`) got **none** of the Metal half:

- Its frame caps are still the ones bug-686 measured sending wind and 5,000-quad scenes to
  software.
- Its polygon shader still loops over every edge per pixel.
- It copies a picture's texels once per occurrence, not once per image.
- Its gradient region base is a hand-maintained literal in checked-in GLSL.
- Every frame is read back and blitted, even in a real window.

The shared half — the native geometry resolve, the native item hash, the carried hashes,
the scene sequence lock and the damage-only work — is built for Vulkan's targets but has
**never run on x86-64**. The bug-686 agents built it for `linux-x86_64`, `windows-x86_64`
and `linux-aarch64`, but ran it only on macOS.

**The single correct behavior a fix produces:**

- On a Linux or Windows machine with a Vulkan GPU, every scene bug-686 put on Metal draws
  on Vulkan, not software: `gpuFrames = frames`, within `Tolerance::GPU_DEFAULT` of the
  software oracle.
- The same frame-rate rows bug-686 measured on Metal hold on that hardware, within the
  hardware's own GPU budget.
- The shared bug-686 native paths are proven on x86-64 at run time: `MFB_CANVAS_GEO_VERIFY`
  reports 0 mismatches.

References:

- `bugs/completed/bug-686-metal-declines-ordinary-scenes-and-falls-back-to-software.md` —
  the Metal work this bug mirrors. Its sections B–H are the measurements and its Phases 1–4
  are the designs.
- `.ai/canvas-threading.md`: §6 (pictures), §10 (the GPU backends, caps, predicates,
  direct present), §11 (stats), §14 (geometry cache, native store, sequence lock), and the
  "Growing `ITEM_BLOCK_SIZE`" list, whose item 5 names Vulkan's `GRADIENT_BASE` literal.
- `src/docs/spec/app/06_canvas.md` — the spec's canvas topic. It describes the Metal caps,
  band index, picture dedupe and direct present, and gives Vulkan's old caps.
- `.ai/remote_systems.md` — the Linux boxes (2228 glibc, 2227 musl) are **QEMU-TCG-emulated
  x86_64 VMs**.
- bug-687 (open): the fp32 coverage flip. It applies to the GLSL shader as much as to MSL.
- plan-152: the program-side cost spike. It applies to every backend.

## Failing Reproduction

Correctness rows run on the Linux box through the existing oracle harness:

```
cargo build --release
scripts/test-canvas-vulkan.sh target/release/mfb --box 2228
```

To reproduce each gap, add bug-686's `rt_canvas_metal.rs` scenes to that harness. The
expectation for each row is "Vulkan draws it, within `GPU_DEFAULT` of software".

| scene (bug-686 section) | Metal at `main` | Vulkan at `main` (expected from the code; Phase 1 measures it) |
| --- | --- | --- |
| 5,000 quads (B) | Metal | software: `quads > __CANVAS_MAX_FRAME_ITEMS` (4,096) |
| wind, ~5,700 items (A) | Metal | software: same cap |
| 40,000 total polygon edges (B) | Metal | software: `total > __CANVAS_VULKAN_MAX_FRAME_EDGES` (16,384) |
| 2,000 tiles from two 32×32 images (H) | Metal | software past 1,024 tiles: per-occurrence texel count against 1M |
| 1920×1080 background (H) | Metal | software: 2.07M texels > 1M |
| 5,000 gradient stops in a frame | Metal | software: `MAX_FRAME_GRADIENT_STOPS` (4,096) |
| one 64,000-edge polygon (E/G) | Metal, band index | software (frame edge cap); even under the cap, the shader loops over every edge |

Contrast cases that work today and must keep working: every scene inside the old caps.
Those are what `test-canvas-vulkan.sh` asserts at `main`.

Environment matrix. **Phase 0 must fill it in; nothing here has been run yet:**

| Environment | GPU | Can check correctness | Can check frame rate |
| --- | --- | --- | --- |
| box 2226, Debian 12 aarch64 glibc (native VM, GNOME Wayland session) | virtio; Mesa lavapipe | **yes — run** | no: CPU driver |
| box 2230, Win11 x86_64 (emulated) | virtio; Mesa lavapipe (`C:\mfbvk`) | **yes — run** | no |
| box 2228, Ubuntu x86_64 glibc | QEMU-TCG VM, software Vulkan ICD | yes | no — not run by this fix |
| box 2227, Alpine x86_64 musl | QEMU-TCG VM, user-local ICD (`--icd auto`) | yes | no — not run by this fix |
| Windows x86_64 with a GPU | — (none reachable) | — | — |
| Linux x86_64 or aarch64 with a GPU | — (none reachable) | — | — |

## Root Cause

bug-686 split into a **shared** half (graphics thread, worker, scene ring) and a
**Metal-only** half (the emitter, shader and predicate in `metal.rs` / `helper_render.rs`).
"Metal only" was a deliberate scope decision in bug-686 ("Vulkan is out of scope"). So each
Metal-only change left a Vulkan twin behind:

1. **Frame caps.** `__canvas_vulkanRenderable` (`src/codegen/builtins/canvas/helper_render.rs`)
   still gates on:
   - `CANVAS_MAX_FRAME_ITEMS` = 4,096;
   - `VULKAN_MAX_FRAME_EDGES` = 16,384;
   - `MAX_FRAME_GRADIENT_STOPS` = 4,096;
   - `VULKAN_MAX_FRAME_GLYPH_SAMPLES` = 1 << 20.

   All four are in `src/codegen/runtime/canvas/mod.rs`. The frame buffer is sized from them
   (`VULKAN_BUFFER_BYTES`), so raising a cap moves every region after it.
2. **Region bases.** The fragment shader's `GRADIENT_BASE` is a literal (`const int
   GRADIENT_BASE = 1114112;`, `src/codegen/runtime/canvas/shaders/mfb_canvas.frag`),
   compiled to the checked-in `mfb_canvas.frag.spv` by `scripts/regen-spirv.sh` and pinned
   to `VULKAN_GRADIENT_BASE_WORDS` only by a unit test. Metal's bases are formatted into its
   MSL at first use (bug-686). Vulkan's SPIR-V is a binary compiled ahead of time, so the
   Metal technique does not transfer as-is (see Fix Design).
3. **Polygon fill.** `edgeDistance` in `mfb_canvas.frag` loops `for (int e = 0; e < count;
   ++e)` over the polygon's edges, per pixel. This is the O(edges × covered pixels) cost
   bug-686 section E measured at 252 ms for 64,000 edges on Metal before the band index.
4. **Pictures.** `emit_picture_upload` (`vulkan.rs`) copies an item's texels into the glyph
   region per item. The predicate adds `__canvas_pictureSamples(offset)` per item. Metal
   copies each distinct block once (`emit_picture_lookup`, a per-frame picture table) and
   counts distinct blocks.
5. **Present.** `canvas::vulkanDrawScene` (`emit_vulkan_draw_scene`) always renders
   offscreen, waits with `vkQueueWaitIdle`, copies image → buffer, and hands the frame to
   `canvas::blitSurface`. On Metal that path cost ~10 ms per megapixel of fixed work before
   bug-686 Phase 4 presented straight to a `CAMetalLayer` (root cause 10 there). Vulkan has
   no swapchain path. The doc comment gives the reason: "no reachable Linux box has a
   display server".
6. **Shared native paths unproven on x86-64.** These are emitted through arch-neutral
   `CodeBuilder`/`abi::` code, so they **build** for x86-64:
   - `canvas::geoBuild` (`func_geo_build.rs`);
   - `canvas::sceneResolve` / `sceneLayout` / `sceneDrawsFlat` / `geoBeginFrame` / the
     table calls (`func_geo_cache.rs`);
   - `canvas::itemHash` / `sceneHashes` (`func_item_hash.rs`);
   - `canvas::carriedHashes`;
   - the scene sequence lock (`scene_base.rs`: `emit_scene_store`, `emit_scene_load`,
     `emit_mark_pending`, `emit_scene_barrier`).

   **None has executed on x86-64.** The sequence lock has **no fences** off aarch64
   (`orders_scene_accesses`). That is correct on x86-64 only because x86-64 keeps stores in
   order, and the doc comment records one step that relies on the store buffer draining
   within a frame.

Hypotheses Phase 0 has to settle, each with how to settle it:

- **H1:** the item buffer, which `ITEM_BLOCK_SIZE` sizes against `CANVAS_MAX_FRAME_ITEMS`,
  is mapped HOST_VISIBLE and sized at pipeline creation. Raising the cap to 65,536 grows it
  16×. The pipeline must still build on a low-memory device. **Settle:**
  `vulkanReady=TRUE` on the boxes with the new caps, and the size in the stats line.
- **H2:** SPIR-V specialization constants (`VkSpecializationInfo`) can carry the region bases
  and band parameters into the checked-in shader. That needs no `glslangValidator` at build
  time, and the vendor Mesa / NVIDIA / AMD drivers honour them. **Settle:** a spike shader
  on the box's ICD.
- **H3:** `vkCmdCopyImageToBuffer` from an OPTIMAL-tiled image is a GPU copy, so bug-686's
  "linear render target" fix (a CPU decompress in `getBytes:`) has **no Vulkan twin**.
  **Settle:** a profile of the readback on real hardware.

## Goal

- **(V1)** No ordinary scene falls to software on Vulkan. Every bug-686 row above draws on
  Vulkan within `GPU_DEFAULT` of software, including 65,536 quads, 262,144 frame edges,
  131,072 gradient stops and 8M picture/glyph texels. Caps and region bases have one
  definition in Rust, and the MFBASIC predicates are generated from it (`RENDER_CAPS`).
- **(V2)** A polygon of any edge count draws on Vulkan through the band index, with the same
  pixels as the full loop.
- **(V3)** A picture's texels upload once per distinct image per frame. The predicate counts
  distinct images.
- **(V4)** On Linux and Windows x86-64 (and Linux aarch64), `MFB_CANVAS_GEO_VERIFY=1` reports
  0 mismatches on every bug-686 row, including the snapshot race test's alternating scenes.
  The sequence lock's x86-64 ordering argument is re-checked against those runs.
- **(V5)** On real GPU hardware, measure the bug-686 fps rows and wind on Vulkan (release,
  `MFB_CANVAS_SYNC=1` and pipelined). The renderer path must not be the limit at 10,000
  moving items. The Metal reference is ~13 ms per frame at 10k quads, where the program's
  own build is the limit.
- **(V6)** In a real window, measure present without readback (swapchain) against the
  readback and blit. If the readback threatens the budget at full-screen, present through
  a swapchain. The oracle tests keep a readback path, as on Metal.

### Non-goals (must NOT change)

- **Software stays the oracle.** Every Vulkan change is accepted on "agrees with software
  within `Tolerance::GPU_DEFAULT`". A golden's byte-identity is not the acceptance test.
- **The honesty gate.** A scene the backend truly cannot draw is declined, never clamped or
  truncated.
- **Metal.** Nothing in `metal.rs` or the Metal predicate changes. Any shared refactor
  (for example hoisting the band-index builder) must leave Metal bit-identical:
  `rt_canvas_metal.rs` and the app-mouse-surface sentinel must move only by the intended
  reorganisation.
- **The API.** No new public canvas surface.
- **The wrong fix:** raising the MFBASIC cap constants without growing the buffer and moving
  the shader bases. Past the old region end, items would read another region's words: a
  plausible wrong picture, the exact hazard `.ai/canvas-threading.md` warns of. Forbidden.
- **The other wrong fix:** calling a scene "on Vulkan" because it drew on the QEMU boxes'
  software ICD, and treating that as a performance result. Frame-rate acceptance needs
  real hardware.

## Blast Radius

Found with `grep -rn "VULKAN_\|vulkan" src/codegen src/target` and a read of each bug-686
commit's Metal-only files:

- `src/codegen/runtime/canvas/mod.rs`: `CANVAS_MAX_FRAME_ITEMS`, `VULKAN_MAX_FRAME_EDGES`,
  `VULKAN_EDGE_BYTES`, `MAX_FRAME_GRADIENT_STOPS`, `VULKAN_MAX_FRAME_GLYPH_SAMPLES`,
  `VULKAN_GLYPH_BASE_WORDS`, `VULKAN_GRADIENT_BASE_WORDS`, `VULKAN_BUFFER_BYTES`. **Fixed
  here.** Give Vulkan its own `VULKAN_MAX_FRAME_*` caps, as bug-686 gave Metal its own,
  rather than raising the shared `CANVAS_MAX_FRAME_ITEMS`, which other code may read. The
  audit decides.
- `src/codegen/builtins/canvas/helper_render.rs`: `__canvas_vulkanRenderable`,
  `__canvas_renderVulkan`, the `@VULKAN_…@` tokens in `RENDER_CAPS`. **Fixed here.** Count
  distinct pictures, and use the new caps.
- `src/codegen/runtime/canvas/shaders/mfb_canvas.frag` / `.vert`, and the `.spv` blobs via
  `scripts/regen-spirv.sh`. **Fixed here:** band loop, region bases, picture table.
- `src/codegen/runtime/canvas/vulkan.rs`: `emit_edge_upload`, `emit_picture_upload`,
  `emit_item_block`, `emit_vulkan_target` (the buffer and mapping size),
  `emit_vulkan_draw_scene`, `emit_vulkan_descriptors`. **Fixed here.**
- `src/target/macos_aarch64/app/metal.rs`: `emit_band_index`, `emit_picture_lookup`.
  **Reference, not changed.** They are written against the macOS `Asm` with aarch64 scratch
  registers, so they cannot be called from Vulkan's `CodeBuilder` code. Port the algorithm
  (bug-686 section G, bit-identical) to `CodeBuilder`, or hoist one `CodeBuilder` version
  that both backends call. The second leaves Metal's instructions changed, and is decided
  in Open Decisions.
- `src/codegen/builtins/canvas/func_geo_build.rs`, `func_geo_cache.rs`, `func_item_hash.rs`,
  `func_carried_hashes.rs`, `scene_base.rs`. **Verified here on x86-64 (V4)**, and fixed if
  the run finds anything.
- `src/target/win_x86_64/**`, `src/target/linux_common/**` (the window, the blit, the
  runtime-call lists). **Changed only by V6** (swapchain surface). Otherwise unaffected: the
  runtime-call lists already carry every bug-686 call.
- `tools/canvas-bench/`: **latent.** macOS-only (it drives headless `.app` bundles). V5
  needs a Linux runner; add one to the tool, not a new script.
- `scripts/test-canvas-vulkan.sh`: **extended here** with bug-686's rows and a
  `MFB_CANVAS_GEO_VERIFY=1` run.
- bug-687 (fp32 coverage flip): **latent, same hazard, out of scope.** Owned by bug-687. A
  Vulkan oracle row that trips it goes there, not here.

## Fix Design

Mirror bug-686 phase for phase, in the same order, with the same proofs. Only the mechanism
differs where Vulkan's toolchain does.

- **Region bases: specialization constants, not formatted source.** Metal compiles MSL
  source at run time, so bug-686 formats the bases in. Vulkan loads checked-in SPIR-V.
  Declare the bases (and later the band parameters) as `layout(constant_id = N) const
  int`, and feed them through `VkSpecializationInfo` at pipeline creation from the Rust
  constants.
  - This removes the hand-edited literal and the test that only mirrors it. `ITEM_BLOCK_SIZE`
    item 5 in `.ai/canvas-threading.md` then goes away.
  - **Rejected:** generating SPIR-V at build time (a `glslangValidator` build dependency the
    project avoids). **Rejected:** a uniform (a per-frame read for a per-build constant).
- **Band index: the same algorithm, emitted through `CodeBuilder`.** Use section G's
  structure: 2 px bands, reach `r = 0.5` fill / `half + 0.5` stroke plus a 1 px guard, scaled
  for transforms, and taller bands when the index budget forces it. It is built from the
  polygon's cached geometry into the edge region, and the item block carries base, band top,
  band height and band count. Keep it bit-identical to the full loop: a band count of 0
  means loop over every edge, as on Metal.
- **Picture table: a per-frame `(block, base)` table in a new last region**, looked up in
  `emit_picture_upload`. The predicate counts each distinct block once (bug-686 section H /
  `5c8bbe36d`).
- **Caps: Vulkan's own constants**, sized like bug-686 spike C: 65,536 items, 262,144 edges,
  131,072 stops, 8M texels, plus the band region. Pass them into the predicates through
  `RENDER_CAPS` tokens. Buffer sizing stays fixed (bug-686's Open Decision), unless H1 shows
  a device that cannot map it.
- **Present:** measure first (V6). If needed, add a `VK_KHR_swapchain` path per window
  system:
  - Win32: `VK_KHR_win32_surface` on the canvas HWND.
  - Linux: `VK_KHR_xlib_surface` / `VK_KHR_wayland_surface` under GTK.

  Keep the readback for headless runs, damage mode and dumps, like Metal's `directFrames`
  split, with a stats counter for it.
- **x86-64 proof before any of the above.** The shared native paths are the base every
  Vulkan frame stands on. A mismatch found there is a bug in shared code, fixed for every
  backend.

Expected output shifts:

- The `app-mouse-surface` sentinel's `.ncodesum` for `linux-*` / `windows-*` (Vulkan
  emitter) and the Linux/Windows helper source.
- No Metal golden changes, unless the band builder is hoisted.
- The `.spv` blobs.

## Phases

### Phase 0: environment and baseline (no behavior change)

- [ ] Find real Linux and Windows machines with a Vulkan GPU, and record them in
      `.ai/remote_systems.md`. **BLOCKED: none is reachable** — 2226 and 2230 run Mesa
      lavapipe, recorded in `.ai/remote_systems.md` (`948ef1044`). This blocks V5 and V6.
- [x] Fill in the environment matrix above, for the boxes this fix could use (2226, 2230).
- [x] Add a Linux runner to `tools/canvas-bench/` (headless GTK app,
      `MFB_GTKAPP_HEADLESS`), with the same subcommands as the macOS one.

Acceptance: `scripts/test-canvas-gpu-rows.sh <main mfb> --target linux-aarch64`
reproduced sections B and H at `main`, each row's renderer shown (`gpuFrames=0`).
Commit: `bd49be0d0`, `948ef1044`

### Phase 1: failing tests + the x86-64 proof of the shared paths

- [x] Add bug-686's `rt_canvas_metal.rs` scale, large-polygon, tilemap, background, layered
      and gradient rows — as `scripts/test-canvas-gpu-rows.sh` over shared
      `tests/canvas/scenes/*.mfb`, not inside `test-canvas-vulkan.sh`, so the Rust suites and
      the script read one copy. RED at `main` on 2226: quads, edges, gradients, tilemap,
      background and the 64k polygon declined (`gpuFrames=0`).
- [x] Run every row, plus the snapshot race scene (`rt_canvas_geo_native`'s alternating
      scenes), with `MFB_CANVAS_GEO_VERIFY=1` on `linux-x86_64` (2228), `linux-x86_64` musl
      (2227), `linux-aarch64` and `windows-x86_64`. Record `geoVerified`, `geoResolvedChecked`
      and mismatches. Any mismatch is a shared-code bug: fix it here, with its own test.
      Run on linux-aarch64 (2226) and windows-x86_64 (2230), at `main` and on the branch:
      0 mismatches everywhere. linux-x86_64 glibc/musl not run (boxes not available to
      this fix).

Acceptance: met on the two targets run. The one row failing at `main` for a non-cap
reason, `polygons`, failed on bug-687's flip (see Deviations).
Commit: `6ef055958`

### Phase 2: caps, derived bases, picture table

- [x] Vulkan's own frame caps and buffer size, and the specialization-constant region bases.
      Delete the `GRADIENT_BASE` literal. Replace `the_shaders_gradient_base_matches_the_buffer_layout`
      with a region-chain test like Metal's. Prove it wrong first (AGENTS.md): the new
      mechanism removes the literal the test mirrors.
- [x] Per-frame picture table and distinct-block predicate count. Regenerate the SPIR-V with
      `scripts/regen-spirv.sh` (`MFB_SPIRV_PORT=2226`).

Acceptance: met on 2226 and 2230.
Commit: `069348f8b`, `937ddc149`

### Phase 3: band index

- [x] Build the band table and edge index with the polygon's cached geometry, through
      `CodeBuilder`. Write them into the edge region, carry band fields in the item block,
      and loop over one band in `edgeDistance`.
- [x] Oracle rows at 300, 1,000, 4,000 and 64,000 edges, plus stroked, transformed and
      self-intersecting polygons (`large_polygons.mfb`, `huge_polygon.mfb`).

Acceptance: met. "Band count 0 forced" was measured without a production switch: the
previous commit's build IS the full loop, so the same scenes were rendered with both
builds on 2226 and byte-compared — identical.
Commit: `05c5ffa81`

### Phase 4: performance on real hardware, and present (needs Phase 0's machines)

- [ ] Release fps rows (1k/4k/10k quads and lines, `MFB_CANVAS_SYNC=1` and pipelined) and
      wind, on Linux and Windows GPUs. Split the frame with the debug phase timers.
- [ ] Measure readback + blit against the frame budget at full-screen. If it threatens the
      budget, add the swapchain present, with the readback kept for headless runs, damage
      and dumps, and a stats counter.

**Partly done (2026-09-27, box 206):** the fps rows are measured (table in STATUS) and two
memory-type fixes landed (`42c207a2e`, `cc5e0ea53`). Still open: 10k quads is
renderer-limited (GPU fragment cost), and the readback-vs-swapchain split at full screen.

Acceptance: at 10k moving items the renderer path is not the limit, and the numbers are
recorded here with commands. A full-screen window holds the rate.
Commit: —

### Phase 5: regenerate outputs, docs, full validation

- [x] Regenerate the `app-mouse-surface` sentinel after the full suite, inspect the diff,
      and prove it is Vulkan-only. (The macOS dumps move too: the Vulkan predicate is
      shared MFBASIC compiled into every app; the normalized `.ir`/`.nir` delta is only
      `__canvas_vulkanRenderable`.)
- [x] Doc sync:
      - `.ai/canvas-threading.md` §10 caps and §6 pictures ("Vulkan still copies and
        counts per item" goes);
      - the `ITEM_BLOCK_SIZE` list item 5;
      - `mfb spec app canvas`'s Vulkan sentences, with `scripts/spec-census.sh
        --citations` showing 0 misses.
- [x] Full suite (`cargo test --release --no-fail-fast`), `scripts/test-canvas-vulkan.sh` on
      2226, `scripts/test-winapp.sh` on 2230, and `rt_canvas_metal` (19 passed).

Acceptance: met. `cargo test --release --no-fail-fast`: 227 suites, 6,032 passed, 0 failed (includes `artifact_gate_all`, after the sentinel regeneration).
Commit: `948ef1044`, `df722b970`, `7668925a4`, `df4f589a7`

## Validation Plan

- Regression: the `test-canvas-vulkan.sh` rows (RED → GREEN per phase), `GEO_VERIFY` on four
  targets, the `vulkan.rs` layout and region-chain unit tests.
- Runtime proof: the oracle rows on 2228 and 2227 (correctness); fps rows and wind on a
  real-GPU Linux and Windows machine (performance).
- Doc sync: `.ai/canvas-threading.md`, `mfb spec app canvas`, `tools/canvas-bench/README.md`.
- Full suite: `cargo test --release --no-fail-fast` (includes `artifact_gate_all`).

## Open Decisions

- **Band-index builder.**
  - *Recommended:* one `CodeBuilder` implementation both backends call. The algorithm then
    exists once, at the cost of re-emitting Metal's instructions: prove it with
    `rt_canvas_metal` unchanged and an inspected sentinel diff.
  - *Alternative:* a Vulkan-only port, with the algorithm kept twice.
- **Swapchain.**
  - *Recommended:* measure first, and build it only if the readback threatens the budget
    at full-screen, as bug-686 Phase 4 did.
  - *Alternative:* build it unconditionally for parity.
- **Hardware.**
  - *Recommended:* V1–V4 land on the QEMU boxes, and V5/V6 wait for a real GPU, stated
    as a blocker.
  - *Alternative:* hold the whole bug for hardware.

## Summary

The engineering risk is in two places:

- **The shader-side changes.** These are the band index and the specialization-constant
  bases. They are only checkable against the software oracle on a device, and the only
  reachable devices are emulated.
- **The first-ever x86-64 run of bug-686's shared native code.** Anything it finds is a bug
  in every backend.

The caps and the picture table are mechanical once the bases are derived.

Left untouched:

- Metal (unless the band builder is hoisted, and then bit-identical);
- the API;
- the software rasteriser;
- bug-687's tolerance question;
- the program-side per-item cost (plan-152).
