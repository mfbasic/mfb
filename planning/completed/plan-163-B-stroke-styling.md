# plan-163-B: stroke styling — caps, joins and dashes on `canvas::Paint`

Last updated: 2026-09-27
Effort: x-large (1d–3d)
Depends on: plan-163-A (see Prerequisites)

SVG strokes are styled by `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`,
`stroke-dasharray` and `stroke-dashoffset`. Canvas has a per-item `cap` on `Line`,
`Arc` and `PolyLine` only, with `Butt`/`Round` but no `Square`. It has no join choice:
every stroked corner has a round outer side. It has no dashes. This letter moves the
cap onto `canvas::Paint` (a dash end needs a cap on every stroked shape, closed ones
included). It adds `Square`, a join style with a miter limit, and a dash pattern with
an offset, in the software renderer, Metal and Vulkan.

**The single behavioral outcome:** a `canvas::Paint` carries `cap`, `join`,
`miterLimit`, `dashes` and `dashOffset`, and every stroked item draws them as SVG
defines them:

- caps at open ends and at every dash end;
- joins at polygon, polyline and rectangle corners, with a miter falling back to a bevel
  past `miterLimit`;
- dashes laid along the outline from `dashOffset`.

A paint with `cap := Butt` on a closed shape, `join := Round` and `dashes := []` draws
**exactly** what HEAD draws.

References:

- plan-163-A (`planning/plan-163-A-polygon-contours-fill-rule.md`): the ring structure
  this letter joins across.
- `src/docs/spec/app/06_canvas.md`:
  - :67–87, Paint's zero-value rule and "built with fill/stroke/fillStroke, refined with
    `WITH`";
  - :124–158, GPU frame caps, and PolyLine reusing edges and stops;
  - :302–360, caps, PolyLine union, ends, degenerate paths and colour;
  - :378–382, a stroke scales with the transform;
  - :438–442, a stroke is never a gradient.
- `.ai/canvas-threading.md`:
  - :509–515 (`strokeHalf` is signed);
  - :517–574, a per-item payload is a frame region plus an index, and region caps are
    frame sums;
  - :1003–1040, "Growing `ITEM_BLOCK_SIZE` moves six things, and three are silent".
    **This letter grows the block** (§7);
  - :1042+, a layout constant shared by MFBASIC and Rust needs a pin test.
- `.ai/testing-gates.md` :235 (software goldens exact, GPU within tolerance) and :309
  (the GPUs are compared against the oracle, so the oracle needs hand-computed pixel
  checks).
- SVG 2 §13.5 "Stroke shape" (linecap, linejoin, miterlimit, dasharray and dashoffset
  semantics) is the external contract. Where this letter departs from it, §4 says so.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-163-A complete and archived | `ls planning/completed/plan-163-A-*` → match | MET (2026-10-05: `planning/completed/plan-163-A-polygon-contours-fill-rule.md`) |

Everything below is written against the world where A has landed:
`Polygon.contours`, `Polygon.rule`, header slot 21 = rule, and item word `arc.y` = rule
for kind 4.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every command before you continue, and again before you decide to stop. If you stop,
> report the status of every row.

## 1. Goal

- `canvas::Paint` gains, in this order after `fillGradient`: `cap AS canvas::CapStyle`,
  `join AS canvas::JoinStyle`, `miterLimit AS Float`, `dashes AS List OF Float`, and
  `dashOffset AS Float`.
- `canvas::CapStyle` gains `Square` (ordinal 2).
- The new exported enum `canvas::JoinStyle` has `Round` (ordinal 0, today's behavior),
  `Miter` and `Bevel`.
- `Line.cap`, `Arc.cap` and `PolyLine.cap` are removed. The cap comes from `paint.cap`.
- All three renderers draw caps, joins and dashes. The software renderer is exact
  against hand-computed pixels; the GPUs are within `Tolerance::GPU_DEFAULT` of it.
- Every item that does not opt in draws what HEAD draws, byte for byte in software.

### Non-goals (explicit constraints)

- **No gradient on strokes.** A stroke stays a flat colour (spec :438–442).
- **No join on smooth outlines.** Circle, Ellipse, Arc and RoundedRect have no corners,
  so `join` does not apply to them. A RoundedRect's stroke keeps its current radius
  behavior.
- **Fill-only text is unchanged.** A glyph bitmap has no stroke. Stroked text (lowered
  to a polygon) takes join and dash like any polygon, but its glyph contours come from
  curve flattening, so its "corners" are mostly nearly straight.
- **No `vector-effect: non-scaling-stroke`.** The stroke keeps scaling with the
  transform (spec :378–382). The dash lengths and offset scale with it too.
- **No change to fill rendering, to the `DrawItem` variant set, or to fill-only items.**
- **No compatibility shim** for `Line.cap`/`Arc.cap`/`PolyLine.cap`. They are removed,
  and every in-tree site is migrated.

## 2. Current State

Paths are under `src/codegen/builtins/canvas/` unless written in full. Line numbers are
at HEAD `8cbd0206a` (pre-A; A does not touch these regions except where noted).

**Cap.**

- `CapStyle` is declared at `mod.rs:635–650` with `Butt` (0) and `Round` (1). The
  `cap` prop is on `Line` (:932–938), `Arc` (:1017–1023) and `PolyLine` (:1263–1270).
- The header slot is `__CANVAS_GEO_CAP = 34`, plus the arc sweep endpoints in 35–38
  (`helper_geometry.rs:63–67`). Picture reuses 35–36 (:82–86).
- The tag is written by `__canvas_segmentHeader` (:553–571), `__canvas_arcHeader`
  (:573–604) and `__canvas_polylineHeader` (:664–727), and by the native
  `func_geo_build.rs:fill_line` (:810–823) and the polyline builder (:1155–1175,
  :1231–1233).
- **The tag is computed by `__canvas_capTag` (`helper_geometry.rs:235–240`) and its
  native twins (`func_geo_build.rs:810–823`, :1155–1170) as "Round → 1, anything else →
  0".**
- The readers disagree on what "not Butt" means:
  - Line treats `cap = 1` as round and anything else as butt: `helper_draw.rs:80`,
    `mfb_canvas.frag:464`, `:553–554`, and `metal.rs:582`, `:683–684`.
  - Arc and PolyLine treat `cap ≠ 0` as round: `helper_draw.rs:95`, `:221`,
    `mfb_canvas.frag:415`, `:479`, `:543`, and `metal.rs:519`, `:597`, `:673`, `:713`,
    `:749`.
  - A third tag would therefore draw Square as butt on a Line and as round on an Arc.
    Every reader must switch to explicit tag comparisons (Phase 2).
- Software butt: `helper_shapes.rs:__canvas_segmentDistanceButt` (:151–169) is the
  round distance, intersected with the two end half-planes, each at distance `−t·len`
  and `(t−1)·len`.
- Software round: `__canvas_segmentDistance` (:94–108). The clamped `t` makes the
  end round.
- Arc caps: `helper_draw.rs:95–107` take the min with a disc at each sweep endpoint.
- PolyLine caps: `__canvas_polylineScan` (`helper_draw.rs:221–229`) puts end planes on
  the outer ends of the first and last live segments only.
- GPU: the tag is in `arc.w` (`ITEM_ARC_CAP = 12`, `runtime/canvas/mod.rs:447–455`),
  stored for every kind (`metal.rs:3837–3846`, `vulkan.rs:3707–3716`). Arc endpoints are
  in `arcCaps` (:490–506), with low words in `capsLo`.
- Bounds padding for a stroked line is `half + 1` (`helper_geometry.rs:569`,
  `func_geo_build.rs:836–847`). That is too small for a diagonal square cap, which
  reaches `half·√2`. The polygon band reach assumes `half` too (`metal.rs` doc near
  `emit_band_index`, `vulkan.rs:4295–4305`).

**Stroke corners today.**

- The stroke coverage is `(|d| − half)` over the shape's signed distance: software
  `helper_items.rs:631–633`, and `dfSub(dfAbs(dRaw), half2)` in the shaders
  (`mfb_canvas.frag:895`, `metal.rs:835`).
- Rectangle and Picture use a box distance (`helper_shapes.rs:48–56`), which is
  Euclidean outside a corner. **The outer stroke corner is round with radius `half`, and
  the inner corner is sharp.**
- A polygon uses the min Euclidean distance to its edges (`helper_draw.rs:136–166`), so
  its convex outer corners are round.
- A PolyLine is the min over round-capped segments (`helper_draw.rs:183–189`, spec
  :328–333), so its joins are round.
- **Round is therefore today's join everywhere, and it is the compatible zero value.**

**Paint.**

- Declared at `mod.rs:786–875` with seven fields.
- The only `Paint[...]` literal in the tree is in `func_fill_stroke.rs:77` (the
  `__canvas_fillStroke` helper). `canvas::fill` (`func_fill.rs:77–79`) and
  `canvas::stroke` (`func_stroke.rs:71–73`) call it.
- The defaults helpers are in `helper_paint_defaults.rs:163–180`.
- "all seven `canvas::Paint` fields" is written at `func_fill.rs:24`,
  `func_stroke.rs:24` and `func_fill_stroke.rs:24`.
- Paint → header: `helper_geometry.rs:__canvas_paintHeader` (:257–373) writes slots 7,
  8–15, 22–33 and 41–46. Its native twin is `func_geo_build.rs:emit_paint_header`
  (:500–584). The native builder refuses paints it cannot build
  (`emit_paint_supported`, :605–642), and such items fall back to the MFBASIC builder.
- Hash: the native `func_item_hash.rs` folds records structurally, so the new fields are
  folded with no change. The MFBASIC `__canvas_hashPaint` (`helper_geometry.rs:1189–1250`)
  is reached for Text and needs the new fields added by hand.
- Every `DrawItem` except `Group` carries a paint (`mod.rs:paint_prop`, test
  `every_draw_item_variant_carries_a_paint`).

**Path parameter.** PolyLine colour uses a **segment-index** parameter
`(e + t)/edges` (`helper_draw.rs:232`, `mfb_canvas.frag:435`, Metal near :536). It is
not arc length. Nothing in the canvas code computes arc length (grep for
`arclength|cumulative|perimeter|path length` over `src/codegen/builtins/canvas`,
`src/codegen/runtime/canvas` and `metal.rs` finds nothing relevant).

**Record tail order.** The gradient stop tail is always **last**, and
`__canvas_gradientStopBase` (`helper_geometry.rs:251–255`) locates it as
`length − stopCount·5`. Anything this letter appends goes **before** the stops.

**GPU item block** (272 bytes, 17 `ivec4`s). It is declared in
`runtime/canvas/mod.rs:415–580`, `mfb_canvas.frag:18–39`, `mfb_canvas.vert:20–40` and
`metal.rs:160–182`. The only word free on every kind is `xform1.w`, which is never
written, so it holds stale stack data. Joins and dashes need at least four words
per item for a polygon or polyline, which already use `arcCaps` and `ellipse.z/w`.
So the block grows (§7). Per-item variable-length data follows the gradient-stop
precedent: a frame region, with a count and base in the block. See Metal
`emit_gradient_buffer` (`metal.rs:4276`), Vulkan `emit_gradient_upload`
(`vulkan.rs:3750`), `gradientAt` (`mfb_canvas.frag:751–753`), and the region constants
at `runtime/canvas/mod.rs:1163–1211`. Frame-sum caps are generated into
`helper_render.rs:742–821` (Metal) and :897–960 (Vulkan).

### Measured populations

The command for every row is `grep -rnoE --exclude-dir=target --exclude-dir=.git '<pattern>' . | sed 's#^\./##' | cut -d/ -f1 | sort | uniq -c`.

| What | Count |
|---|---|
| `cap :=` sites (every one is a Line/Arc/PolyLine construction) | 115 (tests 87, examples 9, planning 6, scripts 5, bugs 4, src 4) |
| of which `canvas::Line[` / `canvas::Arc[` / `canvas::PolyLine[` | 47 / 34 / 34 |
| `src` sites | 4: `func_stroke.rs:37`, `:64`, `func_present.rs:57`, `:80` (man examples) |
| Rust-string sites split across lines in `rt_canvas_rasteriser.rs` | 12, of which 4 format the cap as `CapStyle.{cap}` (:1150, :1208, :1253, :1337) |
| `Paint[` literals in code | 1 (`func_fill_stroke.rs:77`) |
| `WITH canvas::(fill\|stroke\|fillStroke)\(` refinements | 106 (these compile unchanged: `WITH` names only the fields it sets) |
| Cap-tag readers that must become explicit comparisons | 13 (software 3, frag 5, Metal 7 lines listed in §2, "Cap"). Recount with `grep -nE "cap *(==\|!=\|=\|<>) *[01]\|cap != 0\|cap == 1" …` before Phase 2 |

The historical docs (planning 6, bugs 4) are not migrated. That leaves 105 sites to
migrate.

### Verified properties

- **Round is today's join on every cornered stroke.** Verified by reading the box
  distance, `__canvas_edgeDistance` and the PolyLine min-union (§2 citations).
- **The `WITH` refinements survive new Paint fields.** `WITH` names only the fields it
  sets, and the one full literal is the helper (§2 counts).
- **Stops must stay last.** Verified by reading `__canvas_gradientStopBase`.
- UNVERIFIED: that the double-float precision scheme (`capsLo` and friends) is needed
  for join and dash geometry to stay within `GPU_DEFAULT`. The Phase 4 and Phase 6 GPU
  rows measure it. If they are out of tolerance, add low words (a task, not a stop).
- UNVERIFIED: the ellipse arc-length table resolution needed for dash ends to agree
  within tolerance (Phase 5 measures it).

## 3. Design Overview

The work comes in four pieces, in this order:

1. **Cap onto Paint.** This is behavior-neutral: `paint.cap` feeds header slot 34
   exactly as the item's `cap` did.
2. **Square cap.** It needs a third tag, explicit comparisons in every reader, and wider
   bounds and band reach.
3. **Joins.** When `join ≠ Round` on a cornered stroke, the stroke is no longer
   `|d| − half` of the outline distance. It becomes the union of **butt-ended edge
   bands** and **per-vertex join shapes** that are precomputed at geometry build.
4. **Dashes.** A **perimeter parameter** `s` is computed at the pixel's nearest outline
   point. Coverage is the max of the band distance and the along-path distance to the
   nearest dash, with caps applied at the dash ends.

**Design uncertainty concentrates in piece 4 (and piece 3's GPU half).** A distance
field needs, per pixel, the nearest outline point *and* its arc length.

- Near a corner the nearest point jumps between edges, so a dash that crosses a corner
  must still be continuous.
- An ellipse has no closed-form arc length under the oracle's `+ − × ÷ sqrt` rule.

The cheapest experiment that could falsify the design is the software dash on a
polygon corner and on an ellipse. Phase 5 does it first, before any GPU work.

**Correctness risk concentrates in the item-block growth** (§7). It is six coupled
sites, three of them silent (`.ai/canvas-threading.md:1003`). It lands in the joins-GPU
phase behind the four named guard tests.

**Correctness gate: behavioral, with one byte-identity sub-gate.** Every software golden
PNG and hand-computed pixel test for an item that does not opt in stays **exact**. That
is the proof that the neutral defaults are neutral. If one fails, dump ONE scene's
header with `MFB_CANVAS_DUMP` against HEAD's and fix it; never re-baseline. **Expected
to change:** `tests/syntax/app/app-mouse-surface/golden/*` (the type surface), and the
item-hash values (in-process only).

**Rejected alternatives** (do not re-litigate):

- *A `strokeType` enum (Line / Even Dash / Odd Dash / Dot).* It cannot express an
  arbitrary SVG `stroke-dasharray`. A list of lengths covers the enum's cases: a dot is
  a zero-length dash with a round cap.
- *Join typed as `CapStyle`.* Joins (Miter/Round/Bevel) and caps (Butt/Round/Square)
  are different sets.
- *Keeping `cap` on Line/Arc/PolyLine as well as Paint.* That is two sources for one
  value. Dashes need caps on closed shapes, so the cap belongs to the stroke.
- *`Miter` as JoinStyle's zero value (SVG's default).* The zero ordinal is canvas's
  historical behavior (`CapStyle.Butt`, `GradientKind.Linear`, `FillRule.EvenOdd`), and
  today's joins are round. An SVG converter sets `join` explicitly.
- *Computing joins in the shader from neighbouring edges.* A's edge tail has no ring
  boundaries, and a flat edge list cannot tell a ring's wrap from its next ring.
  Precomputed join shapes need no adjacency on the GPU.
- *Arc length by sampling in the shader.* That is per-pixel cost, and the result would
  depend on backend precision. Precomputed cumulative lengths are exact inputs.

## 4. Semantics (the public contract)

- **`cap`**:
  - `Butt` ends the stroke at the endpoint.
  - `Round` adds a half-disc of radius `half`.
  - `Square` adds a `half × 2·half` box extending `half` past the endpoint along the
    tangent.
  - It applies at the two ends of a Line, an Arc or a PolyLine, and at both ends of
    every dash. On an undashed closed shape it has no effect.
  - A zero-length Square end is an axis-aligned square (SVG 2 §13.5.2).
- **`join`**: applies at every vertex where two edges of a Polygon ring or a PolyLine
  meet, and at a Rectangle's (and Picture's) four corners.
  - `Round` is today's behavior.
  - `Bevel` fills the triangle between the vertex and the two offset edge ends.
  - `Miter` extends the two offset edges to their intersection.
- **`miterLimit`**: when `1 / sin(θ/2) > miterLimit`, where θ is the interior angle
  between the edges, a `Miter` join draws as `Bevel` (SVG). A value below 1.0 is treated
  as 1.0, so every miter bevels. `canvas::fill`, `canvas::stroke` and
  `canvas::fillStroke` set 4.0 (SVG's default). The zero value 0.0 is therefore "always
  bevel", which is visible, not silent. See Open Decisions.
- **`dashes`**: alternating on/off lengths in pixels, before the transform.
  - An empty list, or a list whose sum is 0, draws a solid stroke.
  - An odd-length list is repeated once to make it even (SVG).
  - A negative entry makes the whole list count as empty. SVG treats it as an error and
    draws solid; this is recorded here so no one "fixes" it into a trap.
- **`dashOffset`**: how far into the pattern the outline starts. It may be negative or
  larger than the pattern, and is taken modulo the pattern length.
- **Where `s = 0` is**:
  - a Line: at `(x1, y1)`;
  - an Arc: at `startAngle`;
  - a PolyLine: at `points[0]`;
  - a Polygon: at each ring's first point, with each ring restarting its pattern (SVG
    restarts per subpath);
  - a Rectangle: at the top-left corner, going clockwise, meaning toward +X under
    Y-down;
  - a RoundedRect: at the start of its top straight edge, clockwise;
  - a Circle or an Ellipse: at angle 0 (the +X point in the item's own frame), in the
    increasing-angle direction.

  SVG fixes these for `<rect>`, `<circle>` and `<ellipse>` in the same way.

## 5. Caps (Phases 1–2)

- Phase 1 is a move only. `__canvas_capTag`, and its native twins, read `paint.cap`
  instead of the item's `cap`. Slot 34 is written exactly as today, so every item's
  header is byte-identical to HEAD's.
- Phase 2 adds tag 2 = Square:
  - Every reader in §2 "Cap" becomes an explicit three-way test.
  - Software Square on a segment is the butt distance of the segment extended by `half`
    at each end: `max(|perp| − half, −t·len − half, (t−1)·len − half)`, measured against
    the original segment's `t` and `len`. It is **not** the butt planes shifted, which
    would round the corners.
  - Arc Square is the min with an oriented `half × half` box at each sweep endpoint,
    using the tangent from `arcCaps − centre`.
  - PolyLine Square extends only the outer ends of the first and last live segments.
  - Bounds padding becomes `half·√2 + 1` when the cap is Square. The band reach grows to
    match (`metal.rs` / `vulkan.rs` `emit_band_index`).

## 6. Joins (Phases 3–4)

- **Geometry** (both builders, bit-identical):
  - When the stroke is live, `join ≠ Round`, and the item is a Polygon ring, a PolyLine
    or a Rectangle/Picture, the builder emits a **join tail** after the edge tail and
    before the stops.
  - It holds one record per vertex. A PolyLine emits none for its two end vertices,
    which get caps instead.
  - Each record is a convex join polygon of up to 4 points: vertex, offset end A, miter
    tip or nothing, offset end B. That is 8 floats plus 1 point count, 9 floats per
    record.
  - A miter past `miterLimit` is emitted as the bevel triangle, and the builder decides
    it in f64.
  - A new header slot, `__CANVAS_GEO_JOIN_COUNT`, holds the record count (§7 header
    growth).
  - The edge distance switches to butt ends (`segmentDistanceButt`) on items with a join
    tail. The butt-edge union plus the join polygons is exactly the SVG stroke.
  - With `join = Round` no join tail is emitted, and the item is byte-identical to
    today's.
- **Software:** stroke distance = `min(min over edges of butt band distance, min over
  joins of convex-polygon distance)`. Fill is unchanged: the signed crossing distance
  still decides inside.
- **GPU:** a join region (frame-wide, like gradient stops). The item block's new words
  hold the base and count (§7). The shader's stroke path takes the same min.
  Frame-sum cap: joins share the edge budget (each join counts as 2 edges toward the
  262,144 cap), so no new cap constant is needed. Revisit if the Phase 4 rows show
  otherwise.

## 7. Layout growth (Phase 4, once, for joins and dashes)

- **Geometry header**: 47 → 50 slots. The new slots are 47 `JOIN_COUNT`, 48
  `DASH_COUNT` and 49 `DASH_OFFSET`, placed **after** every existing slot so no current
  slot index moves.
  - `__CANVAS_GEO_HEADER` and its Rust mirror change together.
  - The two pin tests (`helper_geometry/tests/mod.rs:33–87`,
    `func_geo_build.rs:1690–1732`) pin the new slots.
  - Every `47 +` record-length expression uses the constant; grep for literal `47` in
    `helper_geometry.rs`/`func_geo_build.rs` and replace any found.
- **Record tail order**: edges | joins | dashes | stops. Stops stay last, so
  `__canvas_gradientStopBase` is unchanged. A new `__canvas_joinBase`/`__canvas_dashBase`
  pair is computed from the header counts, and every reader calls them (the
  2026-09-01 polygon-cache lesson in that function's comment).
- **Item block**: 272 → 288 bytes, one new `ivec4 strokeStyle` holding `joinBase`,
  `joinCount`, `dashBase` and `dashCount`. Cap stays in `arc.w`. `dashOffset` travels as
  the first entry of the item's dash region, so it needs no word.
  - This follows `.ai/canvas-threading.md:1003–1040` item by item: both shader struct
    copies plus `mfb_canvas.vert`, Metal's stack frame, the region bases, and the `.spv`
    regeneration.
  - The guard tests are `the_draw_frame_slots_do_not_overlap`,
    `the_metal_shader_region_bases_match_the_buffer_layout`,
    `the_pipeline_specializes_every_region_base` and
    `the_shared_buffer_holds_every_region`.
  - Also correct the stale `mfb_canvas.vert` comment that calls `surface.w` "unused" (it
    is the gradient kind).

## 8. Dashes (Phases 5–6)

- **Perimeter parameter at geometry build.** Each outline element carries its
  cumulative start length, and the pixel's `s` = element start + position along the
  element.
  - Polygon and PolyLine edges: a dash tail holds one cumulative length per edge. Each
    Polygon ring restarts at 0, and the ring starts are known at build time.
  - Line: `t·len`.
  - Arc: `r·(angle − startAngle)`. The angle comes from the pixel direction, clamped
    into the sweep.
  - Circle: `r·angle`.
  - Rectangle: four sides, cumulative.
  - RoundedRect: four sides and four quarter-arcs, cumulative.
  - Ellipse: a **cumulative-length table** at N angle samples, built in f64 at geometry
    build. `s` is linearly interpolated from the pixel's parametric angle. N is chosen
    in Phase 5 by measurement (UNVERIFIED, §2).
- **Coverage**:
  - `along` = the signed distance from `s` to the nearest on-interval of the pattern
    (negative inside a dash). The pattern comes from the dash region, as `(s +
    dashOffset) mod total`.
  - The dashed stroke distance is `max(band, along)`, which gives butt dash ends.
  - `Round` caps the dash ends with `min(that, distance to the dash end points − half)`.
  - `Square` extends `along` by `half`.
- **Corners**: at a corner, `s` is continuous because both edges meet at the same
  cumulative length. A dash crossing the corner is still covered by the join shapes.
  This is the premise Phase 5 tests first.

## Compatibility / Format Impact

- **Source-breaking, on purpose:**
  - `cap` is removed from `Line`, `Arc` and `PolyLine`. Migration: `cap := X` →
    `paint := WITH <paint> { cap := X }`.
  - `canvas::Paint` gains five fields. Any `canvas::Paint[...]` literal must name them
    all; there is 1 in the tree, the helper. The 106 `WITH` refinements compile
    unchanged.
  - `canvas::JoinStyle` is new, and `CapStyle.Square` is new.
- **Default change on a constructor, visible:**
  - `canvas::fill`, `canvas::stroke` and `canvas::fillStroke` now set `cap := Butt`,
    `join := Round`, `miterLimit := 4.0`, `dashes := []` and `dashOffset := 0.0`.
  - A Line built with `canvas::stroke(...)` and no `WITH { cap := … }` is Butt. Every
    migrated site states its old cap explicitly, so no in-tree picture moves.
- **Unchanged:** the fill rendering, the `DrawItem` variant set, every non-opted-in
  item's pixels, and the draw-entry layout.
- **Internal layout (no external contract):** the header grows to 50 slots and the
  item block to 288 bytes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase lands. **An
> unticked box means NOT DONE.**

### Phase 1 — Cap moves to Paint; the new Paint fields exist, inert

This phase is behavior-neutral: every header stays byte-identical, and every new field
is stored but only `cap` is read.

- [x] `mod.rs`: add the `JoinStyle` enum (mirror `CapStyle`'s descriptor) and five
      `Paint` props per §1, with man-standard descriptions. Remove the `cap` prop from
      `Line`, `Arc` and `PolyLine`, and reword their descriptions to point at
      `paint.cap`. (`CapStyle.Square` was added here too, as Phase 1's own literal and
      descriptions name it; nothing reads tag 2 until Phase 2.)
- [x] `func_fill_stroke.rs:77` literal and `helper_paint_defaults.rs`: set the §Compat
      defaults. Update the "all seven fields" sentence at `func_fill.rs:24`,
      `func_stroke.rs:24` and `func_fill_stroke.rs:24`.
- [x] `__canvas_capTag` callers (`helper_geometry.rs:553–604`, `664–727`) and native
      twins (`func_geo_build.rs:810–823`, `1155–1175`, `1231–1233`) read `paint.cap`.
- [x] `__canvas_hashPaint` (`helper_geometry.rs:1189–1250`): fold the five new
      fields. (The Line/Arc/PolyLine arms of `__canvas_hashItem` no longer fold a cap
      of their own: it is in the paint.)
- [x] `func_geo_build.rs:emit_paint_supported`: refuse (fall back to MFBASIC) any paint
      with `join ≠ Round` or non-empty `dashes`, until Phases 3 and 5 teach the native
      builder.
- [x] Migrate the 105 non-historical `cap :=` sites (§2): `cap := X` →
      `WITH <paint> { cap := X }`. This includes the 12 split Rust strings and the 4
      `CapStyle.{cap}` format sites in `rt_canvas_rasteriser.rs`, the 4 man examples in
      `src`, and `tests/runtime/inplace_self_update/field_kinds.{tsv,_gen.py}` (3
      each; regenerate the `.tsv` with its generator).
      Check: `grep -rnE "^\s*cap :=|, cap :=" --exclude-dir=target --exclude-dir=.git
      --exclude-dir=planning --exclude-dir=bugs . | grep -v "WITH"` → 0.
      (Corrections 1: 109 sites measured at the execution base. The check → 1, the
      `canvas::Paint[...]` constructor literal itself, which now names `cap`.)
- [x] Regenerate `tests/syntax/app/app-mouse-surface/golden/*`
      (`scripts/regen-native-goldens.sh`; expected, §3). Inspect the `.ir` diff: it
      should be the type surface only. (`4 build(s), 6 golden(s) rewritten, 0
      failure(s)`; the `.ir` diff is `canvas.JoinStyle`, `Square`, the Paint fields and
      the helper bodies that read them — `__canvas_hashPaint`'s new steps.)

Acceptance: the move is pixel-neutral.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_geo_native` → pass, with `git status tests/golden/canvas` clean (est. 10
  min, guess; these three binaries build every capped item in both builders).
  `cargo test --bin mfb canvas` → pass (est. 3 min, guess).
  Result: 4 + 22 + 64 passed (2 ignored), `git status tests/golden/canvas` clean;
  `cargo test --bin mfb canvas` → 110 passed. The other suites whose sources were
  migrated: `cargo test --test rt_canvas_geo_arena --test rt_arena_large_block_reuse
  --test cli_canvas_package` → 7 + 3 + 4 passed.
Commit: 3d401ef4a

### Phase 2 — Square cap (all three renderers)

- [x] Make every cap-tag reader (§2 "Cap", 13 lines, recounted first) an explicit test
      for 0/1/2, in software, the frag and Metal. Make the tag builders map `Square` → 2.
      (Recount: `grep -nE "cap *(==|!=|=|<>) *[012]|arc\.w *(==|!=) *[01]"` over the
      three renderers → 3 software, 8 frag, 8 Metal lines. Each now tests 2 explicitly
      where it decides an end's shape; the `!= 0` "has an end shape" tests stay, with
      the square arm inside them. Tag builders: `__canvas_capTag` and the native
      `emit_cap_tag`.)
- [x] Software Square distances per §5 (segment, arc end, polyline outer ends) in
      `helper_shapes.rs`/`helper_draw.rs`. Shader twins are line for line.
      (`__canvas_segmentDistanceSquare`, `__canvas_arcSquareCap`, and the polyline's
      outer ends measured to the segment's line past the endpoint; the shaders'
      `segmentDistanceSquare` (fp32 bound), and the double-float arms of
      `dfShapeDistance` and `polylineDistance`.)
- [x] Bounds padding and band reach for Square (§5), in both builders and both
      `emit_band_index`. (`__canvas_capReach` / native `cap_pad`; the emitters'
      `emit_square_cap_reach`. The GEO_VERIFY matrix gained square Line, zero-length
      Line, PolyLine and all-coincident PolyLine items → `geoVerifyMismatches=0`.)
- [x] Regenerate `mfb_canvas.frag.spv` (`scripts/regen-spirv.sh`, box 2228). (Box 2228
      down — plan-163-A Corrections 8 — so `MFB_SPIRV_PORT=2226`: `.frag.spv` 117,384 →
      132,184 bytes, `.vert.spv` byte-identical; `cargo test --bin mfb vulkan` → 9
      passed.)
- [x] Tests (hand-computed pixels, `rt_canvas_rasteriser.rs`):
      - a horizontal Square-capped line covers `half` past each end with square
        corners;
      - a 45° Square-capped line's corner pixel lies at `half·√2` along x;
      - a zero-length Square end is an axis-aligned square;
      - Square ends on an Arc and on a PolyLine.
      Add the same scenes as `rt_canvas_metal.rs` rows.
      (`a_square_cap_extends_the_stroke_square_past_each_end` → ok, including the 45°
      tip pixel at the hand-computed coverage 153 → sRGB 203. Metal row
      `square_caps_match_the_software_oracle_on_metal` over the new shared scene
      `tests/canvas/scenes/square_caps_on_black.mfb`, also gpu-rows row `squarecaps`.)

Acceptance: Square caps draw per SVG, and Butt/Round pixels are unchanged.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_metal` → pass, goldens unmodified (est. 12 min, guess). Vulkan:
  `scripts/test-canvas-vulkan.sh <the new square-cap scene>` on box 2228 → within
  `GPU_DEFAULT` (est. 10 min, guess; 2228 is the Linux Vulkan lane,
  `.ai/remote_systems.md:112–137`).
  Result: 65 + 22 + 41 passed, goldens unmodified. Vulkan on 2226 (2228 down):
  `scripts/test-canvas-gpu-rows.sh target/debug/mfb --target linux-aarch64 --rows
  squarecaps,linesblack,arcsblack,polylinesblack,strokedblack,matrix` → all ok
  (squarecaps worst=1 1.1936%; the four regression rows worst=1); box 206 `--target
  windows-x86_64 --rows squarecaps,polylinesblack` → ok (worst=1 1.1318%, 1.0231%).
Commit: 854746e15

### Phase 3 — Joins in software (the oracle)

- [x] **Added (Corrections 3): the geometry header grows 47 → 50 here**, not in Phase 4:
      the join tail needs `JOIN_COUNT` (slot 47) now. `__CANVAS_GEO_HEADER`,
      `GEO_HEADER` and `HEADER_SLOTS` → 50, slot 47 `__CANVAS_GEO_JOIN_COUNT` pinned
      against the native `SLOT_JOIN_COUNT`; slots 48–49 (dashes) are named in Phase 5.
      (`grep -rn "\b47\b"` over the canvas sources found only the three constants and
      comments, which now say 50.)
- [x] Join tail in both builders per §6, emitted only when `join ≠ Round` and the
      stroke is live. `emit_paint_supported` accepts `join ≠ Round` once the native
      builder emits it. (`helper_joins.rs`: `__canvas_joinTail` / `__canvas_joinRecord`
      for polygons (rings from `__canvas_ringStarts`), rectangles and pictures with no
      corner radius (`__canvas_rectJoins`), polylines (open path) and stroked text (its
      contours, recorded by `__canvas_textEdges` in `__CANVAS_RING_MARKS`); the bounds
      pad grows to the miter reach (`__canvas_strokePad`, `__canvas_joinReach`). Native
      twins in `func_geo_build.rs` (rect, polygon, polyline, and the record-size pass);
      `emit_paint_supported` now declines only dashes. Corrections 4 for the record
      format.)
- [x] Software stroke distance per §6 in `helper_items.rs`/`helper_draw.rs`. Items
      without a join tail take the exact current path. (`__canvas_joinedStroke` when
      slot 47 > 0; the polyline scan cuts inner segment ends butt and takes the min
      with `__canvas_joinsDistance`, keeping its colour parameter from the round walk.)
- [x] Tests (hand-computed, `rt_canvas_rasteriser.rs`):
      - a 90° Polygon corner under Miter is a sharp square outer corner, under Bevel a
        45° cut, and under Round unchanged;
      - a 20° spike with `miterLimit := 4.0` bevels (because `1/sin(10°) ≈ 5.76 > 4`),
        and with `10.0` it miters;
      - a PolyLine interior corner under each join; its end vertices get caps, not
        joins;
      - a Rectangle under Miter has sharp outer corners;
      - `MFB_CANVAS_GEO_VERIFY=1` under `--debug` reports no builder mismatch on a
        mitered polygon.
      (`a_join_shapes_every_corner_of_a_stroke` → ok, every bullet above in one scene;
      the GEO_VERIFY matrix gained Miter (limit 4, 25.5, 0.5), Bevel and a
      non-stroking Miter paint crossed with every native shape → `geoVerifyMismatches=0`
      with every item built natively.)

Acceptance: joins draw per SVG in software, and Round is byte-identical.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_geo_native` → pass, goldens unmodified (est. 10 min, guess).
  Result: 66 + 22 + 4 passed (2 ignored), goldens unmodified; `cargo test --bin mfb
  canvas` → 110 passed.
Commit: d09be9f0d (the record format was then reworked in Phase 4 — Corrections 6, 7)

### Phase 4 — Layout growth and joins on the GPU (largest blast radius)

- [x] Header 47 → 50 slots and item block 272 → 288, per §7, with every guard test
      named there passing. Fix the stale `.vert` `surface.w` comment. (The header grew
      in Phase 3 and grows once more here, to 51 — Corrections 7. Item block: the
      `strokeStyle` `ivec4` at 272 in `ITEM_BLOCK_SIZE`, both GLSL structs and Metal's
      `MfbItem`; Metal's frame and both backends' region bases move by construction.
      `cargo test --bin mfb canvas` → 110 passed (the four guards among them); glslang
      reflection of `mfb_canvas.vert` on 2226 → `topLevelArrayStride 288`, `strokeStyle`
      at offset 272.)
- [x] A join frame region and emitters (Metal `emit_*_buffer`, Vulkan `emit_*_upload`,
      mirroring the gradient-stop code). The frame-sum predicate in
      `helper_render.rs` counts joins against the edge budget. (Corrections 6: the
      records ride the EDGE region as two edge slots each, after the item's edges —
      `emit_join_buffer` / `emit_join_upload`, mirroring the edge upload, words plus
      residuals — so there is no separate region; both predicates count `2 *
      JOIN_COUNT`. Both `emit_band_index`es widen the reach to `JOIN_REACH`.)
- [x] Shader stroke path per §6 in `METAL_SHADER_BODY` and `mfb_canvas.frag`. Regenerate
      both `.spv`, since the `.vert` struct changed too. (`joinedStroke` for polygons,
      rectangles and pictures; the polyline's distance cuts inner ends butt and takes
      the records; fp32 bounds then double-float, one `dfSegment` call site per
      function. `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh` → `.vert.spv` 5,488,
      `.frag.spv` 172,588 bytes; `cargo test --bin mfb vulkan` → 9 passed.)
- [x] Tests: the Phase 3 scenes as `rt_canvas_metal.rs` rows.
      (`joins_match_the_software_oracle_on_metal` over the new shared scene
      `tests/canvas/scenes/joins_on_black.mfb` — pentagrams at both limits and beveled,
      a concave arrow, two rings, polylines with a repeated point and with colours,
      rectangles, a rotated polygon, a translucent stroke over a fill — also gpu-rows row
      `joins`. The arrow's fill is opaque: a translucent `rgba(40, 90, 200, 160)` fill
      drifts one sRGB level on Metal whatever its join (measured identical under
      `Round`), the hardware blend's rounding, and over that area it alone spent the 2%
      budget.)

Acceptance: both GPUs draw joins within tolerance, and the layout guards pass.
  Check: `cargo test --bin mfb canvas` (layout guards) → pass (est. 3 min, guess);
  `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` on the join rows (est.
  5 min, guess); `scripts/test-canvas-vulkan.sh <join scene>` on 2228 → within
  `GPU_DEFAULT` (est. 10 min, guess).
  Result: `cargo test --bin mfb canvas` → 110 passed; `cargo test --test rt_canvas_metal`
  → 42 passed (`gpuFrames≠0` asserted on the joins row); rasteriser 66, golden 22,
  geo_native 4, goldens unmodified. Vulkan on 2226: `--rows joins,squarecaps,fillrules,
  star301x2nz,polygonsblack,polylinesblack,strokedpolysblack,rectsblack,strokedblack,
  xformblack,matrix,framepass` → all ok (joins worst=2 0.6816%); box 206 `--rows
  joins,squarecaps,polygonsblack` → ok (joins worst=2 0.8590%).
Commit: a3bb3e1b4

### Phase 5 — Dashes in software (the design's riskiest premise, tested first)

- [x] Premise test first: a dashed square Polygon whose dash crosses a corner (for
      Butt, Round and Square), and a dashed Ellipse. Hand-compute the expected on/off
      pixels along the outline. If the corner dash is discontinuous, fix the `s`
      computation in §8 and record the fix in Corrections. This is a bug to fix, not a
      stop. (`a_dash_crossing_a_corner_stays_one_dash`, `a_dashed_ellipse_is_cut_by_arc_length`
      — the ellipse's expected pixels from an independent 200,000-step arc length — pass;
      the `s` computation is Corrections 8.)
- [x] A dash tail and header slots 48–49 in both builders. The cumulative-length
      element per shape kind per §8. The ellipse table's N is chosen by the measured
      max along-path error (record the number and the command here). (Slots 49 and 50 —
      Corrections 7; tail layout Corrections 9. `helper_dashes.rs` builds it; the native
      builder (`func_geo_build.rs`, Rectangle, RoundedRect, Circle, Line, Polygon,
      PolyLine) writes it bit for bit: ten dashed paints added to
      `canvas_geo_native_matrix.mfb`, `cargo test --test rt_canvas_geo_native` → 4 passed,
      0 mismatches over 10,500 native records; a deliberate mutation of the native ring
      length showed `geoVerifyMismatches=310`, so the check sees the tail. N = 256:
      `cargo test --test rt_canvas_rasteriser an_ellipse_dash_table -- --nocapture` →
      worst along-path error 0.02974 px over ellipses to 800 px across and 80:1;
      `python3 /tmp/p163/table.py` (the same construction) → 0.476 at 64, 0.119 at 128,
      0.0297 at 256 steps, so 128 misses a tenth of a pixel.)
- [x] Software dashed coverage per §8. Undashed items take the exact current path.
      (`__canvas_dashedStroke`, behind one per-item `dashed` test in
      `__canvas_drawGeometry` and `__canvas_geoDistance`; Corrections 8.)
- [x] Tests (hand-computed, `rt_canvas_rasteriser.rs`):
      - `dashes := [10.0, 5.0]` on a horizontal Line: on-pixels at x ∈ [0,10) and
        [15,25);
      - an odd-length list is doubled;
      - an empty list and a zero-sum list are solid;
      - a negative entry is solid;
      - `dashOffset` shifts the pattern, including a negative offset;
      - each Polygon ring restarts its pattern;
      - Round and Square dash ends;
      - a dashed Circle's `s = 0` is at +X.
      (`a_dashed_line_draws_its_pattern` checks every column of seven 200 px lines;
      `a_dash_end_takes_the_cap`; `each_ring_restarts_the_pattern_and_a_circle_starts_at_plus_x`;
      and `a_rounded_rect_and_an_arc_start_their_dashes_where_the_contract_says` for the
      two kinds whose perimeter is a formula.)
- [x] The GPU join copy (`emit_join_buffer` / `emit_join_upload`) found its records at
      `slot1 − stops·5 − joins·8`; the dash tail now sits between, so both subtract
      `__CANVAS_GEO_DASH_COUNT` too (`HEADER_DASH_COUNT`, pinned in
      `the_geo_layout_constants_match_their_rust_counterparts`). Found while wiring the
      tail; without it a dashed joined item would upload the wrong floats as corners.

Acceptance: dashes draw per SVG in software, and undashed pixels are unchanged.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_geo_native` → pass, goldens unmodified (est. 10 min, guess).
  Result: 73 + 22 + 4 passed (2 ignored), goldens unmodified; `cargo test --bin mfb
  canvas` → 110 passed.
Commit: 7b9e6e108

### Phase 6 — Dashes on the GPU

- [x] A dash frame region (`dashOffset` as the first entry), emitters, and a
      frame-sum cap: a new constant generated into `helper_render.rs` like the stop cap,
      with its value recorded here. (`METAL_MAX_FRAME_DASH_WORDS` =
      `VULKAN_MAX_FRAME_DASH_WORDS` = 1,048,576 words (`1 << 20`, 4 MiB), between the
      gradient and glyph regions — three words for every edge of the edge cap with room
      for patterns and ellipse tables; `@…@` tokens in `RENDER_CAPS`; an item takes its
      dash tail plus the phase (`__canvas_dashWords`). `emit_dash_buffer` /
      `emit_dash_upload` write float32 bit patterns; Vulkan's base is specialization
      constant 4. The region chains in `the_metal_shader_region_bases_match_the_buffer_layout`
      and `the_shared_buffer_holds_every_region`, and `the_pipeline_specializes_every_region_base`,
      grew by the new link as they say a new region should.)
- [x] Shader dashed coverage per §8, line for line in MSL and GLSL, with per-kind `s`.
      Regenerate `.frag.spv`. (`dashedStroke` and its helpers, the MSL generated from the
      GLSL line for line; Corrections 11. `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh` →
      `.frag.spv` 234,024 bytes, `.vert.spv` 5,488; `cargo test --bin mfb vulkan` → 9
      passed.)
- [x] Tests: the Phase 5 scenes as `rt_canvas_metal.rs` rows. (One scene holding them
      all, `tests/canvas/scenes/dashes_on_black.mfb`: corner-crossing polygons under each
      cap, Miter and Bevel dashed polygons, an odd pattern with an offset, dashed
      polylines and lines, a circle, a rotated ellipse, a rounded rectangle, an arc, a
      rectangle and a rotated polygon — `dashes_match_the_software_oracle_on_metal`, and
      row `dashes` in `scripts/test-canvas-gpu-rows.sh`.)

Acceptance: both GPUs draw dashes within tolerance.
  Check: `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` (est. 5 min,
  guess); `scripts/test-canvas-vulkan.sh <dash scene>` on 2228 and
  `scripts/test-canvas-gpu-rows.sh <dash scene> --target windows-x86_64` on box 206
  (the only real GPU) → within `GPU_DEFAULT` (est. 20 min, guess).
  Result: `cargo test --test rt_canvas_metal` → 43 passed, the new `dashes_match_the_software_oracle_on_metal` among them, after the trig fix (the
  helper asserts `gpuFrames≠0`). `bash scripts/test-canvas-gpu-rows.sh target/release/mfb
  --target linux-aarch64 --rows dashes,...` (box 2226; 2228 refused — Corrections 10) →
  `dashes: ok worst=2 differing=0.5332%`; `--target windows-x86_64` (box 206) →
  `dashes: ok worst=2 differing=0.5160%`, after Corrections 11's trig fix (before it,
  `worst=6` at (99, 357)). joins, squarecaps, ellipsesblack, arcsblack, fillrules,
  strokedblack and matrix (`geoVerifyMismatches=0`) also pass on both boxes.

### Phase 7 — Docs and spec

- [x] `06_canvas.md`:
      - rewrite the caps section (:302–326) around `Paint.cap`, `Square` and joins;
      - add dashes (the perimeter parameter per kind, the `s = 0` points of §4, and the
        ellipse table);
      - add the header growth and item-block growth where the layout is cited;
      - keep the "a stroke is never a gradient" text (:438–442).
      (The cap section is now "A stroke is styled by five `Paint` fields": caps, joins
      and their records, dashes, the tail order and slots 47–50, the `strokeStyle` words
      and the dash region; the frame-caps paragraph counts dash words and join records;
      the PolyLine paragraph its non-round joins. The one-colour-outline text is kept, now
      at :623.)
- [x] `.ai/canvas-threading.md`: record the new `strokeStyle` block word, the join/dash
      regions, and the tail order edges | joins | dashes | stops. (The item-block growth
      list, the frame caps, and a new section "A geometry record's tail is read back from
      its END", which also records the driver-trigonometry trap.)
- [x] Man prose on `Paint`, `CapStyle`, `JoinStyle`, `Line`, `Arc` and `PolyLine`
      (`.ai/man-content.md` bans apply). (Written in Phase 1; `Paint.dashes` now also
      says where each shape's pattern starts.)

Acceptance: man and spec render and cite.
  Check: `mfb man canvas types` shows the new fields and enums;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min, guess).
  Result: `mfb man canvas types` lists `join`, `miterLimit`, `dashes`, `dashOffset`,
  `CapStyle.Square` and `JoinStyle`; `man-census.sh --memory-scope` → 0 unclassified;
  `man-run-examples.sh canvas --run` → 32 examples built and ran, 0 failed; `spec-census.sh --citations` → 0
  missing paths, lines and symbols, `--links` → 0 unresolved; `cargo test --bin mfb
  spec` → 46 passed; `mfb spec app canvas` shows no `[[`.
Commit: 686758d3d

## Validation Plan

- Tests: per phase above, all with hand-computed software pixels (the oracle is not
  checked against itself). Negative cases: the empty, zero-sum, negative and odd-length
  dash lists; a miter limit below 1; a zero-length Square end.
- Coverage check: software paths run in `rt_canvas_rasteriser`/`rt_canvas_golden`. The
  shader paths run only on GPU lanes (`rt_canvas_metal` locally, which skips unless
  `metalReady`, so confirm `gpuFrames≠0`), plus the Vulkan scripts on 2228 and 206.
  The default `cargo test` covers none of the Vulkan shader.
- Runtime proof: an `--app` program drawing an SVG-style icon set, on macOS with
  `MFB_CANVAS_GPU=1`:
  - a mitered star;
  - a beveled polyline;
  - a dashed rounded rect with round dash caps;
  - a dashed ellipse.
  Screenshot it.
- Doc sync: Phase 7.
- Final gate (run ONCE, after Phase 7): `cargo test --no-fail-fast` (est. 40 min, from
  plan-159-D's recorded run) and `bash scripts/artifact-gate.sh target/release/mfb all`
  → 0 diffs outside `tests/syntax/app/app-mouse-surface` (est. 20 min).

## Open Decisions

- **`miterLimit` zero value**: 0.0 → "always bevel", with the constructors setting 4.0
  (recommended). The alternative, 0.0 → "use 4.0", hides the value it acts on, which
  canvas avoids (the zero `Transform` is identity only because the degenerate
  alternative erases the drawing).
- **Rectangle `s = 0` and direction**: top-left, clockwise (recommended, matching SVG's
  `<rect>` path) vs. matching the item's own coordinate origin.
- **Ellipse arc-length method**: a precomputed cumulative table (recommended; exact
  inputs to every backend) vs. a closed-form approximation evaluated per pixel
  (Ramanujan-style), which would be cheaper to store but approximate in a
  backend-dependent way.

## Corrections

1. **`cap :=` population.** Measured at the execution base (`f858ff14b` + A):
   `grep -rnE "cap :=" --exclude-dir=target --exclude-dir=planning --exclude-dir=bugs .`
   → 109 construction sites (the plan's 105 was measured at `8cbd0206a`; tests added
   since account for the rest). 98 were rewritten by script (`cap := X, …, paint := P`
   → `paint := WITH P { cap := X }`, merged into an existing `WITH` block where `P`
   was one), 2 in `func_stroke.rs` likewise, the six `format!` sites by hand (braces
   doubled), and the three `field_kinds.tsv` rows to match the generator's new setup
   column. In the two allocation-measuring tests (`rt_canvas_geo_arena.rs`,
   `rt_arena_large_block_reuse.rs`) the cap went onto the paint the loop already
   builds, so no `WITH` was added per item.
2. **Box 2228 is down for this plan's run** (plan-163-A Corrections 8): every Linux
   Vulkan row and SPIR-V regeneration runs on 2226 (Debian aarch64 lavapipe), and the
   Vulkan commands are `scripts/test-canvas-gpu-rows.sh <mfb> --rows <row>` over a
   shared scene file, not `scripts/test-canvas-vulkan.sh <scene>`.
3. **The header growth moved from Phase 4 to Phase 3.** §6 puts the join count in a new
   header slot, and §7 grew the header only in Phase 4 — after the software joins that
   need it. The geometry header (47 → 50) lands with Phase 3; the item block (272 → 288)
   stays in Phase 4, which is the first phase that needs a GPU word.
4. **Every corner has a record.** §6 has a join emit no record when there is no wedge.
   The native builder sizes a record before it builds it ("allocates nothing and calls
   nothing"), and whether two edges are parallel is a sqrt-and-divide computation the
   size pass would have to repeat bit for bit. So a corner between two live edges always
   gets a 9-float record, with `n = 0` (nine zeros) where `cross = 0`; the count is the
   live-edge count (closed ring: that count when ≥ 2; open path: one fewer), which the
   size pass already computes. A 0-point record draws nothing in every renderer.
5. **Stroked text's rings.** A stroked run's edge tail is every glyph contour
   concatenated, with no ring boundary in it. `__canvas_textEdges` now records each
   contour's first edge in `__CANVAS_RING_MARKS` (graphics-thread state, set only while
   it walks, so the glyph-bitmap walks that share the contour readers never grow it),
   and the Text arm builds the joins from it.
6. **One join record per edge, in the edge region.** Phase 3's records (one per live
   corner, in walk order) leave a GPU no way to find a pixel's corners but to walk every
   record per pixel — 1,000 per pixel for a 1,000-vertex mitered SVG path. Records are
   now one per outline edge (the corner at its start; the no-piece record where there is
   none), so a polygon's or polyline's band index reaches them through the edges it
   already lists, and the native size pass needs only the edge count (superseding
   Corrections 4). They travel as two edge slots each after the item's edges, the
   frame-sum edge cap counting `2 * JOIN_COUNT`, rather than in a new region: the
   precision scheme (16.16 words plus residuals) is the edges' own, and `GPU_DEFAULT`'s
   channel delta of 2 needs it.
7. **The pieces overlap; the header is 51 slots.** §6's union of BUTT bands and join
   polygons draws a seam: where two pieces merely abut (a band's end and the polygon's
   side, or two collinear bands), both distances are 0 on the shared line, so a pixel
   centred on it draws half-covered — measured on the first GPU joins scene as a column of
   half-coverage pixels down the top-right corner of a mitered rectangle, in the oracle as
   much as on Metal (they agreed, so the comparison could not see it). Each corner's
   piece is now built on V's disc of radius `half` (the module comment of
   `helper_joins.rs`): straight on, the disc; Bevel, the disc cut by the chord A-B;
   Miter, the disc and the polygon V, A, M, B; a U-turn, nothing. Every shared line then
   lies inside another piece with the depth the stroke has there. The record stays 4
   points (`V, A, P, B`), its kind read from exact equalities. Header slot 48
   `__CANVAS_GEO_JOIN_REACH` (`half * miterLimit` for a Miter, else `half`) tells the
   band index how far a record reaches; the dash slots become 49 and 50.
8. **A dashed outline of edges is cut edge by edge, not at its nearest point (§8).**
   §8's pixel `s` from the NEAREST outline point is discontinuous on a corner's inner
   side: a pixel there is nearest the second edge, while the first edge's band — still
   inside its dash — is what covers it in SVG, so a dash ending exactly at a vertex would
   lose the inner half of its end. Each edge is instead cut by its own position, `u =
   s0 + (p − a)·d/|d|` (unclamped), against the dashes clipped to that edge's `[s0,
   s1]`: `max(|perp| − half, along)`, the min over edges, with the corner piece at every
   vertex a dash runs THROUGH (on just before and at it). That is the SVG construction
   (each dash a sub-path with butt ends, joins and caps), and so:
   - a dashed `Round` paint carries join records too — discs (`__canvas_joinRecord`'s
     `round` flag, both builders), since the round corner the distance field draws for
     free must vanish where the pattern is off;
   - a dash end is a TRUE end (capped) only where the dash stops; where it runs on past
     an edge's end the edge is cut butt and the corner piece joins it;
   - the closed seam is resolved per boundary (`__canvas_dashLocate`): on at both sides
     is one dash, on at one side ends there; an open path's ends always end a dash.
   Curves (Circle, Arc, Ellipse, RoundedRect) have no corner, so they keep §8's
   nearest-point `s`, with `along` in outline length there. `__canvas_atan2` (a
   `+ − × ÷ sqrt` series, error under 1e-16) gives a circle's, arc's and corner arc's
   angle; an ellipse's angle is its distance bisection unfolded.
9. **The dash tail's layout and the cap slot.** The tail is `[n, period, c1..cn]` (each
   entry's END along the period, so the per-pixel walk is compares only) and then the
   kind's perimeter data: three floats per edge (`s0`, `s1`, ring length, negative for
   an open path) for a polygon, polyline, line, rectangle, picture or stroked text; the
   N + 1 = 257-entry arc-length table for an ellipse; nothing for a circle, arc or
   rounded rectangle, whose perimeter is a formula of the header. §7's "`dashOffset`
   as the first entry" became slot 50, the offset wrapped into one period at build. A
   dashed item writes its cap into slot 34 on every kind (only Line/Arc/PolyLine did),
   since every dash end takes it; an undashed one's header is unchanged. A dashed
   Square paint pads its bounds `half·√2` (`__canvas_dashCapReach`), and
   `__canvas_strokePad` keeps its Miter arm's fused `half * limit + 1.0` unless that
   reach is the longer, so every undashed header stays bit-identical.

10. **Box 2228 is down; the Linux Vulkan rows ran on 2226.** `bash
    scripts/test-canvas-gpu-rows.sh target/release/mfb --rows …` →
    `ssh: connect to host 127.0.0.1 port 2228: Connection refused`, in Phase 4 and again
    here. The same script's `--target linux-aarch64` (box 2226, lavapipe) ran the rows
    instead, and its `dashes` row stands for §Phase 6's `test-canvas-vulkan.sh <dash
    scene>`: the same program, GPU against software, under the same `GPU_DEFAULT`.
11. **The GPU dashes are fp32 at outline-relative offsets, with the oracle's own
    trigonometry.** Each shader reads the pixel's offset from the item's own outline
    (an edge's start, a curve's centre) in double-float and evaluates the dash
    functions in fp32 from there, which holds a thousand-pixel perimeter to 6e-5 px — no
    two-pass double-float, since a dash only moves an end along the outline. The first
    cut used the driver's `atan`, `sin` and `cos`, and box 206's Intel driver drew
    `dashes` at `worst=6` (a round dash end's disc on the circle, (99, 357)) while
    lavapipe and Metal passed: Vulkan allows `atan` 4096 ULP and `sin`/`cos` 2^-11
    absolute. `dashAtan`, `dashSin`, `dashCos` are the oracle's series; 206 then measured
    `worst=2`.
12. **An arc's dash tail carries its start angle and sweep, and a closed outline with no
    length draws no dash dot.** The GPU item block has an arc's sweep END POINTS, not its
    angles, so `__canvas_arcDashes` stores `[startAngle, sweep]` as the arc's perimeter
    data and the software reads them back too. The software drew the zero-length dot for
    any dashed outline of edges with no live edge; a GPU sees only a band of edges, so
    the dot is now an open path's (a Line's or PolyLine's, decided by its length) and a
    closed one with no length draws nothing dashed, on all three.

## Summary

The engineering risk sits in three places:

- the per-pixel perimeter parameter for dashes, especially at corners and on ellipses,
  which Phase 5 tests first in software;
- the single item-block and header growth, which Phase 4 runs behind the four named
  guards;
- keeping two builders and two shader twins in lockstep across three new tails.

Untouched: fill rendering, fill-only text, gradients (strokes stay flat), the
`DrawItem` variant set, and every item that does not opt into a non-default stroke
style.
