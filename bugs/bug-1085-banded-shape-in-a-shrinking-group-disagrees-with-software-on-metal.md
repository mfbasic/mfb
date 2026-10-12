# bug-1085: a banded shape or path clip (32 or more edges) inside a `canvas::Group` whose matrix shrinks disagrees with the software oracle on Metal

Last updated: 2026-10-12
Effort: medium
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) and placed in a later phase.

**The single correct behavior a fix produces:** the scene below draws on the GPU within
`Tolerance::GPU_DEFAULT` of the software frame.

## Reproduction (main `3745e626a`, Metal)

`bugs/repro/bug-1085/group_band.mfb`: a 256-edge ring of radius 3000, as a polygon and as a
path clip, inside groups scaled 0.05.

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=sw.rgba  <binary>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_DUMP=gpu.rgba <binary>
python3 scripts/rgba_compare.py sw.rgba gpu.rgba 900
worst=109 differing=0.1193% first-beyond-tolerance=(161, 155, '251c07ff', '211806ff')
```

Found by bs23-agent-b while proving bug-995; re-run by the coordinator with the same line.
A 40-edge star at the same scale passes, and so does a 0.5-scale group (the agent's runs).

## Cause, by reading and NOT confirmed

`emit_band_index` (`metal.rs`, `vulkan.rs`) builds a band's reach from the ITEM's inverse
only, while under a group matrix the five samples sit `gi * 0.5` apart in the group's space:
a shrinking group puts neighbours outside the band the centre chose. Vulkan not run.
