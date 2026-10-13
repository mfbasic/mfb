# bug-1088: a butt-capped `canvas::Arc` under a magnifying transform draws dotted rays from its centre along the ends of its sweep

Last updated: 2026-10-12
Effort: unknown
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) by bs23-agent-d, re-run by the coordinator with main's compiler (`7011bb534`), and placed in a later phase.

**The single correct behavior a fix produces:** the scene below draws the quarter-ring and
nothing else, on software and on the GPU.

## Reproduction

`bugs/repro/bug-1088/butt_arc_rays.mfb`: an arc of radius 0.5 under a transform that scales
by 100, so on the surface a quarter-ring of radius 50 about (300.3, 300.7).

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=sw.rgba <binary>
lit pixels: 1190, x 161..455, y 301..456
```

A ring of radius 50 with a stroke that ends near 55 cannot light x = 161 or x = 455: those
are the rays, out to the edge of the item's box. The agent measured 110 lit pixels more than
57 px from the centre, the same on software and Metal (Metal not re-run by the coordinator).

## Cause, by reading and NOT confirmed

An arc's distance is not continuous across its sweep ends (the shader's own comment: "never
for an arc, whose distance jumps"), so under a transform the central-difference gradient is
wrong along those two lines and the coverage `d / g` lights pixels far from the stroke.
bug-991's fix has to leave an arc's box alone for this reason.
