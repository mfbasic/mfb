# plan-145-G: Module-level records (S5)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-F

Prerequisites: see plan-145-A.

All 344 expressible S5 cells are `n (StoreGlobal)`, including the 10 rows that
have a record arm (findings §3.2 item 5). plan-142-H taught `NirOp::StoreGlobal`
(`builder_control.rs:1076`) to reach the seam, but only for `g = f(g, …)` and `&`
chains (`is_global_self_update_call`, `self_update.rs:262`). A `WITH` is neither,
so `gR = WITH gR { … }` always rebuilds the record through `lower_with_update`,
then frees the old block (`store_global_old`/`store_global_new`). plan-142-I
measured a global record's last-field update at 70,099 ns.

After G, a global record's field self-update runs the same field site as a local's:

- scalar stores (C);
- arms (B, D, E);
- fixed-size overwrite and nested paths (F).

The owner's block pointer comes from the global's slot.

## 1. Goal

- `field_expect.tsv` and `field_kinds.tsv`: every S5 line matches its S4
  verdict. Check with `diff <(cut -f1,3 of S4 lines) <(… S5 lines)` → empty,
  command recorded.
- Value-semantics cases, the plan-142-H set applied to a record:
  - `LET y = gR` taken before the update is unchanged after it;
  - `f(gR)` where `f` updates `gR` (bug-665's shape) prints the entry value;
  - `FOR EACH v IN gR.xs` whose body updates `gR.xs` (bug-666's shape) visits the
    entry elements;
  - a failed update inside `TRAP` leaves `gR` unchanged.
- Timing (recorded, not a gate): the plan-142-I global record last-field loop,
  beside its 70,099 ns.

### Non-goals

- A `STATE` handle cannot be module-level (findings §2b). There is no global
  `STATE` site.
- `StoreGlobal` for `gR = other` is unchanged.

## 2. Current State

- plan-142-H's `InPlaceDest::Global { name, block_slot }` loads the global's block
  pointer into a scratch slot and stores it back after the arm
  (`open_inplace_ref_dest`/`close_inplace_dest`, `inplace_dest.rs:640`, `:666`).
- `G-global-operand` (`resolve_self_update`) declines when an operand or the
  call itself can store to the global (`values_reach_store`/`call_reaches_store`,
  `store_reach.rs:79`, `:100`, with `StoreLeaf::Global`).
- bug-665 and bug-666 are fixed (plan-142-A prerequisites), so no parameter or
  loop holds a raw pointer into a global's block across a write.

## 3. Design

- `InPlaceDest::Inlined`'s `write_back` becomes an enum:
  `WriteBack::{None, State(StateWriteBack), Global(name)}`. For `Global`, the
  site builder loads the record pointer from the global into `block_slot`. The
  arm or `InlineGrow` may repoint it, and `close_inplace_dest` stores it back into
  the global. The load happens after the gates, in `open_inplace_dest`, so a
  declined statement emits nothing (the same rule letter B used for `STATE`).
- `StoreGlobal`: add `is_global_field_self_update(value, name)`, true when the
  value is a `WithUpdate` whose target is `Global(name)`. It builds the field
  site with the `Global` holder and tries C's scalar/pointer stores, F's overwrite,
  then the seam.
- `G-global-operand` applies to every update value in the `WITH`, not only the
  arm's call. So does a scalar store, because an operand that writes `gR` would
  have its write discarded by the `WITH` semantics.
- A scalar store to a global needs no scratch slot. It loads the record pointer,
  stores the field, and nothing moves.

Risk: aliasing. The global's block is reachable from every function. The bug-665
and bug-666 guarantees, plus `G-global-operand`, are what make an in-place write
sound. The four value cases above are their tests.

## Phases

### Phase 1: The global holder

- [x] `WriteBack` enum; `open_inplace_dest`/`close_inplace_dest` for
      `WriteBack::Global`.
      `WriteBack::{None, State, Global}` on `InPlaceDest::Inlined`; the unopened
      `InPlaceDest::GlobalField` (as `StateField`) is opened after the gates —
      the global's block pointer loaded into a working slot — and closed by
      storing it back. `FieldContainer::Global` is the owner
      (`emit_field_owner_block` loads the global's slot).
- [x] `StoreGlobal`: `is_global_field_self_update` and the field-site path.
      `StoreGlobal` tries the store routine (`try_inplace_scalar_fields` —
      C's stores, F's overwrite and nested paths), then the field seam
      (`field_self_update_site` with the `Global` container, `GlobalField`
      destination), then the mixed `WITH`, before plan-142-H's plain arm. The
      recognition is `field_owner_is`'s `Global` case (a `WITH` whose target is
      the global) rather than a separate `is_global_field_self_update` (Correction
      G1). The self-update scratch prescan and the data-object prescan see it
      through `with_holds_field_self_update` (Correction G2).
- [x] `G-global-operand` over every update value.
      The store routine and the mixed `WITH` decline when any update value can
      reach a store to the global (`values_reach_store`, `StoreLeaf::Global`);
      the seam's `resolve_self_update` applies plan-142-H's gate (operands and the
      call) to a global field site as to a plain global.
- [x] Runtime cases (`tests/runtime/rt_inplace_global_record.rs` + stanza): the
      four in §1.
      Plus the operand that writes `gR` (the RED proof's case) and an in-place
      measurement (a scalar store and an `append` per run: `N` more runs allocate
      next to nothing more). The global's own final block stays live until exit,
      as for every global (Correction G3). `MFB_TEST_EXE=target/debug/mfb cargo
      test --test rt_inplace_global_record` → "2 passed"; on the plan-145-C
      compiler the values pass (the rebuild) and the in-place test fails ("2000
      more runs allocated 8000 more blocks — a rebuild").
- [x] Flip every S5 line. Remove the S5 `FIELD_PENDING` entries.
      `LANDED += G`: 52 `field_expect.tsv` lines → `arm` at S5; `field_kinds.tsv`
      S5 → `arm` for the scalar, pointer and fixed kinds (41 lines); 25
      `GLOBAL` entries removed. `cargo test --bin mfb self_update` → "6 passed".
- [x] RED proof: skip `G-global-operand` for a scalar update whose operand calls a
      `SUB` that writes `gR`, and confirm the value case fails. Restore.
      In a copy of the tree under `/tmp`, the store routine's `Global` gate
      removed: `a_global_record_keeps_value_semantics` → FAILED, `operand 9 n=1`
      where `WITH` semantics give `operand 8 n=1` (the operand's append survived).
      The worktree was never changed.

Acceptance: `cargo test --test rt_inplace_global_record` passes, and
`MFB_SELF_UPDATE_SITES=S5 cargo test --test rt_inplace_self_update` passes (est.
6 min).
`MFB_TEST_EXE=target/release/mfb MFB_SELF_UPDATE_SITES=S5,S6,T6 cargo test --test
rt_inplace_self_update` (the compiler at `ea14b83e6`'s source before plan-145-F
Correction F6): every S5 line and kind passed; the run's only 5 failures were
`json::` kinds at S6/T6 (letter F's, fixed by F6). `cargo test --test
rt_inplace_global_record` → "2 passed".
Commit: `ea14b83e6`

### Phase 2: Goldens and timing

- [x] Run `scripts/artifact-gate.sh target/release/mfb all` once. Every diff must
      be a global-record fixture: objdump one per directory, then regenerate the
      confirmed ones. Record the count.
      Run once for G and the letters after it, with the compiler at H/I's source
      (`1ab6c7d0f` + I's test-only edits): `artifact-gate [all]: 1485 tests, 1660
      build(s), 2098 golden(s) checked, 0 diff(s)` — no golden holds a global
      record's field self-update, so the count is 0 and nothing was regenerated
      (Correction G4). The runtime tests above are the coverage.
- [x] The timing loop, recorded here.
      plan-141's `/tmp/inplace_probe` (a copy under `/tmp/p145/tprobe`), built by
      the compiler at the H/I source and by the plan-145-C compiler, same machine,
      ns/op: global record last field **9** (C: 69,223; plan-142-I: 70,099);
      local record first field 9 (C: 70,374 — D's not-last field); local record
      last field 8 (C: 8); global `List` set 8, global `Map` set 51 (C: 8, 47).

Acceptance: the artifact gate is clean after regeneration (est. 20 min: global
records appear in fixtures across directories, and only the full gate finds them
all).
Clean (above); timing recorded.
Commit: `3e542ca19`

## Validation Plan

- Tests above. Per-letter unit gate: `cargo test --bin mfb` — run at plan-145-I's
  full gate (plan-145-D Correction D6).

## Corrections

- **G1 — no separate recognizer.** `FieldContainer::Global` makes the existing
  field-site builders (`peel_field_path`, `field_self_update_site`) recognize
  `gR = WITH gR { … }` through `field_owner_is`, so the plan's
  `is_global_field_self_update` was not needed.
- **G2 — the prescans had to see a global owner.** `same_field_owner` compared
  only locals and member chains, so a global field self-update got no
  self-update scratch and, for `replace`, no `ErrIndexOutOfRange` data object —
  the matrix failed "native code string literal … has no data object while
  lowering store global gR". It now compares globals too.
- **G4 — no golden covers a global record update.** The gate's 0 diffs mean no
  fixture's `.ncodesum` contains `gR = WITH gR { … }` (as plan-121-D found for
  `STATE` collections); the plan's expected diffs did not exist.
- **G3 — a global's final value is live at exit.** A module-level value has no
  scope drop (plan-142-H's global tests measure growth for that reason), so the
  runtime cases require `alloc_calls = free_calls + 1` — the global's own block —
  and nothing else live.

## Summary

A global record is a field site whose owner pointer lives in a global slot. The
seam, C and F apply unchanged. The aliasing proof is plan-142-H's, extended to
every value in the `WITH`.
