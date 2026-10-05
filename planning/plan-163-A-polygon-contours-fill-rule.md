# plan-163-A: `canvas::Polygon` takes contours and a fill rule

Last updated: 2026-09-27
Overall Effort: UNMEASURED — plan-163 is the canvas work toward drawing SVG content,
and later letters are not scoped yet. Re-estimate each time a letter is added.
Effort: large (3h–1d)
Depends on: nothing

plan-163 covers the canvas changes needed to draw SVG content. The letters are listed in
implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | `Polygon.points` → `Polygon.contours` (several closed rings in one item) and a new `Polygon.rule AS canvas::FillRule` (`EvenOdd` / `NonZero`), in the software oracle, Metal and Vulkan | large |
| B | Stroke styling on `canvas::Paint`: `cap` moves off Line/Arc/PolyLine, `Square` cap, `JoinStyle` + `miterLimit`, `dashes` + `dashOffset` (`plan-163-B-stroke-styling.md`) | x-large |
| C | Path clipping (`Paint.clip` as rings + `clipRule`, following the transform and group offset; `canvas::Bounds` removed), `Paint.transparency` (0 = opaque), picture tint = fill colour with alpha as tint strength (`plan-163-C-clip-path-transparency.md`) | large |
| D | Gradients: `spread` (Pad/Reflect/Repeat), `transform`, radial focal point, `strokeApply` (gradient paints the stroke, tinted by the stroke colour); the ramp moves into item space; `fillGradient` → `gradient` (`plan-163-D-gradient-spread-transform-focal.md`) | large |

Today a `canvas::Polygon` is one closed ring of points. That means it cannot draw a
shape with a hole (an "O", a donut) or a shape whose outline is made of several
subpaths, which is what an SVG `<path>` with more than one `M` is. The rasteriser
already handles several rings in one item: stroked text is emitted as one
`__CANVAS_GEO_POLYGON` whose edge tail is every glyph contour, one after another
(`helper_geometry.rs:__canvas_textEdges`). What is missing is the public surface and
the choice of fill rule. SVG and TrueType both default to non-zero, while every
renderer here counts even-odd crossings.

**The single behavioral outcome:** a `canvas::Polygon[contours := [outer, inner],
rule := …, paint := …]` fills the region its rings enclose under the chosen rule. Under
`EvenOdd`, a ring inside another is a hole whatever its direction. Under `NonZero`, an
inner ring wound the same way as the outer ring is filled, and one wound the opposite
way is a hole. All three renderers draw the same pixels (the software renderer
exactly, the GPUs within `Tolerance::GPU_DEFAULT`). A single-ring `EvenOdd` polygon
produces **the same geometry header and edge tail it produces at HEAD**, so every
existing polygon picture is unchanged.

References:

- `src/docs/spec/app/06_canvas.md` — the rendering conventions (§"Rendering
  conventions", lines 105–565: GPU edge caps, the band index, the double-float
  even-odd test at :179 and :193–197, the `Polygon` corners sentence at :324–326). The
  point-list sentence is at :52.
- `.ai/canvas-threading.md` §14 "The geometry cache and item hashes" (:906). A cache hit
  is trusted on the hash alone, so the rule and the ring structure must be hashed. See
  also :1003 "Growing `ITEM_BLOCK_SIZE` moves six things" (this plan does **not** grow
  the block) and :1053 "A layout constant shared by MFBASIC source and the emitter has
  no compiler between them".
- `.ai/testing-gates.md` :235 "Canvas reference images: exact for software, tolerance
  for GPU", :309 "The GPU comparison is against the ORACLE, not a stored picture"
  (agreement with the oracle cannot catch an oracle bug, so the rasteriser needs its own
  hand-computed pixel checks), :13 and :25 (`.ncodesum` / broadly-emitted goldens).
- `.ai/remote_systems.md` :112–137 (which box renders Vulkan). `scripts/README.md`
  :76–78 (`regen-spirv.sh`).
- `.ai/specifications.md`, `.ai/spec-content.md` (spec sync), `.ai/man-content.md`
  (registry descriptor prose).
- `src/codegen/builtins/canvas/helper_glyph.rs:12–18`: the documented even-odd
  limitation for text. **Out of scope here** (see Non-goals).

## Prerequisites

None. plan-163-A depends on no other plan or bug.

| Must be true | Command | Status |
|---|---|---|
| No other plan-163 file claims the number | `ls planning/plan-163-* planning/completed/plan-163-*` → only plan-163 letters | MET (2026-10-05: A, B, C, D — all letters of this plan, no foreign plan-163; see Corrections 1) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every command before you continue, and again before you decide to stop.

## 1. Goal

- `canvas::Polygon` is `{ contours AS List OF List OF canvas::Point, rule AS
  canvas::FillRule, paint AS canvas::Paint }`. The `points` field no longer exists.
- `canvas::FillRule` is a new exported enum: `EvenOdd` (ordinal 0) and `NonZero`.
- Each ring closes itself (last point → first point of **that** ring). All rings'
  edges form one edge list, filled under `rule` and stroked along every ring.
- The software renderer, Metal and Vulkan implement both rules. Metal is checked by
  `rt_canvas_metal` and Vulkan by `scripts/test-canvas-vulkan.sh`.
- A single-ring `EvenOdd` polygon produces the same geometry as HEAD, byte for byte
  (§3, "Correctness gate").

### Non-goals (explicit constraints)

- **Text keeps even-odd.** `__canvas_edgeDistanceIn` (the glyph-bitmap rasteriser,
  `helper_glyph_cache.rs`) is not touched. Stroked text, which is also kind
  `__CANVAS_GEO_POLYGON`, keeps rule 0 = EvenOdd because its header's rule slot stays
  0. Moving text to NonZero changes text pixels and interacts with plan-162 (text), so
  it is a later letter.
- **No growth of the GPU item block.** `ITEM_BLOCK_SIZE` stays 272 bytes. The rule
  rides in a word polygons already leave unread (§6).
- **No new `DrawItem` variant.** The union's variant set and order are frozen by
  `mod.rs` (test at :1482–1509). `Polygon` keeps tag 3.
- **No change to any other item's geometry, hash or pixels.**
- **No stroke joins, caps or dashes on polygons.** A stroked ring's corners stay
  whatever the distance field produces (spec :324). Stroke styling is a later letter.
- **No compatibility shim.** `points` is removed, not deprecated, and every caller in
  the tree is migrated in this letter.

## 2. Current State

All paths are under `src/codegen/builtins/canvas/` unless written in full. Line
numbers are at HEAD `8cbd0206a`.

**The public type.** `mod.rs:943–957` declares `Polygon { points AS List OF Point,
paint }`. `mod.rs:1096–1155` is the `DrawItem` union, with Polygon the 4th variant (tag
3). Union tags and enum ordinals are declaration order, and native code looks them up
through the type model (`func_geo_build.rs:variant`, `:enum_ordinal`,
`func_carried_hashes.rs:variant_tag`). Nothing hardcodes Polygon's tag or field offsets
(`grep -rn "compare_immediate(&tag" src/codegen/builtins/canvas` → 3 hits, each
comparing against a looked-up tag). `CapStyle` (`mod.rs:635–650`) is the enum
descriptor to mirror.

**Named construction requires every field** (`06_canvas.md:85`, `:321`). So a zero
ordinal is a tag number, not a default: every `canvas::Polygon[...]` must name
`rule`. That costs nothing extra here, because every site is edited for `contours`
anyway.

**Publishing (deep copy).** `gen_present.rs:emit_publish` and
`gen_group.rs` (setGroup) call `copy_flat_block` on the whole `List OF DrawItem` or
`List OF DrawLayer`. There is no per-variant code (`gen_present.rs:77–81`). The
frame-skip compare and `func_carried_hashes.rs` are raw byte compares and do not
depend on the type.

**Item hash.** The native `func_item_hash.rs:emit_item_hash` folds `Polygon`
structurally: `fold_record` → `fold_field` (an enum field folds as a u64, so `rule` is
picked up automatically) → `fold_list`. `fold_list` (:168–213) accepts only
fixed-width or record elements, and returns `"a list of '…' is neither fixed-width nor
a record"` for anything else. **A `List OF List OF Point` fails there.** The MFBASIC
fallback `helper_geometry.rs:__canvas_hashItem` has a Polygon arm (:1296–1302) that
iterates `p.points`. It runs only when the native hash returns -1 (Text, Group), but it
must compile.

**Geometry build (two builders that must agree).**

- MFBASIC: `helper_geometry.rs:__canvas_polygonHeader` (:606–632) returns an empty
  header for fewer than 2 points. Otherwise it writes bounds from min/max over the
  points into slots 16–19, slot 1 = `47 + count*5`, and slot 20 (`AUX0`) = `count`.
  `__canvas_polygonEdges` (:738–761) emits edge `i` = `points[i]` →
  `points[(i+1) MOD count]` as `x0, y0, dx, dy, invLenSq`, with `invLenSq = 0` for a
  zero-length edge.
  - The callers are `__canvas_geometryFor` (:1082–1086) and the debug
    `__canvas_geoReference` (:1407–1411).
  - Edges keep the direction they were authored in (signed `dx`, `dy`, never
    normalised), so winding is available to a NonZero count.
- Native: `func_geo_build.rs:fill_polygon` (:850–996) reads `"points"` at :860 and
  writes the wrap edge at :946–950. `emit_record_size` has a Polygon arm (:1431–1446),
  and `emit_fill_record` dispatches it at :1503. With `--debug` and
  `MFB_CANVAS_GEO_VERIFY=1`, the native build is compared against
  `__canvas_geoReference` (`geoSame`).
- Header layout: `helper_geometry.rs:GEO_LAYOUT` (:52–95), `__CANVAS_GEO_HEADER = 47`,
  `__CANVAS_GEO_POLYGON = 4`. The Rust mirrors are at
  `src/codegen/runtime/canvas/mod.rs:802–960`. Each slot constant is pinned against its
  mirror by `helper_geometry/tests/mod.rs:33–87` and `func_geo_build.rs:1690–1732`.
  Aliasing a slot per kind has a precedent: `__CANVAS_GEO_POLYLINE_FIRST/LAST` = 43/44
  reuse the gradient slots.

**Multi-ring precedent.** `helper_geometry.rs:__canvas_textEdges` (:921–939) joins every
glyph contour into one flat tail. `__canvas_textHeader` (:941–988) writes kind
POLYGON, slot 1 = `47 + tailLen`, slot 20 = `tailLen/5`. The tail has no ring
separators and the header has no ring count, because each ring closes itself.

**Software fill test.** `helper_draw.rs:__canvas_edgeDistance` (:136–166) is called
only from the fallthrough of `__canvas_geoDistance` (:117), which is called 6 times
from `helper_items.rs:__canvas_drawGeometry`. The header is reachable as
`tail - __CANVAS_GEO_HEADER`, as `__canvas_polylineScan` already does
(`helper_draw.rs:197–199`). The crossing test is
`IF (ay > py) <> (by > py) THEN … IF px < ax + u * dx THEN inside = NOT inside`.

**GPU fill test** (the same shape on both backends):

- Metal is in `src/target/macos_aarch64/app/metal.rs:METAL_SHADER_BODY`.
  - `edgeToggles32` (:391–395) returns `b.y > a.y ? cross < 0.0 : cross > 0.0`, and
    `dfToggles` (:439–444) is its double-float twin.
  - `edgeDistance` (:445–486) holds `bool inside` (:450), which it flips in pass 1
    (:461–462, fp32, sure edges) and pass 2 (:483, double-float, unsure edges only).
    The return is at :485.
  - It is dispatched from `sampleDistance` at :761–768 (`kind == 4`).
- Vulkan is in `src/codegen/runtime/canvas/shaders/mfb_canvas.frag`: `edgeToggles32`
  :276–280, `dfToggles` :324–329, `edgeDistance` :330–371 (flips at :347 and :368),
  and the `kind == 4` dispatch at :632.
- The SPIR-V (`mfb_canvas.{vert,frag}.spv`) is **checked in** and loaded with
  `include_bytes!` (`vulkan.rs:213–214`). It is regenerated by hand with
  `scripts/regen-spirv.sh`, which defaults to glslang on box 2228.
  `the_embedded_spirv_is_well_formed` checks only the header, so **a stale `.spv`
  would not be caught by any test.**

**Item block.** Each drawn item is a 272-byte `ItemBlock`/`MfbItem`, laid out
identically in both shaders (`mfb_canvas.frag:18–39`). `arc` is `ivec4(glyph/picture
height, arc reflex flag, edgeBase, capStyle)`.

- `arc.y` (`ITEM_ARC_REFLEX`, `runtime/canvas/mod.rs:580`) is written for every kind
  by `runtime/canvas/mod.rs:emit_item_arc_flags` (:729–765).
- It is read only by the arc sweep test (`grep -nE "arc\.y" mfb_canvas.frag metal.rs`
  → `mfb_canvas.frag:477`, `metal.rs:595`, both `inSweep`; the other hits are
  comments).
- A polygon therefore leaves `arc.y` unread.
- The polygon emitters are:
  - Metal: `emit_item_block` (:3636–3847), `emit_edge_buffer` (:4778), band index
    (:5052).
  - Vulkan: `emit_item_block` (:3505–3717), `emit_edge_upload` (:3929), band index
    (:4233).

**Header slot 21 (`AUX1`) on a polygon.** The polygon builders never write it (it stays
0). The emitters read `HEADER_AUX1` only for a Picture (`metal.rs:4493`,
`vulkan.rs:4724`, both under a `GEO_KIND_PICTURE` guard) and in the arc reflex
computation. The reflex word is exactly the word this plan overrides for polygons.

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::Polygon[` construction sites, whole tree | 65 | `grep -rE "canvas::Polygon\[" --exclude-dir=target --exclude-dir=.git . \| wc -l` |
| of which `tests/canvas/scenes/*.mfb` | 32 in 15 files | same grep, `\| cut -d: -f1 \| sort \| uniq -c` |
| of which Rust tests (`rt_canvas_rasteriser` 6, `rt_canvas_metal` 5, `rt_canvas_golden` 3, `cli_canvas_package` 1, `rt_canvas_present_deep_copy` 1) | 16 | same |
| of which scripts/tools (`scripts/test-canvas-vulkan.sh` 3, `scripts/test-macapp.sh` 1, `tools/canvas-bench/gen/{worker,stress,poly}.sh` 3, `tests/runtime/inplace_self_update/field_kinds.tsv` 1, `field_kinds_gen.py` 1) | 9 | same |
| of which examples (`examples/water/src/shapes.mfb` 2, `examples/weather/src/{timeline,main,land}.mfb` 3, `examples/swat/canvas/src/main.mfb` 1) | 6 | same |
| of which planning/bug docs (historical — **not migrated**) | 6 | same |
| of which `src/` | 0 | `grep -rn "canvas::Polygon\[" src/ \| wc -l` |
| Polygon `p.points` reads in MFBASIC helpers | 9 (all `helper_geometry.rs`: 607, 612, 619, 1083, 1084, 1298, 1299, 1408, 1409) | `grep -rn "p\.points" src/codegen/builtins/canvas` → 16, minus 7 PolyLine |
| Polygon `"points"` reads in Rust | 3 (`mod.rs:950`, `func_geo_build.rs:860`, `:1433`) | `grep -rn "\"points\"" src/codegen/builtins/canvas` → 5, minus 2 PolyLine |
| `CASE Polygon` arms | 3 (`helper_geometry.rs` 1082, 1296, 1407) | `grep -rn "CASE Polygon" src/codegen/builtins/canvas` |
| Even-odd flip sites | 4 files: `helper_draw.rs`, `helper_glyph_cache.rs` (not touched, Non-goals), `metal.rs`, `mfb_canvas.frag` | `grep -rlnE "inside = !inside\|inside = NOT inside" src` |
| `.ir`/`.nir`/`.nplan` goldens naming `canvas.Polygon` | 7 / 7 / 5 in `tests/syntax/app/app-mouse-surface/golden/` | `grep -c "canvas.Polygon" tests/syntax/app/app-mouse-surface/golden/*` |

### Verified properties

- **Nothing hardcodes Polygon's layout or tag.** Verified by the `compare_immediate(&tag`
  grep above and by reading `func_geo_build.rs:field`, `:scalar_offset` and
  `:emit_field_block` (offsets come from the type model).
- **Deep copy handles a nested list with no new code.** Read
  `gen_present.rs:77–81` and `builder_collection_layout.rs:record_field_is_inlined`
  (:3121–3130). A memcpy-copyable collection is inlined into the flat block. A
  three-deep precedent already ships: `List OF DrawLayer` → `items` → `Polygon.points`.
  The first `List OF List` **field** is still new (`grep -rn "List OF List"
  src/codegen/builtins/canvas` → 0), so Phase 1 tests it directly rather than assuming
  it.
- **`arc.y` is unread for kind 4.** Verified by the grep in §2 "Item block".
- **Header slot 21 is unwritten and unread for kind 4.** Verified by reading
  `__canvas_polygonHeader`, `fill_polygon`, `__canvas_textHeader`, and the two
  `HEADER_AUX1` emitter reads (Picture-guarded).
- **Band pruning is safe under a winding count.** `polygonBand`
  (`metal.rs:499–506`, `mfb_canvas.frag:394–401`) skips edges whose y-span misses the
  pixel row. Such an edge cannot satisfy `(ay > py) <> (by > py)`, so it adds 0 to a
  count, just as it adds no flip today. A sum does not depend on edge order.
- UNVERIFIED: whether any test compares `__canvas_hashItem` against the native
  `emit_item_hash` for a Polygon. It gets a task in Phase 1.
- UNVERIFIED: the Vulkan scripts' current pass state on box 2228. Phase 3 runs a
  baseline first.

## 3. Design Overview

The work comes in three pieces, layered in this order:

1. **Surface and geometry (program side).** Add the new record shape and enum. Both
   builders emit one concatenated edge tail from every ring plus a rule slot in the
   header. Hashing covers the rings and the rule. The software renderer ignores the
   rule for now.
2. **Software NonZero.** `__canvas_edgeDistance` reads the rule slot and counts
   windings instead of flipping. The software renderer is the oracle that the GPUs are
   compared against, so it is proven by **hand-computed pixels**, not by agreement with
   anything.
3. **GPU NonZero.** Both emitters copy the rule into `arc.y` for kind 4. Both shaders
   count windings when `arc.y != 0`. The SPIR-V is regenerated.

**Correctness risk concentrates in piece 1's two builders.** The native `fill_polygon`
and the MFBASIC `__canvas_polygonEdges` must stay bit-identical, and the ring-wrap
index is new arithmetic in hand-written native code. `MFB_CANVAS_GEO_VERIFY=1` compares
them. **Design uncertainty concentrates in the nested-list plumbing**: the first
`List OF List` field in a `DrawItem`, through `fold_list`, `copy_flat_block` and the
self-update matrix. So Phase 1 builds and publishes a two-ring polygon before anything
else is changed.

**Correctness gate: behavioral, with one byte-identity sub-gate.** This plan changes
behavior, so the gate is tests and pixels. One sub-gate is byte-identity: the software
golden PNGs (`tests/golden/canvas/transforms.png`, `gradients.png`) and every existing
hand-computed polygon pixel test must stay **exact**, because a single-ring `EvenOdd`
polygon builds the same header and tail as HEAD. If that fails, a builder is wrong:
dump ONE scene's `MFB_CANVAS_DUMP` header against HEAD's, then fix the builder. It is
never a reason to re-baseline.

**Expected to change** (a diff here is the plan working, not failing):

- `tests/syntax/app/app-mouse-surface/golden/*.ir/.nir/.nplan/.ncodesum`: the type
  surface and the canvas runtime changed.
- The item-hash *values*: the fold now covers an outer count and `rule`. Hashes are
  in-process only, and no golden stores them. `cache_hit_skips_geometry_generation`
  (`rt_canvas_rasteriser.rs:445`) counts generations, not hash values, and must stay
  `generations=3,4,4`.

**Rejected alternatives** (do not re-litigate):

- *Keep `points` and add `contours` for extra rings.* Rejected by the user as hacky: it
  gives the first ring special status that the fill has no concept of.
- *A new `Path` variant.* It would duplicate every Polygon behavior (fill, stroke,
  transform, clip, cache) under a second name. It would also append a 12th variant to a
  union frozen by test.
- *`NonZero` as the zero ordinal to match SVG.* The zero value is only a tag number
  (named construction names every field), but canvas convention makes the zero ordinal
  the historical behavior (`CapStyle.Butt`, `GradientKind.Linear`). EvenOdd is what
  every renderer does at HEAD.
- *A ring-separator or ring-count in the tail or header.* Not needed: each ring closes
  itself, and the crossing count sums across rings. The text precedent already relies
  on this.
- *Carrying the rule in the `CAP` slot (34) / `arc.w`.* It would need no emitter
  change, but a later letter that adds caps and dashes to polygon strokes would collide
  with it. Slot 21 → `arc.y` is unused by polygons and has no such future.
- *Growing the item block.* `.ai/canvas-threading.md:1003` lists six things that move
  when it grows, three of them silently. A free word avoids all six.

## 4. Record, enum and ring semantics

```
canvas::FillRule   (enum, exported)
  EvenOdd   — ordinal 0. A point is inside when a ray from it crosses the rings an odd
              number of times. A ring inside another is a hole whatever its direction.
  NonZero   — A point is inside when the rings wind around it a non-zero number of
              times: +1 for each crossing of an edge running one way, -1 the other way.
              A ring inside another is a hole only if it runs the opposite way.

canvas::Polygon
  contours AS List OF List OF canvas::Point
  rule     AS canvas::FillRule
  paint    AS canvas::Paint
```

- A ring's last point joins back to that ring's own first point.
- A ring with fewer than 2 points contributes no edges and no bounds. With 0 or 1
  points it has no outline or area to add. The same limit applies at HEAD to the whole
  point list, so a single-ring polygon draws what it does today.
- A 2-point ring contributes its two coincident edges and so draws only its stroke, as
  a 2-point `points` does at HEAD.
- If no ring contributes an edge, the item's header is the empty header and it draws
  nothing (HEAD: fewer than 2 points).
- The stroke outlines every contributing ring. This comes free: the distance is a min
  over all edges.
- The winding sign convention (an edge with `by > ay` counts +1 when it crosses left of
  the pixel, otherwise −1) is internal. Only non-zero-ness is observable, but the
  convention must be **the same in all three renderers**, and each shader comment names
  it.

## 5. Geometry: header and tail

- **New slot constant** `__CANVAS_GEO_POLYGON_RULE = 21` in `GEO_LAYOUT`. It aliases
  `AUX1`, following the `POLYLINE_FIRST`/`LAST` aliasing precedent. Its Rust mirror is
  `HEADER_POLYGON_RULE` in `src/codegen/runtime/canvas/mod.rs`, next to
  `HEADER_AUX1`. The value is the `FillRule` ordinal as a Float, 0.0 or 1.0.
- **Header** (`__canvas_polygonHeader(rings, rule, paint)`):
  - bounds = min/max over the points of every contributing ring;
  - slot 20 = total edges = the sum of the contributing rings' point counts;
  - slot 1 = `47 + 5 × total edges`;
  - slot 21 = rule.
  - Everything else is as at HEAD.
- **Tail** (`__canvas_polygonEdges(rings)`): for each contributing ring in order, edges
  `ring[i] → ring[(i+1) MOD len(ring)]`, the arithmetic of today's per-point loop
  unchanged.
- For one ring and `EvenOdd` this is exactly HEAD's header and tail. Slot 21 is 0 at
  HEAD, and it is 0 for EvenOdd.
- `__canvas_textHeader` does not change. Its slot 21 stays 0 = EvenOdd.

## 6. Renderers

- **Software** (`helper_draw.rs:__canvas_edgeDistance`): read
  `__CANVAS_GEO_DATA[tail - __CANVAS_GEO_HEADER + __CANVAS_GEO_POLYGON_RULE]` once
  (the precedent is `__canvas_polylineScan`). Under EvenOdd keep the exact current
  statement sequence (`inside = NOT inside`), so EvenOdd output cannot move. Under
  NonZero, on the same crossing condition, do
  `winding = winding + (IF by > ay THEN 1 ELSE -1)` (written as whatever MFBASIC
  conditional form the file already uses), and treat the pixel as inside when
  `winding <> 0`.
- **Emitters**: a new shared helper `emit_item_polygon_rule(header, block_at, …)` in
  `src/codegen/runtime/canvas/mod.rs`, next to `emit_item_arc_flags`. When
  `HEADER_KIND == GEO_KIND_POLYGON`, it stores the rule slot (converted to an int) into
  `ITEM_OFFSET_ARC + ITEM_ARC_REFLEX`. It is called **after** `emit_item_arc_flags` in
  both `metal.rs:emit_item_block` and `vulkan.rs:emit_item_block`. For every other kind
  it stores nothing.
- **Shaders** (MSL in `METAL_SHADER_BODY`, GLSL in `mfb_canvas.frag`, line for line):
  - `edgeDistance` takes an `int nonZero` argument (`item.arc.y` at the `kind == 4`
    dispatch). `bool inside` becomes `int winding`.
  - The fp32 pass and the double-float pass each add `+1`/`−1` from a new signed
    variant of `edgeToggles32`/`dfToggles`. It is multiplied out with no division, like
    today: it returns 0 when the edge does not cross left of the pixel, otherwise
    `b.y > a.y ? 1 : -1`.
  - The result is inside when `nonZero != 0 ? winding != 0 : (winding & 1) != 0`.
    EvenOdd by parity of a count is identical to an XOR of flips.
  - Each shader comment states the sign convention.

## Compatibility / Format Impact

- **Source-breaking, on purpose:** `Polygon.points` is gone. Every construction must
  write `contours := [pts], rule := canvas::FillRule.EvenOdd`. All 65 in-tree sites
  (§2) are migrated except the 6 in historical planning/bug docs. Any program outside
  the tree breaks with a compile error naming the unknown field. There is no silent
  behavior change.
- `canvas::FillRule` is a new exported name.
- Unchanged: the `DrawItem` tag set and order, `ITEM_BLOCK_SIZE`, the draw-entry
  layout, every other item's geometry and pixels, and every single-ring EvenOdd
  polygon's pixels.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with one line on what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase lands. **An
> unticked box means NOT DONE.**

### Phase 1 — Surface, builders, hash, migration (rule stored, not yet honored)

This phase delivers the new record, rings and hashing. Every existing picture stays
byte-identical. It can land alone: after it, EvenOdd is the only rule that draws
differently from HEAD, and it draws the same.

- [x] `mod.rs`: add the `FillRule` enum (mirror `CapStyle`'s descriptor, with a
      description per variant per §4). Replace Polygon's `points` prop with
      `contours: list_of(list_of(named("Point")))` and add `rule: named("FillRule")`
      before `paint_prop()`. Rewrite the record `description` for rings and rules
      (developer-facing, `.ai/man-content.md` bans apply).
- [x] `helper_geometry.rs:GEO_LAYOUT`: add `__CANVAS_GEO_POLYGON_RULE = 21`.
      `src/codegen/runtime/canvas/mod.rs`: add `HEADER_POLYGON_RULE`. Extend the two
      pin tests (`helper_geometry/tests/mod.rs:33–87`,
      `func_geo_build.rs:1690–1732`) to pin the new pair.
      (`cargo test --bin mfb canvas` → 110 passed, both pins included.)
- [x] `helper_geometry.rs`: `__canvas_polygonHeader` and `__canvas_polygonEdges`
      take the rings (and the rule, for the header) per §5. Update the three
      `CASE Polygon` arms (:1082, :1296, :1407). The hash arm folds
      `len(p.contours)`, then per ring `len` and its points, then `rule`, then paint.
- [x] `func_geo_build.rs`: `fill_polygon` iterates the outer list and each ring,
      wrapping within the ring. `emit_record_size`'s Polygon arm sums the contributing
      ring lengths. The rule ordinal is written to slot 21 (via `enum_ordinal`).
      Keep every per-edge instruction sequence unchanged (bit-identity with the
      MFBASIC builder).
      (`cargo test --test rt_canvas_geo_native` → 4 passed, `geoVerifyMismatches=0` over
      the matrix with the six ring items g8–g13 added.)
- [x] `func_item_hash.rs:fold_list`: add an arm for an element that is itself a list
      (recurse into `fold_list` per element). Keep the error for anything else.
- [x] Resolve the UNVERIFIED hash-parity item: `grep -rn "hashItem"
      tests src/codegen/builtins/canvas` for a native-vs-MFBASIC hash comparison. If
      one exists, it must pass for a two-ring polygon. If none exists, record that
      here. **None exists** (the grep finds only doc comments and the `.ir` golden), and
      none can: `func_item_hash.rs` hashes every kind but Text/Group, and "a kind is
      hashed by exactly one of the two paths, so their keys never have to agree". What
      the cache needs instead — that the rings and the rule are in the key — is pinned
      by the new `a_polygon_hash_covers_its_rings_and_rule` (`generations=1,2,3` → ok).
- [x] `src/codegen/collection/assign/self_update.rs:2502` (`("canvas.Polygon",
      SIZE_VARIES)`), `tests/runtime/inplace_self_update/field_kinds.tsv:35`, and
      `field_kinds_gen.py:164` (`len(before.points)` → `len(before.contours)`).
      Regenerate the `.tsv` with the generator the file names rather than by hand.
      (Corrections 3: only the Polygon row is taken from the generator; the
      `self_update.rs` row needed no change. `MFB_SELF_UPDATE_FILTER=canvas::Polygon
      cargo test --release --test rt_inplace_self_update every_field_kind_meets_its_expectation
      -- --ignored` → ok.)
- [x] Migrate every construction site in §2's table (59 = 65 minus 6 historical docs):
      `points := X` → `contours := [X], rule := canvas::FillRule.EvenOdd`. This covers
      `tests/canvas/scenes/*.mfb`, the five Rust test files, the scripts and tools,
      and the `examples/`.
      Check: `grep -rnE "canvas::Polygon\[points" --exclude-dir=target
      --exclude-dir=.git --exclude-dir=planning --exclude-dir=bugs .` → 0.
      (57 sites rewritten by script + the generator line + the `.tsv` row = 59; the
      check grep → 0.)
- [x] Tests (new, in `tests/canvas/rt_canvas_rasteriser.rs`, hand-computed pixels
      from `MFB_CANVAS_DUMP`):
      - a square ring with a concentric square hole, EvenOdd: the hole's centre pixel
        is the background, and the band between the rings is the fill;
      - two disjoint rings in one item both fill;
      - a ring with 1 point next to a normal ring draws only the normal ring;
      - `contours := []` draws nothing and does not trap.
      (`a_polygon_draws_each_of_its_rings` → ok; it also covers the 2-point ring.)
- [x] Test: a two-ring polygon published through `canvas::presentLayers` and
      `canvas::setGroup`, then mutated after publish, still draws the published
      rings. This extends `rt_canvas_present_deep_copy.rs` (the first `List OF List`
      field through `copy_flat_block`).
      (`published_polygon_rings_are_copied_not_aliased` → ok; Corrections 4.)
- [x] Test: the same two-ring scene under `--debug` with `MFB_CANVAS_GEO_VERIFY=1`
      reports no `geoSame` mismatch (native builder == MFBASIC builder).
      (Corrections 5: the ring items g8–g13 joined the GEO_VERIFY matrix, crossed with
      every paint → `geoVerifyMismatches=0`.)
- [x] Regenerate `tests/syntax/app/app-mouse-surface/golden/*` with
      `scripts/regen-native-goldens.sh` (expected to diff, §3). Before committing,
      inspect the `.ir` diff and confirm it is only the Polygon/FillRule type surface.
      (`regen-native-goldens: 4 build(s), 6 golden(s) rewritten, 0 failure(s)`; the
      `.ir` by `mfb build -ast -ir`, `.ast` unchanged. The `.ir` diff is the new
      `canvas.FillRule` enum, the Polygon record, and the rewritten polygon helper
      bodies — the helpers are in the IR, so their change is the predicted surface.)

Acceptance: every existing polygon picture is unchanged and the new ring tests pass.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_present_deep_copy --test rt_canvas_geo_native` → all pass, with
  `transforms.png`/`gradients.png` compared exactly and unmodified (`git status
  tests/golden/canvas` clean). Est. 10 min (guess). These four binaries are the ones
  that build polygon geometry; nothing smaller covers both builders and the deep copy.
  Also `cargo test --bin mfb canvas` → pass (the pin tests and the frozen-variant test)
  (est. 3 min, guess).
  Result: the four binaries → 4 + 22 + 9 + 62 passed (2 ignored), `git status
  tests/golden/canvas` clean; `cargo test --bin mfb canvas` → 110 passed.
Commit: —

### Phase 2 — Software NonZero (the oracle)

This phase delivers the rule in the software renderer, proven by hand-computed
pixels. That proof has to exist before the GPUs are compared against it.

- [ ] `helper_draw.rs:__canvas_edgeDistance`: read the rule slot and count windings
      per §6. The EvenOdd statement sequence stays textually unchanged.
- [ ] Update the doc comment on `__canvas_edgeDistance` and on
      `helper_glyph.rs:12–18`. Glyphs still use even-odd, and the sentence must no
      longer say the rasteriser *only* counts even-odd.
- [ ] Tests (`rt_canvas_rasteriser.rs`, hand-computed pixels):
      - square with a same-direction inner square, NonZero: the centre is filled;
        under EvenOdd it is a hole;
      - square with an opposite-direction inner square, NonZero: the centre is a hole;
      - pentagram {5/2} (one self-intersecting ring): the centre pentagon is filled
        under NonZero (winding 2) and a hole under EvenOdd;
      - a NonZero single-ring convex polygon gives the same pixels as EvenOdd
        (a symmetry check that does not rely on the oracle agreeing with itself).
- [ ] New scene `tests/canvas/scenes/star_301_2_nonzero_on_black.mfb`: a copy of
      `star_301_2_on_black.mfb` with `rule := canvas::FillRule.NonZero`, for the GPU
      rows in Phase 3.

Acceptance: NonZero fills by winding in the software renderer, and EvenOdd output is
unchanged.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden` → pass, with
  goldens exact and unmodified (est. 6 min, guess).
Commit: —

### Phase 3 — GPU NonZero (Metal and Vulkan; largest blast radius)

This phase delivers the rule in both shaders. It is last because it edits two
hand-kept shader twins and a checked-in binary that no test fully checks.

- [ ] `src/codegen/runtime/canvas/mod.rs`: add `emit_item_polygon_rule` (§6). Call it
      after `emit_item_arc_flags` in `metal.rs:emit_item_block` and
      `vulkan.rs:emit_item_block`.
- [ ] `metal.rs:METAL_SHADER_BODY`: add signed-crossing variants of
      `edgeToggles32`/`dfToggles`, change `edgeDistance` to a winding count with the
      `nonZero` argument, and pass `item.arc.y` at the `kind == 4` dispatch. Update the
      `arc.y` comment (:561, and the struct comment) to "arc reflex / polygon fill
      rule".
- [ ] `mfb_canvas.frag`: the same change, line for line with the MSL. Update the
      `ItemBlock` `arc` comment.
- [ ] Baseline first: `scripts/test-canvas-vulkan.sh tests/canvas/scenes/star_301_2_on_black.mfb`
      on box 2228 at HEAD, and record the result here. Then regenerate the SPIR-V with
      `scripts/regen-spirv.sh` (box 2228). Commit only the `.frag.spv` unless the
      `.vert` changed.
- [ ] Tests: add the Phase 2 NonZero scenes (ring-with-hole both directions, pentagram,
      `star_301_2_nonzero_on_black`) to `rt_canvas_metal.rs`'s scene rows.

Acceptance: both GPUs draw both rules within tolerance of the software renderer.
  Check (Metal, local Mac): `cargo test --test rt_canvas_metal` → pass with
  `gpuFrames≠0` on the new rows (est. 5 min, guess).
  Check (Vulkan, box 2228 — the only lane that runs the Vulkan shader on Linux x86_64;
  `.ai/remote_systems.md:112–137`): `scripts/test-canvas-vulkan.sh
  tests/canvas/scenes/star_301_2_nonzero_on_black.mfb` and the same for
  `star_301_2_on_black.mfb` → both within `GPU_DEFAULT` (est. 10 min, guess). Also
  `scripts/test-canvas-gpu-rows.sh tests/canvas/scenes/star_301_2_nonzero_on_black.mfb
  --target windows-x86_64` on box 206, the only real GPU (est. 10 min, guess).
Commit: —

### Phase 4 — Docs and spec

- [ ] `src/docs/spec/app/06_canvas.md`: at :52, "a `Polygon`'s point list" → its
      rings. At :179/:193–197, describe the crossing count under both rules and the
      signed-crossing test, citing `[[…:edgeDistance]]` and the new
      `[[src/codegen/runtime/canvas/mod.rs:HEADER_POLYGON_RULE]]` and
      `[[…:emit_item_polygon_rule]]`. At :324–326, keep "no cap and no join style" and
      reword "the crossing-count fill rule" to name `Polygon.rule`. Add the item-block
      `arc.y` dual use where `ITEM_ARC_REFLEX` is cited (:219).
- [ ] `func_present.rs:23` (the deep-copy paragraph): "a Polygon's point list" → its
      rings.
- [ ] `.ai/canvas-threading.md`: if it documents the item-block words or the header
      slots, add the `arc.y`/slot-21 polygon use there.

Acceptance: the man page and the spec render and cite correctly.
  Check: `mfb man canvas types` shows `FillRule` and the new Polygon fields;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min total, guess).
Commit: —

## Validation Plan

- Tests: Phase 1 (ring semantics, deep copy, builder parity), Phase 2 (hand-computed
  NonZero/EvenOdd pixels, including a symmetry check), and Phase 3 (GPU scene rows)
  above. The negative cases are the empty `contours`, the 1-point ring, and the
  2-point ring (stroke only).
- Coverage check: the new software code runs in `rt_canvas_rasteriser`. The shader
  code runs only on a GPU lane: `rt_canvas_metal` on this Mac (it skips when
  `metalReady` is false, so confirm `gpuFrames≠0` on the new rows) and the Vulkan
  scripts on 2228 and 206. The default `cargo test` covers none of the Vulkan shader,
  so Phase 3's script runs are that coverage.
- Runtime proof: an `--app` program that draws an "O" (outer ring clockwise, inner
  ring counter-clockwise) under both rules and a same-direction ring under both,
  screenshotted on macOS with `MFB_CANVAS_GPU=1`. The hole is visible in 3 of the 4
  cases.
- Doc sync: Phase 4.
- Final gate (run ONCE, after Phase 4): `cargo test --no-fail-fast` (est. 40 min,
  from plan-159-D's recorded run) and `bash scripts/artifact-gate.sh
  target/release/mfb all` → 0 diffs outside the predicted
  `tests/syntax/app/app-mouse-surface` set (est. 20 min). Any other diff is a bug hunt.

## Open Decisions

- **Rule slot and item word**: header slot 21 → `arc.y` (recommended, §3 rejected
  alternatives) vs. slot 34 → `arc.w`. The CAP overload is cheaper now and collides
  with polygon stroke caps later.
- **Winding sign convention**: `by > ay` → +1 (recommended; it matches the existing
  `edgeToggles32` branch on `b.y > a.y`). It is not observable, so it only needs to be
  the same everywhere.

## Corrections

1. **Prerequisites row.** Written when A was the only file; B, C and D are now letters
   of the same plan, so "only this file" is read as "no foreign plan-163". Re-measured
   2026-10-05: `ls planning/plan-163-*` → A, B, C, D.
2. **Line numbers moved** between `8cbd0206a` and the execution base `f858ff14b`: the
   self-update row is `self_update.rs:2735` (`("canvas.Polygon", SIZE_VARIES)`,
   unchanged by this plan — a ring list is still size-varying), and the generator line is
   `field_kinds_gen.py:166`.
3. **`field_kinds.tsv` is not regenerated wholesale.** Running the generator
   (`python3 tests/runtime/inplace_self_update/field_kinds_gen.py target/debug/mfb`)
   rewrites 144 expectation tokens that bug-766 D flipped by hand (`b309807ec`; the
   file's own header: "a letter that lands a kind flips its sites by hand"), so a
   wholesale regeneration would re-baseline unrelated rows. Only the Polygon row's
   setup and check columns were taken from the generator; its expectations are
   unchanged and pass (filtered `--ignored` run → ok).
4. **The deep-copy test is a runtime test.** `rt_canvas_present_deep_copy.rs` held only
   build-only `-ncode` checks; the new test renders (its own small `MFB_CANVAS_DUMP`
   harness) because "still draws the published rings" is a pixel claim. MFBASIC has no
   record field assignment, so the in-place rewrite is `p = WITH p { contours := … }`.
5. **GEO_VERIFY coverage** went into the existing matrix
   (`tests/canvas/scenes/canvas_geo_native_matrix.mfb`, items g8–g13) rather than a
   separate scene, so every ring case is crossed with every paint the matrix has.
6. `HEADER_POLYGON_RULE` is referenced only by its pin test until Phase 3's
   `emit_item_polygon_rule` reads it (a `dead_code` warning in the Phase 1–2 commits).

## Summary

The real engineering risk sits in two places. The first is keeping the native polygon
builder bit-identical to the MFBASIC one across a new ring loop; `GEO_VERIFY` and the
exact software goldens guard it. The second is the two shader twins plus a checked-in
SPIR-V that no test fully checks; the explicit Vulkan script runs guard those.
Untouched: text rendering (glyph bitmaps stay even-odd), the item-block size, the
`DrawItem` variant set, and every other item's geometry.
