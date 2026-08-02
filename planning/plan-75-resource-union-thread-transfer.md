# plan-75: thread::transfer / accept of a resource union

Last updated: 2026-08-02
Effort: medium–large (measure Phase 0 first)
Depends on: plan-74 (union STATE — landed). This plan is the transfer facet plan-74's
Phase 5 turned out to require, and which plan-74 mis-scoped as a one-function STATE deep-copy.

## Why this is its own plan

plan-74 delivered uniform STATE on a resource union at the binding, parameter, return,
`.state` access (value / parameter / MATCH), and scope-drop. Its Phase 5 assumed
`thread::transfer` of a resource union already worked and needed only a STATE deep-copy in
`copy_union_to_current_arena`. **Measured 2026-08-02: resource-union `thread::transfer` /
`thread::accept` does not compile at all** — for a *stateless* resource union as much as a
stateful one. It is an unimplemented feature spanning many layers, each a separate STATE-suffix
or resource-union-classification gap. plan-74's §2.3 "Verified" row only checked that
`copy_union_to_current_arena` raw-copies `{tag,ptr}`; it never compiled an end-to-end transfer.

The type surface works — `ThreadWorker OF RES Stream STATE Cursor TO Integer` and
`thread::accept` returning `RES Stream STATE Cursor` both compile and produce a `.mfp`. The
*consumer* side (transfer) is what fails, in the native lowering.

## Measured gaps (the cascade), in the order the compiler hits them

Reproduce with a worker package exporting
`FUNC take(t AS ThreadWorker OF RES Stream STATE Cursor TO Integer, seed AS String) AS Integer`
and a consumer that `thread::transfer`s a `RES Stream STATE Cursor`. Each gap below blocked the
next; all are keyed on a resource union being spelled `Stream STATE Cursor` (STATE suffix) and/or
being a `{tag,ptr}` pointer handle rather than a data block:

1. **Declared runtime-helpers** — `runtime/usage.rs::push_op_helpers` looks up the union
   close-helper map with the full `Stream STATE Cursor` against a bare-`Stream` key, so a
   transferred union's variant closes are never *declared*, while the validator marks them
   *used* → `NIR runtime call requires undeclared helper 'net'`. Fix: `base_resource_name`.
2. **Transfer copy dispatch** — `builder_arena_transfer.rs::emit_thread_copy_real` routes on
   `union_names.contains(other)`, which fails on the STATE suffix, so a stateful union never
   reaches `copy_union_to_current_arena`. Fix: strip base.
3. **Variant-record deep-copy** — `copy_union_to_current_arena` raw-copies the `{tag,ptr}`
   block only; the `+8` record pointer still aliases the sender's arena (a bug-257-class UAF,
   **true for a stateless resource union too**). The variant's 80-byte record (with its uniform
   STATE) must be deep-copied via `copy_resource_to_current_arena` and the copy's `+8`
   repointed. `union_is_data` / `inline_collection_payload_size` also need the base.
4. **Transfer size / Result payload** — the transfer materializes a `Result OF <resource-union>`
   whose payload classification (`result_payload_is_block`) treats a resource union as an
   inlinable data block (`union_names.contains`), so `emit_inlined_block_size_from_ptr_slot`
   fails: `native inlined field size not available for type 'Stream STATE Cursor'`. A resource
   union is a scalar *pointer* payload like a concrete resource — only a **data** union is a
   block. Fix: `union_is_data(base)`.
5. **Accept side (unexplored)** — `thread::acceptResource` materializing / extracting the
   resource union on the receiver, and the STATE-plane agreement across the boundary, are not
   yet audited; expect further resource-union / STATE-suffix gaps mirroring the send side.

## Prerequisites

| Must be true | Command |
|---|---|
| plan-74 landed (union STATE) | `ls planning/completed/plan-74-*` |
| A stateful **concrete**-resource transfer works (the reference path) | `tests/rt-behavior/threads/thread-transfer-state-rt` is green |

## Phases (sketch — Phase 0 measures before scheduling the rest)

- **Phase 0** — write the failing worker+consumer fixture; confirm each gap above by symbol;
  audit the accept side (`thread.acceptResource`) for the remaining resource-union gaps.
- **Phase 1** — the send-side fixes (gaps 1–4): declared helpers, dispatch, variant-record
  deep-copy (fixes the stateless-union aliasing UAF too — file that as the bug it is), Result
  payload classification.
- **Phase 2** — the accept-side fixes (gap 5) + STATE-plane agreement across the boundary.
- **Phase 3** — the runtime fixture: a worker package under `tools/thread-package-sources/`
  (regenerate its `.mfp` via `scripts/sync-package-mfp.sh` with the **release** build), a
  consumer that transfers a stateful union and asserts the STATE arrives intact (`99`), and a
  20× repeat-run proving sender/receiver payloads are independent (no shared-arena UAF).
  Add a **stateless** resource-union transfer fixture too — it was equally broken.

## Validation

`cargo test --bin mfb`, `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, plus the on-device
thread run. Because `result_payload_is_block` and the transfer copy are hot paths, run the full
acceptance suite — a resource-union classification change can ripple to `Result` handling.

## Corrections

<!-- Filled in during execution. -->
