# plan-172-D: `position` on `Circle`, `Ellipse`, `Arc` and `Text`

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-C, plan-172-N

The four shapes placed by a single point trade `x` and `y` for
`position AS vector::Float2`, and the transform pivots about that point. `Text.size`
becomes `fontSize`. This is the letter that introduces the placement rule every later
family follows.

Behavioral outcome: `canvas::Circle` at `position (200, 380)` with a rotation
`transform` turns about (200, 380), not about the surface origin; and every existing
scene, ported faithfully, draws the same pixels.

References:

- `planning/new_canvas.md` — "Transform order, spelled out", and the `Circle`,
  `Ellipse`, `Arc`, `Text` listings.
- `planning/plan-172-A-canvas-shapes-premises.md` §3 "The identity rule that keeps B–F
  pixel-neutral", and Open Decision 1 (constructor functions).
- `src/codegen/builtins/canvas/helper_geometry.rs` — the per-kind header builders.
- `src/codegen/builtins/canvas/func_geo_build.rs` — the native builders, which read
  `x` and `y` by name.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-C complete and archived | `ls planning/plan-172-C-*` → no matches | MET — `no matches found` |
| Every shape carries its own `transform` | `grep -c 'transform_prop()' src/codegen/builtins/canvas/mod.rs` → ≥ 10 | MET by meaning — the command prints 9: seven records call it and `rect_props` once for three more, plus the definition. `mfb man canvas types` shows `transform` on all ten |
| plan-172-N complete and archived (an `ENUM` has a default) | `ls planning/plan-172-N-*` → no matches | MET — `no matches found` |
| Open Decision 1 of 172-A is answered | `grep -c '\*\*ANSWER:' planning/completed/plan-172-A-*.md` → 5 | MET — measured 5 |

If plan-172-C is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `Circle`, `Ellipse`, `Arc` and `Text` have `position AS vector::Float2` and no `x`/`y`.
- `Text` has `fontSize` and no `size`.
- For these four, the effective placement is `T(position) ∘ transform`: local geometry
  has its origin at the reference point, and `paint.clip` and `paint.gradient` are in
  that local space.
- `Circle`, `Ellipse` and `Arc` are built from a defaulted record with `WITH` (Open
  Decision 1 of 172-A, answered: no constructor functions; 172-N makes the records
  defaultable). `Text` holds a `RES` font and keeps its full constructor.

### Non-goals (explicit constraints)

- **`Text.text` stays a `String`.** Attributed text is under `## Future` in
  `planning/new_canvas.md`.
- **`Rectangle`, `Picture`, `Line`, `Polygon`, `PolyLine` and `Group` are untouched.**
- **An arc still has no interior.**
- **No pixel changes for a faithfully ported scene.**

## 2. Current State

All four carry `x` and `y` scalars. Their header builders write the centre (or the
baseline start) straight into header slots 2–3, and the native builders read `x` and `y`
by name. The transform, when set, is applied about the surface origin: the registry's
own advice is to "give the item coordinates around `(0, 0)` and put the position in `tx`
and `ty`".

`paint.clip` rings and gradient points are in the item's own coordinates, which today
are surface coordinates whenever the transform is all-zero.

### Measured populations

`$D` is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| `canvas::Circle[` | 135 in 47 files | `grep -rhoE 'canvas::Circle\[' $D \| wc -l` → 135 |
| `canvas::Ellipse[` | 33 in 19 files | → 33 |
| `canvas::Arc[` | 36 in 16 files | → 36 |
| `canvas::Text[` | 62 in 26 files | → 62 |
| Sites this letter ports | 266 | sum of the four rows |
| Of those, sites that name a clip or gradient inline | at least 15 | `grep -rhoE 'canvas::(Circle\|Ellipse\|Arc\|Text)\[[^]]*' $D \| grep -cE 'clip\|gradient'` → 15. The pattern stops at the first `]`, so this is a lower bound |
| Sites that build a `canvas::Transform` inline | 0 found | same pattern, `grep -c 'canvas::Transform'` → 0. Transforms reach these sites through a paint variable, so the true count of transformed sites is UNMEASURED and is taken per file during the port |

### Verified properties

- **The native builders read the position by field name.** VERIFIED by reading
  `func_geo_build.rs` (`scalar_offset` lookups of `"x"`, `"y"`): a rename breaks the
  build rather than reading a wrong slot.
- **UNVERIFIED — how many ported sites set both a transform and a non-origin position.**
  Those are the only sites whose matrix must be rewritten. No grep finds them, because
  the transform arrives through a variable; Phase 2 lists them per file as it ports.

## 3. Design Overview

**The placement rule.** For a shape with `position = P` and `transform = M` (linear part
`L`, translation `t`):

- If `M` is all-zero: build the geometry at `P`, as today's builder does for
  `x = P.x, y = P.y`. Clip rings and gradient points are offset by `P` at build time.
  The record is byte-identical to today's for the same surface coordinates.
- Otherwise: build the geometry at the local origin, leave clip and gradient points
  local, and pass `__canvas_paintHeader` the affine with linear part `L` and translation
  `P + t`. The pivot is `P`.

One helper holds the rule: `__canvas_placement(position, transform)` returns the
effective `Transform` and the build offset, and every header builder in D, E and F calls
it. The native builder's equivalent is one emitter helper beside `emit_paint_supported`.

**Porting a transformed site.** Today's item at `(x, y)` with `M` draws
`L·(x, y) + t`. To keep its pixels with `position = (x, y)`, the new translation is
`t' = L·(x, y) + t − (x, y)`. Sites that followed the registry's advice (`x = y = 0`,
position in `tx`/`ty`) port to `position = (tx, ty)` and a translation of zero.

**No constructor functions** (Open Decision 1 of 172-A). A new program writes `MUT c AS
canvas::Circle` and `WITH c { position := …, radius := …, paint := … }`, leaving `id`
empty and `transform` all-zero; the man example for each of `Circle`, `Ellipse` and
`Arc` shows that form. The ported sites keep their existing full constructors, with the
fields renamed in place: a codemod of a constructor that already names every field is
smaller and safer than rewriting 266 sites into a different statement shape.

**Gate class: behaviour-preserving for ported scenes, plus one new behaviour** (the
pivot), which gets its own test.

### Rejected alternatives

- **Always building locally with `T(position) ∘ transform`.** Rejected in 172-A §3: it
  sends every positioned item down the transformed path.
- **Leaving clip and gradient points in surface coordinates.** Rejected by
  `planning/new_canvas.md`: a shape then could not be moved by changing `position`
  alone.

## Compatibility / Format Impact

- **Changed:** four records' placement fields; `Text.size` → `fontSize`.
- **Changed meaning:** for these four, `paint.clip` and `paint.gradient` points are
  offsets from `position`; the transform pivots about `position`.
- **Added:** nothing.
- **Unchanged:** the geometry header, both shaders, every pixel of a ported scene.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The placement helper, behind a test

- [x] Add `__canvas_placement` to `helper_paint_defaults.rs` beside
      `__canvas_invertTransform`.
      → three helpers: `__canvas_transformActive` (the one test for "is this transform in effect"), `__canvas_placement` (the effective affine) and `__canvas_placedPaint` / `__canvas_placedClip` (clip and gradient points moved to the build origin).
- [x] Add helper unit tests beside it: all-zero transform returns the offset and an
      all-zero affine; a rotation returns translation `P + t`; a no-area transform is
      treated as all-zero.
      → as four rasteriser cases named `placement_*` (see Corrections: a companion helper cannot be run from a unit test). `cargo test --test rt_canvas_rasteriser placement` → `4 passed`.

Acceptance: the helper's three cases pass.
  Check: `cargo test --bin mfb canvas_placement` → pass (est. 3 min).
Commit: —

### Phase 2 — Records, builders and the port

- [x] `mod.rs`: `Circle`, `Ellipse`, `Arc`, `Text` take `position`; `Text.size` →
      `fontSize`; rewrite the `Transform` description's pivot sentence.
      → `mfb man canvas types` shows `position` on the four and `fontSize` on `Text`.
- [x] `helper_geometry.rs`: the four header builders call `__canvas_placement`; clip and
      gradient points are offset in the all-zero branch.
      → and the offset is an argument of `__canvas_paintHeader` for every kind (zero for the kinds not positioned yet).
- [x] `func_geo_build.rs`: the circle, ellipse and arc native builders read `position`
      and add it.
      → only the circle is built natively; it reads `position`, and every clip point is read through `clip_point`, which adds the build offset. `cargo test --test rt_canvas_geo_native` passes.
- [x] `func_measure_text.rs` and the glyph path read `fontSize`.
      → 8 reads in `helper_geometry.rs`; `func_measure_text.rs` takes its size as a parameter and needed nothing. `cargo test --test rt_canvas_font` → `33 passed`.
- [x] ~~Add the four constructor functions (`func_circle.rs`, `func_ellipse.rs`,
      `func_arc.rs`, `func_text.rs`), each with a man example, and register them.~~ —
      moot: Open Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1).
- [x] Rewrite the registry examples of `Circle`, `Ellipse` and `Arc` to the defaulted
      `MUT` + `WITH` form, and render each with `mfb man canvas <Type>`.
      → the three record descriptions show the `MUT` + `WITH` form; `/tmp/plan-172-A/idiom.mfb` builds it and draws all three; `canvas::present`'s example turns its ellipses with a plain rotation.
- [x] Port the 266 sites. For each file, list the sites whose item is transformed and
      not at the origin, record the count in Corrections, and rewrite each matrix per §3.
      → `codemod_d.py` → `266 constructor(s) in 63 file(s)`. Transformed and not at the origin: 15 listed, of which 6 needed a new translation (see Corrections).
- [x] `FIELD_KIND_TABLE` and its generated `.tsv`; the IR-embedding goldens.
      → no table change; four `.tsv` rows read `before.id` instead of the removed `before.x`.

Acceptance: every canvas pixel test passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_font --test rt_canvas_metal` → pass (est. 12 min); `git status --porcelain
  tests/golden/canvas` → empty.
Commit: —

### Phase 3 — The pivot, proved

- [x] Add to `tests/canvas/rt_canvas_rasteriser.rs`: an ellipse at `position (300, 200)`
      with a quarter-turn `transform`, asserting the pixels at the ends of its long
      axis are above and below (300, 200).
      → `placement_a_transform_turns_a_shape_about_its_position`, plus `placement_a_transforms_translation_adds_to_the_position`.
- [x] Add: a circle with a two-stop linear gradient and a `position` moved by 100
      pixels between two renders, asserting the two frames are equal under that shift.
      → `placement_a_clipped_gradient_shape_moves_whole_with_its_position`: gradient and clip, 40,000 pixels compared exactly under a 100 px shift.
- [x] Add a Metal parity case for the first scene in `tests/canvas/rt_canvas_metal.rs`.
      → `shapes_turned_about_their_position_match_the_software_oracle`; `cargo test --test rt_canvas_metal` → `46 passed`.
- [x] `src/docs/spec/app/06_canvas.md`: add "Placement" under "Coordinates and angles"
      stating the rule and the pivot, with `[[…:__canvas_placement]]`.
      → `### Placement`, citing `TRANSFORM_ACTIVE`, `PLACEMENT`, `PLACED_PAINT` and `clip_point`.

Acceptance: the pivot is the reference point, and a clipped or gradient-filled shape
moves whole when only `position` changes.
  Check: `cargo test --test rt_canvas_rasteriser placement` → pass (est. 3 min).
Commit: —

## Validation Plan

- Tests: the existing suites; three new placement cases; four man examples gated by
  `tests/cli/cli_canvas_man_examples_compile.rs`.
- Coverage check: `cargo test --test rt_canvas_geo_native` → pass, proving the native
  builders still accept positioned, untransformed circles rather than declining them.
- Runtime proof: `tests/golden/canvas/smiley.png` scene, dumped before and after,
  byte-identical.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas circle` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None beyond 172-A's Open Decision 1.

## Corrections

- **Phase 1's unit tests are rasteriser cases.** `cargo test --bin mfb canvas_placement`
  cannot exist: the bin tests lower programs and never run them, and
  `__canvas_placement` is MFBASIC inside the package, callable from no test. Its three
  cases are proved by what they draw — `placement_*` in `rt_canvas_rasteriser.rs` —
  which is a stronger check than the one named, and they landed with Phase 2 because
  the helper is unreachable until a record has a `position`.
- **The three phases landed as one commit**, for the reason 172-C's did.
- **The offset is an argument of `__canvas_paintHeader`, for every kind.** §3 had each
  builder offset its own clip and gradient points. One place is fewer chances to
  disagree, and it had to cover the unpositioned kinds too: the native builder adds the
  offset to every clip point unconditionally, `-0.0 + 0.0` is `+0.0`, and a builder
  that skipped the add for a zero offset would differ from it in a sign bit.
- **A gradient's points are moved only when it has two stops.** With fewer it is not a
  gradient, and the native builder — which takes exactly those items — copies the raw
  points into the header as it always did.
- **`clip` and gradient points had to be made local at the sites, which §3 did not
  say.** Moving the points at build time means every existing clip ring and gradient
  point on these four kinds was `position` too far out. Five tests failed and named
  them (`a_clip_cuts_a_circle_in_half`, `a_clip_larger_than_the_item_changes_nothing`,
  `a_radial_gradient_runs_outward_from_its_centre`, and the two gradient goldens);
  seven more sites that no exact-pixel test covers were found by listing every
  positioned constructor with its paint (`/tmp/plan-172-A/list_d_paints.py`).
  `canvas_geo_native_matrix.mfb` was left alone: it compares the two builders' records,
  not a picture, and a clip that is now offset exercises the new path.
- **Transformed sites not at the origin: 15 listed, 6 rewritten.** A 2× scale of a
  circle at (150, 150) gained the translation (150, 150); a quarter-turned text at
  (40, 60) went from translation (500, 100) to (400, 80); three shapes in
  `transformed_shapes_on_black.mfb` and the turned-ellipse test lost a translation that
  existed only to pivot about their own centre. The other 9 carry a transform that is
  all-zero at run time. The 17 ellipse rotations 172-C wrote became plain rotations.
- **Every program that builds one of the four needs `IMPORT vector`.** 75 import blocks
  gained it (`/tmp/plan-172-A/add_vector_import.py`); 19 font tests failed to build
  until they did.
- **The examples were ported by a sub-agent** (the `canvas-demo` kit's helpers take
  paints in surface coordinates and now localize them; its `placed` and `restyleItem`
  account for the pivot). They build; they are not pixel-tested.
- **No reference image changed.** `git status --porcelain tests/golden/canvas` is empty.
- **From 172-A, Open Decision 1:** no constructor functions. The task is struck as moot
  and replaced by a task that shows the `MUT` + `WITH` idiom in the three records'
  examples; §1, §3 and Compatibility are corrected to match.

## Summary

This letter sets the rule — bake the position when the transform is unset, compose it
when it is not — in one helper, and proves it on the four simplest shapes. E and F apply
the same helper to shapes with more geometry.
