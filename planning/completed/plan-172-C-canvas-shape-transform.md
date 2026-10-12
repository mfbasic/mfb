# plan-172-C: `transform` and `id` move onto every shape

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-B

Each of the ten drawn records — `Rectangle`, `RoundedRect`, `Line`, `Polygon`, `Circle`,
`Arc`, `Text`, `Ellipse`, `PolyLine`, `Picture` — gains `id AS String` and
`transform AS canvas::Transform`. `Paint.transform` and `Ellipse.angle` are removed.

Behavioral outcome: a program writes `shape.transform` where it wrote
`shape.paint.transform`, an ellipse is turned by its `transform`, and every canvas pixel
test passes unchanged.

References:

- `planning/new_canvas.md` — "Remove transform from canvas::Paint", "Remove
  `Ellipse.angle`", and the per-type listings that put `id` first and `transform` last.
- `src/codegen/builtins/canvas/helper_geometry.rs:__canvas_paintHeader`,
  `helper_paint_defaults.rs:__canvas_invertTransform` — where the transform is read.
- `src/codegen/builtins/canvas/func_geo_build.rs:emit_paint_supported` — the native
  builder's all-zero-transform test.
- `src/docs/spec/app/06_canvas.md` — "`Paint.transform` maps the item's coordinates onto
  the surface" and the clip and gradient sections that cite it.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-B complete and archived | `ls planning/plan-172-B-*` → no matches | MET — `no matches found` |
| `canvas::Point` is gone | `grep -c 'name: "Point"' src/codegen/builtins/canvas/mod.rs` → 0 | MET — measured 0 |

If plan-172-B is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- Every drawn record has `id` as its first prop and `transform` as its last.
- `canvas::Paint` has no `transform` prop; `canvas::Ellipse` has no `angle` prop.
- A shape's `transform` carries its `paint.clip` and `paint.gradient` exactly as
  `Paint.transform` did.
- `id` is stored, copied and hashed; nothing reads it for meaning until 172-I.

### Non-goals (explicit constraints)

- **No `position` yet.** Placement stays in each record's existing scalars. The pivot is
  therefore still the surface origin, as it is today; 172-D, E and F move it.
- **`Picture` keeps its own `paint`, `x`, `y`, `w`, `h`** until 172-E and 172-L.
- **`Group` is untouched** until 172-G.
- **The all-zero and no-area rules are unchanged**, and stay in the one function that
  implements them.
- **No pixel changes.**

## 2. Current State

The transform is a `Paint` prop. `__canvas_paintHeader(h, paint)` reads it, calls
`__canvas_invertTransform`, and writes the inverse and the `hasTransform` flag to header
slots 27–33. `__canvas_hashPaint` folds it. The native builder reads it by field name
through `emit_paint_supported` and declines any item where it is not all-zero. The
native hash walks every field of the record from the type model, so a moved field is
hashed without an edit there.

`Ellipse.angle` rotates the radii about the centre; it reaches the header as a cosine
and sine in slots 39–40.

`every_draw_item_variant_carries_a_paint` requires each non-container variant to have a
`paint` prop of type `Paint`. It does not inspect `Paint`'s fields, so it is unaffected.

### Measured populations

`$D` is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| `transform :=` assignments | 148 in 38 files | `grep -rhoE '\btransform *:=' $D \| wc -l` → 148 |
| `canvas::Transform[` constructors | 103 in 34 files | `grep -rhoE 'canvas::Transform\[' $D \| wc -l` → 103 |
| Reads of `paint.transform` in the companion source | 2 | `grep -rhoE '(paint\|p)\.transform' src/codegen/builtins/canvas \| wc -l` → 2 |
| `Ellipse` constructors that set `angle` | 28 of 33 | `grep -rhoE 'canvas::Ellipse\[[^]]*' $D \| grep -c 'angle *:='` → 28 |
| Item constructors that must gain `id` and `transform` | 853 | 954 total less 101 `Group` (172-A table) |
| `canvas::Paint[` constructors by name | 4 | `grep -rhoE 'canvas::Paint\[' $D \| wc -l` → 4; each loses one field |

### Verified properties

- **The native hash needs no per-field edit.** VERIFIED by reading
  `func_item_hash.rs:emit_item_hash`: it folds every field in type-model slot order.
- **The gradient has its own, separate transform.** VERIFIED:
  `Gradient.transform` and `__canvas_gradientInverse` are independent of
  `Paint.transform` and stay.
- **UNVERIFIED — that an ellipse's `angle` and a rotation `transform` about the surface
  origin produce the same pixels.** They do not in general: `angle` turns about the
  centre. Phase 3 handles the 28 sites; see §3.

## 3. Design Overview

1. **Registry.** `paint_prop()`'s siblings: add an `id_prop()` and a `transform_prop()`
   and put them on the ten records; delete `Paint.transform` and `Ellipse.angle`.
2. **Builders.** `__canvas_paintHeader` takes the transform as a second argument instead
   of reading it from the paint; every per-kind header builder passes its record's
   `transform`. `emit_paint_supported` reads the all-zero test from the record's
   `transform` field rather than the paint's.
3. **Hashing.** `__canvas_hashPaint` drops the transform; `__canvas_hashItem`'s two live
   arms (Text, Group) fold the Text's `transform` and `id`.
4. **The ellipse.** An ellipse turned by θ about its centre `(cx, cy)` is the transform
   `R(θ)` with translation `(cx, cy) − R(θ)·(cx, cy)`. Each of the 28 sites is ported to
   exactly that matrix, so the pixels hold. 172-D then makes the centre the pivot and
   the translation term disappears.
5. **The port.** 148 `transform :=` sites move from the paint to the item; 853
   constructors gain `id := ""` and a `transform`.

**Gate class: behaviour-preserving.** No PNG may change. The ellipse port is the one
place a pixel could move: `cos` and `sin` are now evaluated by the program and passed
through the general transformed path rather than slots 39–40.

**If `ellipses.png` moves:** that is a prediction to correct, not a design failure.
Inspect one ellipse. If the difference is the transformed path's antialiasing against
the dedicated ellipse path, the reference is re-derived only after the four questions
in `AGENTS.md` are answered for `ellipses.png` (introduced by plan-116-E), and the
commit shows the per-pixel delta.

### Rejected alternatives

- **Keeping `Ellipse.angle` beside `transform`.** Rejected by `planning/new_canvas.md`;
  two rotations on one record need an order.
- **Adding `id` in 172-I, where it is first read.** Rejected: that would touch all 853
  constructors a second time. `id` is real state from this letter on — stored, deep
  copied, and part of the item hash — and its reader lands in 172-I.

## Compatibility / Format Impact

- **Added:** `id` and `transform` on ten records.
- **Removed:** `Paint.transform`, `Ellipse.angle`.
- **Unchanged:** the geometry header layout (slots 27–33 and 39–40 keep their meaning;
  39–40 are written as the identity rotation), `ITEM_BLOCK_SIZE`, both shaders, every
  pixel.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Measure, then the registry and builders

- [x] `mod.rs`: add `id_prop()` and `transform_prop()`; add both to the ten records;
      delete `Paint.transform` and `Ellipse.angle`; rewrite the `Transform` record's
      description, which says "an unset `canvas::Paint.transform`".
      → `mfb man canvas types`: `id` first and `transform` last on the ten records; none on `Paint`.
- [x] `helper_geometry.rs`: `__canvas_paintHeader(h, paint, transform)`; update every
      caller; `__canvas_hashPaint` drops the transform; the Text arm of
      `__canvas_hashItem` folds `id` and `transform`.
      → 9 header call sites, 11 builder call sites and 10 hash call sites rewritten (`/tmp/plan-172-A/edit_c1.py`); `hashPaint` keeps folding the transform, now passed in, so the MFBASIC arms hash what they hashed.
- [x] `func_geo_build.rs:emit_paint_supported`: read the six transform fields from the
      record, not the paint.
      → reads `transform` from the item record; `cargo test --test rt_canvas_geo_native` passes (the native path still takes untransformed items).
- [x] `helper_paint_defaults.rs`: `canvas::fill` / `stroke` / `fillStroke` no longer
      zero a transform.
      → the constructor is in `func_fill_stroke.rs`; `transform := __canvas_noTransform()` removed from it.
- [x] `self_update.rs:FIELD_KIND_TABLE` and `field_kinds_gen.py` / `.tsv`.
      → no table row changes (every shape was already `SIZE_VARIES`); the `.tsv` and generator constructors were ported by the codemod; `cargo test --bin mfb field_kind` passes.

Acceptance: the compiler builds and the registry tests pass.
  Check: `cargo test --bin mfb canvas` → pass (est. 4 min).
Commit: 23c119ade

### Phase 2 — Port every site but the ellipses

- [x] Move each of the 148 `transform :=` from the paint expression to its item.
      → 146 measured on entry, 35 of them `Gradient`'s own (which stay). Inline ones moved by `/tmp/plan-172-A/codemod_c.py`; 57 carried by a paint value moved by hand, by `port_paintvar.py`, or by a sub-agent (the two example programs).
- [x] Add `id := ""` and an all-zero `transform` to each constructor that sets none.
      → `codemod_c.py` → `849 constructor(s) in 112 file(s)`; `grep` for a drawn constructor without `id` → the spec example only, fixed by hand.
- [x] Remove `transform` from the 4 `canvas::Paint[...]` built by name.
      → 1 real constructor (`rt_canvas_picture.rs`); the other three matches are prose.
- [x] Regenerate the IR-embedding fixtures' goldens per `.ai/testing-gates.md`.
      → see the gate line under Phase 3.

Acceptance: every canvas suite except the ellipse cases passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_metal --test rt_canvas_damage` → pass apart from tests naming `ellipse`
  (est. 12 min); `git status --porcelain tests/golden/canvas` → empty.
Commit: 23c119ade

### Phase 3 — The ellipses

- [x] Port the 28 `angle :=` sites to the matrix in §3.4.
      → 28 sites: 10 were `angle := 0.0` (dropped), 17 became a rotation about the centre, 1 had both an angle and a transform and was composed by hand (`transformed_shapes_on_black.mfb`).
- [x] Add a case to `tests/canvas/rt_canvas_rasteriser.rs`: an ellipse turned by 0.5
      radians through `transform`, asserting the pixels at the two ends of its long axis.
      → `an_ellipse_is_turned_by_its_transform`: two ends of the long axis, the centre, and a control pixel that only an unturned ellipse covers.
- [x] Update `src/docs/spec/app/06_canvas.md`: every `Paint.transform` becomes the
      shape's `transform`; the ellipse paragraph loses `angle`.
      → 7 mentions rewritten; the ellipse paragraph rewritten; `cargo test --bin mfb spec` passes.

Acceptance: `ellipses.png` matches unregenerated, or §3's four-question procedure was
followed and the commit carries the per-pixel proof.
  Check: `cargo test --test rt_canvas_golden ellipses` → pass (est. 3 min).
Commit: 23c119ade

## Validation Plan

- Tests: the existing suites; one new rasteriser case for a transform-turned ellipse; a
  syntax fixture `tests/syntax/canvas/canvas_paint_transform_removed_invalid/`.
- Coverage check: `emit_paint_supported` is reached only by untransformed items. Confirm
  the native builder still takes them with `cargo test --test rt_canvas_geo_native`
  (est. 3 min) — a native path that silently declines everything would still be green
  on pixels.
- Runtime proof: `tests/golden/canvas/transforms.png` scene, dumped before and after,
  byte-identical.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas types` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None.

## Corrections

- **The three phases landed as one commit.** The tree does not compile between Phase 1
  and Phase 2 (the in-process tests embed canvas programs, and ten of them failed with
  `TYPE_CONSTRUCTOR_ARITY_MISMATCH` after Phase 1 alone), and the ellipse port is the
  same codemod pass as the rest. Phase 1's own check, `cargo test --bin mfb canvas`,
  is `113 passed` on the ported tree.
- **The native item hash could not fold a `String`.** Verified property 1 said it
  "needs no per-field edit". It did: `canvas::itemHash: 'Rectangle.id' has type
  'String', which the structural hash does not fold`. `func_item_hash.rs:fold_field`
  gained a `String` arm (length, then each byte).
- **`__canvas_hashPaint` still folds the transform, passed as an argument.** The task
  said it "drops the transform". Dropping it would have required every MFBASIC hash arm
  to fold the six fields itself; keeping the parameter leaves each arm's hash what it
  was.
- **Counts.** `transform :=`: 146 on entry, not 148, of which 35 are `Gradient`'s own.
  Constructors that gained `id` and `transform`: 849 by the codemod plus one spec
  example by hand, against the 853 predicted. `canvas::Paint[` built by name: 1 real
  site, not 4.
- **57 transforms were carried by a paint value, not written inline**, and each needed
  the item that used the paint found: 10 in the test and registry files (by hand and by
  `/tmp/plan-172-A/port_paintvar.py`), the rest in `examples/canvas-demo` and
  `examples/water` (a sub-agent, both examples built afterwards). `examples/canvas-demo`
  gained a helper, `placed(item, transform)`, because its scenes build an item first
  and position it later. The examples are not pixel-tested; the sub-agent's report
  names the ellipse sites it could not verify by rendering.
- **`canvas_geo_native_matrix.mfb` was restructured, not just ported.** One entry of its
  paint list carried a negative-zero transform, to prove that still builds natively.
  `shapes` now takes the transform as a second argument and `main` calls it once more
  with the negative-zero one, so the same items are built, in a different order.
- **`ellipses.png` moved, as §3 predicted, and was re-derived.** 13 of 576,000 pixels
  differ, each by one step in one or more channels, all on the antialiased edge of the
  two rotated ellipses: ten on `thinTurned` (x 781–818) and three on `hollow`
  (`/tmp/plan-172-A/png_diff.py` lists them). The four answers: the reference was
  added by `71bf744b6` (plan-116-E Phase 4); it protects the ellipse's exact rendering,
  rotated cases included; only `rt_canvas_golden.rs` reads it (two tests, the second
  within GPU tolerance); and it is wrong because it records the output of
  `Ellipse.angle`, which rotated the query point by a double-precision cosine and sine
  in header slots 39–40 — a field the owner removed. A turned ellipse now goes through
  the item transform, whose inverse is stored as float32 bit patterns (slots 27–32), as
  every other transformed shape's already was. No program can produce the old pixels.
  The unrotated ellipses and `tilted` did not move.
- **A regression this letter introduced and fixed: identical re-presents drew a frame
  every time.** `rt_canvas_frame_stats::identical_presents_count_as_presents_not_frames`
  failed with `frames=10`, expected 5. `canvas::present`'s frame skip compares bytes,
  and every item now begins with a string: a string sub-block is `length + 9` bytes, so
  up to seven bytes before the next inlined field were whatever the arena held.
  Measured with `/tmp/plan-172-A/run_skip.sh`: `frames=10` for an `id` of length 0 and
  3, `frames=5` only for length 7. Fixed in
  `builder_collection_layout.rs:emit_build_inlined_record`, which now zeroes the last
  word a sub-block reaches before copying it. That is a change to how **every** record
  with an inlined field is built, so the per-target native goldens across the tree were
  regenerated (`scripts/regen-native-goldens.sh`, then `artifact-gate.sh … all` → 0
  diffs). New test: `identical_presents_are_skipped_whatever_the_id_length`. The same
  weakness was latent before this plan for any item with a string ahead of its paint
  (`canvas::Text`).
- **`tests/cli/cli_canvas_package.rs` read `filled.transform.a`** to prove a paint's
  zero transform; the check is removed with the field (found after the main commit, by
  running the seven non-`tests/canvas` targets that embed canvas programs — all pass).
- **One unit test is red and is not this letter's:**
  `the_syntax_corpus_reproduces_its_goldens_in_process` fails on `resource-state-invalid`
  alone, which is bug-1013 (172-N Corrections). The full `cargo test --bin mfb` run was
  cut off by a 30-minute limit after that one failure; the `canvas` subset is 113 passed.
- **`canvas::Ellipse`'s man example gained a `turnAbout` helper.** A transform pivots
  about the surface origin until 172-D, so tilting an ellipse in place needs a
  translation; written inline it was unreadable.

## Summary

The transform changes owner, not meaning. The one real risk is the ellipse, where a
dedicated rotation path is replaced by the general one; it is isolated in its own phase
so a moved pixel names its cause.
