# bug-1045: on Windows, a real canvas window drawn on the GPU writes no `MFB_CANVAS_STATS` line

Last updated: 2026-10-12
Effort: unknown — not root-caused
Severity: LOW (2026-10-12: no frame and no report is lost once the pipeline exists; the wait is bug-996)
Class: Other

Status: Open — root-caused 2026-10-12 as bug-996's cold pipeline compile; nothing left that is this bug's own (owner to decide: close as a duplicate of 996)
Regression Test: none

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Same cause as bug-996 (measured 2026-10-12, below): no frame completes while `vkCreateGraphicsPipelines` runs, so no stats line is written. One fix with 996; nothing here is this bug's own. Waiting on the owner with 996.
- OWNER RULING 2026-10-12: closes with bug-996's fix (software frames until the pipeline exists, so a frame completes and a stats line is written during the compile).

A `--debug` canvas program run in a REAL window on box 206 (Windows 11, Intel UHD 630)
with the default renderer leaves no `MFB_CANVAS_STATS` file at all — not one line in
about fifteen seconds of a scene presented ten times a second. The same binary writes
the file when the software renderer is forced, and when a frame dump is requested.

**The single correct behavior a fix produces:** the run below, with neither
`MFB_CANVAS_GPU=0` nor `MFB_CANVAS_DUMP`, appends one stats line per rendered frame,
with `gpuFrames` counting them.

References:

- `src/codegen/builtins/canvas/helper_render.rs:__canvas_renderVulkan` — the direct
  swapchain present (bug-688 V6): `canvas::vulkanPresentScene`, then
  `__canvas_presentedDirect`, which in a `--debug` build calls `__canvas_writeStats`.
  It is skipped when `__canvas_dumpRequested()`, which is why a dump changes the
  outcome: the frame then takes the readback path.
- `tools/pointer-probe/windows-pointer.ps1` and its README — the harness.
- Found while proving plan-172-J's pointer publication in a real Windows window.

## Root-caused 2026-10-12: the first frame is waiting on bug-996's pipeline compile (HEAD `555130a25`)

`bugs/repro/bug-1045/window_stats.mfb`: a rectangle presented at a new position every
100 ms, 120 times, then one line with `canvas::getFrameStats()`. Built `--app --debug
--target windows-x86_64`, run on box 206 in a REAL window (a scheduled task with `/it`),
`MFB_CANVAS_STATS` set and nothing else unless the row says so:

| run | program's line after its 12 s loop | stats file | last stats line |
|---|---|---|---|
| GPU, first launch of this exe name | `presents=120 frames=0` | 2 lines, written after the loop | `gpuFrames=2 directFrames=2 phaseRenderMs=37427` |
| GPU, second launch | `presents=120 frames=120` | 120 lines | `gpuFrames=120 directFrames=120 phaseRenderMs=494` |
| `MFB_CANVAS_GPU=0` | `presents=120 frames=120` | 120 lines | `gpuFrames=0 phaseRenderMs=398` |

So the direct swapchain path does report: one line a frame, `directFrames` counting each.
What the original run saw is the first row. Under an executable name Intel's driver has
not seen, the graphics thread spends the first ~37 s inside `vkCreateGraphicsPipelines`
(bug-996; ~25 s when this was filed), no frame completes, and so no line is written. "Not
one line in about fifteen seconds" is inside that wait. The table below fits: its software
and dump rows ran after the first row had warmed the driver's cache for that name.

That answers "What is not known": frames are not lost once the pipeline exists, and
neither is the report. Not looked at here: the screen during the wait
(`tools/win-screen-capture`); bug-996 already records that the window is blank then.

## Repro (as first filed)

Cross-build a `--debug` windows-x86_64 app that calls `app::setMode(app::Mode.Canvas)`
and presents a rectangle at a new position every 100 ms for a minute. Ship it to box
206 with `windows-pointer.ps1`, and run, inside the desktop session (a scheduled task
with `/it`), a `.cmd` that sets `MFB_CANVAS_STATS=C:\mfbreal\stats.txt` and starts the
PowerShell driver. The driver reports the window's client origin and `done`, so the
window exists and the process lives for the whole run.

| Environment | `stats.txt` |
|---|---|
| nothing else set (GPU, direct present) | **not created** |
| `MFB_CANVAS_GPU=0` | written, one line per frame |
| `MFB_CANVAS_DUMP=…` (GPU, readback path) | written (55,798 bytes), and a 400,896-byte dump |

The first row is the same at `6a473b1c7`, before plan-172-J touched the window
procedure, so it is not that letter's.

## What is not known

Whether the direct path draws and merely fails to report, or never completes a frame.
`__canvas_presentedDirect` is called only when `canvas::vulkanPresentScene` answers
TRUE, and a FALSE answer falls through to the readback path, which writes the line too
— so "no line at all" means neither path reached its end, or the stats write itself
failed. Nothing here looked at the screen.

## First steps

1. `tools/win-screen-capture` during the first-row run: is the rectangle moving?
2. If it is: the frame is drawn and the report is lost — look at whether
   `__canvas_writeStats` runs on the direct path in a real window (the `TRAP` around
   `fs::appendText` swallows a failure silently).
3. If it is not: the direct present never returns, which bug-688 V6's own proof (a
   fullscreen window) would not have shown for a windowed one.
