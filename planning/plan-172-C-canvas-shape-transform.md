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
| plan-172-B complete and archived | `ls planning/plan-172-B-*` → no matches | NOT MET |
| `canvas::Point` is gone | `grep -c 'name: "Point"' src/codegen/builtins/canvas/mod.rs` → 0 | NOT MET |

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

- [ ] `mod.rs`: add `id_prop()` and `transform_prop()`; add both to the ten records;
      delete `Paint.transform` and `Ellipse.angle`; rewrite the `Transform` record's
      description, which says "an unset `canvas::Paint.transform`".
- [ ] `helper_geometry.rs`: `__canvas_paintHeader(h, paint, transform)`; update every
      caller; `__canvas_hashPaint` drops the transform; the Text arm of
      `__canvas_hashItem` folds `id` and `transform`.
- [ ] `func_geo_build.rs:emit_paint_supported`: read the six transform fields from the
      record, not the paint.
- [ ] `helper_paint_defaults.rs`: `canvas::fill` / `stroke` / `fillStroke` no longer
      zero a transform.
- [ ] `self_update.rs:FIELD_KIND_TABLE` and `field_kinds_gen.py` / `.tsv`.

Acceptance: the compiler builds and the registry tests pass.
  Check: `cargo test --bin mfb canvas` → pass (est. 4 min).
Commit: —

### Phase 2 — Port every site but the ellipses

- [ ] Move each of the 148 `transform :=` from the paint expression to its item.
- [ ] Add `id := ""` and an all-zero `transform` to each constructor that sets none.
- [ ] Remove `transform` from the 4 `canvas::Paint[...]` built by name.
- [ ] Regenerate the IR-embedding fixtures' goldens per `.ai/testing-gates.md`.

Acceptance: every canvas suite except the ellipse cases passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser --test
  rt_canvas_metal --test rt_canvas_damage` → pass apart from tests naming `ellipse`
  (est. 12 min); `git status --porcelain tests/golden/canvas` → empty.
Commit: —

### Phase 3 — The ellipses

- [ ] Port the 28 `angle :=` sites to the matrix in §3.4.
- [ ] Add a case to `tests/canvas/rt_canvas_rasteriser.rs`: an ellipse turned by 0.5
      radians through `transform`, asserting the pixels at the two ends of its long axis.
- [ ] Update `src/docs/spec/app/06_canvas.md`: every `Paint.transform` becomes the
      shape's `transform`; the ellipse paragraph loses `angle`.

Acceptance: `ellipses.png` matches unregenerated, or §3's four-question procedure was
followed and the commit carries the per-pixel proof.
  Check: `cargo test --test rt_canvas_golden ellipses` → pass (est. 3 min).
Commit: —

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

<Filled in DURING execution.>

## Summary

The transform changes owner, not meaning. The one real risk is the ellipse, where a
dedicated rotation path is replaced by the general one; it is isolated in its own phase
so a moved pixel names its cause.
