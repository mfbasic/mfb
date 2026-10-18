# bug-1089: with `MFB_CANVAS_DAMAGE=1` an unchanged item overlapping a changed one is blended twice along its edge

Last updated: 2026-10-12
Effort: unknown
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-a, bug-staging-24)
Regression Test: `tests/canvas/rt_canvas_damage.rs::damage_does_not_blend_an_unchanged_overlapping_item_twice`

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

## Fix (bs24-agent-a, bug-staging-24)

The Cause note's design. The rectangle reaches `__canvas_drawGeometry` as draw state
current for the walk, beside the group matrix (`__CANVAS_GM`), rather than as four new
parameters: `helper_render/tests/mod.rs::both_renderers_cull_through_the_one_viewport_predicate`
pins the call `__canvas_drawGeometry(buffer, width, height, offset, gdx, gdy)` and is not
shown wrong by this bug.

- `helper_items.rs`: `__CANVAS_SCISSOR_X0/Y0/X1/Y1` (half-open, unbounded until set) and
  `__canvas_enterScissor`. `__canvas_drawGeometry` takes `drawX0..drawY1`, the surface
  narrowed to the scissor, and bounds all three pixel loops by them (the shape loop, the
  transformed glyph walk, the untransformed glyph blit) where they were bounded by the
  surface.
- `helper_render.rs:__canvas_renderScene` makes the whole surface current, the damage
  rectangle when it clears one, and the whole surface again after the walk.
- `mfb spec app canvas` gains "A partial redraw writes only the pixels it cleared";
  `.ai/canvas-threading.md`'s `MFB_CANVAS_DAMAGE` bullet says the redraw is scissored.

The Metal path never showed it (`MFB_CANVAS_GPU=1 MFB_CANVAS_DAMAGE=1` on the repro was
identical to the full redraw before and after: it draws the whole frame).

Commands run (`CARGO_TARGET_DIR=/tmp/bs24-agent-a`):

- RED: `cargo test --release --test rt_canvas_damage damage_does_not_blend` failed before
  the fix: `damage changed 797 pixels; first at (638, 215), got [128, 22, 22, 255] want
  [97, 14, 14, 255]` (the scene adds a turned square, whose box is its hull, to the repro).
- GREEN: `cargo test --release --test rt_canvas_damage` 12 passed. The repro, software and
  Metal, damage on vs off: `cmp` identical (it was worst=32 with main's compiler).
- `cargo test --release --bin mfb -- canvas::` 84 passed.
- `scripts/spec-census.sh --citations/--render/--links app`: 0 missing, 0 leaks, 0
  unresolved.
- `scripts/artifact-gate.sh <mfb> all`: the same 10 diffs bug-1085/1088 listed (no new
  fixture). Localized on `resources/canvas-append-unconsumed-valid.ir` against the
  bug-1088 build's dump, line numbers normalised: the new scissor globals and
  `#canvas_enterScissor`, `drawX0..drawY1` and the loop bounds that read them.

Goldens expected to change (not regenerated): `app-mouse-surface` `.ir`,
`.macos-aarch64.app.nir`, `.app.nplan`, the four `.app.ncode`; `app-window-surface`
`.macos-aarch64.app.ncode`; `resources/canvas-append-unconsumed-valid` and
`resources/loop-body-resource-reference-valid` `.ir` -- the canvas helper bodies
(`__canvas_drawGeometry`, `__canvas_renderScene`, the scissor) and the source line numbers
behind them. No pixel reference moves: a full redraw's scissor is the surface, which is
what the loops were bounded by.
