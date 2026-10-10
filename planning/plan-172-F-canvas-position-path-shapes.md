# plan-172-F: `position` and local points on `Line`, `Polygon` and `PolyLine`

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-E

The three shapes defined by lists of points take a `position` reference point, and
their points become offsets from it. `Line` trades `x1`/`y1`/`x2`/`y2` for `startAt`
and `endAt`. With this letter every shape in the tree follows the placement rule.

Behavioral outcome: a polygon is moved by changing `position` alone, turns about
`position`, and every existing scene, ported faithfully, draws the same pixels.

References:

- `planning/new_canvas.md` — the `Line`, `Polygon` and `PolyLine` listings.
- `planning/plan-172-D-canvas-position-centre-shapes.md` §3 — the placement rule and
  `__canvas_placement`.
- `src/codegen/builtins/canvas/helper_geometry.rs` — the polygon, line and polyline
  builders; `func_geo_build.rs` — their native twins.
- `planning/plan-152-canvas-program-side-cost-spike.md` — a program's per-item build
  cost; the reason the ported polygons must not gain a per-point addition they did not
  have.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-E complete and archived | `ls planning/plan-172-E-*` → no matches | NOT MET |
| `RoundedRect` is gone | `grep -c 'name: "RoundedRect"' src/codegen/builtins/canvas/mod.rs` → 0 | NOT MET |

If plan-172-E is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::Line` is `id`, `position`, `startAt`, `endAt`, `paint`, `transform`.
- `canvas::Polygon` and `canvas::PolyLine` gain `position`; their points are offsets
  from it.
- No shape record carries a bare coordinate scalar.
- `Line`, `Polygon` and `PolyLine` are built from a defaulted record with `WITH` (Open
  Decision 1 of 172-A, answered: no constructor functions).

### Non-goals (explicit constraints)

- **`PolyLine.colors` and `Polygon.rule` are unchanged.**
- **No change to dash start points or join rules.**
- **`Group` is untouched** until 172-G.
- **No pixel changes for a faithfully ported scene.**

## 2. Current State

`Line` is four scalars. `Polygon.contours` is `List OF List OF vector::Float2` and
`PolyLine.points` is `List OF vector::Float2` (since 172-B), both in surface
coordinates. Their builders flatten the points into the geometry record's edge tail
(five floats an edge: `x0`, `y0`, `dx`, `dy`, inverse length squared) and compute bounds
from them. The native builders walk the same lists.

### Measured populations

`$D` is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| `canvas::Line[` | 67 in 26 files | `grep -rhoE 'canvas::Line\[' $D \| wc -l` → 67 |
| `canvas::Polygon[` | 116 in 36 files | → 116 |
| `canvas::PolyLine[` | 47 in 16 files | → 47 |
| Sites this letter ports | 230 | sum of the three rows |
| Bare coordinate scalars left on shape records before this letter | 4 | `Line`'s `x1`, `y1`, `x2`, `y2` in `mod.rs` |

### Verified properties

- **The edge tail stores absolute coordinates.** VERIFIED from the tail layout
  (`x0`, `y0`, `dx`, `dy`, `invLenSq`): adding `position` to `x0`/`y0` at build time
  yields the same record as today's absolute points.
- **UNVERIFIED — the build cost of adding `position` to every point of an untransformed
  polygon.** Phase 3 measures it with `tools/canvas-bench` rather than assuming it is
  free.

## 3. Design Overview

The rule is 172-D's, applied to a list:

- **All-zero `transform`:** each point is `position + point` as it enters the edge tail.
  The record equals today's for the same surface coordinates.
- **Otherwise:** points enter the tail as given, and the effective affine has
  translation `position + t`.

**Porting.** A faithful port sets `position` to `(0, 0)` and leaves every point as it
is. That is always pixel-identical and is what the bulk of the 230 sites get. Sites that
build a shape around a centre and translate it by hand are rewritten to use `position`
where that removes arithmetic, one at a time, each checked against its dump.

**`Line`.** `startAt` and `endAt` are offsets from `position`. The three-point shape is
deliberate: `position` is the pivot.

**Gate class: behaviour-preserving for ported scenes, plus the new pivot**, tested here
for a polygon.

### Rejected alternatives

- **Making `position` the first point of the list.** Rejected: the pivot would then be a
  vertex, and a polygon could not turn about its centre without a transform translation.
- **Leaving `Line` as two absolute points with no `position`.** Rejected by
  `planning/new_canvas.md`; every shape places the same way.

## Compatibility / Format Impact

- **Changed:** `Line`'s fields; `Polygon` and `PolyLine` gain `position`, and their
  points change meaning from surface coordinates to offsets.
- **Added:** nothing.
- **Unchanged:** the edge tail layout, the geometry header, both shaders.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Records, builders and the faithful port

- [ ] `mod.rs`: rebuild `Line`; add `position` to `Polygon` and `PolyLine`; rewrite the
      three descriptions to say the points are offsets.
- [ ] `helper_geometry.rs`: the line, polygon and polyline builders call
      `__canvas_placement` and add the build offset per point.
- [ ] `func_geo_build.rs`: the three native builders read `position` and add it.
- [ ] `helper_dashes.rs` and `helper_joins.rs`: confirm they read the tail, not the
      record's points, and so need no edit; say so in a comment at the builder.
- [x] ~~Add `canvas::line`, `canvas::polygon`, `canvas::polyLine` with man examples.~~ —
      moot: Open Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1).
- [ ] Rewrite the registry examples of `Line`, `Polygon` and `PolyLine` to the defaulted
      `MUT` + `WITH` form, and render each with `mfb man canvas <Type>`.
- [ ] Port the 230 sites with `position` at the origin.
- [ ] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens.

Acceptance: every canvas pixel test passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_metal --test rt_canvas_damage` → pass (est. 12 min); `git status --porcelain
  tests/golden/canvas` → empty.
Commit: —

### Phase 2 — The pivot and the move, proved

- [ ] Rasteriser case: a triangle with points around `(0, 0)`, `position (400, 300)` and
      a half-turn `transform`; assert its apex pixel is below (400, 300).
- [ ] Rasteriser case: a clipped polygon rendered at two `position`s 100 pixels apart;
      the frames are equal under that shift.
- [ ] Rasteriser case: a `PolyLine` with per-point colours at a non-zero `position`
      equals the same path with the offset folded into its points.
- [ ] A Metal parity case for the first scene.
- [ ] `src/docs/spec/app/06_canvas.md`: the three shapes' sections state that points are
      offsets from `position`.

Acceptance: the three cases pass and Metal agrees within `Tolerance::GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_rasteriser path_placement --test rt_canvas_metal
  path_placement` → pass (est. 5 min).
Commit: —

### Phase 3 — Cost, measured

- [ ] Run `bash tools/canvas-bench/bench.sh target/release/mfb sweep` on a release build
      at this letter's starting commit and at its end; record the moving-polygon rows of
      both runs in Corrections.
- [ ] If the end figure is more than 3% below the start, root-cause it in the builder
      (the offset should be two additions per point, in the loop that already reads the
      point) and fix it before closing the letter.
- [ ] Rewrite the hand-translated sites to use `position`, each checked against its
      dumped frame.

Acceptance: the two bench figures are recorded and the end figure is within 3% of the
start, or the regression is explained and fixed.
  Check: `bash tools/canvas-bench/bench.sh target/release/mfb sweep` → two recorded
  runs (est. UNMEASURED — time the first run and record it; a build-cost regression is
  invisible to every pixel test, so nothing smaller catches it).
Commit: —

## Validation Plan

- Tests: the existing suites; three rasteriser cases; one Metal parity case; three man
  examples.
- Coverage check: `cargo test --test rt_canvas_geo_native` → pass; the native polygon
  builder must still take untransformed polygons.
- Runtime proof: `tests/golden/canvas/polyline.png` scene, dumped before and after,
  byte-identical.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas types` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None.

## Corrections

- **From 172-A, Open Decision 1:** no constructor functions; the task is struck as moot
  and replaced by the `MUT` + `WITH` example task.

## Summary

The last of the record reshaping. The port is deliberately the dullest possible one —
`position` at the origin, points untouched — so that 230 sites move with no pixel at
risk, and the new capability is proved by three small scenes rather than by the port.
