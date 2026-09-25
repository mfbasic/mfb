# plan-154-A: `canvas::PolyLine` — the item, and the software oracle

Last updated: 2026-09-24
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

Add `canvas::PolyLine` to the `DrawItem` union. It is an open path through a
list of points, stroked as one item, with a round join at every interior
vertex, a `CapStyle` at the two ends, and an optional colour per point that the
stroke blends between along the path (Corrections A1).

A fading trail of `k` points is then one item, not `k-1` `canvas::Line` items.
In the measured frame below, that turns wind's scene from 11,291 items into
about 3,200. That number is a guess: one item per drawn particle, not yet
measured.

This sub-plan defines the record, draws it in the software rasteriser (the
exact-match oracle), and makes every closed-set seam accept it. On a GPU
frame containing a PolyLine, Metal and Vulkan **decline the frame**, and it
falls back to software until plan-154-B (Metal) and plan-154-C (Vulkan) land.
That fallback is the existing, documented behavior for anything a backend
does not draw (`spec app canvas`, "What Metal declines"). It is not a stub.

The four sub-plans run in letter order:

| Sub-plan | Effort | Scope |
|---|---|---|
| A (this) | large | item, oracle, seams, docs |
| B | medium | Metal |
| C | medium | Vulkan |
| D | small | wind adopts it, measured |

**Correct behavior.** For `PolyLine[points := P, colors := C, cap := c, paint := s]`
with `n = len(P)`:

- **What is stroked.** The union of the `n-1` segments `P[i]→P[i+1]`, each
  stroked with half-width `s.strokeWidth / 2`, in `s.stroke`.
- **Joins.** Every interior vertex is round. This follows from taking the
  minimum of the segment distances.
- **Ends.** `P[0]` and `P[n-1]` are capped by `c`: `Butt` ends flush at the
  point, `Round` adds a half-disc.
- **Colour.** An empty `C` strokes the whole path in `s.stroke`. Otherwise
  point `k` has colour `C[k]`, or `s.stroke` for a `k` past the end of `C`. A
  pixel whose nearest segment is `i`, at parameter `t` along it, is drawn in
  the colour a `Gradient` with stops `(k / (n-1), colour k)` gives at
  `(i + t) / (n - 1)`. Channels mix in linear light and alpha mixes straight,
  exactly as `Paint.fillGradient` does. Extra entries in `C` are ignored. A
  non-empty `C` draws even when `s.stroke` is transparent; `s.strokeWidth <= 0`
  draws nothing.
- **Degenerate inputs.** `n < 2` draws nothing. A zero-length segment adds
  nothing. A path whose points all coincide draws what a zero-length `Line`
  with the same cap draws: a dot for `Round`, nothing for `Butt`. The butt
  ends are cut at the first and last segments that have length.
- **Fill.** `fill` is ignored, as it is for `Line`.

References:

- `mfb spec app canvas` (`src/docs/spec/app/06_canvas.md`) — rendering conventions, caps/joins text, what backends decline.
- `.ai/canvas-threading.md` — the three-thread model and the per-item GPU block; read it before touching the render path.
- `.ai/man-content.md` — the `mfb man canvas types` prose.
- bug-686/bug-688 docs (`bugs/completed/bug-686-*`, `bugs/bug-688-*`) — frame caps and per-item costs.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| `DrawItem` has 10 members, `PolyLine` not among them | `grep -n "fn draw_item_variant_set_is_frozen" -A25 src/codegen/builtins/canvas/mod.rs` → 10 names, no `PolyLine` | MET (2026-09-24) |

Everything below assumes it holds.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue, and again before you decide to stop.

## 1. Goal

- `canvas::PolyLine` constructs and type-checks.
- `canvas::present` accepts it.
- The software rasteriser draws it exactly as defined above, pinned by a new
  golden. The per-point colours reuse the gradient-stop tail, so both GPUs can
  read them from the stop region they already upload (Corrections A1).
- Metal and Vulkan decline a frame that contains one and draw that frame in
  software. A test shows the fallback.
- `mfb man canvas types` documents it, and every example runs.

### Non-goals (explicit constraints)

- **The existing 10 members keep their tags.** `PolyLine` is appended as tag
  10, because the tag order is frozen by `draw_item_variant_set_is_frozen`.
- **No existing golden changes** (`tests/golden/canvas/*.png`).
- **No change to `canvas::Line`, `Polygon` or `Paint`.** In particular, no
  stroke gradient is added to `Paint`.
- **No GPU drawing in this sub-plan** (that is B and C).
- **No miter or bevel joins.** See Open Decisions.

## 2. Current State

**The union.** `DrawItem` is registered in
`src/codegen/builtins/canvas/mod.rs`: the records are `pkg.add_record` calls,
and the union is `add_union(RegistryUnion { name: "DrawItem", … })`. Its
members, in tag order: Picture, Rectangle, Line, Polygon, Circle, Arc, Text,
RoundedRect, Ellipse, Group.

- `Line` is `x1, y1, x2, y2, cap AS CapStyle, paint AS Paint`.
- `Polygon` is `points AS List OF Point, paint`, and is always closed.
- `CapStyle` is `Butt` (the zero value) or `Round`.
- No member has joins; the spec's caps/joins text says so.

**Geometry.** Each item resolves to a cached geometry record: a 47-float
header (`__CANVAS_GEO_HEADER`) plus an optional tail
(`src/codegen/builtins/canvas/helper_geometry.rs`).

- The polygon tail is built by `__canvas_polygonEdges` at 5 floats per edge,
  `x0, y0, dx, dy, invLenSq`. It was read: it wraps with `(i + 1) MOD count`,
  and a zero-length edge stores `invLenSq = 0`.
- Line uses `__canvas_segmentHeader` (`KIND_SEGMENT = 2`), and Polygon uses
  `__canvas_polygonHeader` (`__CANVAS_GEO_POLYGON = 4`).
- A cache miss is built natively by `canvas::geoBuild`
  (`func_geo_build.rs`, `Kind::ALL`), for Rect, Rounded, Circle, Line and
  Polygon only. Anything else is built by `__canvas_geometryFor`.

**Software drawing.**

- `__canvas_drawGeometry` (`helper_items.rs`) calls `__canvas_geoDistance`
  (`helper_draw.rs`).
- A segment uses `__canvas_segmentDistance` / `__canvas_segmentDistanceButt`
  (`helper_shapes.rs`).
- A polygon uses `__canvas_edgeDistance`, and its stroke band is
  `abs(dRaw) - half` (`helper_items.rs`).

**Closed-set seams a new member must pass.**

- The MFBASIC `MATCH`es without `CASE ELSE` in `helper_geometry.rs`:
  `__canvas_geometryFor`, `__canvas_hashItem` and `__canvas_geoReference`.
- `func_item_hash.rs`: `HASHED` / `NOT_HASHED`. A variant in neither list
  fails the build.
- `mod.rs` tests: `draw_item_variant_set_is_frozen` and
  `every_draw_item_variant_carries_a_paint`.
- `helper_geometry.rs` `GEO_LAYOUT`, and `src/codegen/runtime/canvas/mod.rs`
  `GEO_KIND_*`.
- Tests that construct every variant: `tests/cli/cli_canvas_package.rs`,
  `tests/canvas/rt_canvas_geo_native.rs`, and
  `tests/runtime/inplace_self_update/field_kinds.tsv`.

**Declining.** The GPU paths decide per frame whether they can draw it:
`__canvas_metalRenderable` and `__canvas_vulkanRenderable`, called from
`__canvas_renderFrame` in `helper_render.rs`. A declined frame is drawn by
`__canvas_renderScene`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` members | 10 | `grep -n "fn draw_item_variant_set_is_frozen" -A25 src/codegen/builtins/canvas/mod.rs` |
| `MATCH`es without `CASE ELSE` over every kind | 3 | `grep -n "CASE Ellipse(" src/codegen/builtins/canvas/helper_geometry.rs` → 3 (lines 923, 1147, 1228) |
| `HASHED` kinds | 8 | `grep -n "const HASHED" src/codegen/builtins/canvas/func_item_hash.rs` → `[&str; 8]` |
| wind item blocks per frame | 11,291 | `--debug` wind, `MFB_CANVAS_STATS=/tmp/wstats.txt`, 15 s on Metal → `blocks=11291` |
| wind geometry builds per frame | 9,237 | same run: `generations=3094391` over `frames=335` |
| canvas goldens | 7 | `ls tests/golden/canvas/*.png \| wc -l` |

### Verified properties

- **The polygon edge tail fits an open path.** Read in `__canvas_polygonEdges`.
  The only difference is the wrap edge, which a polyline omits. So the
  PolyLine tail is the same 5-float layout for `n-1` edges, and plan-154-B/C
  can upload it through the existing edge regions.
- UNVERIFIED: whether `__canvas_segmentDistanceButt` gives the right butt end
  when it is applied only at the path's two ends while interior ends stay
  round. Phase 1 settles this with a golden.

## 3. Design Overview

- **Record.**
  ```
  TYPE PolyLine
    points AS List OF Point
    colors AS List OF color::Color
    cap AS CapStyle
    paint AS Paint
  END TYPE
  ```
  It is appended to `DrawItem`.
- **Geometry kind.** Add `__CANVAS_GEO_POLYLINE`, with a new `GEO_KIND_POLYLINE`
  in `runtime/canvas/mod.rs`. Like `Line`, it is
  stroke-as-fill: the stroke colour is in the fill slots, the half-width is in
  the radius slot, and slot 7 is `-1`. The header records:
  - the edge count `n-1` (slot 20);
  - the first and last edge with length (slots 21 and `__CANVAS_GEO_CAPSTARTX`);
  - the cap (`__CANVAS_GEO_CAP`);
  - the colour stop count (`__CANVAS_GEO_GRADIENT_COUNT`, 0 or `n`);
  - bounds padded by the half-width plus 1 px (the same padding as
    `__canvas_polygonHeader`).
  
  The tail is `n-1` edges in the polygon layout, then `n` colour stops in the
  gradient layout (`offset, r, g, b, a`, with offset `k / (n-1)`), so
  `__canvas_gradientStopBase` finds them unchanged. `Paint.fillGradient` is
  ignored, as it is for `Line`.
- **Distance and coverage (software).** Scan the edges from the first live
  one to the last, skipping zero-length ones. For each, compute the segment
  distance with the same arithmetic as `__canvas_segmentDistance`, and track
  the minimum and its `(i, t)`. When `cap = Butt`, cut the first live edge's
  start and the last live edge's end with the slab `__canvas_segmentDistanceButt`
  uses, one-sided. Coverage is the same antialiasing ramp as `Line`. With
  colours, the fill colour is `__canvas_gradientColor` at `(i + t) / (n - 1)`.
- **Hashing.** Add PolyLine to `HASHED`, with a native hash in
  `func_item_hash.rs` over points, colours, cap and paint.
- **Native build.** Add the kind to `geoBuild` (`Kind::ALL`). A polyline's
  geometry changes every frame in wind's use (9,237 builds per frame
  measured), so the interpreted `__canvas_geometryFor` path would dominate.
- **Decline on GPU (this sub-plan only).** Both renderable checks return
  FALSE when the frame contains a PolyLine. B and C remove the decline, each
  for its own backend.

**Where correctness risk concentrates.**

- The butt caps at the path ends. The golden pins them.
- The `(i, t)` of the minimum at a join, where two edges are equally near.
  Either choice gives the same colour, because the parameter is continuous at
  the vertex: `(i + 1) + 0 = i + 1`.

**Where design uncertainty concentrates.** The same butt-cap composition, so
Phase 1 is a golden-first experiment.

**Gate class:** new behavior. The gates are the new golden and the
fallback-behavior test. The existing 7 goldens must stay byte-identical,
because nothing about the other kinds changes. A diff there is a bug to
root-cause.

Rejected alternatives:

- **A stroke gradient on `Paint`.** It would change every item's `Paint`, and
  its hash and GPU block, to serve one use.
- **One `fade` Float.** It covers only an alpha ramp on one colour. The
  per-point colour list was chosen instead (Corrections A1), and it needs no
  new GPU region because it rides the gradient-stop tail.
- **An open flag on `Polygon`.** It would change a frozen record, and a
  polygon's fill semantics don't apply to an open path.
- **Building the polyline from `Line` items in the runtime.** That keeps the
  per-item cost the feature exists to remove.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; strike moot tasks with evidence; fill
> `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — record, oracle geometry, golden (uncertainty first)

- [x] `mod.rs`: add the `PolyLine` record with `description` prose, and
      append it to `DrawItem`. Update `draw_item_variant_set_is_frozen` to 11
      names; `every_draw_item_variant_carries_a_paint` passes as written.
      `cargo test --bin mfb canvas` → 97 passed.
- [x] `helper_geometry.rs`: add `__CANVAS_GEO_POLYLINE` to `GEO_LAYOUT`,
      `__canvas_polylineHeader`, and `__canvas_polylineEdges` (the polygon
      layout without the wrap edge). Add a `CASE PolyLine(p)` to
      `__canvas_geometryFor`, `__canvas_hashItem` and `__canvas_geoReference`.
      The edges function is `__canvas_polylineTail`, because it also appends the
      colour stops (Corrections A1). Also `__canvas_polylineHalf`, and a
      `__CANVAS_GEO_POLYLINE` skip in `__canvas_paintHeader`'s stop count.
- [x] `helper_draw.rs` / `helper_items.rs`: the polyline branch of
      `__canvas_geoDistance`, with minimum, `(i, t)`, end caps and colours as §3
      describes. One walk, `__canvas_polylineScan`, answers both the distance
      and the colour parameter.
- [x] `runtime/canvas/mod.rs`: `GEO_KIND_POLYLINE`, pinned to
      `__CANVAS_GEO_POLYLINE` by `the_geo_layout_constants_match_their_rust_counterparts`.
- [x] `func_item_hash.rs`: add to `HASHED` with a native hash. The structural
      walk is generated from the record layout, so adding the name was the whole
      change.
- [x] Add a golden `tests/golden/canvas/polyline.png` in the
      `tests/canvas/rt_canvas_golden.rs` harness. The scene draws:
      - a zig-zag with round caps;
      - the same zig-zag with butt caps;
      - a path with a 180° switch-back;
      - a two-point path, which must match a `Line` with the same cap pixel
        for pixel (this is the check that falsifies the design);
      - a zero-length segment;
      - a path with a colour per point, fading to transparent;
      - a path with fewer colours than points (the rest take `paint.stroke`);
      - a one-point path, which draws nothing.
      
      Create it with `MFB_UPDATE_CANVAS_GOLDEN=1` and inspect the PNG by eye
      before committing. Inspected: round joins in every row, butt ends cut
      square at the marks (including the repeated-end path), the trail fades to
      transparent, red→green→blue blends, the partial path turns white after its
      third point, the round coincident path is a dot, and the butt coincident
      and one-point paths are absent. The two-point check is its own test,
      `a_two_point_polyline_draws_the_line_between_its_points`, over both caps,
      a fractional diagonal and a 1.5 px line.
- [x] (added) Decline a PolyLine frame in `__canvas_metalRenderable` and
      `__canvas_vulkanRenderable` now, not in Phase 2: the item exists from this
      phase on, and a GPU frame must not draw an unknown kind. Phase 2's task
      keeps the fallback test.

Acceptance: the golden passes; the two-point path is pixel-identical to the
matching `Line`; the 7 existing goldens are unchanged.
  Check: `cargo test --test rt_canvas_golden` → pass, 8 goldens (est. 4 min).
  Result: 21 passed, 0 failed; `ls tests/golden/canvas/*.png | wc -l` → 8;
  `git status --short tests/golden` shows only the new `polyline.png`.
Commit: —

### Phase 2 — native build, fallback, seams, docs

- [ ] `func_geo_build.rs`: add PolyLine to `Kind::ALL`, with native
      header+tail build. `tests/canvas/rt_canvas_geo_native.rs` gets a
      PolyLine case that matches the MFBASIC build bit for bit.
- [ ] `helper_render.rs`: `__canvas_metalRenderable` and
      `__canvas_vulkanRenderable` return FALSE when a PolyLine is present.
      Add a test in `tests/canvas/rt_canvas_metal.rs`: a PolyLine scene run
      with `MFB_CANVAS_GPU=1` produces a frame identical to the software
      oracle, and the stats line shows `gpuFrames=0`.
- [ ] Update the tests that construct every variant:
      `tests/cli/cli_canvas_package.rs`, and
      `tests/runtime/inplace_self_update/field_kinds.tsv` (regenerate with
      `field_kinds_gen.py`).
- [ ] Spec, `src/docs/spec/app/06_canvas.md`: add PolyLine to the item list;
      rewrite the caps/joins text (a round join, the caps at the ends, the
      per-point colours); state that a PolyLine frame falls back to software until
      its backend draws it. Cite `__canvas_polylineEdges` and the renderable
      checks.
- [ ] Man page: the record `description` in `mod.rs`, with an example that
      draws a fading trail with per-point colours.

Acceptance: the native build matches bit for bit; the fallback test passes;
the docs render and their examples run.
  Check: `cargo test --test rt_canvas_geo_native --test rt_canvas_metal --test cli_canvas_package` → pass (est. 8 min);
  `scripts/man-run-examples.sh canvas --run` → all pass (est. 3 min);
  `scripts/spec-census.sh --citations` → `MISS-SYMBOL 0` (est. 1 min).
Commit: —

## Validation Plan

- Tests: the `polyline.png` golden (including the two-point-equals-`Line`
  check), native-vs-MFBASIC geometry, the Metal fallback, and every-variant
  construction.
- Coverage check: `grep -rln "PolyLine" tests/` → the golden harness, geo-native, rt_canvas_metal and cli_canvas_package.
- Runtime proof: the golden PNG, inspected by eye.
- Doc sync: `spec app canvas`, `mfb man canvas types`.
- Final gate: once, at the end of plan-154-D.

## Open Decisions

- ~~**Fade model: one `fade` Float vs a per-vertex colour list.**~~ Decided
  2026-09-24 by the user: a per-vertex colour list (Corrections A1).
- **Joins: round only** (recommended; free under the min-distance SDF)
  **vs a `JoinStyle` with miter and bevel.** Miter and bevel need
  per-vertex half-plane tests in three renderers. Round-only can be extended
  later without breaking the record, by adding a field whose zero value is
  `Round`.

## Corrections

- **A1 (2026-09-24): the fade model is a per-point colour list, not a `fade`
  Float.** The user resolved the Open Decision (`/follow-plan 154 - for fade -
  per-vertex colour list`). The plan said this "needs a per-vertex colour region
  on both GPUs". That is false: the colours are stored as gradient stops in the
  record's tail, the same layout `__canvas_gradientTail` writes, and both GPUs
  already upload the stops to a per-frame region with a base and count in the
  item block (`ITEM_ELLIPSE_GRADIENT_COUNT`/`_BASE`,
  `grep -n "ITEM_ELLIPSE_GRADIENT" src/codegen/runtime/canvas/mod.rs`). The
  record, the behavior text, §3, Phase 1's golden scene and the man page task
  were rewritten for it.
- **A2 (2026-09-24): a zero-length butt `Line` draws nothing, not a dot.** The
  plan said a zero-length segment behaves "the same as a zero-length `Line`
  today", which is a round dot. That holds only for `Round`:
  `__canvas_segmentDistanceButt` returns `1000000.0` when `len2 <= 0.0`
  (`grep -n "RETURN 1000000.0" src/codegen/builtins/canvas/helper_shapes.rs`).
  The degenerate-input rule was restated to match, and the butt ends move to the
  first and last edges with length, so a repeated end point cannot turn a butt
  end round.

## Summary

The risk is the oracle's end-cap and colour composition, and the two-point
`Line` equivalence is the check that would catch it. Nothing about the other
10 item kinds, their tags or the existing goldens changes. GPU frames with a
PolyLine are correct from day one, because they fall back to software.
