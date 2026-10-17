# bug-1089: with `MFB_CANVAS_DAMAGE=1` an unchanged item overlapping a changed one is blended twice along its edge

Last updated: 2026-10-12
Effort: unknown
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: none yet

## Cause (coordinator, bug-staging-24)

- Rule: damage mode changes no pixel: a partial redraw keeps the previous frame outside the damage rectangle and repaints only inside it (`.ai/canvas-threading.md` `MFB_CANVAS_DAMAGE`; `tests/canvas/rt_canvas_damage.rs`).
- Where: `helper_render.rs:__canvas_renderScene` clears only the damage rectangle, then calls `__canvas_drawGeometry` over each meeting item's WHOLE box, so an unchanged item's pixels outside the rectangle are blended a second time over the kept frame.
- Fix: the partial redraw draws each item clipped to the damage rectangle (the rectangle is the draw's scissor, passed to `__canvas_drawGeometry`'s pixel loop bounds), so nothing outside the cleared area is touched. Nothing special-cased to delete.

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
