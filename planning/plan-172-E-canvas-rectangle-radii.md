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
| plan-172-D complete and archived | `ls planning/plan-172-D-*` → no matches | MET (2026-10-10) |
| The placement helper exists | `grep -c 'FUNC __canvas_placement' src/codegen/builtins/canvas/helper_paint_defaults.rs` → 1 | MET (2026-10-10) |
| Open Decision 2 of 172-A is answered | `grep -c '\*\*ANSWER:' planning/completed/plan-172-A-*.md` → 5 | MET (2026-10-10) |

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
- ~~**`canvas::Size` is unchanged.**~~ Corrected: `canvas::Size` becomes `width AS Float`,
  `height AS Float` (owner's decision, 2026-10-10; see Corrections). Its two field
  names and its use as `canvas::getSize`'s return type are unchanged.
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

- **Added:** `canvas::BindTo`; `Rectangle`'s new fields. No constructor functions (Open
  Decision 1 of 172-A).
- **Removed:** `canvas::RoundedRect`; `x`/`y`/`w`/`h` on `Rectangle` and `Picture`.
- **Unchanged:** every geometry kind, the header, the item block, both shaders.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The corner resolver, as a pure function

- [x] Add `__canvas_cornerRadii` and `__canvas_uniformRadius` to `helper_shapes.rs`.
- [x] Unit-test them beside the helper: radius `0.0`, `-1.0`, `0.5` on a 100×40 size
      (→ 25 by 10), `1.0` (→ 50 by 20), `1.01` (→ 1.01 circular), `500.0` (→ 20,
      clamped), and a 1×1 size.

Acceptance: the seven cases pass, including the jump between `1.0` and `1.01`.
  Check: `cargo test --bin mfb canvas_corner` → pass (est. 3 min).
  Observed (2026-10-10): the resolver is MFBASIC companion source, so it has no Rust
  unit to call; the seven cases run through the compiled helper instead (Corrections).
  `cargo test --test rt_canvas_rasteriser corner_rule` → `7 passed; 0 failed`, of which
  `corner_rule_every_meaning_of_a_radius_draws_the_corner_it_names` is the seven cases.
Commit: —

### Phase 2 — Records, the two exact build paths, and the port

- [x] `canvas::Size` becomes Float: `mod.rs` (`width`, `height` as `Float`, description),
      `func_get_size.rs` (both overloads write Floats), every program that reads
      `getSize().width` / `.height` as an `Integer`, and the spec and man text that say
      the size is whole pixels. A test that `canvas::getSize()` on a 900 × 640 surface
      reads `900.0` and `640.0`.
- [x] `mod.rs`: add the `BindTo` enum; rebuild `Rectangle` and `Picture` without
      `rect_props`; delete `RoundedRect` and its `DrawItem` variant.
- [x] Amend `draw_item_variant_set_is_frozen` to the ten remaining names. The four
      answers: introduced by `d0db90709` (plan-98-B); it protects user `MATCH`
      exhaustiveness; dependants are found with `grep -rn RoundedRect src tests`; the
      proof is the owner's removal of the type in `planning/new_canvas.md`. Only the
      `RoundedRect` entry is removed.
- [x] `helper_geometry.rs`: `__canvas_rectHeader` takes position, bind, size and the
      resolved uniform radius, through `__canvas_placement`; `__canvas_pictureHeader`
      takes `position` and `size`.
- [x] `func_geo_build.rs`: `fill_rect`, `emit_rect_joined`, `emit_rect_dash_size` read
      `position`, `size`, `bind` and the four radii, and decline to the MFBASIC builder
      when the radii are not uniform.
- [x] `__canvas_hashItem`: drop the `RoundedRect` arm and renumber the literal start
      tags.
- [x] ~~Add `canvas::rectangle` and `canvas::picture` with man examples.~~ — moot: Open
      Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1).
- [x] Rewrite `Rectangle`'s registry example to the defaulted `MUT` + `WITH` form (it
      leaves `id`, `bind`, the four radii and `transform` at their zeros), and render it
      with `mfb man canvas Rectangle`.
- [x] Port the 357 sites; each `RoundedRect` becomes a `Rectangle` with its
      `cornerRadius` in all four radii. A `cornerRadius` in `(0, 1]` is ported as the
      equivalent pixel value above 1 — or, where it is below 1 pixel, kept as a square
      corner only if the dumped frame is unchanged.
- [x] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens.

Acceptance: every canvas pixel test passes with no PNG regenerated, `blendmodes.png`
included.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_picture --test rt_canvas_metal` → pass (est. 12 min); `git status
  --porcelain tests/golden/canvas` → empty.
  Observed (2026-10-10): `rt_canvas_golden` 23 passed, `rt_canvas_rasteriser` 98 passed
  (2 ignored), `rt_canvas_picture` 11 passed, `rt_canvas_metal` 47 passed, 0 failed in
  each; all 25 canvas-embedding targets and `cargo test --bin mfb canvas` (113 passed)
  green. `git status --porcelain tests/golden/canvas` → `?? tests/golden/canvas/corners.png`
  only, which is Phase 3's new reference; no existing PNG changed.
Commit: —

### Phase 3 — The flattened path, `Center`, and their proof

- [x] `helper_shapes.rs`: `__canvas_rrectRing(size, bind, radii)` returning the outline
      ring; `helper_geometry.rs` routes build path 3 through the polygon builder.
- [x] Measure the flattening error: render one rectangle with four equal circular
      corners through path 2 and, by forcing path 3, through the ring; record the
      largest per-channel pixel delta in Corrections.
- [x] Add scene `tests/canvas/scenes/rect_corners_on_black.mfb` and golden
      `tests/golden/canvas/corners.png`: one rectangle per corner mode, one with four
      different radii, one elliptical, one `Center`-bound and rotated, one dashed.
- [x] Add rasteriser cases: a `Center` rectangle's centre pixel is at `position`; a
      `radiusTL`-only rectangle has a rounded top-left and three square corners.
- [x] Add a Metal parity case for the scene, and add the scene to
      `scripts/test-canvas-gpu-rows.sh`.
- [x] `src/docs/spec/app/06_canvas.md`: the corner rule, the three build paths and the
      0.05-pixel bound, with citations; remove the three `RoundedRect` mentions.

Acceptance: the new golden renders identically on two consecutive runs, and Metal
matches it within `Tolerance::GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_golden corners --test rt_canvas_metal corners` →
  pass (est. 5 min).
  Observed (2026-10-10): `corners_match_their_reference_exactly` passed on two
  consecutive runs against the one committed PNG, and
  `corners_match_the_software_oracle_on_metal` passed. Flattening error measured by
  `corner_rule_the_flattened_outline_is_within_its_bound_of_the_exact_one`:
  `656 pixels differ, worst coverage delta 0.0499`.
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

- **`canvas::Size` held Integers, and a rectangle's size is fractional.** §1's non-goal
  said `canvas::Size` is unchanged, and `planning/new_canvas.md` gives `Rectangle` and
  `Picture` a `size AS canvas::Size`. But `Size` was `width AS Integer`, `height AS
  Integer` (`grep -n 'name: "Size"' -A12 src/codegen/builtins/canvas/mod.rs`), the return
  type of `canvas::getSize` and nothing else, while rectangles have Float extents that
  the tree depends on (`7.25`, `9.125`, `0.001` in `canvas_geo_native_matrix.mfb`; the
  "rectangles at a step" pixel scenes). Put to the owner on 2026-10-10 with three
  options; the answer: **make `Size` Float**. `new_canvas.md` stands as written, there is
  one size type, and `canvas::getSize` returns Floats. Phase 2 gains the task.
- **The three phases landed as one commit.** The record change, the builders and the
  port cannot be separated: removing `x`/`y`/`w`/`h` breaks every site, and the flattened
  path is one branch of the same `Rectangle` arm. Each phase's boxes and observations are
  its own; the `Commit:` lines name the same commit.
- **Phase 1's unit tests are compiled-program tests, and its Check command changed.**
  `__canvas_cornerRadii` and `__canvas_uniformRadius` are MFBASIC text in a Rust string
  constant (`helper_shapes.rs:CORNERS`); there is nothing for `cargo test --bin mfb
  canvas_corner` to call. The seven cases are
  `tests/canvas/rt_canvas_rasteriser.rs:corner_rule_every_meaning_of_a_radius_draws_the_corner_it_names`,
  which draws each and probes either side of the corner the rule names. The check is
  `cargo test --test rt_canvas_rasteriser corner_rule` (7 tests).
- **§2's "UNVERIFIED" is measured, in coverage and not in bytes.** A rectangle with four
  24 px corners (exact path) against the same with one corner at 24.000001 px (flattened
  path): 656 pixels differ, all on the four corner arcs, worst coverage delta 0.0499 —
  inside the 0.05 px chord bound. The frame is sRGB-encoded, so the same pixels differ
  by up to 53 of 255 in raw bytes near black; a first version of the test asserted on
  bytes and failed for that reason alone (`/tmp` probe: at (399, 120) bytes 245 vs 239
  are the largest LINEAR difference). The test decodes before comparing.
- **The segment cap bounds the guarantee.** The count per corner is clamped to 4..32, and
  32 segments keep a chord within 0.05 px only up to a radius of about 166 px
  (`0.05 / (1 − cos(π/128))`). Above that the error grows with the radius — 0.1 px at
  332 px. The spec section "A rectangle's corners" states the limit.
- **The native builder declines more than "not uniform".** `func_geo_build.rs:
  emit_rect_inputs` declines a rectangle whose four radii are unequal, or whose common
  radius is in `(0, 1]` (a fraction, so elliptical unless the rectangle is square), and
  the MFBASIC builder takes it. Consequence for the tree:
  `tests/canvas/scenes/canvas_geo_native_matrix.mfb` asserts every item is built
  natively, and its sliver `q4` had `cornerRadius := 0.2`; ported to `1.5`, a pixel
  radius clamped to the sliver's half-width exactly as 0.2 was clamped — the header
  bytes the scene compares are what its reference builder produces either way
  (`cargo test --test rt_canvas_geo_native` → 4 passed).
- **A circular corner of one pixel or less is no longer expressible, and four sites
  computed one.** The magnitude rule reads `(0, 1]` as a fraction, so a computed radius
  that can fall there must be kept out of the band by the program. Three examples
  compute their radii and were ported with a guard, each an approximation of a
  sub-pixel corner rather than the old pixels: `examples/canvas-demo/src/kit.mfb:rrect`
  draws its own 8-step polygon ring for a radius in `(0, 1]`;
  `examples/swat/canvas/src/main.mfb:slab` squares the corner there; two radii in
  `examples/weather` (`mercuryRadius`, `stopRadius`) are held at `math::max(…, 1.01)`.
  The dumped frames of canvas-demo and swat are unchanged (below), so neither guard is
  reached at the default surface. The only literal radius in the band was the matrix
  scene's, above.
- **The runtime proof moved off `examples/weather`.** Weather animates from wall-clock
  time: two dumps of the SAME pre-change build differ (`shasum` of two runs at
  `9a08e4d72`: `c3b6c9ad…`, `1c9ac02b…`), so it can prove nothing. The three other
  examples that used `RoundedRect` are deterministic and were dumped twice before
  (`9a08e4d72`, detached worktree) and twice after: `examples/canvas-demo`
  `90f2a990…` ×4, `examples/water` `9db5937a…` ×4, `examples/swat/canvas` `35efe86a…` ×4
  — byte-identical, each frame non-blank (7303, 477 and 319 distinct colours).
- **`mfb man canvas rectangle` does not exist** — there is no such function (Open
  Decision 1). `mfb man canvas types` was rendered; it shows `Rectangle`'s ten fields,
  `BindTo`, the Float `Size`, and no `RoundedRect`. The `MUT` + `WITH` form its
  description gives was compiled and run: 9600 white pixels, x 20..139, y 20..99.
- **Scripts and tools that no test builds** (`scripts/test-canvas-*.sh`,
  `scripts/test-macapp.sh`, `scripts/test-winapp.sh` heredocs) were ported by the same
  codemod and read, not run on their platforms; 172-M's cross-platform proof runs them.
- **A compiler panic surfaced by this port was fixed in its own commit** (`9a08e4d72`):
  a `vector::Float2[…]` built in place as an argument of an inline builtin.
- **From 172-A, Open Decision 1:** no constructor functions; the task is struck as moot
  and replaced by the `MUT` + `WITH` example task.
- **From 172-A, Open Decision 2:** answered as recommended; §3's flattening stands.

## Summary

The exact circular corner that exists is kept for every case it can serve, so no ported
scene moves. Everything new — different radii per corner, elliptical corners — rides the
polygon machinery that already strokes, dashes and clips on all three renderers, at a
measured 0.05-pixel flattening bound.
