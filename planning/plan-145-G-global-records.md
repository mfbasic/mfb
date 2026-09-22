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

- [ ] `WriteBack` enum; `open_inplace_dest`/`close_inplace_dest` for
      `WriteBack::Global`.
- [ ] `StoreGlobal`: `is_global_field_self_update` and the field-site path.
- [ ] `G-global-operand` over every update value.
- [ ] Runtime cases (`tests/runtime/rt_inplace_global_record.rs` + stanza): the
      four in §1.
- [ ] Flip every S5 line. Remove the S5 `FIELD_PENDING` entries.
- [ ] RED proof: skip `G-global-operand` for a scalar update whose operand calls a
      `SUB` that writes `gR`, and confirm the value case fails. Restore.

Acceptance: `cargo test --test rt_inplace_global_record` passes, and
`MFB_SELF_UPDATE_SITES=S5 cargo test --test rt_inplace_self_update` passes (est.
6 min).
Commit:

### Phase 2: Goldens and timing

- [ ] Run `scripts/artifact-gate.sh target/release/mfb all` once. Every diff must
      be a global-record fixture: objdump one per directory, then regenerate the
      confirmed ones. Record the count.
- [ ] The timing loop, recorded here.

Acceptance: the artifact gate is clean after regeneration (est. 20 min: global
records appear in fixtures across directories, and only the full gate finds them
all).
Commit:

## Validation Plan

- Tests above. Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

## Summary

A global record is a field site whose owner pointer lives in a global slot. The
seam, C and F apply unchanged. The aliasing proof is plan-142-H's, extended to
every value in the `WITH`.
