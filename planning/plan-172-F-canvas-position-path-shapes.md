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
| plan-172-E complete and archived | `ls planning/plan-172-E-*` → no matches | MET (2026-10-10) |
| `RoundedRect` is gone | `grep -c 'name: "RoundedRect"' src/codegen/builtins/canvas/mod.rs` → 0 | MET (2026-10-10) |

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

- [x] `mod.rs`: rebuild `Line`; add `position` to `Polygon` and `PolyLine`; rewrite the
      three descriptions to say the points are offsets.
- [x] `helper_geometry.rs`: the line, polygon and polyline builders call
      `__canvas_placement` and add the build offset per point.
- [x] `func_geo_build.rs`: the three native builders read `position` and add it.
- [x] `helper_dashes.rs` and `helper_joins.rs`: confirm they read the tail, not the
      record's points, and so need no edit; say so in a comment at the builder.
- [x] ~~Add `canvas::line`, `canvas::polygon`, `canvas::polyLine` with man examples.~~ —
      moot: Open Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1).
- [x] Rewrite the registry examples of `Line`, `Polygon` and `PolyLine` to the defaulted
      `MUT` + `WITH` form, and render each with `mfb man canvas <Type>`.
- [x] Port the 230 sites with `position` at the origin.
- [x] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens.

Acceptance: every canvas pixel test passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_metal --test rt_canvas_damage` → pass (est. 12 min); `git status --porcelain
  tests/golden/canvas` → empty.
  Observed (2026-10-10): `rt_canvas_golden` 23 passed, `rt_canvas_rasteriser` 101 passed
  (2 ignored), `rt_canvas_metal` 48 passed, `rt_canvas_damage` 6 passed, 0 failed in
  each, on the final tree. Of the 25 canvas-embedding targets, 24 are green there, with
  `cargo test --bin mfb canvas` (113 passed) and `spec` (46 passed); `rt_canvas_font` is
  31 passed, 2 failed — two Metal text frames with unwritten tiles, which passed in
  this letter's first round (33 passed) and are not this letter's (Corrections;
  `bugs/bug-1016-*`). `git status --porcelain tests/golden/canvas` → empty.
  `scripts/artifact-gate.sh target/release/mfb all` → `2308 golden(s) checked, 0 diff(s)`.
Commit: —

### Phase 2 — The pivot and the move, proved

- [x] Rasteriser case: a triangle with points around `(0, 0)`, `position (400, 300)` and
      a half-turn `transform`; assert its apex pixel is below (400, 300).
- [x] Rasteriser case: a clipped polygon rendered at two `position`s 100 pixels apart;
      the frames are equal under that shift.
- [x] Rasteriser case: a `PolyLine` with per-point colours at a non-zero `position`
      equals the same path with the offset folded into its points.
- [x] A Metal parity case for the first scene.
- [x] `src/docs/spec/app/06_canvas.md`: the three shapes' sections state that points are
      offsets from `position`.

Acceptance: the three cases pass and Metal agrees within `Tolerance::GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_rasteriser path_placement --test rt_canvas_metal
  path_placement` → pass (est. 5 min).
  Observed (2026-10-10): `rt_canvas_rasteriser path_placement` → `3 passed; 0 failed`;
  `rt_canvas_metal path_placement` → `1 passed; 0 failed`.
Commit: —

### Phase 3 — Cost, measured

- [x] Run `bash tools/canvas-bench/bench.sh target/release/mfb sweep` on a release build
      at this letter's starting commit and at its end; record the moving-polygon rows of
      both runs in Corrections.
- [x] If the end figure is more than 3% below the start, root-cause it in the builder
      (the offset should be two additions per point, in the loop that already reads the
      point) and fix it before closing the letter.
- [x] Rewrite the hand-translated sites to use `position`, each checked against its
      dumped frame.

Acceptance: the two bench figures are recorded and the end figure is within 3% of the
start, or the regression is explained and fixed.
  Check: `bash tools/canvas-bench/bench.sh target/release/mfb sweep` → two recorded
  runs (est. UNMEASURED — time the first run and record it; a build-cost regression is
  invisible to every pixel test, so nothing smaller catches it).
  Observed (2026-10-10): one sweep takes about 4 minutes. The figures, the regression
  found and its fix are in Corrections.
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

- **Three phases, one commit**, for 172-E's reason: removing `Line`'s four scalars
  breaks every site, and the cost work is in the builder the port depends on.
- **231 sites, not 230.** `codemod_f` reported `TOTAL 231 constructor(s) in 50 file(s)`;
  the extra one is 172-E's new scene `rect_corners_on_black.mfb`.
- **The offset is folded into the points, not added per slot.** §3 says "each point is
  `position + point` as it enters the edge tail". Taken literally that leaves the edge
  vector `dx = b.x − a.x` computed from the unmoved points, and then a shape at a
  position and the same shape with the position added to its points differ in the last
  bit of `x0 + dx`. Both builders move the point FIRST and compute everything — edge
  vectors, bounds, a polyline's "has length" test, joins, dashes — from the moved
  points, so the two spellings build one record
  (`path_placement_a_clipped_polygon_moves_whole_with_its_position` compares the frames
  whole, for both builders).
- **The MFBASIC builder moves the list once; the native builder adds per point.**
  `__canvas_placedRings` and `__canvas_polylinePlaced` return a moved copy when no
  transform is in effect, and every builder downstream is unchanged; with a transform
  in effect they return the list as it is. The native builder
  (`func_geo_build.rs:moved_point`) adds as it reads. The add happens whatever the
  position, zero included, in both — `-0.0 + 0.0` is `+0.0`, and the two builders are
  compared bit for bit (`rt_canvas_geo_native`).
- **`helper_dashes.rs` and `helper_joins.rs` needed no edit**, confirmed by reading:
  `__canvas_lineDashes` reads the two ends back from header slots 2–5, and
  `__canvas_joinTail` / `__canvas_edgeDashes` / `__canvas_dashEdgeData` take the edge
  list. The comments saying so are at `__canvas_lineHeader` and at the `Polygon` arm.
- **`clip_point` is renamed `placed_point`** (it now moves `Line` ends too); the spec
  citation moved with it.
- **§2's "UNVERIFIED" build cost, measured — and it was not free.** `phaseOffsetsMs` is
  the frame's geometry-build phase; the sweep's fps is useless on this machine (the same
  binary ran one row at 68 and at 390 fps an hour apart), so the build phase is what
  was compared, start and end alternated and the minimum over runs taken.
  - *First end build* (the offset re-read from the frame for every point read): 3 to 6%
    slower on the moving rows — `1000/8` 0.278 → 0.286 ms, `100/300` 0.299 → 0.314,
    `10/4000` 0.356 → 0.376.
  - *Root cause*: `placed_point` loaded the two offset doubles per call, and a polygon
    reads each point three times (once for the bounds, twice as an edge end).
  - *Fix*: the offset is loaded once per walk (`build_offset`), and an edge's first
    point is carried from the edge before instead of being read and moved again — per
    edge one point read where there were two, before this letter as well.
  - *Final build*, three alternated pairs:

    | items | points | moving | start, build ms (3 runs) | end, build ms (3 runs) | min start | min end | end / start |
    |---|---|---|---|---|---|---|---|
    | 95 | 8 | 1 | 0.032 0.031 0.031 | 0.031 0.033 0.032 | 0.031 | 0.031 | 1.000 |
    | 1000 | 8 | 1 | 0.281 0.275 0.276 | 0.297 0.284 0.277 | 0.275 | 0.277 | 1.007 |
    | 1000 | 8 | 0 | 0.068 0.065 0.060 | 0.083 0.066 0.066 | 0.060 | 0.066 | 1.100 |
    | 2000 | 8 | 1 | 0.548 0.547 0.545 | 0.632 0.553 0.553 | 0.545 | 0.553 | 1.015 |
    | 2500 | 8 | 1 | 0.685 0.699 0.706 | 0.727 0.702 0.713 | 0.685 | 0.702 | 1.025 |
    | 5000 | 3 | 1 | 1.164 1.173 1.127 | 1.149 1.148 1.197 | 1.127 | 1.148 | 1.019 |
    | 10000 | 4 | 1 | 2.491 2.500 2.482 | 2.594 2.459 2.491 | 2.482 | 2.459 | 0.991 |
    | 10 | 300 | 1 | 0.033 0.034 0.033 | 0.036 0.033 0.035 | 0.033 | 0.033 | 1.000 |
    | 100 | 300 | 1 | 0.294 0.291 0.288 | 0.308 0.290 0.292 | 0.288 | 0.290 | 1.007 |
    | 10 | 4000 | 1 | 0.353 0.358 0.354 | 0.494 0.355 0.364 | 0.353 | 0.355 | 1.006 |

    The largest end / start among the moving rows is 1.025: every moving row is within
    3%. The `moving=0` row builds 0.7 items a frame, so its 0.06 ms is the cache walk,
    not building; the start build alone measured 0.060 to 0.076 ms on it across its
    quiet runs, which is wider than the difference shown.

  The sweep's moving rows are its `moving=1` rows; the polygons are at `position`
  `(0, 0)`, so this is the cost every ported polygon pays.
- **Five hand-translated sites were rewritten; the rest were left, with the reason.**
  Rewritten — each is a shape around one centre, so `position` removes the centre from
  every point: `examples/swat/canvas/src/main.mfb:star` and its tree tiers
  (`pt(cx, top)` …), `examples/canvas-demo/src/ui.mfb`'s two diamonds (through a new
  `kit.mfb:polyAt`), and `examples/weather/src/timeline.mfb:tick`. Each was checked by a
  probe that draws 40 instances of varied size and place in both spellings and compares
  the frames: byte-identical (`shasum` `1b53d6fc…` for the stars, `cc364ea9…` for the
  diamond, tier and tick), and the dumped frames of canvas-demo, water and swat are
  the ones 172-E recorded (`90f2a990…`, `9db5937a…`, `35efe86a…`). The probe is the
  check because the examples' first frames do not all show these shapes, and weather's
  is not reproducible. Left as ported (`position` at the origin): point lists whose
  points are anchored to more than one reference (`dawn.mfb`, `twilight.mfb`,
  `scenery.mfb` mix a centre `x` with `baseY`, `top` or `horizon`), generators that
  return a point list other code reuses (`kit.mfb` ellipse rings, `svg.mfb`), and
  `kit.mfb:beam`, whose gradient is placed by its own transform in surface
  coordinates. In those `position` removes no arithmetic or would add some.
- **`rt_canvas_font` is red on the final tree, and it is not this letter's.**
  `text_on_the_gpu_matches_the_software_oracle` and
  `a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle` passed in this
  letter's first verification round and failed in its last, with whole 32 × 32 blocks of
  the Metal frame unwritten and a different number of them on each run (37888, 38912,
  36864, 37888, then all 576000 pixels). The only compiler source changed between the
  two rounds is `func_geo_build.rs` (the offset hoist and the carried point); with the
  earlier file put back and the compiler rebuilt the test still fails, the scene is a
  single `canvas::Text`, which that builder does not build, and Metal shape scenes pass
  at the same moment (`rt_canvas_golden the_gpu_draws` → 6 passed). The machine's load
  average was 10 to 21 throughout. Filed as `bugs/bug-1016-*` with the reproduction;
  **172-G's prerequisites require it re-run on a quiet machine before that letter
  starts.**
- **A test program lost its build to the port and was fixed**:
  `tests/runtime/rt_arena_large_block_reuse.rs` draws only `canvas::Line`s, which now
  name `vector::Float2`, and had no `IMPORT vector`.
- **The runtime proof is the golden test.** `polylines_match_their_reference_exactly`
  compares the dumped `polyline` scene with the committed PNG byte for byte, before and
  after: the PNG is unchanged (`git status --porcelain tests/golden/canvas` → empty).
- **The three `MUT` + `WITH` examples were compiled and run**, since the registry does
  not: `mfb man canvas types` renders them, and a probe with the three as written draws
  3918 white pixels spanning x 20..219, y 39..360.
- **`field_kinds.tsv`: five rows' check column aligned with the generator**
  (`Rectangle`, `Line`, `Polygon`, `Picture`, `PolyLine` → `before.id`). `Rectangle`'s
  and `Picture`'s read `before.x`, which 172-E removed, and `Line`'s read `before.x1`.
  The grid for those rows: all 28 `S`-site pairs and `Picture`'s 15 pass; the 32
  `T`-site pairs fail, on an expectation plan-172-N made stale — filed as
  `bugs/bug-1015-*`, not fixed here.
- **From 172-A, Open Decision 1:** no constructor functions; the task is struck as moot
  and replaced by the `MUT` + `WITH` example task.

## Summary

The last of the record reshaping. The port is deliberately the dullest possible one —
`position` at the origin, points untouched — so that 230 sites move with no pixel at
risk, and the new capability is proved by three small scenes rather than by the port.
