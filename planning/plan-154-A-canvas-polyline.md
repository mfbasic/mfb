# plan-154-A: `canvas::PolyLine` — the item, and the software oracle

Last updated: 2026-09-24
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

Add `canvas::PolyLine` to the `DrawItem` union. It is an open path through a
list of points, stroked as one item, with a round join at every interior
vertex, a `CapStyle` at the two ends, and a linear alpha fade from the first
point to the last.

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

**Correct behavior.** For `PolyLine[points := P, cap := c, fade := f, paint := s]`
with `n = len(P)`:

- **What is stroked.** The union of the `n-1` segments `P[i]→P[i+1]`, each
  stroked with half-width `s.strokeWidth / 2`, in `s.stroke`.
- **Joins.** Every interior vertex is round. This follows from taking the
  minimum of the segment distances.
- **Ends.** `P[0]` and `P[n-1]` are capped by `c`: `Butt` ends flush at the
  point, `Round` adds a half-disc.
- **Fade.** A pixel whose nearest segment is `i`, at parameter `t` along it,
  has its stroke alpha multiplied by `1 - (1 - f) * (i + t) / (n - 1)`. So
  `P[0]` is full strength and `P[n-1]` is at `f`. `f = 1.0` means no fade.
- **Degenerate inputs.** `n < 2` draws nothing. A zero-length segment behaves
  like a round dot of the stroke width, the same as a zero-length `Line`
  today.
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
  golden.
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
    cap AS CapStyle
    fade AS Float
    paint AS Paint
  END TYPE
  ```
  It is appended to `DrawItem`.
- **Geometry kind.** Add `__CANVAS_GEO_POLYLINE`, with a new `GEO_KIND_POLYLINE`
  in `runtime/canvas/mod.rs`. The header records:
  - the edge count;
  - the cap;
  - `fade`;
  - the stroke half-width;
  - bounds padded by the half-width plus 1 px (the same padding as
    `__canvas_polygonHeader`).
  
  The tail is `n-1` edges in the polygon layout.
- **Distance and coverage (software).** Scan the edges; for each, compute the
  unsigned segment distance and its parameter `t`. Keep the minimum and its
  `(i, t)`. At edge 0's start and edge `n-2`'s end, when `cap = Butt`, use the
  butt slab. Coverage is the same antialiasing ramp as `Line`, and alpha is
  multiplied by the fade formula.
- **Hashing.** Add PolyLine to `HASHED`, with a native hash in
  `func_item_hash.rs` over points, cap, fade and paint.
- **Native build.** Add the kind to `geoBuild` (`Kind::ALL`). A polyline's
  geometry changes every frame in wind's use (9,237 builds per frame
  measured), so the interpreted `__canvas_geometryFor` path would dominate.
- **Decline on GPU (this sub-plan only).** Both renderable checks return
  FALSE when the frame contains a PolyLine. B and C remove the decline, each
  for its own backend.

**Where correctness risk concentrates.**

- The butt caps at the path ends. The golden pins them.
- The `(i, t)` of the minimum at a join, where two edges are equally near.
  Either choice gives the same alpha only if the fade formula is continuous at
  the vertex. It is, because `(i + 1) + 0 = i + 1`.

**Where design uncertainty concentrates.** The same butt-cap composition, so
Phase 1 is a golden-first experiment.

**Gate class:** new behavior. The gates are the new golden and the
fallback-behavior test. The existing 7 goldens must stay byte-identical,
because nothing about the other kinds changes. A diff there is a bug to
root-cause.

Rejected alternatives:

- **A stroke gradient on `Paint`.** It would change every item's `Paint`, and
  its hash and GPU block, to serve one use.
- **Per-vertex colour lists.** More general, but they need a per-vertex colour
  region on both GPUs. The alpha fade covers the trail case, which is the
  motivating one. See Open Decisions.
- **An open flag on `Polygon`.** It would change a frozen record, and a
  polygon's fill semantics don't apply to an open path.
- **Building the polyline from `Line` items in the runtime.** That keeps the
  per-item cost the feature exists to remove.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; strike moot tasks with evidence; fill
> `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — record, oracle geometry, golden (uncertainty first)

- [ ] `mod.rs`: add the `PolyLine` record with `description` prose, and
      append it to `DrawItem`. Update `draw_item_variant_set_is_frozen` to 11
      names; `every_draw_item_variant_carries_a_paint` passes as written.
- [ ] `helper_geometry.rs`: add `__CANVAS_GEO_POLYLINE` to `GEO_LAYOUT`,
      `__canvas_polylineHeader`, and `__canvas_polylineEdges` (the polygon
      layout without the wrap edge). Add a `CASE PolyLine(p)` to
      `__canvas_geometryFor`, `__canvas_hashItem` and `__canvas_geoReference`.
- [ ] `helper_draw.rs` / `helper_items.rs`: the polyline branch of
      `__canvas_geoDistance`, with minimum, `(i, t)`, end caps and fade as §3
      describes.
- [ ] `runtime/canvas/mod.rs`: `GEO_KIND_POLYLINE`.
- [ ] `func_item_hash.rs`: add to `HASHED` with a native hash.
- [ ] Add a golden `tests/golden/canvas/polyline.png` in the
      `tests/canvas/rt_canvas_golden.rs` harness. The scene draws:
      - a zig-zag with round caps;
      - the same zig-zag with butt caps;
      - a path with a 180° switch-back;
      - a two-point path, which must match a `Line` with the same cap pixel
        for pixel (this is the check that falsifies the design);
      - a zero-length segment;
      - `fade = 0.2`;
      - a one-point path, which draws nothing.
      
      Create it with `MFB_UPDATE_CANVAS_GOLDEN=1` and inspect the PNG by eye
      before committing.

Acceptance: the golden passes; the two-point path is pixel-identical to the
matching `Line`; the 7 existing goldens are unchanged.
  Check: `cargo test --test rt_canvas_golden` → pass, 8 goldens (est. 4 min).
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
      fade formula); state that a PolyLine frame falls back to software until
      its backend draws it. Cite `__canvas_polylineEdges` and the renderable
      checks.
- [ ] Man page: the record `description` in `mod.rs`, with an example that
      draws a fading trail.

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

- **Fade model: one `fade` Float** (recommended; it covers trails and costs
  one header float) **vs a per-vertex colour list** (general, but it needs a
  new per-frame GPU region). Decide before Phase 1; changing it later changes
  the record.
- **Joins: round only** (recommended; free under the min-distance SDF)
  **vs a `JoinStyle` with miter and bevel.** Miter and bevel need
  per-vertex half-plane tests in three renderers. Round-only can be extended
  later without breaking the record, by adding a field whose zero value is
  `Round`.

## Corrections

(none yet)

## Summary

The risk is the oracle's end-cap and fade composition, and the two-point
`Line` equivalence is the check that would catch it. Nothing about the other
10 item kinds, their tags or the existing goldens changes. GPU frames with a
PolyLine are correct from day one, because they fall back to software.
