# plan-164-B: the software renderer draws `canvas::Path`

Last updated: 2026-09-28
Effort: large (3h–1d)
Depends on: plan-164-A (see plan-164-A Prerequisites; this letter also needs A complete)

The software renderer is the reference the GPUs are compared against. This letter
teaches it to draw a `Path` from the y-monotone rational-quadratic pieces plan-164-A's
reducer produces. That covers fill under `rule`, the stroke with caps at open ends and
joins at real corners, dashes along the curves, per-curve `paints`, and everything
plan-163 gave `Paint` (clip, `transparency`, gradients). The GPUs follow in C.

**The single behavioral outcome:** a `Path` draws in software exactly as §3 defines,
proven by hand-computed pixels. Every other item stays byte-identical.

References: plan-164-A §1 and §3 (types, piece format, `ε`); plan-163-A (the winding
count), B (caps, joins, dashes, the join tail, the arc-length table), C (the clip, the
transparency fold) and D (gradients in item space, `strokeApply`);
`.ai/testing-gates.md` :309 (the oracle needs hand-computed checks).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-164-A complete and archived | `ls planning/completed/plan-164-A-*` → match | NOT MET (2026-09-28) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- A `Path` fills under `rule`, strokes under `paint` and per-curve `paints` (§3), and
  honours `paint.transform`, the group offset, the clip, `transparency` and the
  gradient.
- It is drawn only in the software renderer in this letter. On the GPU path, a frame
  containing a `Path` falls back to software, using the existing whole-frame fallback
  predicate in `helper_render.rs` until C.

### Non-goals

- GPU drawing (that is C).
- A native geometry builder for `Path` (plan-164-A §3).

## 2. Current State

- The software draw loop, `helper_items.rs:__canvas_drawGeometry`, reads one geometry
  record per item and dispatches on kind. `__canvas_geoDistance`
  (`helper_draw.rs`) is the per-kind distance.
- The fill test is `helper_draw.rs:__canvas_edgeDistance`, a crossing count under the
  rule (plan-163-A). Joins come from plan-163-B's join tail. Dashes use plan-163-B's
  perimeter parameter and its arc-length tables.
- **One item → one record** (`__canvas_geometryFor` returns one offset). Per-curve
  paints need several strokes per item, so this letter adds **sub-records** (§3).
- UNVERIFIED: the GPU whole-frame fallback's exact predicate name and location. Phase 1
  reads `helper_render.rs` (the Metal and Vulkan fit predicates, near :748 and :897 at
  HEAD `76d62ee7a`) and records them.

## 3. Design

- **The record.** A `PATH` header, then its **piece tail**: 7 floats per piece
  (`x0,y0,x1,y1,x2,y2,w1`). The pieces of all subpaths come first, in order. Each
  subpath is followed by its implied closing line piece for the **fill**, which
  closes the outline, then plan-163-B's join tail (corners) and dash tail (the
  arc-length table per piece), then C's clip edges, then the gradient stops (always
  last).
  - A header slot `PATH_PIECE_COUNT` holds the number of pieces.
  - A per-subpath table in the tail holds the first piece, the piece count, a closed
    flag and a stroke-closing piece index, so the stroke knows which pieces to outline
    and whether to close.
- **Fill (inside/outside).** For each piece, if `(y0 > py) <> (y2 > py)`, solve
  `Y(t) − py·W(t) = 0` for its one root in `[0,1]` (the piece is y-monotone). With
  `w0 = w2 = 1` this is a quadratic in `t`, using a numerically stable form (the
  citardauq variant when `b` dominates). Take `x = X(t)/W(t)`, and when `px < x` add
  +1 if `y2 > y0`, else −1. The piece is inside under `rule` exactly as polygons are.
- **Distance.**
  - For a polynomial piece (`w1 = 1`): the closed-form nearest point, by solving the
    cubic `(P(t) − p)·P'(t) = 0` with the trigonometric/Cardano method. Clamp the roots
    to `[0,1]`, and take the min over them and the two endpoints.
  - For a rational piece: a **fixed** search, identical in all three renderers:
    - 8 uniform samples of `|P(t) − p|²`;
    - then 6 halvings of the bracket around the best sample, choosing by comparing
      the squared distance at the two quarter-points;
    - then the min with the endpoints.

    The step counts are fixed, as in the ellipse's 24 halvings. They are recorded here
    after the Phase 1 measurement: raise them if the dense-sample error exceeds 0.01
    px.
  - The signed distance is `±min over pieces`, signed by the fill test.
- **Stroke.**
  - The stroke is drawn only along the stroke pieces: not the implied fill-closing
    line of an open subpath, but the explicit closing piece of a `closed` one.
  - Caps (plan-163-B `paint.cap`) go at the two ends of an open subpath.
  - Joins (plan-163-B) go only at **real corners**: the boundaries between input
    curves (and the implied join lines) where the unit tangents differ by more than
    1e-6 rad. Never inside one input curve's pieces: the reducer's split points are
    smooth by construction, and must not be mitered.
  - The join tail entries are built from the true tangents of the input curves at
    those boundaries.
- **Dashes.** plan-163-B's perimeter parameter `s` per piece is a cumulative
  arc-length table: 16 samples per piece, integrated in f64 at build, and interpolated
  by `t`. The nearest point's `t` comes from the distance step. `s` continues along
  the whole subpath and restarts per subpath (SVG).
- **Per-curve `paints`: sub-records.** A `Path` whose curves carry paints builds its
  main `PATH` record (the fill, plus the stroke of every curve with an empty `paints`)
  followed by **stroke sub-records**, one per `(curve, paint)` in curve order, then
  paint order.
  - Each sub-record is a stroke-only `PATH` record covering that curve's pieces.
  - Its stroke fields come from that paint: `stroke`, `strokeWidth`, `cap`, `join`,
    `miterLimit`, `dashes`, `dashOffset`, `blend`, `transparency`, and the gradient
    when `strokeApply = FillAndStroke`. Its `transform` and `clip` come from the
    **Path's** paint, so the curve stays attached to its path.
  - Its `s` starts at the curve's cumulative offset in its subpath, so the dashes line
    up with the main stroke.
  - Each sub-record strokes its curve as an open stroke, with caps at the curve's own
    ends.
  - The header slot `PATH_SUBRECORDS` holds the count. `__canvas_geometryFor` still
    returns the main offset, and the draw loop draws the sub-records that follow it,
    in order.
  - Draw order is: the fill, then the main stroke, then the sub-records.
- **Bounds.** The union of the pieces' control-point hulls (a valid bound for a
  rational piece with positive weights), padded by the largest stroke reach
  (plan-163-B's cap and join padding) over the main paint and every sub-record paint.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Fill and distance

- [ ] Resolve §2's UNVERIFIED row and make a frame containing a `Path` take the
      whole-frame software fallback on the GPU backends.
- [ ] `PATH` record builder (`helper_geometry.rs`): the piece tail and the per-subpath
      table, per §3.
- [ ] `helper_draw.rs`: the piece fill test, the polynomial distance and the rational
      fixed search, per §3. Dispatch from `__canvas_geoDistance` on `PATH`.
- [ ] Measure the rational search's error against a 4096-sample brute-force minimum on
      a quarter circle (`w1 = √2/2`), a thin conic (`w1 = 5`) and a near-parabola
      (`w1 = 1.01`). Record the numbers and the chosen step counts here.
- [ ] Tests (hand-computed, `tests/canvas/rt_canvas_curves.rs`):
      - a filled circle built from 4 rational quadratics has its centre pixel covered
        and the pixel at `r + 2` not covered, and matches `canvas::Circle`'s pixels
        at the same radius exactly or within 1 coverage step (record which);
      - a figure-8 cubic under EvenOdd and under NonZero gives the loop-crossing
        pixels by rule;
      - a subpath with a hole gives the hole under both rules, per plan-163-A;
      - an open subpath fills as if closed.

Acceptance: `Path` fill and antialiasing are correct in software.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_rasteriser` → pass,
  rasteriser goldens unmodified (est. 8 min, guess).
Commit: —

### Phase 2 — Stroke, caps, joins, dashes, per-curve paints

- [ ] The stroke along the stroke pieces, caps at open ends, and joins at real corners
      only (the join tail built from the input tangents).
- [ ] The dash arc-length table per piece, and `s` along subpaths.
- [ ] Stroke sub-records for `paints`, and the draw loop walking them.
- [ ] Tests (hand-computed):
      - a stroked circle made of 4 rational quadratics under `join := Miter` has no
        miter spikes at the 4 piece boundaries (they are smooth);
      - a cubic followed by a line meeting at 90° under Miter gives a sharp corner;
      - dashes `[10, 5]` on a quarter circle of radius 100 give on-pixels at arc
        lengths [0,10), [15,25), …;
      - a curve with `paints := [thick black, thin white]` draws white over black
        (outline plus inner line);
      - a curve paint's `transform` is ignored and the Path's applies;
      - a sub-record's dashes continue from the curve's offset in its subpath.

Acceptance: the stroke is correct per §3 in software.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_rasteriser --test
  rt_canvas_golden` → pass, goldens unmodified (est. 10 min, guess).
Commit: —

### Phase 3 — Paint integration (clip, transparency, gradient, transform, group)

- [ ] Tests (hand-computed):
      - a `Path` under a 30° rotation;
      - a `Path` inside a translated group;
      - a `Path` clipped by a ring clip (plan-163-C);
      - `transparency := 0.5` on a `Path`;
      - a radial gradient on the fill, and with `FillAndStroke` on the stroke
        (plan-163-D).
      Fix whatever fails in the `PATH` arms. These use shared paint code, so a failure
      is a missing arm, not a design question.

Acceptance: `Path` honours every `Paint` field in software.
  Check: `cargo test --test rt_canvas_curves` → pass (est. 5 min, guess).
Commit: —

## Validation Plan

- Tests: Phases 1–3, all hand-computed. Negative cases: invalid curves inside a
  subpath (skipped, with the join line implied), empty subpaths, and a Path with no
  valid curve (draws nothing).
- Coverage check: every new arm runs in `rt_canvas_curves`.
- Runtime proof: an `--app` program in the software renderer (no `MFB_CANVAS_GPU`)
  drawing a filled rounded star path, a dashed spiral from cubics, and a curve with
  stacked strokes, screenshotted.
- Final gate: at the end of plan-164-C.

## Open Decisions

None. The step counts are measured and recorded in Phase 1.

## Corrections

(none yet)

## Summary

The risk is the rational piece's fixed-step distance search (measured, not assumed)
and the sub-record draw path, the first time one item has several records. Every
existing item takes unchanged paths.
