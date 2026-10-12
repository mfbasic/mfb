# bug-1083: an item inside a magnifying `canvas::Group` is rasterised over a box that scales with the group, so 49 small polygons cost 40 ms a frame

Last updated: 2026-10-12
Effort: medium (a geometry-record change)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) and placed in a later phase.

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
