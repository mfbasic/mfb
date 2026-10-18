# bug-1087: on macOS the first canvas frame of a newly built app waits about 5 s for the Metal pipeline

Last updated: 2026-10-12
Effort: medium (bug-996's design, on Metal)
Severity: MEDIUM
Class: Other (Performance)

Status: Fixed (bs24-agent-c, bug-staging-24)
Regression Test: `cargo test --test codegen_canvas_metal_off_frame_path` (the emitted members in the `-ncode` interpreter); `cargo test --bin mfb -- the_render_loop_starts_the_metal_build_where_the_handoff_says`; `tools/canvas-bench/bench.sh <mfb> coldstart` (its window rows)

## Cause (coordinator, bug-staging-24)

- OWNER RULING 2026-10-17: bug-996's rule extends to Metal. The Metal pipeline is never on a frame's critical path: frames draw in software until it exists, then Metal frames begin; `MFB_CANVAS_GPU=1` keeps blocking. `mfb spec app canvas`'s "Metal builds its pipeline on the graphics thread at the first GPU frame in every case" is replaced by that rule.
- Rule: as above, the one bug-996 states for Vulkan (`mfb spec app canvas`, "The Vulkan pipeline is never on a frame's critical path").
- Where: `helper_render.rs`'s first-frame `canvas::metalReady` call (`render_loop_tail!`), which builds the pipeline inline; Fix: build it off the frame's path through the same hand-off shape bug-996 gave Vulkan (`vulkan_handoff`), with the frame drawn in software while it is not ready. Perf: runs alone.

Found during `bug-staging-23` (2026-10-12) by bs23-agent-c while auditing Metal for bug-996,
and placed in a later phase. bug-996 took the Vulkan pipeline off the first frame's path;
Metal has the same shape and was only audited there.

**The single correct behavior a fix produces:** the first present of a never-run app name is
on screen in software-frame time, and Metal frames begin once the pipeline exists, as bug-996
made Vulkan do. (`MFB_CANVAS_GPU=1` keeps blocking, by the same ruling.)

## Reproduction (main's compiler at `7011bb534`, M2 Max, load average 13)

```
bash tools/canvas-bench/bench.sh <mfb> coldstart
headless, MFB_CANVAS_GPU=1  first  launch | firstPresentMs=4745 secondPresentMs=2
headless, MFB_CANVAS_GPU=1  second launch | firstPresentMs=18   secondPresentMs=2
headless, MFB_CANVAS_GPU=0                | firstPresentMs=1    secondPresentMs=1
(a second fresh name: 4830, 20, 1)
```

The agent's runs, by hand in a real window under new names: 13221, 7647 and 5643 ms.

## Cause, by reading

`canvas::metalReady` builds the Metal pipeline (the MSL source is compiled at first use) the
first time it is asked, inside the first frame's render phase
(`helper_render.rs`, `render_loop_tail!`). Nothing is drawn until it returns.

## Not decided

Whether the owner's ruling for Vulkan (software until the pipeline exists) extends to Metal
as written; `MTLBinaryArchive` as Metal's counterpart of `planning/cache.md`.

## Fix (bs24-agent-c, bug-staging-24)

bug-996's shape, on Metal, in the place the Cause note names.

- `canvas::metalReady()` is now a question only: one load-acquire of
  `GRAPHICS_OFFSET_MTL_READY` and a compare (`runtime/canvas/metal.rs:emit_metal_is_ready`),
  so the render branch and the `--debug` stats line ask it without building anything.
- New internal `canvas::metalBuild(place)` (`func_graphics.rs:lower_metal_build`) puts
  `place` in `c_arg(0)` and calls `_mfb_macapp_metal_init` through the existing
  `emit_metal_init` hook. That function now claims the word (untried 0 → building 3,
  store-release, graphics thread only, before the builder exists) and either starts
  `_mfb_macapp_metal_builder` (`pthread_create` with the graphics thread's 8 MiB attribute
  object, then `pthread_detach`; a failed create publishes failed) or, for `place` 1,
  builds on the caller. The build publishes built/failed LAST with a store-release; the
  word's encoding is Vulkan's (`METAL_UNTRIED/BUILT/FAILED/BUILDING`).
- `_mfb_macapp_metal_builder` saves `x19`–`x29` and `lr`, runs the build (place 2,
  "claimed") inside its own autorelease pool, and returns.
- The render loop asks `__canvas_metalReady()` (the `__canvas_vulkanReady` twin):
  first call `canvas::metalBuild(1)` when `MFB_CANVAS_GPU` is set, else `(0)`; every call
  answers `canvas::metalReady()`. Frames are software until it is TRUE.
- macOS imports gain `_pthread_detach` on the canvas-graphics arm (every drawing program
  carries `_mfb_macapp_metal_init`); `canvas.metalBuild` joins the targets' canvas lists.
- Spec `mfb spec app canvas`: "The GPU pipeline is never on a frame's critical path"
  replaces the Vulkan-only paragraph and its "Metal builds ... in every case" sentence;
  the hand-off and `metalReady=` stats paragraphs cover Metal.
  `.ai/canvas-threading.md` §10 and §10.1 updated. `tools/canvas-bench` `coldstart` gains
  a macOS real-window row pair and reports `firstRenderer` / `firstGpuFrameMs`.

**Tests.** RED before the fix (this branch with the test only):
`cargo test --release --test codegen_canvas_metal_off_frame_path` → 6 failed
(`canvas::metalReady` called `_MTLCreateSystemDefaultDevice`; no `metalBuild`, no
builder entry, no store-release). GREEN after: 6 passed. Unit test
`the_render_loop_starts_the_metal_build_where_the_handoff_says` holds the render loop's
literals to the constants. `MFB_CANVAS_GPU=1` keeps blocking: the coldstart headless
rows below draw their first frame on Metal.

**Numbers** (`bash tools/canvas-bench/bench.sh <mfb> coldstart`, M2 Max, fresh app
names, load average 24 to 28; the window row opens a real window):

| Compiler | Row | firstPresentMs | firstRenderer | firstGpuFrameMs |
|---|---|---|---|---|
| main | window, default renderer, first launch | 10,508 | metal | 10,508 |
| main | window, second launch | 64 | metal | 64 |
| this fix | window, default renderer, first launch | 630 | software | 12,991 |
| this fix | window, second launch | 30 | software | 46 |
| this fix | headless `MFB_CANVAS_GPU=1`, first launch (two names) | 10,553 / 13,882 | metal | same |
| this fix | headless `MFB_CANVAS_GPU=0` | 1 / 3 | software | -1 |

**Goldens expected to change** (the coordinator's regeneration): the IR goldens that
carry the render loop (`__canvas_metalReady`, `__CANVAS_METAL_ASKED`, the `metalBuild`
call): `tests/syntax/app/app-mouse-surface` (`.ir`, `.macos-aarch64.app.nir`, and
`.macos-aarch64.app.nplan`, whose runtime symbols gain `canvas_metalBuild` and imports
gain `_pthread_detach`), `tests/syntax/resources/canvas-append-unconsumed-valid` (`.ir`),
`tests/syntax/resources/loop-body-resource-reference-valid` (`.ir`). No picture golden
moves: a headless run's frames are what they were (software, or Metal from the first
frame under `MFB_CANVAS_GPU=1`).
