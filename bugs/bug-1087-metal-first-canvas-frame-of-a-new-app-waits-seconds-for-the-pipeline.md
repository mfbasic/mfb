# bug-1087: on macOS the first canvas frame of a newly built app waits about 5 s for the Metal pipeline

Last updated: 2026-10-12
Effort: medium (bug-996's design, on Metal)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: `tools/canvas-bench/bench.sh <mfb> coldstart` (exists; reports the wait)

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
