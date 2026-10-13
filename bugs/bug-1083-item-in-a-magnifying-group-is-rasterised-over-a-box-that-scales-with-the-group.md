# bug-1083: an item inside a magnifying `canvas::Group` is rasterised over a box that scales with the group, so 49 small polygons cost 40 ms a frame

Last updated: 2026-10-12
Effort: medium (a geometry-record change)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12). Added to that phase by the owner on 2026-10-12.

## Cause (coordinator, 2026-10-12, bug-staging-23)

OWNER RULING 2026-10-12 (third note; supersedes "leaves the phase"): keep bug-991's built fix
(`5752c7e5b`, reverted by `6d3161619`) and fix bug-1083 NOW, in this phase, so the fix's one
changed case is repaired. bug-991 and bug-1083 are ONE fix with one cause and go to one agent.

- Rule (as bug-991's second note): a pixel takes coverage only where `d < 0.5 * g`, and `g` is bounded by the norm of the COMPOSED inverse (item, then groups). The rim a box needs is therefore known only where the last matrix is known.
- Place: three sites carry a FINISHED header box (slots 16-19, builder rim folded in) through a group's forward part: `helper_items.rs:__canvas_drawBound`, `mfb_canvas.vert` and Metal's `mfbVertex`. They cannot replace the rim, they pad differently from each other (a pixel after the map in software; `length(gi)` before it on the GPUs), and so a magnifying group scales the rim (bug-1083's cost) while a shrinking one under-covers an unevenly transformed item (bug-991's changed pixels: 26 -> 24 at group scale 0.1; 12 -> 10 software, 8 Metal at 0.03).
- Fix: the geometry record carries its rim-free box (the item's extent and reach under its own transform, hull taken, nothing added) beside the padded one. The three group sites take ONE expression from it: map the rim-free box grown by `(padded - rimfree) * |gi|` (the header's own rim, scaled by the group inverse's norm, which bounds the composed norm by the product), then add the pixel. Ungrouped, slots 16-19 are read as today, so nothing ungrouped moves. A header whose pad is not a rim (glyph run, text box, picture, arc) stores rim-free = padded, which is today's box under a group.
- Proof the note is right before anything lands: the two scenes that stopped bug-991 (`bugs/repro/bug-991/halo_in_shrinking_group.mfb` at group scale 0.1 and 0.03) keep their existing lit pixels on software AND Metal with both halves in place, and `magnified-polygons` `group` falls to `flat`'s cost.

Split out of bug-991 on 2026-10-12. bug-991 fixes the box of an item under its OWN transform.
An untransformed item inside a `canvas::Group` whose matrix magnifies still draws over a box
far larger than the item: its header's box carries a 1.0 antialiasing rim in the group's
space, and the three sites that carry a finished header through the group's matrix scale that
rim with it.

**The single correct behavior a fix produces:** the area rasterised for an item inside a group
is the group-mapped extent of the item plus its reach, plus the rim the mapped coverage needs
(half the composed inverse's norm, see bug-991's second Cause note) and one pixel; it does not
grow with the group's magnification. Every pixel stays identical.

## Reproduction (main `3745e626a`, Metal, M2 Max, load average 28, so indicative)

```
bash tools/canvas-bench/bench.sh <mfb> magnified-polygons
mode=group      | ms/frame=40.3     (the same items untransformed, in a Group scaled 170)
mode=pix        | ms/frame=4.5      (the same picture in pixel coordinates)
```

`bugs/repro/bug-991/magnified.mfb group` is the same scene as a program.

## Cause, by reading (bug-991's audit, bs23-agent-b)

- `helper_items.rs:__canvas_drawBound` maps the header's box (slots 16-19) through the groups'
  forward linear part and pads by a pixel; `mfb_canvas.vert` and Metal's `mfbVertex` grow the
  box by `length(gi)` and map it. All three start from a box that already contains the
  builder's rim.
- The header does not say how wide its rim is, and the builders' rims differ
  (`__canvas_glyphRunHeader`, `__canvas_textBoxHeader`, the picture's 2.0), so the mappers
  cannot take it off without naming kinds. A fix needs the record to carry the rim-free box or
  the rim (a header slot, hashed and built natively too), and one expression at the three sites.
