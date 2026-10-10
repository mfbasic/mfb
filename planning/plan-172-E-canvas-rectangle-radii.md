# plan-172-E: `Rectangle` with `position`, `bind`, `size` and four radii

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-D

`canvas::Rectangle` takes `position`, `bind`, `size` and a radius per corner.
`canvas::RoundedRect` is removed. `canvas::Picture` moves from `x`/`y`/`w`/`h` to
`position` and `size`; its reshaping around a shape is 172-L.

Behavioral outcome: a rectangle is placed by its top-left corner or its centre as `bind`
says, each corner is square, circular or elliptical by its own radius, and every
existing scene, ported faithfully, draws the same pixels.

References:

- `planning/new_canvas.md` — the `BindTo` enum, the corner-radius comment block and the
  `Rectangle` listing.
- `planning/plan-172-A-canvas-shapes-premises.md` — Open Decision 2 (flattening).
- `src/codegen/builtins/canvas/helper_geometry.rs:__canvas_rectHeader`,
  `__canvas_pictureHeader` — the rectangle and picture builders.
- `src/codegen/builtins/canvas/func_geo_build.rs:fill_rect`, `emit_rect_joined` — the
  native rectangle builders, which read `cornerRadius` by name.
- `src/codegen/builtins/canvas/helper_dashes.rs`, `helper_joins.rs` — rectangle dashes
  and joins.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-D complete and archived | `ls planning/plan-172-D-*` → no matches | NOT MET |
| The placement helper exists | `grep -c 'FUNC __canvas_placement' src/codegen/builtins/canvas/helper_paint_defaults.rs` → 1 | NOT MET |
| Open Decision 2 of 172-A is answered | `grep -c 'ANSWER:' planning/completed/plan-172-A-*.md` → 5 | NOT MET |

If plan-172-D is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::Rectangle` is `id`, `position`, `bind`, `size`, `radiusTL`, `radiusTR`,
  `radiusBL`, `radiusBR`, `paint`, `transform`, in that order.
- `canvas::BindTo` exists with `TopLeft` first (so it is the zero value) and `Center`.
- `canvas::RoundedRect` is not a type.
- Each radius follows the magnitude rule: 0 or less is square; above 0 through 1 is a
  fraction of the half-size and gives an elliptical corner; above 1 is pixels, clamped
  to `min(width / 2, height / 2)`, and gives a circular corner.
- A rectangle whose four corners resolve to one circular radius draws exactly what
  `RoundedRect` drew for that `cornerRadius`.
- `canvas::Picture` is placed by `position` and `size`.

### Non-goals (explicit constraints)

- **`Picture` keeps its own `id`, `paint` and `transform` and gets no `shape`, `clip`
  or `fit`.** That is 172-L.
- **No new geometry kind and no shader change.** See §3.
- **`canvas::Size` is unchanged.**
- **No pixel changes for a faithfully ported scene**, including every former
  `RoundedRect`.

## 2. Current State

`Rectangle`, `RoundedRect` and `Picture` share `rect_props` (`x`, `y`, `w`, `h`,
`paint`); `RoundedRect` adds `cornerRadius`. All three build through
`__canvas_rectHeader(x, y, w, h, cornerRadius, paint)`, which clamps the radius to
`[0, min(w, h) / 2]`, stores the half-extents less the radius in header slots 4–5 and
the radius in slot 6. There is one geometry kind for both (`__CANVAS_KIND_RECT`, 0);
distance is the box distance less the radius, in MFBASIC (`__canvas_geoDistance`), GLSL
(`rectDistance`) and MSL.

Nothing supports per-corner or elliptical corners: the radius is one scalar.
`__canvas_rectJoins` returns no joins when it is non-zero, and `__canvas_rectDashes`
walks the outline by a closed-form circular-corner formula (`__canvas_rrectPosition`).

A picture is a rectangle record with slot 0 rewritten to `__CANVAS_GEO_PICTURE` (9); its
sampling rectangle is read back from slots 2–5, so a non-zero radius there would corrupt
it.

### Measured populations

`$D` is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| `canvas::Rectangle[` | 250 in 63 files | `grep -rhoE 'canvas::Rectangle\[' $D \| wc -l` → 250 |
| `canvas::RoundedRect[` | 36 in 22 files | → 36 |
| `canvas::Picture[` | 71 in 24 files | → 71 |
| Sites this letter ports | 357 | sum of the three rows |
| `cornerRadius` in the companion source | 13 in 5 files | `grep -rho cornerRadius src/codegen/builtins/canvas \| wc -l` → 13 |
| `RoundedRect` mentions outside constructors (types, `MATCH` arms, prose) | 73 | `grep -rho 'RoundedRect' $D \| wc -l` → 109, less the 36 constructors |
| `RoundedRect` constructors in the golden suite | 4 | `grep -c 'canvas::RoundedRect\[' tests/canvas/rt_canvas_golden.rs` → 4 |

### Verified properties

- **Uniform circular corners need no new code path.** VERIFIED by reading
  `__canvas_rectHeader`: it already takes one clamped radius. A `Rectangle` whose four
  radii resolve equal and circular passes that radius and is the old `RoundedRect`
  record byte for byte.
- **A polygon ring already strokes, joins, dashes and clips on all three renderers.**
  VERIFIED by reading `helper_geometry.rs` (`__CANVAS_GEO_POLYGON`, the edge tail),
  `helper_joins.rs` and `helper_dashes.rs`.
- **UNVERIFIED — the antialiased edge of a 16-segment flattened quarter-ellipse against
  the true curve.** Phase 3 measures the worst pixel delta rather than assuming it.

## 3. Design Overview

**Resolving a corner.** `__canvas_cornerRadii(size, r)` returns `(rx, ry)` for one
corner by the magnitude rule. A helper beside it answers "are all four corners equal and
circular", and with what radius.

**Three build paths, chosen at build time:**

1. **All four square** → today's rectangle record.
2. **All four equal and circular** → today's rounded-rectangle record (kind 0, radius in
   slot 6). This is every ported `RoundedRect`.
3. **Anything else** → a polygon record (kind 4): the outline is walked clockwise from
   the top edge where it leaves the top-left corner, each rounded corner flattened to
   segments. The segment count per corner is the smallest that keeps the chord within
   0.05 pixels of the ellipse quadrant, clamped to 4..32. The ring starts where
   `RoundedRect`'s dash pattern starts today, so a dashed outline begins at the same
   place on every path.

Path 3 costs edges against the frame's edge cap and is exact to 0.05 local pixels; a
transform that scales the shape scales that error (Open Decision 2 of 172-A).

**`bind`.** The local origin is the bind point. `TopLeft`: the rectangle spans
`(0, 0)..(w, h)`. `Center`: `(−w/2, −h/2)..(w/2, h/2)`. The placement helper from 172-D
then applies `position` and `transform`, so a `Center` rectangle spins in place.

**`Picture`.** `position` and `size` replace `x`/`y`/`w`/`h`; it binds top-left and
always takes build path 1, which keeps its sampling rectangle intact.

**Gate class: behaviour-preserving for ported scenes, plus new behaviour** (per-corner
and elliptical corners, `Center` binding), each with its own test and one new golden.

### Rejected alternatives

- **An exact per-corner elliptical distance field.** Rejected (172-A §3): a new kind on
  three renderers, and no closed-form dash length along an ellipse quadrant.
- **Always flattening rounded rectangles.** Rejected: it would move every ported
  `RoundedRect`'s pixels and drop the exact circular field that exists.
- **A separate unit field for the radius.** Rejected by the owner; the magnitude rule in
  `planning/new_canvas.md` stands, jump at 1.0 included.

## Compatibility / Format Impact

- **Added:** `canvas::BindTo`; `Rectangle`'s new fields; `canvas::rectangle` and
  `canvas::picture` constructor functions.
- **Removed:** `canvas::RoundedRect`; `x`/`y`/`w`/`h` on `Rectangle` and `Picture`.
- **Unchanged:** every geometry kind, the header, the item block, both shaders.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The corner resolver, as a pure function

- [ ] Add `__canvas_cornerRadii` and `__canvas_uniformRadius` to `helper_shapes.rs`.
- [ ] Unit-test them beside the helper: radius `0.0`, `-1.0`, `0.5` on a 100×40 size
      (→ 25 by 10), `1.0` (→ 50 by 20), `1.01` (→ 1.01 circular), `500.0` (→ 20,
      clamped), and a 1×1 size.

Acceptance: the seven cases pass, including the jump between `1.0` and `1.01`.
  Check: `cargo test --bin mfb canvas_corner` → pass (est. 3 min).
Commit: —

### Phase 2 — Records, the two exact build paths, and the port

- [ ] `mod.rs`: add the `BindTo` enum; rebuild `Rectangle` and `Picture` without
      `rect_props`; delete `RoundedRect` and its `DrawItem` variant.
- [ ] Amend `draw_item_variant_set_is_frozen` to the ten remaining names. The four
      answers: introduced by `d0db90709` (plan-98-B); it protects user `MATCH`
      exhaustiveness; dependants are found with `grep -rn RoundedRect src tests`; the
      proof is the owner's removal of the type in `planning/new_canvas.md`. Only the
      `RoundedRect` entry is removed.
- [ ] `helper_geometry.rs`: `__canvas_rectHeader` takes position, bind, size and the
      resolved uniform radius, through `__canvas_placement`; `__canvas_pictureHeader`
      takes `position` and `size`.
- [ ] `func_geo_build.rs`: `fill_rect`, `emit_rect_joined`, `emit_rect_dash_size` read
      `position`, `size`, `bind` and the four radii, and decline to the MFBASIC builder
      when the radii are not uniform.
- [ ] `__canvas_hashItem`: drop the `RoundedRect` arm and renumber the literal start
      tags.
- [ ] Add `canvas::rectangle` and `canvas::picture` with man examples.
- [ ] Port the 357 sites; each `RoundedRect` becomes a `Rectangle` with its
      `cornerRadius` in all four radii. A `cornerRadius` in `(0, 1]` is ported as the
      equivalent pixel value above 1 — or, where it is below 1 pixel, kept as a square
      corner only if the dumped frame is unchanged.
- [ ] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens.

Acceptance: every canvas pixel test passes with no PNG regenerated, `blendmodes.png`
included.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_picture --test rt_canvas_metal` → pass (est. 12 min); `git status
  --porcelain tests/golden/canvas` → empty.
Commit: —

### Phase 3 — The flattened path, `Center`, and their proof

- [ ] `helper_shapes.rs`: `__canvas_rrectRing(size, bind, radii)` returning the outline
      ring; `helper_geometry.rs` routes build path 3 through the polygon builder.
- [ ] Measure the flattening error: render one rectangle with four equal circular
      corners through path 2 and, by forcing path 3, through the ring; record the
      largest per-channel pixel delta in Corrections.
- [ ] Add scene `tests/canvas/scenes/rect_corners_on_black.mfb` and golden
      `tests/golden/canvas/corners.png`: one rectangle per corner mode, one with four
      different radii, one elliptical, one `Center`-bound and rotated, one dashed.
- [ ] Add rasteriser cases: a `Center` rectangle's centre pixel is at `position`; a
      `radiusTL`-only rectangle has a rounded top-left and three square corners.
- [ ] Add a Metal parity case for the scene, and add the scene to
      `scripts/test-canvas-gpu-rows.sh`.
- [ ] `src/docs/spec/app/06_canvas.md`: the corner rule, the three build paths and the
      0.05-pixel bound, with citations; remove the three `RoundedRect` mentions.

Acceptance: the new golden renders identically on two consecutive runs, and Metal
matches it within `Tolerance::GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_golden corners --test rt_canvas_metal corners` →
  pass (est. 5 min).
Commit: —

## Validation Plan

- Tests: the existing suites; seven resolver unit cases; two rasteriser cases; one new
  golden with a Metal parity case; `tests/syntax/canvas/canvas_roundedrect_removed_invalid/`.
- Coverage check: `cargo test --test rt_canvas_geo_native` → pass, and its stats show
  the native builder took the uniform-radius rectangles.
- Runtime proof: `examples/weather` (ten `RoundedRect`, the most of any example) dumped
  before and after, byte-identical.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas types` and
  `mfb man canvas rectangle` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None beyond 172-A's Open Decision 2.

## Corrections

<Filled in DURING execution.>

## Summary

The exact circular corner that exists is kept for every case it can serve, so no ported
scene moves. Everything new — different radii per corner, elliptical corners — rides the
polygon machinery that already strokes, dashes and clips on all three renderers, at a
measured 0.05-pixel flattening bound.
