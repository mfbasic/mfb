# bug-1089: with `MFB_CANVAS_DAMAGE=1` an unchanged item overlapping a changed one is blended twice along its edge

Last updated: 2026-10-12
Effort: unknown
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) by bs23-agent-d, re-run by the coordinator with main's compiler (`7011bb534`), and placed in a later phase.

`MFB_CANVAS_DAMAGE` promises to change no pixel (`.ai/canvas-threading.md`;
`tests/canvas/rt_canvas_damage.rs` asserts it byte for byte on its own scenes).

**The single correct behavior a fix produces:** the scene below gives the same frame with and
without `MFB_CANVAS_DAMAGE=1`.

## Reproduction

`bugs/repro/bug-1089/damage_double_blend.mfb`: a static square, and a small rectangle whose
colour changes each present and whose box overlaps the square.

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=plain.rgba  <binary>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DAMAGE=1 MFB_CANVAS_DUMP=damage.rgba <binary>
python3 scripts/rgba_compare.py plain.rgba damage.rgba 900
worst=32 differing=0.0762% first-beyond-tolerance=(239, 239, '630e0eff', '821616ff')
```

## Cause, by reading the output and NOT confirmed

The unchanged square's antialiased edge pixels that fall outside the damage rectangle but
inside its redrawn box are blended a second time over the kept frame (the agent's reading;
its second `damage=` was `338,278,125,45`).
