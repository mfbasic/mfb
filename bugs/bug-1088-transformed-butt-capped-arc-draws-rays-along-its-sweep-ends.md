# bug-1088: a butt-capped `canvas::Arc` under a magnifying transform draws dotted rays from its centre along the ends of its sweep

Last updated: 2026-10-12
Effort: unknown
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet

## Cause (coordinator, bug-staging-24)

- Rule: "Each cap is defined on the signed distance the coverage rule already uses, so a cap edge is antialiased exactly as any other edge is" (`mfb spec app canvas`, `cap`); a butt LINE follows it (`max(d − half, −t·|v|, (t−1)·|v|)`, `helper_shapes.rs:__canvas_segmentDistanceButt`).
- Where: a butt ARC breaks it: `helper_draw.rs` (the `__CANVAS_GEO_ARC` arm) answers `1000000.0` for any point outside the sweep, a jump rather than a distance, and both shaders mirror it (`metal.rs`, `mfb_canvas.frag`: "out of sweep it jumps to 1e6"). Under a transform the central-difference gradient across that jump is what lights the rays.
- Fix: a butt arc's distance is the band intersected with the sweep wedge as signed half-plane distances through the two sweep-end radii (`max` of the two for a sweep ≤ 180°, `min` for a reflex one), `max`'d with `|ρ − r| − half`, exactly as the butt line takes its end planes; the same expression in the software oracle and both shaders. That makes an arc 1-Lipschitz, so the "a transformed arc takes no shortcut" exception and the 1e6 sentinel are deleted. Round and Square arcs union their end shapes with this band as they do now. Expected pixel change: a butt arc's ends become antialiased (they are hard cuts today); goldens that hold a butt arc move.

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
