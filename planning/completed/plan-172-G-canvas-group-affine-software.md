# plan-172-G: `Group` as a full affine — software renderer, damage and hashing

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-F

`canvas::Group` becomes `position`, `transform`, `name`. A group rotates, scales and
shears its contents, and nested groups compose. This letter delivers it on the software
renderer — the oracle — and through the damage diff and the draw hash. While a scene
holds a group with a non-identity transform, both GPU backends decline the frame to
software; 172-H teaches them.

Behavioral outcome: a group with a rotation `transform` draws its items rotated about
the group's `position` on the software renderer, a nested group composes with its
parent, and a group whose `transform` is all-zero draws exactly what a `dx`/`dy` group
drew.

References:

- `planning/new_canvas.md` — "Group transform is a renderer change, not a field rename",
  and "A group composes the same way".
- `.ai/canvas-threading.md` §10 "A group is one instanced draw per node, with an offset
  bound to BOTH stages"; §13 the named-group table; §14 item hashes.
- `src/codegen/builtins/canvas/helper_render.rs` — `__canvas_appendDraw`,
  `__canvas_pushOneDraw`, `__canvas_drawGroup`, `__canvas_sceneDrawsWalk`,
  `__canvas_metalRenderable`, `__canvas_vulkanRenderable`.
- `src/codegen/builtins/canvas/helper_items.rs:__canvas_drawGeometry` — the software
  rasteriser's use of the group offset.
- `src/codegen/builtins/canvas/helper_damage.rs` — the three offset consumers.
- `src/codegen/builtins/canvas/func_geo_cache.rs` — native `canvas::sceneLayout` and
  `canvas::sceneDrawsFlat`.
- `planning/completed/plan-116-G-canvas-groups-storage.md`,
  `plan-116-H-canvas-groups-gpu.md` — the software-first, GPU-second split this letter
  and 172-H repeat.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-F complete and archived | `ls planning/plan-172-F-*` → no matches | MET (2026-10-10) |
| No shape record carries a bare coordinate scalar | `grep -c 'name: "x1"' src/codegen/builtins/canvas/mod.rs` → 0 | MET (2026-10-10) |
| The group suites are green | `cargo test --test rt_canvas_group_ownership --test rt_canvas_damage` → pass | MET (2026-10-10; run first, 11 and 6 passed) |
| The Metal text tests pass on a quiet machine, or `bugs/bug-1024-*` is root-caused (added at 172-F's close: they were red there, on a machine at load average 10–21, with source that had passed an hour earlier) | `cargo test --test rt_canvas_font gpu` → 2 passed | MET (2026-10-10 17:25, twice running; `bugs/bug-1024-*` stays open as an intermittent) |
| The field-kind grid's `STATE`-site expectations are current (`bugs/completed/bug-1023-*`, fallout of plan-172-N found at 172-F's close) | the `T1`–`T8` run in `bugs/completed/bug-1023-*` → `1 passed` (208 pairs) | MET (2026-10-10, `e3ed16804`). The whole grid is `4 of 1155` failed — `http::Route` at four `S` sites, the same at `main`; that is `bugs/bug-1014-*`, not this row. |

If plan-172-F is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::Group` is `position AS vector::Float2`, `transform AS canvas::Transform`,
  `name AS String`.
- An item inside a group reaches the surface as
  `G1(G2(… item placement …))`, each `G` being `T(group.position) ∘ group.transform`.
- A group's transform scales strokes, carries clips and gradients, and obeys the
  all-zero and no-area rules.
- A group whose transform is all-zero renders byte-identically to today's translate-only
  group at the same offset, including the bug-735 sub-pixel residual.
- A frame holding a non-identity group transform is rendered by software on every
  platform, and the stats line says so.

### Non-goals (explicit constraints)

- **No shader, SPIR-V or GPU emitter change.** That is 172-H. The GPU predicates decline;
  they do not draw wrongly.
- **The group table, `setGroup`, `removeGroup`, ownership and the retire-then-drain rule
  are unchanged.** A group's stored items are the same; only the node that references it
  changes.
- **The depth guard stays 64**, raised on the worker in `__canvas_groupSignature`.
- **The draw entry stays a power-of-two word count**, so the emitters keep striding with
  a shift.

## 2. Current State

A `Group` node is `dx`, `dy`, `name`. Two walks expand it, and they accumulate the
offset independently:

- **The software and damage walk**, `__canvas_appendDraw`, recurses with
  `gdx + g.dx, gdy + g.dy` and records one offset per draw entry in the parallel globals
  `__CANVAS_DRAW_DX` / `__CANVAS_DRAW_DY`. It folds the offset into the draw hash.
- **The GPU walk**, `__canvas_sceneDrawsWalk` → `__canvas_drawGroup`, recurses the same
  way and emits eight-word draw entries through `__canvas_pushOneDraw`: base, count,
  `dx` and `dy` as 16.16, blend mode, two residual words, one reserved word. A group's
  blocks are memoised by slot (`__canvas_memoGroup`) and shared by every use; the memo
  holds nothing position-dependent.

The software rasteriser, `__canvas_drawGeometry(…, gdx, gdy)`, adds the offset to the
bounds and subtracts it from the query point, then runs the item's own inverse
transform when `hasTransform` is set. Damage reads the offsets in `__canvas_damageFor`,
`__canvas_boundsMeetOffset` and `__canvas_rememberScene`. Native `canvas::sceneLayout`
copies the offset arrays, and native `canvas::sceneDrawsFlat` writes entries for a
group-free scene.

### Measured populations

`$D` is `src tests examples tools scripts benchmark`.

| What | Count | Command |
|---|---|---|
| `canvas::Group[` sites, all setting `dx` | 101 in 22 files | `grep -rhoE 'canvas::Group\[' $D \| wc -l` → 101 |
| Reads of a group's `dx`/`dy` in the companion source | 8 | `grep -rhoE '\bg\.d[xy]\b' src/codegen/builtins/canvas \| wc -l` → 8 |
| `__CANVAS_DRAW_DX` references | 8 in 3 files | `grep -rho '__CANVAS_DRAW_DX' src/codegen/builtins/canvas \| wc -l` → 8 |
| `CANVAS_DRAW_ENTRY_*` references | 35 in 4 files | `grep -rho 'CANVAS_DRAW_ENTRY_[A-Z]*' src \| wc -l` → 35 |
| Draw entry width today | 8 words, 1 reserved | `grep -n 'const CANVAS_DRAW_ENTRY_WORDS' src/codegen/runtime/canvas/mod.rs` → `= 8` |

### Verified properties

- **The group memo is position-independent.** VERIFIED by reading `__canvas_memoGroup`:
  it is keyed on the slot and stores a block base and count; position arrives as
  arguments to `__canvas_pushDraw`. A matrix can arrive the same way.
- **One reserved word cannot carry a matrix.** VERIFIED: a 2×2 linear part is four
  floats; the entry has one free word. The entry must widen.
- **The item path already handles a general affine.** VERIFIED by reading
  `__canvas_drawGeometry` and `__canvas_boundsHeader`: with `hasTransform` set the query
  point goes through `__canvas_invX` / `__canvas_invY`, distance is rescaled, and bounds
  are the hull of the forward-mapped corners. Composing a group matrix into that path
  reuses all of it.

## 3. Design Overview

**One composed inverse per draw entry.** A draw entry already means "this run of blocks,
at this offset". It now means "this run of blocks, under this affine `A`", where `A` is
the composition of every enclosing group. For an item with its own inverse `I`, the
query point is `I(A⁻¹(p))`. So the rasteriser composes `A⁻¹` with the item's inverse
once per entry and runs the existing transformed path with the result.

**The identity fast path is today's path.** When every enclosing group's transform is
all-zero, `A` is a pure translation. The entry's matrix flag is 0, the rasteriser takes
the existing offset code, and nothing about the frame changes. That is what makes the
existing group goldens hold.

**The draw entry widens to 16 words:**

| Word | Meaning |
|---|---|
| 0–1 | block base, block count (unchanged) |
| 2–3 | translation `x`, `y` as 16.16 (unchanged) |
| 4 | blend mode (unchanged) |
| 5–6 | translation residuals (unchanged) |
| 7 | `hasMatrix`, 0 or 1 (was reserved) |
| 8–11 | `A⁻¹` linear part `ia`, `ib`, `ic`, `id`, as float32 bits |
| 12–15 | reserved, zero |

The translation stays in words 2–3 and 5–6 for both cases, so a translate-only entry is
the first eight words of today's entry followed by zeros.

**The software and damage side** carries the same data in a third parallel global,
`__CANVAS_DRAW_MATRIX` (six floats an entry: the forward affine), beside
`__CANVAS_DRAW_DX` / `_DY`. Bounds for damage are the hull of the item's bounds mapped
through it. The draw hash folds all six.

**Composition** is one function, `__canvas_composeGroup(parent, position, transform)`,
used by both walks, so the software walk and the GPU walk cannot disagree about what a
nested group means.

**The GPU predicates decline.** The walk sets a flag when it emits an entry with
`hasMatrix = 1`; `__canvas_metalRenderable` and `__canvas_vulkanRenderable` return FALSE
when it is set. `.ai/canvas-threading.md` §10 says a predicate cannot find a group by
searching the offsets list, and that a backend needing to decline has to be told by the
walk. This is that flag. 172-H removes it.

**Gate class: new behaviour, with an identity case that must not move.** Expected to
diff: nothing among the existing goldens. `groups.png` holds.

**Where the risk is:** the composed inverse under a non-uniform scale. Distance is
rescaled by the item path's existing estimate, which was written for one affine; two
composed affines are still one affine, so it should hold, and Phase 3's scene has a
group scaled 3× by 0.5× holding a stroked circle to prove it.

### Rejected alternatives

- **Folding the group matrix into each item's own transform at expansion.** Rejected: a
  group's blocks are built once and shared by every use, and a diamond — two parents,
  one child — would need the child's geometry twice.
- **Carrying forward and inverse matrices in the entry.** Rejected: the forward 2×2 is
  the inverse of the inverse, cheap to recompute where it is needed, and carrying both
  invites them to disagree.
- **Declining a transformed-group frame permanently.** Rejected: that is this letter's
  interim state only, exactly as plan-116-G's was before plan-116-H.

## Compatibility / Format Impact

- **Changed:** `canvas::Group`'s fields; `CANVAS_DRAW_ENTRY_WORDS` 8 → 16.
- **Unchanged:** the group table and its slot layout, `ITEM_BLOCK_SIZE`, the geometry
  header, both shaders and both `.spv` blobs, the 16-byte per-draw GPU payload.
- `scripts/regen-spirv.sh` is not run by this letter; a diff in either `.spv` is a bug.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The record and the port, translate-only

Lands the field change with no new behaviour, so the port is checked against unchanged
pixels.

- [x] `mod.rs`: `Group` becomes `position`, `transform`, `name`; rewrite its description
      (it says "drawn translated by `dx`/`dy`").
- [x] `helper_render.rs`: the eight reads of `g.dx` / `g.dy` read `g.position`;
      `__canvas_hashItem`'s Group arm folds `position` and the six transform fields.
- [x] ~~Add `canvas::group(name, position)` with a man example.~~ — moot: Open
      Decision 1 of 172-A was answered "no constructor functions"
      (`grep -c 'ANSWER: no constructor functions' planning/completed/plan-172-A-*.md`
      → 1). `Group`'s description gives the `MUT` + `WITH` form instead.
- [x] Port the 101 sites: `dx`/`dy` become `position`, `transform` all-zero.
- [x] `FIELD_KIND_TABLE` and its `.tsv`; the IR-embedding goldens.

Acceptance: every group test passes with no PNG regenerated.
  Check: `cargo test --test rt_canvas_golden groups --test rt_canvas_group_ownership
  --test rt_canvas_damage` → pass (est. 8 min); `git status --porcelain
  tests/golden/canvas` → empty.
  Observed (2026-10-10, with phase 1 alone applied): `rt_canvas_golden` 23 passed,
  `rt_canvas_group_ownership` 11 passed, `rt_canvas_damage` 6 passed; no PNG changed.
Commit: c9668565f

### Phase 2 — The widened entry, still translate-only

- [x] `src/codegen/runtime/canvas/mod.rs`: `CANVAS_DRAW_ENTRY_WORDS = 16`; add
      `CANVAS_DRAW_ENTRY_HAS_MATRIX` and `CANVAS_DRAW_ENTRY_INVERSE`; update every one of
      the 35 references whose stride or offset depends on the width.
- [x] `helper_render.rs:__canvas_pushOneDraw`, `__canvas_pushDraw`, `__canvas_drawsJoin`:
      write and compare 16 words; two entries join only if their matrix words are equal.
- [x] `func_geo_cache.rs:canvas::sceneDrawsFlat` and `canvas::sceneLayout`: emit and
      copy 16-word entries.
- [x] `src/target/macos_aarch64/app/metal.rs:emit_draw_list_pass` and
      `src/codegen/runtime/canvas/vulkan.rs:emit_draw_list_pass`: stride by 16 words,
      still binding the same 16 bytes.

Acceptance: every canvas suite passes on software and Metal with no PNG regenerated —
the entry is wider and nothing reads the new words yet.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_metal --test
  rt_canvas_graphics_thread` → pass (est. 12 min).
  Observed (2026-10-10, with phases 1 and 2 applied): `rt_canvas_golden` 23 passed,
  `rt_canvas_metal` 48 passed, `rt_canvas_graphics_thread` 13 passed (2 ignored, as
  before); no PNG changed.
Commit: c9668565f

### Phase 3 — The affine, on software (largest blast radius)

- [x] `helper_paint_defaults.rs`: `__canvas_composeGroup` and its unit tests (identity
      under identity; translation under rotation; a no-area group transform treated as
      identity).
- [x] `helper_render.rs`: both walks call it; `__CANVAS_DRAW_MATRIX` is filled beside
      the offset arrays; entries carry `hasMatrix` and `A⁻¹`; the walk sets the
      decline flag; both `*Renderable` predicates read it.
- [x] `helper_items.rs:__canvas_drawGeometry`: when the entry has a matrix, compose
      `A⁻¹` with the item's inverse and take the transformed path, for shapes, text and
      pictures; bounds are the forward hull.
- [x] `helper_damage.rs`: the three consumers map bounds through the entry's affine.
- [x] `helper_render.rs:__canvas_appendDraw`: fold the six matrix floats into the draw
      hash.
- [x] Scene `tests/canvas/scenes/group_affine_on_black.mfb` and golden
      `tests/golden/canvas/group_affine.png`: a rotated group, a group scaled 3× by 0.5×
      holding a stroked circle, a nested rotated group inside a translated one, a group
      holding a gradient-filled item, a `Text` item and a clipped item, and one
      translucent item drawn through a diamond.
- [x] Rasteriser cases: a rotated group's item lands at the hand-computed pixel; a
      group scaled 2× draws a 4-pixel stroke 8 pixels wide; a no-area group transform
      draws as identity.
- [x] Damage case in `tests/canvas/rt_canvas_damage.rs`: changing only a group's
      `transform` repaints, and the damaged frame equals a full redraw byte for byte.
- [x] A stats assertion: a transformed-group scene under `MFB_CANVAS_GPU=1` reports the
      software renderer.
- [x] `src/docs/spec/app/06_canvas.md` "Named groups": replace "A group is translated,
      not transformed" with the composition rule, the widened entry, and the interim
      decline, each cited.

Acceptance: the new golden renders identically on two consecutive runs; `groups.png`
still matches unregenerated; and a damaged frame equals a full redraw.
  Check: `cargo test --test rt_canvas_golden group --test rt_canvas_rasteriser group
  --test rt_canvas_damage` → pass (est. 10 min).
  Observed (2026-10-10): `group_affine_matches_its_reference_exactly` passes, and renders the same on two consecutive runs inside one test; `groups_match_their_reference_exactly` passes against the unregenerated `groups.png`; `rt_canvas_rasteriser group` → 21 passed (five new `group_affine_*`); `rt_canvas_damage` → 7 passed, the new `turning_a_group_repaints_and_matches_a_full_redraw` among them (the damaged frame equals the full redraw); `a_group_with_a_matrix_falls_back_to_the_software_renderer` passes (`gpuFrames=0`). Whole round on the final tree: all 25 canvas-embedding targets green (`rt_canvas_golden` 24, `rt_canvas_rasteriser` 106, `rt_canvas_metal` 49, `rt_canvas_font` 33), `cargo test --bin mfb canvas` 115 passed, `spec` 46 passed. `git status --porcelain tests/golden/canvas` → `?? group_affine.png` only. `scripts/artifact-gate.sh target/release/mfb all` → `2322 golden(s) checked, 0 diff(s)`.
Commit: c9668565f

## Validation Plan

- Tests: one new golden; three rasteriser cases; one damage case; one stats assertion;
  unit tests for the composition; the group ownership suite unchanged.
- Coverage check: the decline flag must be reached. `MFB_CANVAS_STATS` on the new scene
  with `MFB_CANVAS_GPU=1` must report software; without the flag a GPU frame would draw
  every item at its untransformed place and still pass a software-only test.
- Runtime proof: `examples/canvas-demo` built and run headless, its dump byte-identical
  to one taken at the letter's starting commit (it uses translate-only groups).
- Doc sync: `src/docs/spec/app/06_canvas.md`; `.ai/canvas-threading.md` §10's entry
  layout line; `mfb man canvas group` rendered.
- Final gate: none in this letter; 172-M.

## Open Decisions

None.

## Corrections

- **Three phases, one commit**, each verified before the next was applied (the
  observations above were taken phase by phase). The IR-embedding goldens change at
  every phase, so three commits would each have re-baselined the same two fixtures.
- **Two prerequisite rows were added at 172-F's close and settled before this letter**:
  the Metal text tests (`bugs/bug-1024-*`) passed twice running once the machine's
  load fell, and the field-kind grid's `STATE` sites (`bugs/completed/bug-1023-*`) were
  fixed in `e3ed16804`.
- **`canvas::group` is struck** (Open Decision 1 of 172-A); the description carries the
  `MUT` + `WITH` form.
- **No `CANVAS_DRAW_ENTRY_HAS_MATRIX` / `_INVERSE` constants.** Phase 2 names them, but
  nothing native reads words 7–11 in this letter — the GPU emitters are 172-H — and a
  constant with no reader is dead code the build warns about. The layout is stated in
  `CANVAS_DRAW_ENTRY_WORDS`'s doc and written by `__canvas_pushOneDraw`; 172-H adds the
  constants with their readers. `the_draw_entry_width_agrees_with_the_emitter` still
  holds the sixteen appends to the constant.
- **The 35 `CANVAS_DRAW_ENTRY_*` references needed two edits, not 35.** Both emitters
  stride by `CANVAS_DRAW_ENTRY_SHIFT` and count by `_COUNT_SHIFT`, which derive from the
  word count. What changed by hand: `__canvas_pushOneDraw` (eight more appends),
  `__canvas_drawsText` (its stride), and `canvas::sceneDrawsFlat`, which had the width
  as a literal `3` and an eight-element array and now uses the constants.
  `__canvas_drawsJoin` needed no change: it joins two BLOCKS of one run, and a run has
  one matrix by construction — a group node ends the run.
- **The software side carries a matrix NUMBER per entry, not six floats.** §3 says a
  third parallel global `__CANVAS_DRAW_MATRIX`, six floats an entry. `canvas::sceneLayout`
  lays the parallel lists out natively, one word per entry per list; a six-wide list
  would have needed its own strided copy. Instead `__CANVAS_DRAW_MAT` is one Integer an
  entry (0, or a row of `__CANVAS_GROUP_ROWS` plus one), which `sceneLayout` moves with
  the others, and the row holds the linear part forward and inverse. The translation
  stays in `__CANVAS_DRAW_DX` / `_DY` for every entry, so a translate-only entry is
  untouched and the identity path is today's by construction rather than by a flag.
- **`__canvas_composeGroup` is in `helper_render.rs`, takes the parent as translation
  plus matrix number, and answers through two globals.** It writes
  `__CANVAS_GROUP_ROWS`, so it lives with the walks, not in `helper_paint_defaults.rs`.
  MFBASIC returns one value: the matrix number is the result and the composed
  translation is left in `__CANVAS_COMPOSED_X` / `_Y`, which avoids a list allocation
  per group node, on a path both walks take for every node of every frame.
- **Its "unit tests" are compiled-program tests**, as 172-D's and 172-E's were: the
  function is MFBASIC text. `group_affine_*` (five rasteriser cases) put a pixel where
  the composition says and assert it: a turn about the position, a nested node under a
  turned parent (which distinguishes the two composition orders), a scale and its
  stroke, a no-area transform, and `tx`/`ty` adding to `position`.
- **The rasteriser reads the group matrix from globals set per entry**
  (`__canvas_enterGroupMatrix`), not from new parameters: `__canvas_invX` / `_invY` are
  called in five places per pixel and inside two clip helpers. With no matrix they
  evaluate the expression they always did; `rt_canvas_golden transforms` and every
  transformed-item test are unchanged.
- **Three things the design did not list, found while writing the rasteriser:**
  - *A rectangle clip.* Under an item's own transform a clip is built as a path, so the
    rectangle path never met a rotation. Under a group matrix it does: it is evaluated
    like a path clip (`__canvas_clipRectSurface`), and the loop's clip narrowing is
    skipped. The golden's clipped band is this case.
  - *The bounds pad.* A header's antialiasing rim is in the group's space; a group
    scaled to 0.6 would have clipped it. The hull is padded by a pixel after mapping.
  - *A composed affine with no area* draws nothing (the node is skipped), which is not
    the single-node rule (not in effect → translate only). It can only be reached by
    scaling through several nodes until nothing is left.
- **The decline flag is set by the software walk** (`__canvas_appendDraw`), which runs
  on every frame before either predicate, rather than by the GPU walk.
- **Runtime proof.** `examples/canvas-demo`'s first frame is byte-identical
  (`90f2a990…`), but that frame is the title screen, which uses no group. The two
  scenes that do — the spring's parallax layers and the wanderer's level — were run
  headless and looked at; they are animated, so they are not byte-comparable.
- **`mfb man canvas group` does not exist** (no such function); `mfb man canvas types`
  was rendered and shows `Group`'s three fields.

## Summary

The record change, the wider entry and the affine land as three phases so that each has
one thing that can go wrong. The first two must not move a pixel. The third is checked
against the software oracle alone, with the GPU backends told to stand down until 172-H.
