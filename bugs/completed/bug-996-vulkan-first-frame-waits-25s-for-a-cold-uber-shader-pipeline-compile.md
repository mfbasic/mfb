# bug-996: On Intel Vulkan the first canvas frame of every newly built program waits ~25 s for the pipeline compile, behind a blank window

Last updated: 2026-10-12
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (Performance)

Status: Fixed 2026-10-12 (bug-staging-23, round 2). The persisted `VkPipelineCache` and the shader split are not this bug (owner ruling below); they are `planning/cache.md`.
Regression Test: `cargo test --test codegen_canvas_vulkan_off_frame_path` (the emitted members run in the `-ncode` interpreter, three targets); `cargo test --bin mfb -- vulkan_handoff::` (the hand-off seam); `tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host test@192.168.1.206 coldstart` (box 206, fresh exe names)

## Cause (coordinator, 2026-10-12, bug-staging-23)

- No existing rule: the spec states the opposite of the fix. `mfb spec app canvas`: "Which renderer draws is chosen once, at the first present", and `helper_render.rs` `render_loop_tail!` makes the first frame wait for `canvas::vulkanReady()`, which builds device and pipeline (`vulkan.rs:emit_vulkan_ready`, `emit_vulkan_pipeline`, no `VkPipelineCache`).
- A fix needs a NEW rule (the pipeline is never on a frame's critical path: software until it exists) and a new mechanism (a build off the graphics thread; a persisted cache in an MFBASIC-wide directory). Both Open Decisions below are unanswered. Waiting on the owner; bug-1045 is this bug's first frame and closes with it.
- OWNER RULING 2026-10-12: the new rule is approved — the pipeline is never on a frame's critical path; frames draw in software until it exists; `MFB_CANVAS_GPU=1` keeps blocking so tests stay deterministic. The window is cleared to the canvas background on creation. The persisted `VkPipelineCache` is NOT this bug: it is `planning/cache.md` (an idea, not a plan). The shader split (piece 3) is not this bug either. bug-1045 closes with this fix.

## Resolution (2026-10-12)

**The rule:** the Vulkan pipeline is never on a frame's critical path. A frame that wants
the GPU starts the build on a thread of its own and is drawn in software; the first frame
after the build has finished is a Vulkan frame. `MFB_CANVAS_GPU` set builds on the
graphics thread before the first frame, as before, so tests stay deterministic.

### What changed

- `canvas::vulkanReady()` was the build. It is now one load of the ready word
  (`vulkan_handoff::emit_vulkan_is_ready`): it starts nothing and waits for nothing.
- `canvas::vulkanBuild(place)` is the build (`vulkan::emit_vulkan_build`, the old body):
  `0` on a thread of its own, `1` on the caller, `2` for the builder thread's own entry.
- `__canvas_vulkanReady` (`helper_render.rs`, the render loop) calls `vulkanBuild` once,
  the first time a frame wants the GPU (`1` if `MFB_CANVAS_GPU` is set, else `0`), then
  answers `canvas::vulkanReady()`. The Vulkan arm of `__canvas_renderFrame` asks it.
- `_mfb_rt_canvas_vulkan_builder` is the builder thread's start routine. It shares its
  frame with the graphics entry (`emit_thread_entry`, split out of
  `emit_graphics_trampoline`; `_mfb_rt_canvas_graphics_entry` is instruction for
  instruction what main's compiler emits on all four targets, compared in `-ncode`).
  Unlike that entry it saves the C ABI's whole callee-saved integer bank
  (`builder_entry_saved`): it calls a hand-written member directly, and on x86-64 that
  member stages values in `r12`, which no frame saves. The first cut reused the graphics
  entry's list and `cargo test --test rt_foreign_entry_callee_saved` caught it on both
  x86-64 targets (`_mfb_rt_canvas_vulkan_builder {"r12"}`).
- Windows: `WM_PAINT` in canvas mode with no frame committed fills the client black
  (`wnd_canvas_clear`), unless the swapchain's child window is up.

### Which thread builds, and who owns what

A helper thread builds the device and the pipeline. It has **no arena**: its entry pins
nothing into the arena-state register, and the build uses its own stack, the loader and
the process-global graphics state (`_mfb_rt_canvas_graphics`, static data). So nothing it
hands over was allocated from an arena, which is the hazard `.ai/canvas-threading.md` §2
warns about. What it hands over is the state's words.

The hand-off is one word, `GRAPHICS_OFFSET_VULKAN_READY`: untried 0, built 1, failed 2,
building 3.

1. Only the graphics thread takes it off 0 (`emit_claim`: load, compare, store 3), and it
   does that before the builder thread exists. One builder at most; no compare-and-swap.
2. The builder fills its words with plain stores: `LIB`, `INSTANCE`, `PHYSICAL`, `DEVICE`,
   `QUEUE`, `QUEUE_FAMILY`, `WSI`, `SET_LAYOUT`, `DESC_POOL`, `DESC_SET`, `EDGE_*`,
   `ITEM_*`, `PIPELINE_LAYOUT`, `RENDER_PASS`, `PIPELINE`, `PIPELINE_MODES`. It writes the
   ready word last (`emit_publish_ready`: a store-release on AArch64, a plain store on
   x86-64, which keeps stores in order) and returns.
3. The graphics thread reads the ready word first (`emit_load_ready`: a load-acquire on
   AArch64) and an object word only after it has read 1.

So the graphics thread never touches a half-built pipeline: every read of a builder word
is behind a read of 1, and 1 is written after all of them. And the builder never touches
what the graphics thread owns: the graphics thread's objects (render target, readback
buffer, command pool and buffer, surface, swapchain, semaphores, present pass) are made in
`emit_vulkan_target` and `emit_ensure_swapchain`, which run only on a GPU frame, which
runs only after 1, by which time the builder has returned. No main-thread code touches a
`GRAPHICS_OFFSET_VULKAN_*` word (`grep -rn GRAPHICS_OFFSET_VULKAN src/target`: no hit).
Vulkan objects are not tied to their creating thread; the API asks that two threads not
use one object at once, and the word is what keeps them apart.

Watched on box 2226 (lavapipe, a real GTK window) with a watchpoint on the word,
`printf 'wp _mfb_rt_canvas_canvas_vulkanBuild+0x25cf0 8 bt 6\nbp _mfb_rt_canvas_vulkan_builder bt\nrun\nquit\n' | mfb dbg wsl`:

```
--[WP] T: 6 … _mfb_rt_canvas_canvas_vulkanBuild+0x40   0 -> 3   (canvas::#vulkanReady <- #renderFrame <- #renderLoop <- graphics_entry)
--[BP] T: 7 … _mfb_rt_canvas_vulkan_builder
--[WP] T: 7 … _mfb_rt_canvas_canvas_vulkanBuild+0x17e8 3 -> 1   (<- mfb_rt_canvas_vulkan_builder)
--[EX] Program exit (0)
```

Two writes in the whole run, the first by the graphics thread, the second by the builder.

### Shutdown while the build is running

Nobody joins the builder and nobody stops it: it is detached at birth (`pthread_detach`,
`CloseHandle`). Shutdown joins the graphics thread, which is not inside the build (with
`MFB_CANVAS_GPU` set it is, and an exit then waits for the pipeline as before), and
the process ends with the builder wherever it is. Nothing the builder uses is freed (static
state, its own stack, the driver's memory), and no Vulkan object is destroyed at exit,
built or not, so there is nothing to use after a free.

Box 206, real windows, new exe names, each `vulkanReady=FALSE` on its last stats line (the
build was in flight):

| What ended the program | Before | After |
|---|---|---|
| `EXIT PROGRAM` 2 s in (20 presents) | 45,505 ms, `frames=0` | 2,680, 2,577, 2,558 and 2,541 ms, `frames=20`, no fault event |
| the window closed 3 s in (`CloseMainWindow`) | not run | 3,307, 3,306, 3,276 and 3,292 ms, exit code 0, no fault event |
| left canvas mode 1 s in, back 45 s later | not run | the frame before leaving is software; the 21 after the return are Vulkan frames presented direct (`frames=22 gpuFrames=21 directFrames=21`) |

"No fault event" is `Get-WinEvent` for `Application Error`, `Windows Error Reporting` and
`Application Hang` naming the exe since its start: 0. Box 2226 (lavapipe, real GTK window):
`EXIT PROGRAM 7` during the build exits 7, 10 of 10; SIGTERM during it exits 143, 5 of 5
(3,198 and 3,080 ms for a 3 s `timeout`, against 3,421 and 3,088 ms with `MFB_CANVAS_GPU=0`).
On Windows the same `EXIT PROGRAM 7` exits 0 with and without this change and with
`MFB_CANVAS_GPU=0`; that is a windowed program's exit code, not this bug's.

### A failed build

`READY = 2`: no loader, a missing entry point, no device, a failed create, or a thread
the system would not start. `canvas::vulkanReady` answers FALSE for it on every frame, so
frames are software for the life of the process, as before. No reachable box lacks Vulkan
(2227 is down), so this is proven on the emitted code:
`a_thread_that_cannot_be_started_is_a_failed_build`,
`asked_for_by_name_the_build_runs_on_the_calling_thread` and
`the_builder_thread_publishes_its_answer_and_returns` run the members against a loader
that has no Vulkan and a thread library that refuses, on all three Vulkan targets.

### The stats line's `vulkanReady=`

It used to build the device (the "Measurement trap" below). It now means "the pipeline
exists as this line is written": the line asks `canvas::vulkanReady`, which starts no
build and waits for none. FALSE on every line of a software run; FALSE on the software
frames of a window while its pipeline is built, TRUE from the first Vulkan frame; TRUE
from the first line of a run with `MFB_CANVAS_GPU` set. A `--debug` software run no
longer touches Vulkan at all, so the trap is gone. The spec states the meaning
(`mfb spec app canvas`, "Rendering conventions"). No script reads it from a software run
(`grep -rn vulkanReady scripts tools tests`: every reader is a `MFB_CANVAS_GPU=1` run).

### Verdicts

- **Swapchain and surface (`vulkan_present.rs`): stay on the graphics thread, on the
  first GPU frame.** They need the window and the client size current at that frame, are
  torn down and rebuilt on every resize, and compile nothing (one
  `vkCreateGraphicsPipelines` in the tree, in `emit_vulkan_pipeline`). Measured on box
  206: the first GPU frame of a window, which creates the render target, the surface and
  the swapchain, took 29, 28, 25 and 24 ms on four runs against 2 to 3 ms for the frames
  around it (`phaseRenderMs` deltas of the stats lines).
- **Linux Vulkan: the same code, the same behaviour.** Box 2226, lavapipe, a real GTK
  window, `-O0 --debug`: frames 1 to 66 software, 67 on Vulkan, 598 frames for 600
  presents. `scripts/test-canvas-gpu-rows.sh --target linux-aarch64` and
  `scripts/test-canvas-vulkan.sh --box 2226 --target linux-aarch64` pass.
- **Metal: the same hazard, measured, not changed.** `canvas::metalReady` builds the
  pipeline from MSL on the graphics thread at the first GPU frame, and macOS keeps the
  compiled form per application. `tools/canvas-bench/bench.sh <mfb> coldstart` on this
  Mac, headless with `MFB_CANVAS_GPU=1`, under app names never run before: first launch
  `firstPresentMs=10307` and `9067` (load average 27), second launch `30` and `83`; by
  hand under two more new names `13221` and `7647` (load 34), and `5643` once at load 15.
  A real window asks for the GPU by default, so the first launch of any new MFBASIC app
  on macOS shows a black window for those seconds. It wants its own bug; nothing here
  changes it.
- **Linux lavapipe keeps no cache.** `bench.sh <mfb> --target linux-aarch64 --box 2226
  coldstart`: `MFB_CANVAS_GPU=1` first present 3,968 and 4,668 ms on first launches,
  3,916 and 4,938 ms on second ones (the host was at load 28). That was every windowed
  launch's first frame before this fix.

### Proof on box 206 (Intel UHD 630), every exe name new

`bugs/repro/bug-996/coldstart.mfb`, release, a real window, `MFB_CANVAS_SYNC=1`, default
renderer. A headless run cannot show this: headless is software unless `MFB_CANVAS_GPU`
is set, and a set variable waits by ruling.

| | Before | After |
|---|---|---|
| first present, by hand, two names | 38,246 ms, 38,259 ms | 23 ms, 21 ms |
| `bench.sh … coldstart`, window, first launch, two names, run twice | 44,748 ms, 38,725 ms | 17 ms, 13 ms; 18 ms, 17 ms |
| the same, second launch | 127 ms, 99 ms | 27 ms, 17 ms; 16 ms, 18 ms |
| `coldstart`, headless `MFB_CANVAS_GPU=1`, first launch | 37,309 ms, 37,212 ms | 37,679 ms, 39,427 ms; 35,206 ms, 35,147 ms (blocks, as ruled) |
| the same, second launch | 69 ms, 69 ms | 72 ms, 85 ms; 71 ms, 70 ms |

"Before" is main at `3745e626a` (`/tmp/main-dbg-target/release/mfb`); the second pair of
each "after" cell is the final commit's compiler. The times are the program's own, on
box 206's clock; whether another agent was using the box during a row was not
controlled, so read them as indicative. To repeat:

```
tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host test@192.168.1.206 coldstart
```

- **GPU frames begin later, and none is lost at the change.**
  `bugs/repro/bug-1045/window_stats_long.mfb` (600 presents, 100 ms apart), `--debug` with
  stats, three runs: one line a frame with `frames=` rising by one a line; software to
  line 406, 354 and 334, `gpuFrames=1 directFrames=1 vulkanReady=TRUE` on line 407, 355
  and 335. Two runs drew 600 frames; the third drew 595 (five presents coalesced, where
  is not known: bug-1045's document has the follow-up, four instrumented runs at
  600 of 600 with the change at iteration 331 to 337 and no long iteration).
- **No pixel difference between the renderers of one run.** A program that presents a
  scene (rounded rectangle, 40-point polygon, circle), keeps presenting until
  `getFrameStats().renderer` is `Vulkan`, and presents the scene again, with
  `MFB_CANVAS_DUMP`: frame 1 (software) against frame 409 (Vulkan),
  `scripts/rgba_compare.py sw.rgba gpu.rgba 384` answers `ok worst=1 differing=0.3782%`
  (`Tolerance::GPU_DEFAULT` is 2 and 2%).
- **`MFB_CANVAS_GPU=1` is as before.** `scripts/test-canvas-gpu-rows.sh <mfb> --target
  windows-x86_64`: 200 `ok`, 0 `fail`, exit 0.
- **The window is black before its first frame.** A program that enters canvas mode and
  presents nothing, captured 3 s in with Desktop Duplication: the 384 by 261 client is
  100.0% (255,255,255) before and 100.0% (0,0,0) after, both for a program with no
  `canvas::` call and for one that calls `canvas::getSize`.
- **The scene is on screen through the change.** The 600-present loop with its rectangle
  kept in view, captured the same way: 97.0% black and 3.0% the rectangle's red 5 s in
  (software) and 50 s in, where the run's stats lines 480 to 520 read
  `gpuFrames == directFrames`, rising. With `MFB_CANVAS_GPU=1` in a window: the first
  line is `frames=1 gpuFrames=1 directFrames=1 vulkanReady=TRUE` and the capture is the
  same picture.

## One existing test's count changed

`the_canvas_child_window_is_shown_by_one_arm_and_hidden_by_the_others`
(`src/target/win_x86_64/app/mod.rs`) counted 14 relocations to the swapchain child
window's symbol in the window procedure and now counts 16.

1. Written by `76ac7170f` (bug-688 V6, `git log -S` of the test's name), which added the
   child window.
2. It protects: the child is shown by one arm and hidden by the others, and a program
   with no `canvas::` call emits no reference to it.
3. Nothing else depends on the count (`grep -rn` of the test's name and of
   `CANVAS_CHILD_SYM` in `src`, `tests`, `bugs`, `planning`: the test alone).
4. The count is a census of the arms that load the symbol, and this bug adds one by the
   owner's ruling 3: the paint that has no frame asks `IsWindowVisible(child)` before it
   fills, because the window has no `WS_CLIPCHILDREN` and a fill would land on a child
   that is up. The new arm neither shows nor hides the child, and the test's second half
   (0 references without canvas) holds unchanged: there the fill is emitted with no
   question. Only the number and its message were edited.

## Golden impact

No golden was regenerated. Expected to change, and why:

`scripts/artifact-gate.sh target/release/mfb all` reports
`1689 tests, 1868 build(s), 2352 golden(s) checked, 13 diff(s)` in 6 fixtures. Nine of
the 13, in three fixtures, were already there at this bug's base: round 1's shader and
oracle changes, listed in bug-1079's and bug-995's documents, against goldens that are
still main's. What this change adds:

- `syntax/app/app-mouse-surface` (`.ir`, `.macos-aarch64.app.nir`, `.nplan`, and the four
  `.app.ncode`; all seven were already different): the render loop's source grew by
  `__canvas_vulkanReady` and the global `__CANVAS_VULKAN_ASKED`, and its Vulkan arm calls
  the wrapper, so the `.ir` and `.nir` gain a function, a global and a call, and every
  later builtin line number moves; the `.nplan` gains the runtime symbol
  `_mfb_rt_canvas_canvas_vulkanBuild` and its import rows. In the code: on every target
  the new member and one more global; on Linux and Windows `canvas::vulkanReady` shrinks to a load
  and a compare, `canvas::vulkanBuild` is the old body with the claim, the spawn and the
  ordered publish, and `_mfb_rt_canvas_vulkan_builder` is new; on Windows also the window
  procedure's black paint.
- `syntax/resources/loop-body-resource-reference-valid` (`.ir`, already different): the
  canvas package's source lines and the new global.
- New, Windows only, the window procedure and nothing else:
  `syntax/app/app-window-surface`, `syntax/app/macos-app-mode-io`,
  `syntax/app/macos-app-mode-plumbing` and `syntax/app/macos-app-mode-term`, each its
  `.windows-x86_64.app.ncode`. These programs do not import `canvas`; every Windows app
  program has the window procedure, whose canvas paint arm now fills black when no frame
  is committed. Checked on `macos-app-mode-io`: against main's compiler the only function
  that differs is `_mfb_winapp_wndproc` (323 to 349 instructions) and no data object does.
- Not this change: `syntax/app/app-window-surface` `.macos-aarch64.app.ncode` (round 1's
  embedded MSL).

No pixel golden moves: `rt_canvas_golden` 26 of 26, `rt_canvas_metal` 62 of 62,
`rt_canvas_curves` 40 of 40, `rt_canvas_rasteriser` 106 of 106 (2 ignored).

Any Linux or Windows native-plan golden of a canvas program, if one is added, will carry
two more imports for the canvas-graphics set (`pthread_detach`; `CloseHandle`) and, on
Windows, `IsWindowVisible` for `_main`.

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

## Re-verified 2026-10-12: still reproduces, and the wait is now ~37 s (HEAD `555130a25`)

`bugs/repro/bug-996/coldstart.mfb` (the program below in today's record syntax, writing
its line to the file named by its first argument), `mfb build --app --target windows-x86_64`
(not `--debug`), copied to box 206 under a name the driver had never seen, run from a CRLF
`.bat` with `MFB_WINAPP_HEADLESS=1 MFB_CANVAS_SYNC=1`:

```
MFB_CANVAS_GPU=1, first launch    firstPresentMs=37325 secondPresentMs=6
MFB_CANVAS_GPU=1, second launch   firstPresentMs=70    secondPresentMs=7
MFB_CANVAS_GPU=0                  firstPresentMs=6     secondPresentMs=4
```

One cold launch was measured. A second new name, in a real window (bug-1045's probe),
reported `phaseRenderMs=37427` for its first two frames.

The fragment shader has grown since the 25 s measurement: `mfb_canvas.frag` is 2,206 lines
(`wc -l`), `mfb_canvas.frag.spv` 298,680 bytes (`ls -l`), against 1,843 lines and 248,628
bytes; plan-164-C added the curve shapes. There is still no `VkPipelineCache`
(`grep -rn PipelineCache src` finds nothing) and `emit_vulkan_pipeline` still passes 0 for
the cache.

`tools/canvas-bench/bench.sh --target windows-x86_64 … sweep` does not discard a first
run, so its first row on box 206 is the compile, not the scene (`fps=0.4
phaseRenderMs=18500` for 95 items on 2026-10-12). Later rows reuse the exe name and are
warm. The `coldstart` subcommand's work in Phase 1 should give the bench a warm-up run.

**This is also what bug-1045 saw.** A program that presents for 12 s in a real window under
a new exe name ends its loop with `frames=0`; see that document.

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

- [x] `tools/canvas-bench` `coldstart` subcommand: build, copy to a fresh unique exe name, run twice, report `firstPresentMs`. The rows at HEAD are in the Resolution's table. The bench also warms each scene name up before its first measured run.
- [ ] Probe (throwaway, `/tmp`) whether an application `VkPipelineCache` blob saved by exe A makes exe B's `vkCreateGraphicsPipelines` fast on box 206. Not done: piece 2 is `planning/cache.md` by the owner's ruling.
- [ ] Time pipeline creation against shader size (piece 3 probe). Not done: piece 3 is not this bug by the same ruling.
- [x] Audit `vulkan_present.rs` first-frame cost, Linux lavapipe first frame, Metal first frame. Verdicts are in the Resolution.

Acceptance: the cold/warm gap reproduces from the tool; the cache probe result is recorded.
Commit: —

### Phase 2 — the fix

- [x] Piece 1 (readiness off the frame path) + spec update in `06_canvas.md`.
- [ ] Piece 2 (persisted cache). Not this bug (owner ruling): `planning/cache.md`.
- [x] Window cleared before the first frame (`src/target/win_x86_64/app/mod.rs`).

Acceptance: `coldstart` first present ≤ 100 ms on a fresh name; GPU frames begin later and are pixel-identical to today's.
Commit: —

### Phase 3 — full validation

- [x] `scripts/test-canvas-gpu-rows.sh` on box 206 and box 2226 (2227 and 2228 are down: not run). The full suite is the coordinator's gate.
- [x] Windowed cold start on box 206 with `tools/win-screen-capture`'s `dd.cs`: the repro's scene is on screen 5 s in (97.0% black, 3.0% the rectangle's red) and, on the GPU, 50 s in. `examples/canvas-demo` itself was not run.

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
