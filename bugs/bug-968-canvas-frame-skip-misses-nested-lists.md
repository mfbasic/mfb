# bug-968: canvas frame skip republishes an identical scene when it contains a nested list (Polygon points, DrawLayer items) built by a different allocation history

Last updated: 2026-10-12
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (proposed) `tests/canvas/rt_canvas_graphics_thread.rs`-style SYNC test: `present([Polygon[points := mkpts()]])` twice (mkpts builds with `collections::append`) yields exactly one frame; `presentLayers([DrawLayer[items := [a]]])` inline twice yields one frame

`canvas::present` / `presentLayers` promise that re-presenting identical content
publishes nothing, so an animation loop that rebuilds an unchanged frame does not
redraw. That holds for a flat list of fixed-size items, but not when an item carries a
nested collection: two scenes with byte-for-byte equal *content* compare unequal and
each present draws a frame.

Measured with `MFB_CANVAS_SYNC=1` (frame counter after each call):

| call | frames |
|---|---|
| `present([poly])` (LET-bound polygon, literal points) ×2 | 1, 1 ✓ |
| `present([Polygon[points := pts]])` (LET pts) ×2 | 1, 1 ✓ |
| `present([Polygon[points := mkpts()]])` (points built by `collections::append`) ×2 | 2, 3 ✗ |
| `present([LET-bound poly from mkpts()])` | 4 ✗ |
| `presentLayers(one)` (LET-bound layers) ×2 | 1, 1 ✓ |
| `presentLayers([DrawLayer[items := [a]]])` inline ×2 (same content as `one`) | 2, 3 ✗ |
| flat `present(scene())` built by `append` ×3 | 1, 1, 1 ✓ |

The single correct behavior: equal scene content ⇒ no publish, independent of how the
nested lists were built.

References:

- `src/docs/spec/app/06_canvas.md` "Retained, not immediate": "Re-presenting identical
  content is free of publication ... The comparison is exact rather than hashed: both
  sides are shrink-to-fit copies, so equal content is equal bytes"; "Both calls share
  one implementation and therefore one set of guarantees".
- `.ai/canvas-threading.md` §3.1 "The frame skip compares CONTENT, not bytes".

## Re-verified 2026-10-12: still reproduces (HEAD `555130a25`, after plan-172 and plan-164)

The program below no longer compiles: plan-172 changed the item records (`Polygon` takes
`contours`, `rule`, `id`, `position` and `transform`; a point is `vector::Float2`). The
same cases in today's syntax are `bugs/repro/bug-968/frame_skip.mfb`. Built with
`mfb build --app` and run with `MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1`, the frame counter
after each call:

| call | frames | |
|---|---|---|
| `present([bound])`, one LET-bound polygon, twice | 1, 1 | ✓ |
| `present([Circle[…]])` written inline, twice | 2, 2 | ✓ |
| `present([poly(lit)])`, polygon built by a function from LET-bound literal points, twice | 3, 4 | ✗ |
| `present([poly(mkpts())])`, points built by `collections::append`, three times | 5, 6, 7 | ✗ |
| `presentLayers(one)`, LET-bound layers, twice | 8, 8 | ✓ |
| `presentLayers([DrawLayer[items := [a]]])` inline, twice | 9, 10 | ✗ |

`gen_present.rs:emit_publish` has not changed since bug-686 (`git log -- src/codegen/builtins/canvas/gen_present.rs`:
newest commit `e5e8167ef`, 2026-09-24). The root-cause hypothesis is still not instrumented.
One row is new: a polygon rebuilt by a function from the *same* LET-bound literal points
also redraws, so a different allocation history is enough even with no `append`.

## Failing Reproduction

`/tmp/hardening/B17/fsl` (polygon variant) and the layers variant:

```
FUNC mkpts() AS List OF canvas::Point
  MUT pts AS List OF canvas::Point = []
  pts = collections::append(pts, canvas::Point[x := 1.0, y := 1.0])
  pts = collections::append(pts, canvas::Point[x := 9.0, y := 1.0])
  pts = collections::append(pts, canvas::Point[x := 5.0, y := 9.0])
  RETURN pts
END FUNC
...
  app::setMode(app::Mode.Canvas)
  LET p AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))
  canvas::present([canvas::Polygon[points := mkpts(), paint := p]])
  io::print(toString(canvas::getFrameStats().frames))     ' 1 expected
  canvas::present([canvas::Polygon[points := mkpts(), paint := p]])
  io::print(toString(canvas::getFrameStats().frames))     ' 1 expected, prints the next count
```

```
mfb build --app fsl && MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 fsl/build/p1.app/Contents/MacOS/p1
```

- Observed: each identical present increments `frames`.
- Expected: the second present is skipped.

## Root Cause

`src/codegen/builtins/canvas/gen_present.rs:emit_publish` compares `count`,
`dataLength` and the outer list's **data region** byte-wise. A nested collection
(the polygon's `points`, a layer's `items`) is inlined *inside* that data region with
its own header and lookup entries. Hypothesis (not instrumented): `copy_flat_block` /
`copy_collection_tight` tightens only the outer block, so the inner block keeps its
allocation-dependent bytes — the unwritten `keyOffset`/`keyLength` words of each list
lookup entry (the exact junk the outer-level comment in `emit_publish` describes) and/or
capacity slack — and the byte compare sees them. Confirm by dumping both copies'
data regions for the `mkpts()` case.

## Goal

- Content-equal scenes with nested lists skip, for both `present` and
  `presentLayers`.

### Non-goals (must NOT change)

- No hashing (no collision reasoning); content-different scenes still publish.

## Blast Radius

- Every variant with a list field: `Polygon.points`, `PolyLine.points`/`colors`,
  `Paint.fillGradient.stops`, `DrawLayer.items` — same hazard; Polygon and DrawLayer
  measured.
