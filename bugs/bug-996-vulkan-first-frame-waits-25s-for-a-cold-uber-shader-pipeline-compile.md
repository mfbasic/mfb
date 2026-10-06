# bug-996: On Intel Vulkan the first canvas frame of every newly built program waits ~25 s for the pipeline compile, behind a blank window

Last updated: 2026-10-06
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: planned `tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host test@192.168.1.206 coldstart` (time to first present, fresh exe name vs second launch); unit tests in `src/codegen/runtime/canvas/vulkan.rs` for the pipeline-cache seam

On box 206 (Intel UHD 630, real Vulkan driver), the first `canvas::present` of a
program that has never run under that executable name takes **~25 s**. The second launch
takes **~70 ms**, and a software frame takes **7 ms**. While it waits, the window (400×300
on Windows, see bug-997) shows nothing but its unpainted background, and one core is busy
in the driver. Every rebuild that changes the exe name or path, and every user's first
launch of any MFBASIC app, pays it again. The developer loop pays it on each new name. The
canvas-demo's reported 27–45 s blank start is this. The time goes into
`vkCreateGraphicsPipelines` compiling the runtime's single fragment uber-shader, with no
`VkPipelineCache`, synchronously on the graphics thread before the first frame.

**The single correct behavior a fix produces:** a canvas program's first frame is on screen
within normal software-frame time (tens of ms) on every launch, on any driver. The Vulkan
pipeline is never on the first frame's critical path: it is built off the frame path, and
frames draw in software until it exists, then switch to the GPU. Its compiled form is reused
across launches *and across MFBASIC programs* through a persisted `VkPipelineCache`, so the
compile is paid at most once per driver/shader version rather than once per executable.

References:

- `mfb spec app canvas` (`src/docs/spec/app/06_canvas.md`) "Which renderer draws is chosen once, at the first present" and "A GPU frame is used only when a pipeline exists". A fix that draws software frames until the pipeline is ready changes the first sentence, and the spec must say so.
- `mfb man os` `os::appCachePath` (per-user rebuildable cache directory)
- bug-688 (Vulkan parity, V5/V6 measured on box 206)
- Found while bringing up `examples/canvas-demo` on box 206. A prior session's cold-cache screenshot (`/tmp/cdwin/cold/probe.png`, scratch) shows the blank 400×300 window during the wait.

## Failing Reproduction

```
IMPORT app
IMPORT canvas
IMPORT color
IMPORT datetime
IMPORT io

FUNC main() AS Nothing
  LET start AS Integer = datetime::monotonicNanos()
  app::setMode(app::Mode.Canvas)
  LET it AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 100.0, h := 80.0, paint := canvas::fill(color::rgb(200, 40, 40))]
  canvas::present([it])
  LET first AS Integer = datetime::monotonicNanos()
  LET it2 AS canvas::DrawItem = canvas::Rectangle[x := 20.0, y := 10.0, w := 100.0, h := 80.0, paint := canvas::fill(color::rgb(200, 40, 40))]
  canvas::present([it2])
  LET second AS Integer = datetime::monotonicNanos()
  io::print("firstPresentMs=" & toString((first - start) / 1000000) & " secondPresentMs=" & toString((second - first) / 1000000))
END FUNC
```

```
mfb build --app --debug --target windows-x86_64 /tmp/coldprobe
# box 206, CRLF .bat:  MFB_WINAPP_HEADLESS=1  MFB_CANVAS_GPU=1  MFB_CANVAS_SYNC=1
coldprobe.exe                         -> firstPresentMs=24504 secondPresentMs=7
coldprobe.exe        (again)          -> firstPresentMs=72    secondPresentMs=7
copy coldprobe.exe coldprobe-copy2.exe
coldprobe-copy2.exe                   -> firstPresentMs=24733 secondPresentMs=8
coldprobe-copy2.exe  (again)          -> firstPresentMs=75    secondPresentMs=8
MFB_CANVAS_GPU=0, warm name           -> firstPresentMs=7     secondPresentMs=5
```

- Observed: ~25 s to the first frame for any executable name the Intel driver has not seen.
  It reproduced on 2/2 coldprobe names and on all 8 fresh probe executables of bug-995's
  matrix (first-run `phaseRenderMs` 24,936–25,627 ms with `gpuFrames=2`). The driver's
  per-user cache (`%USERPROFILE%\AppData\LocalLow\Intel\ShaderCache`) grew by one ~388 KB
  file (21,718,581 → 22,106,781 bytes, 85 → 86 files) for the first new name. Identical
  SPIR-V under a new exe name is cold again, so the driver keys its cache per application.
- Expected: first frame in software-frame time. GPU frames start once a pipeline exists.

Contrast cases: a second launch of the same exe (70 ms); software rendering (7 ms);
macOS/Metal (the Metal pipeline compiles from MSL source at first use on Apple's
compiler, not measured slow here).

| Environment | Driver | First present |
| --- | --- | --- |
| box 206 Win11, Intel UHD 630, new exe name | Intel Vulkan 1.3 | 24.5–25.6 s ✗ |
| box 206, same exe second launch | Intel driver cache warm | 68–75 ms ✓ |
| box 206, `MFB_CANVAS_GPU=0` | software | 7 ms ✓ |

**Measurement trap (record, do not "fix"):** a `--debug` build's `MFB_CANVAS_STATS` line
calls `canvas::vulkanReady()` (`src/codegen/builtins/canvas/helper_surface.rs`, the stats
line, documented as building the device on purpose). A *software* stats run therefore pays
the 25 s too: `MFB_CANVAS_GPU=0` on a fresh name gave `firstPresentMs=24929`, and the
headless default gave 24623. Benchmarks on box 206 must discard each new executable's first run.

## Root Cause

- `src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_pipeline` calls
  `vkCreateGraphicsPipelines(device, 0 /* no cache */, 1, &info, NULL, &out)`. There is no
  `VkPipelineCache` anywhere in the runtime (`grep -n PipelineCache` finds none), so nothing
  the runtime controls carries a compiled pipeline across launches. Only the driver's
  own per-application disk cache does.
- `src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_ready` builds instance, device, queue,
  descriptors and that pipeline on the first `canvas::vulkanReady()` call. The graphics
  thread makes that call inside the first frame's render phase
  (`src/codegen/builtins/canvas/helper_render.rs`, `render_loop_tail!`:
  `IF canvas::useGpu() AND canvas::vulkanReady() THEN`). The first frame cannot be drawn by
  anything until it returns.
- There is one pipeline, not many permutations. The cost is its size: the fragment shader
  is an uber-shader, `src/codegen/runtime/canvas/shaders/mfb_canvas.frag` (1,843 lines),
  compiled to `mfb_canvas.frag.spv` at **248,628 bytes** (the vertex shader is 6,208). It
  carries every shape kind, double-float distance paths, dashes, joins, clips, gradients
  and pictures. Region bases are specialization constants
  (`emit_vulkan_pipeline`, `STAGE_INFO_SPECIALIZATION`), so the driver compiles the
  specialized whole at pipeline creation.
- Nothing is drawn meanwhile: the Win32 window class leaves `hbrBackground` zero
  (`src/target/win_x86_64/app/mod.rs`, the zeroed `WNDCLASSEXW`), and the first frame is
  the first thing ever painted.

Purely the driver's doing? Partly. Intel's compile time for this shader is the driver's.
The runtime chooses to (1) make every first frame wait on it, (2) keep no compiled form of
its own, so each new exe name starts cold, and (3) ship one very large shader. All three
are the runtime's to change.

## Goal

- The coldprobe first present on box 206 under a never-seen exe name is ≤ 100 ms (drawn in
  software if the pipeline is not ready), and the window shows the scene, not a blank client.
- With a persisted pipeline cache populated by any earlier MFBASIC program, a new exe's
  GPU frames start within ~1 s (the cache-hit pipeline creation), not 25 s. Phase 1 proves
  or rejects that Intel honours an application `VkPipelineCache` blob across processes.
- No change to which pixels a GPU or software frame draws.

### Non-goals (must NOT change)

- Shader semantics and the GPU-vs-oracle tolerance. Splitting the uber-shader is NOT
  this bug's fix: it is an option to measure, not a requirement.
- The stats line's `vulkanReady=` meaning. It deliberately builds the device. Note the
  benchmark trap; do not change the line to dodge it.
- Do NOT "fix" by defaulting Windows to software rendering, or by hiding the window until
  the pipeline exists (that makes it worse).

## Blast Radius

- `src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_pipeline`: fixed (cache argument, cache load/save).
- `src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_ready` and `helper_render.rs` `render_loop_tail!`: fixed (readiness off the first frame's path).
- `src/codegen/runtime/canvas/vulkan_present.rs`: swapchain pipeline state. Its swapchain/surface creation sits on the same first frame; audit it in Phase 1 (verdict here).
- Linux Vulkan (box 2226 lavapipe; any real Linux GPU): same code, same exposure. lavapipe compiles on the CPU too. Measure there in Phase 1.
- macOS Metal (`src/target/macos_aarch64/app/metal.rs`): builds its pipeline from MSL source on the first `metalReady()`. Same structural hazard, not observed slow. Audit only.

## Fix Design

Three independent pieces. Land them in order of user-visible value:

1. **Off the critical path.** Build the Vulkan device and pipeline on a helper thread
   started with the graphics thread. `__canvas_renderFrame` draws in software while it is
   pending and switches to Vulkan at the first frame after it is ready. The spec sentence
   "chosen once, at the first present" becomes "software until the GPU pipeline is ready,
   then the GPU". `MFB_CANVAS_GPU=1` keeps forcing GPU for tests: it waits for readiness
   once, as today, so the gates stay deterministic.
2. **Persisted `VkPipelineCache`.** Create the cache from a blob under a per-user,
   **MFBASIC-wide** cache directory (not per app, or every new program is cold again).
   `os::appCachePath`'s parent or a fixed `mfb` subdirectory, decided in Open Decisions.
   Validate the header (vendorID, deviceID, `pipelineCacheUUID`) before use, pass the cache
   to `vkCreateGraphicsPipelines`, and write it back with `vkGetPipelineCacheData` after a
   successful build (atomic rename). A corrupt or foreign blob is ignored, never fatal.
3. **Smaller shader (measure first).** Phase 1 times pipeline creation with the fragment
   shader stripped to one shape kind, to learn how compile time scales with shader size.
   Splitting into per-feature pipelines only pays if it is large. It would also put a
   pipeline switch in the draw loop.

Also clear the window to the canvas background on creation (a background brush or a first
`WM_PAINT` fill) so a pending first frame shows the canvas colour, not a white client.

## Phases

### Phase 1 — failing measurement + audit (no behavior change)

- [ ] `tools/canvas-bench` `coldstart` subcommand: build, copy to a fresh unique exe name, run twice, report `firstPresentMs`. Record the 25 s / 70 ms rows at HEAD.
- [ ] Probe (throwaway, `/tmp`) whether an application `VkPipelineCache` blob saved by exe A makes exe B's `vkCreateGraphicsPipelines` fast on box 206. This decides piece 2.
- [ ] Time pipeline creation against shader size (piece 3 probe).
- [ ] Audit `vulkan_present.rs` first-frame cost, Linux lavapipe first frame, Metal first frame. Write verdicts above.

Acceptance: the cold/warm gap reproduces from the tool; the cache probe result is recorded.
Commit: —

### Phase 2 — the fix

- [ ] Piece 1 (readiness off the frame path) + spec update in `06_canvas.md`.
- [ ] Piece 2 (persisted cache), if Phase 1 proved it helps; unit tests for header validation and the ignore-on-mismatch path.
- [ ] Window cleared before the first frame (`src/target/win_x86_64/app/mod.rs`).

Acceptance: `coldstart` first present ≤ 100 ms on a fresh name; GPU frames begin later and are pixel-identical to today's.
Commit: —

### Phase 3 — full validation

- [ ] Full suite, `scripts/test-canvas-gpu-rows.sh` per target, Windows rows on box 206.
- [ ] Windowed canvas-demo cold start on box 206 with `tools/win-screen-capture`: the menu is visible within 1 s.

Acceptance: full suite green; no golden delta; cold-start numbers recorded here.
Commit: —

## Validation Plan

- Regression test(s): `tools/canvas-bench` `coldstart` (box 206); `vulkan.rs` unit tests for the cache seam.
- Runtime proof: canvas-demo windowed cold start captured with `tools/win-screen-capture`.
- Doc sync: `mfb spec app canvas` renderer-selection paragraph; `.ai/remote_systems.md` note on the first-run compile trap.
- Full suite: `cargo test`; `scripts/test-canvas-gpu-rows.sh`.

## Open Decisions

- Cache location: a fixed MFBASIC-wide per-user directory (recommended: shared across programs, which is the point) vs `os::appCachePath` (per app, so every new program is cold).
- Software-until-ready vs block-until-ready: recommended software-until-ready for real windows; `MFB_CANVAS_GPU=1` keeps blocking for determinism.

## Summary

The engineering risk is piece 1, the renderer switch mid-run: thread ownership of the
Vulkan objects (`.ai/canvas-threading.md`) and a deterministic test story. The cache is
contained, and the shader split is speculative until measured. Pixels and shader
semantics stay as they are.
