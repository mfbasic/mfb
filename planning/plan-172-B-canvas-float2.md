# plan-172-B: `canvas::Point` becomes `vector::Float2`

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-A

Every canvas coordinate pair becomes a `vector::Float2`, and `canvas::Point` is removed.
`vector::Float2` is a value record of two `Float`s named `x` and `y`, the same field
names and the same 16 bytes, so this is a rename of a type and nothing else.

Behavioral outcome: a canvas program writes `vector::Float2[x := 1.0, y := 2.0]` wherever
it wrote `canvas::Point[...]`, `canvas::Point` is an unknown type, and every canvas
pixel test passes unchanged.

References:

- `planning/new_canvas.md` — "Remove canvas::Point replace with vector::Float2."
- `planning/plan-172-A-canvas-shapes-premises.md` §4 P3 — whether `canvas` importing
  `vector` needs a late injection pass, and what it costs.
- `src/codegen/builtins/canvas/mod.rs` — the seven `Point`-typed props.
- `src/codegen/builtins/canvas/func_poll_mouse.rs` — the hand-built `MouseEvent` block.
- `.ai/resources-packages.md` — builtin-package authoring seams, for the import.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-A complete and archived | `ls planning/plan-172-A-*` → no matches | NOT MET |
| Premise P3 has a verdict | `grep -c 'P3.*\(CONFIRMED\|FALSIFIED\)' planning/completed/plan-172-A-*.md` → ≥ 1 | NOT MET |

If plan-172-A is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::Point` is not a type: `canvas::Point[x := 0.0, y := 0.0]` fails to compile.
- `Gradient.startPoint`, `Gradient.endPoint`, `Gradient.focalOffset`, `Paint.clip`,
  `Polygon.contours`, `PolyLine.points` and `MouseEvent.position` are `vector::Float2`
  (or lists of it).
- Every canvas test, scene, example, tool and script in the tree compiles and passes.

### Non-goals (explicit constraints)

- **No field is added, removed or renamed in this letter** other than the type of the
  seven props. `Circle.x` / `y` and the rest stay scalars until 172-D.
- **No pixel changes.** The eight PNG goldens and every pixel assertion hold as they are.
- **`vector` gains nothing.** No function or type is added to it for canvas's benefit.
- **`term::` keeps its own mouse types.** Only `canvas::MouseEvent.position` changes.

## 2. Current State

`canvas::Point` is a registry record of `x` and `y` (`mod.rs`, the first `add_record`).
Seven props name it through `ParameterType::named("Point")`. The companion MFBASIC
source names the type too.

`MouseEvent.position` is an inlined record prop: its slot holds a block-relative offset
and the two floats follow the fixed slots. `func_poll_mouse.rs` builds that layout by
hand (`MOUSE_EVENT_BLOCK_BYTES = 64`, the sub-block at 48 and 56).

Canvas already names another package's record: `color::Color`, through
`color::COLOR_TYPE_ID`, with `"color"` in `pkg.add_imports`. `color` needed a dedicated
late injection pass (`color::augmented_project`). `vector` has no `*_TYPE_ID` constant
and no such pass; premise P3 says whether it needs one.

### Measured populations

| What | Count | Command |
|---|---|---|
| `Point`-typed registry props | 7 | `grep -c 'named("Point")' src/codegen/builtins/canvas/mod.rs` → 7 |
| `Point` in the companion MFBASIC source | 44 in 7 files | `grep -rhoE '\b(AS\|OF) Point\b\|\bPoint\[' src/codegen/builtins/canvas \| wc -l` → 44 |
| `canvas::Point` in `src` | 16 | `grep -rhoE 'canvas::Point\b' src \| wc -l` → 16 |
| `canvas::Point` in `tests` | 491 | same, `tests` → 491 |
| `canvas::Point` in `examples` | 202 | same, `examples` → 202 |
| `canvas::Point` in `tools` | 18 | same, `tools` → 18 |
| `canvas::Point` in `scripts` | 32 | same, `scripts` → 32 |
| Golden files that name `canvas::Point` / `canvas.Point` | 4 | `grep -lE 'canvas[.:]+Point' tests/syntax/app/app-mouse-surface/golden/* tests/syntax/resources/loop-body-resource-reference-valid/golden/* \| wc -l` → 4 |

### Verified properties

- **`vector::Float2` and `canvas::Point` have the same layout.** VERIFIED by reading
  `src/codegen/builtins/vector/mod.rs:add_records` (props `x`, `y`, both `Float`) against
  the `Point` record in `canvas/mod.rs`.
- **`canvas_types_are_builtin_types` names `Point`.** VERIFIED: its list is
  `DrawItem, DrawLayer, Paint, Point, Size, …`. See §3 for the four answers.
- **UNVERIFIED — that a cross-package record prop inlines exactly as a same-package one
  does.** That is premise P3's `MouseEvent` read.

## 3. Design Overview

Three pieces, in dependency order: the import, the type swap, the port.

1. **The import.** `"vector"` joins `pkg.add_imports`; a `FLOAT2_TYPE_ID` constant is
   added to `vector/mod.rs` on the `COLOR_TYPE_ID` precedent. If P3 was FALSIFIED the
   late injection pass is wired here first.
2. **The type swap.** The seven props, the companion source's 44 mentions, and the
   `Point` record's removal. `func_poll_mouse.rs` keeps its 64-byte block: the sub-block
   is two floats either way.
3. **The port.** Every `canvas::Point` in the tree becomes `vector::Float2`, and every
   file that gains the name gains `IMPORT vector` if it lacks it.

This is a rename, so a codemod does the bulk: `canvas::Point` → `vector::Float2` is a
pure token replacement. The judgment is per file — whether `IMPORT vector` is present.

**Gate class: behaviour-preserving, proved by the pixel suites.** Codegen changes (a
type id moves packages), so the two IR-embedding fixtures are EXPECTED to diff and are
regenerated. No PNG may change.

**The frozen test.** `canvas_types_are_builtin_types` (introduced by `d0db90709`,
plan-98-B Phase 1) protects that the named canvas types resolve as builtin types. Its
list contains `Point`. The proof that entry is wrong is the owner's decision in
`planning/new_canvas.md` to remove the type; nothing else depends on the entry
(`grep -rn '"Point"' src/codegen/builtins/canvas/mod.rs`). Only that one entry is
removed, and `Size` and the others stay.

### Rejected alternatives

- **Keeping `canvas::Point` as an alias of `vector::Float2`.** Rejected: the language
  has no type alias for records, and two spellings of one type is the state this letter
  exists to end.
- **Folding this into the per-family letters D–F.** Rejected: `Paint.clip` and the
  gradient points are on every family, so the type would be half-migrated for four
  letters.

## Compatibility / Format Impact

- **Removed:** `canvas::Point`.
- **Changed type:** the seven props above.
- **Unchanged:** the `MouseEvent` block size and slot offsets, every geometry record,
  every pixel.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — The import

- [ ] If P3 was FALSIFIED: add `vector::augmented_project` on the `color` precedent in
      `src/ir/lower.rs` and `src/resolver/mod.rs`, and the generic-injection skip in
      `src/codegen/registry/mod.rs`.
- [ ] Add `pub(crate) const FLOAT2_TYPE_ID: &str = "vector.Float2"` to
      `src/codegen/builtins/vector/mod.rs`.
- [ ] Add `"vector"` to `pkg.add_imports` in `src/codegen/builtins/canvas/mod.rs`, and
      extend the import-cost comment there with the measured byte cost from P3.

Acceptance: a canvas program that names no `vector::` type still builds and runs, and
its `.ncode` grew by the amount P3 recorded.
  Check: `cargo test --test rt_canvas_debug_hooks` → pass (est. 3 min).
Commit: —

### Phase 2 — The type swap and the port

One phase, because the tree does not compile between the two halves.

- [ ] Retype the seven props in `mod.rs` to `ParameterType::named(vector::FLOAT2_TYPE_ID)`
      and delete the `Point` record.
- [ ] Replace the 44 `Point` mentions in the companion source
      (`src/codegen/builtins/canvas/*.rs`) with `vector::Float2`.
- [ ] Remove the `"Point"` entry from `canvas_types_are_builtin_types`, citing §3 in the
      commit message.
- [ ] Update the test-only `FIELD_KIND_TABLE`
      (`src/codegen/collection/assign/self_update.rs`) and regenerate
      `tests/runtime/inplace_self_update/field_kinds.tsv` from `field_kinds_gen.py`.
- [ ] Port `tests/` (491), `examples/` (202), `tools/` (18), `scripts/` (32) and the
      registry example strings in `src/` (16): replace the token, add `IMPORT vector`
      where missing.
- [ ] Regenerate the two IR-embedding fixtures' goldens with
      `scripts/sync-goldens.sh` and `scripts/regen-native-goldens.sh` per
      `.ai/testing-gates.md`, after the full canvas suites are green.
- [ ] Add `tests/syntax/canvas/canvas_point_removed_invalid/` — a program naming
      `canvas::Point`, with its `build.log` golden, on the
      `canvas_color_type_removed_invalid` precedent.
- [ ] Update `src/docs/spec/app/06_canvas.md` and
      `src/docs/spec/language/18_builtin-functions.md` where they name `canvas::Point`.

Acceptance: no `canvas::Point` remains in the tree outside the removed-type fixture, and
every canvas pixel test passes with no PNG regenerated.
  Check: `grep -rlE 'canvas::Point\b' src tests examples tools scripts | grep -vc
  canvas_point_removed_invalid` → 0, and `cargo test --test rt_canvas_golden --test
  rt_canvas_rasteriser --test rt_canvas_picture --test rt_canvas_font` → pass
  (est. 10 min), and `git status --porcelain tests/golden/canvas` → empty.
Commit: —

## Validation Plan

- Tests: the existing `rt_canvas_*` suites, unchanged in what they assert; one new
  removed-type syntax fixture.
- Coverage check: the changed code is the registry and the hand-built `MouseEvent`
  block. `tests/syntax/app/app-mouse-surface` covers the block; confirm it is in the
  artifact gate's set with `scripts/artifact-gate.sh target/release/mfb canvas`
  (est. 1 min).
- Runtime proof: `examples/weather` (the heaviest `canvas::Point` user, 16 constructors)
  built and run headless with `MFB_CANVAS_SYNC=1` and `MFB_CANVAS_DUMP`, the dump
  byte-identical to one taken at the letter's starting commit.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `mfb man canvas types` rendered and read.
- Final gate: none in this letter. The plan's full gates run once, in 172-M.

## Open Decisions

None. P3 decides the only fork, in 172-A.

## Corrections

<Filled in DURING execution.>

## Summary

A type rename across 759 mentions. The risk is not the rename; it is the import, which
is why P3 probes it in 172-A and Phase 1 lands it alone before a single call site
moves.
