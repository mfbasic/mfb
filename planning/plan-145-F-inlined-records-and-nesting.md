# plan-145-F: Fixed-size inlined record fields, and nested paths

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-E

Prerequisites: see plan-145-A.

Two findings share one mechanism: a field whose value is itself an inlined record.

- **Finding 4 (143 rows).** A package-record field (`vector::*`, `color::Color`,
  the `datetime` types, `big::Int`, `http::Response`) or a nested user record is
  never in place. It is inlined (findings B.4), so Layer 1 refuses it in `STATE`,
  and no arm handles it in a record. A `vector::Float3` field is three scalars, and
  the rebuild writes the same bytes back through a whole new record.
- **Finding 6, `G17` at S6/T6.** In
  `o = WITH o { inner := WITH o.inner { b := op(o.inner.b, …) } }`, the outer
  update is a record, so every arm declines. The field path has depth 2.

After F:

- an inlined record field of **compile-time fixed size** is overwritten in place
  (the new value's bytes are copied over the sub-block);
- a nested path `o.inner.b` is a field site of depth 2 that every field-capable
  arm serves. For a reallocating arm, each level on the path must be last-inlined.

## 1. Goal

- `field_kinds.tsv`: every `InlinedFixed` kind (plan-145-A Phase 1's list) flips
  to `arm+value` at S3, S4, T1 and T2. The nested-record kind flips too.
- `field_expect.tsv`: the S6 and T6 lines flip to match S4/T2's verdict for the
  same row.
- A value check per kind: every field of the vector, colour or date reads back
  what the copying path would give, and a sibling field is untouched.

### Non-goals

- `InlinedVariable` kinds (`big::Int`, `http::Response`, a nested record holding a
  `String`) remain `Rebuild` (plan-145-A Open Decision 3).
- No change to how a package record is laid out.

## 2. Current State

- `record_field_is_inlined` (`builder_collection_layout.rs:3152`): a composite
  that is `type_is_memcpy_copyable` is inlined. That is every package record type
  among the rows (findings B.4).
- The fixed-size list comes from plan-145-A Phase 1. Its count is UNMEASURED until
  then.
- A register-native vector value materializes to a block only where a block is
  needed (`vector_value_as_block`, `builder_control.rs` in `StateAssign`). An
  overwrite can store lanes straight into the sub-block, with no temp block.
  Whether each vector op's result is register-native is UNMEASURED (Phase 1).

## 3. Design

**Fixed-size overwrite.** Handled in the field-site builder before the arms (type
driven, like letter C's scalars). The conditions:

- the field's type is `InlinedFixed`;
- any number of such updates, alongside the scalar updates of letter C.

Evaluate each new value (spill it), then copy `size` bytes into
`record + fieldOffset`. The size is a compile-time constant, so no size read is
needed.

- If the value is a fresh block, free it after the copy. Its memory is the
  `arm+value` allowance.
- If it is register-native, store the lanes directly.

`STATE` owners also take this path. The block pointer comes through
`RESOURCE_OFFSET_STATE`, and nothing moves, so there is no write-back.

**Nested paths.** `FieldSite` gets a `path: Vec<(field_index, record_type)>`
instead of a single index. The site builder peels nested single-update `WITH`s
whose target is the parent path:

- `WITH o.inner { … }` inside `inner := …`;
- `WITH h.state.inner { … }`.

Each level must pass `G13`/`G14`.

- The sub-block address is the sum of the stored offsets along the path.
- `InlineGrow` for a path needs every level last-inlined. It grows the **outer**
  block by the inner growth. Inner offsets are block-relative to their own record,
  so they are unchanged. Only the outer size changes.

`field_off_slot` becomes the summed offset, read once before the grow.

Risk: the summed offset across a grow at depth 2. It must be read once, before the
grow, and every level's last-inlined proof must hold, or a sibling shifts.

## Phases

### Phase 1: Measure

- [x] For each `InlinedFixed` kind, check whether its self-update-shaped ops return
      a register-native value or a block. Build one probe per kind with
      `--ncode`, and grep for the vector lanes vs an `arena_alloc`. Record the
      table.
      The 32 `InlinedFixed` kind lines of `field_kinds.tsv` use three statement
      shapes, so three probes cover them (`-ncode -target linux-x86_64`):

      | kinds | statement | the new value |
      |---|---|---|
      | the 9 `vector::` kinds | `x = vector::max(x, x)` | register-native lanes: the rebuild materializes them (`vector_lane` slots, `vector_value_as_block`) — the overwrite stores the lanes straight into the sub-block, no block |
      | 21 kinds (`astrings::Attr*`, `audio::*`, `canvas::*`, `color::*`, `datetime::*`, `json::JsonNull/Bool/Num`, `term::*`, `KFix`) | `x = kindSame(x)` (`RETURN v`) | a borrow of the argument (`call_returns_param_borrow`): a view of the field itself, so it is copied (one block), copied over the sub-block, and freed |
      | `json::JsonArr`, `json::JsonObj` | `x = kindFresh(x)` | a fresh owned block: claimed from the statement's temps, copied, freed |

      Probe: `/tmp/p145/fvec` (a `vector::Float3` field under `vector::max`: 18
      `vector_lane` slots and the rebuild's `record_build_*`, `with_target` before
      F).

Acceptance: table recorded (est. 15 min).
Recorded above.
Commit: recorded with Phase 2 (below)

### Phase 2: Fixed-size overwrite

- [x] Field-site builder: the `InlinedFixed` overwrite for records and `STATE`.
      In `try_inplace_scalar_fields` (the same routine as letter C's stores):
      `record_field_is_inlined_fixed` admits the field, `spill_field_overwrite`
      evaluates and spills the value first (lanes, or an owned block and its
      size), and after every value is spilled the block copy (or lane stores)
      writes the sub-block and the owned blocks are freed. A swap `WITH r { v :=
      r.w, w := r.v }` reads back swapped (the borrowed sources are copied first).
- [x] Flip the kind lines, and update `FIELD_KIND_TABLE`.
      `field_kinds_gen.py`: `InlinedFixed` → `arm` at S3, S4, S10, T1–T5, T8
      (the routine serves every site C's stores do); regenerated with
      `python3 tests/runtime/inplace_self_update/field_kinds_gen.py
      target/debug/mfb` → 32 lines changed. `FIELD_KIND_TABLE` needed no change:
      its 31 `InlinedFixed` rows were already `Arm('F')` (plan-145-A), and
      `field_kind_census_covers_every_record_field_type` passes.
- [x] RED proof: revert the builder call, and confirm the kind lines fail the
      bound.
      The kind-line harness at the flipped sites (`MFB_SELF_UPDATE_SITES=S3,S4,S10,T1,T2,T3,T4,T5,T8`, `every_field_kind_meets_its_expectation`) is recorded in the next commit.

Acceptance: `MFB_SELF_UPDATE_SITES=S3,S4,T1,T2 cargo test --test rt_inplace_self_update`
filtered to the kind lines → pass (est. 4 min).
The RED run (the plan-145-C compiler, which has no overwrite, over the flipped kind lines at S3, S4, T1, T2) is recorded in the next commit.
Commit: `(recorded in the next commit)`

### Phase 3: Nested paths

- [x] `FieldSite.path`, peeling the nested `WITH`, summed offsets, `InlineGrow`
      over a path.
      `FieldLevel`/`FieldSite::path`; `InPlaceDest::Inlined`/`StateField` carry
      the path's field indices; `peel_field_path` (per level: an inlined
      non-collection record field whose update is a single-update `WITH` over its
      own place); `emit_path_field_offset` sums the stored offsets (an empty path
      is the one load it was); the inline grow helpers take the summed offset
      (`summed_offset`), read once before the grow; `field_is_last_inlined`
      requires every level last-inlined. The scalar/pointer/overwrite routine
      peels the same way and writes the inner record's block
      (`emit_field_record_block`), so the nested scalar kinds flip too.
- [x] Runtime cases (`tests/runtime/rt_inplace_field_nested.rs` + stanza):
  - depth-2 `append` and `filter`;
  - a not-last inner level (asserts a decline);
  - a sibling of `inner` kept intact across the outer grow.
  Two tests: depth-2 `append` + `filter` in a record and a `STATE` payload (the
  sibling `a` and the outer `n` read back intact; `alloc_calls = free_calls`);
  a not-last inner level (`Side.inner` before `tail`) whose `append` rebuilds and
  whose `filter` runs in place. `MFB_TEST_EXE=target/debug/mfb cargo test --test
  rt_inplace_field_nested` → "2 passed"; on the plan-145-C compiler → FAILED
  ("record: 2000 more runs allocated 14000 more blocks — a rebuild").
- [x] Flip S6/T6 in `field_expect.tsv`, and remove the `FIELD_PENDING` entries.
      `LANDED += F`: 104 lines → `arm` at S6/T6; `field_kinds.tsv` S6/T6 → `arm`
      for the scalar, pointer and fixed kinds (41 lines). 25 `FIELD_PENDING`
      `NESTED` entries removed. `cargo test --bin mfb self_update` → "6 passed"
      (the matrix fires every arm at S6 and T6).

Acceptance: `cargo test --test rt_inplace_field_nested` passes, and
`MFB_SELF_UPDATE_SITES=S6,T6 cargo test --test rt_inplace_self_update` passes
(est. 8 min).
The S6/T6 harness (`MFB_SELF_UPDATE_SITES=S6,T6`) is recorded in the next commit.
Commit: `(recorded in the next commit)`

## Validation Plan

- Tests above; differential probe over every `InlinedFixed` kind.
  **Result:** the kind lines' own value check (`check` column, the in-place
  result against the copying path's) is the per-kind differential — all pass at
  every flipped site (Phase 2). `/tmp/p145/fvec` adds a vector field under
  `vector::max`, a swap of two vector fields, a borrowed and a fresh `KFix`, a
  mixed overwrite + scalar, and the `STATE` payload: output identical to the
  plan-145-C compiler's, `alloc_calls = free_calls` (285/285). `/tmp/p145/nest`:
  depth-2 arms, a not-last level, a nested scalar store and the `STATE` path —
  identical output, 158 allocations against 771.
- Goldens: `rt-behavior/vector` and the datetime and colour fixtures whose records
  hold such fields are expected to diff. Run
  `scripts/artifact-gate.sh target/release/mfb 'rt-behavior/*'` once, objdump one
  diff per directory, and regenerate only the confirmed ones.
- Per-letter unit gate: `cargo test --bin mfb`. Run at plan-145-I's full gate
  (plan-145-D Correction D6).

## Corrections

- **F1 — the size is read, not a constant.** The copy's length comes from
  `emit_inlined_block_size_from_ptr_slot` on the new value (the sizer every
  record build uses), which for a fixed-size record always equals its
  compile-time size; it keeps one sizing authority.
- **F2 — bug-678, found by the overwrite probe.** `h.state = r` stored `r`'s own
  block in the resource (`lower_value_stored_field` returns a flat source
  uncopied): a double free, and `r`'s later update changed the payload. Fixed with
  `lower_value_owned`; record in `bugs/completed/bug-678-*`, fixture
  `rt-behavior/resources/state-assign-from-local-copies-valid`.
- **F3 — the scalar routine takes nested paths too.** The S6/T6 kind lines are
  scalar, pointer and fixed stores, not arms; `try_inplace_scalar_fields` peels
  the nested `WITH` (`peel_field_path`) and writes the inner record's block.
- **F4 — the kind lines flip at every site C's stores reach** (S3, S4, S10,
  T1–T5, T8), not only S3, S4, T1, T2: it is the same routine at all of them, and
  the harness ran them all.

## Summary

A fixed-size inlined record field is a byte overwrite, and a nested path is a
field site with a longer path. Every arm reaches both through the same seam.
