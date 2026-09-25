# plan-156-D: threads in records, lists and maps; transferring the parent view; the two new errors

Last updated: 2026-09-24
Effort: large (3h–1d)
Depends on: plan-156-C

With threads as resources (C), the §15.6 rules apply to them:
- A `Thread` may sit in a `RES` record field, a `List OF RES Thread…` element
  or a map value, as a pointer (never an owner).
- A record holding one is not copyable. That fixes bug-692.
- The parent view becomes thread-sendable, so it can be `thread::transfer`red
  to another thread, but not to its own worker. Two new compile errors and one
  runtime error enforce the limits.

**Checkable outcome:**
- A worker pool builds and runs: `MUT pool AS List OF RES Thread OF Integer TO Integer`
  with 8 workers, each `waitFor`ed through the list.
- bug-692's reproduction is a compile error at the copy.
- A parent view transferred to a coordinator thread is `waitFor`ed there.
- `thread::transfer(t, t)` is a compile error.
- Transferring a view to its own worker through an alias fails at runtime with
  the new error.

References: spec §15.6, §16 (resource planes); bug-692;
`src/ir/resource_escape.rs`; plan-156-A (settled semantics).

## Prerequisites

See plan-156-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-156-C complete | `ls planning/plan-156-C-*` → no match | NOT MET |

## 1. Goal

- Everything in the checkable outcome above holds.

### Non-goals

- A `ThreadWorker` stays non-storable outside its entry function's scope and
  non-transferable.
- No STATE (E).
- `LET`/`MUT` threads are still accepted (F).

## 2. Current State

- Collections reject threads:
  `src/ir/verify/values.rs:check_collection_element_thread_free` /
  `check_collection_element_ownership` → `TYPE_COLLECTION_OWNERSHIP_VIOLATION`.
- Record fields are not checked:
  `src/ir/verify/types.rs:check_type_declarations` never rejects a
  `ThreadHandle` field. That is bug-692.
- Transfer of a `Thread` is rejected through
  `resources.rs:thread_unsendable_cause` → `TYPE_THREAD_NOT_SENDABLE`.
- Resource-plane moves use `copy_resource_to_current_arena`
  (`builder_arena_transfer.rs`) and mark the source moved
  (`emit_flag_resource_source_moved`, bug-425).
- §15.6 escape analysis: `src/ir/resource_escape.rs`. Its soundness layer 3
  relies on the closed flag at offset 16, which C's view records now have.
- Rule codes: the next free one is `2-203-0140` (`grep 'code: "2-203-' src/rules/table.rs | sort | tail`).
  The next free runtime `7-703` code is `0011`
  (`src/docs/spec/diagnostics/02_error-codes.md`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Verifier tests pinning the collection ban and unsendability | 8: `rejects_a_thread_handle_as_a_list_element`, `…_as_a_map_value`, `rejects_a_thread_carrying_union_as_a_map_key`, `rejects_map_key_thread_ownership`, `rejects_unsendable_thread_message_in_a_record_field`, `unsendable_record`, `func_and_thread_handle_planes_keep_the_generic_unsendable_rule`, `cause_walk_reports_func_and_thread_handle_as_other_not_resource` | census 2026-09-24 in `src/ir/verify/tests.rs` |
| Fixtures with a Thread record field | 2 (`syntax/threads/func_thread_{start,send}_invalid`) | same census |

## 3. Design

- **Storage.** A `ThreadHandle` (worker = false) takes the resource path in
  `check_collection_element_ownership` and `res_axis_slot`: `RES` is required
  on the field, element or value, and a bare field is `TYPE_RESOURCE_REQUIRES_RES`.
  A record with a `RES` field is not copyable (existing §15.6 rule).
- **Collection drop.** `collection_resource_drop` / `resource_cleanup_symbol`
  learn `thread.__drop` (C).
- **Sendability.** The parent view is thread-sendable. Transfer copies the
  96-byte view record into the receiver's arena
  (`copy_resource_to_current_arena`); the control-block pointer carries over,
  and nothing in the block depends on which thread owns the parent view
  (plan-156-A §4).
- **`TYPE_THREAD_WORKER_NOT_TRANSFERABLE` (2-203-0141).** Any
  `thread::transfer` whose resource argument is a `ThreadWorker`. Also add a
  `ThreadWorker` in a resource-plane type (`Thread OF RES ThreadWorker…`).
- **`TYPE_THREAD_TRANSFER_TO_OWN_WORKER` (2-203-0140).** Rejects
  `thread::transfer(t, t)` and the same binding through a provable alias.
- **`ErrThreadTransferToSelf` (7-703-0011), runtime.** `transferResource`
  compares the destination's control block with the transferred view's block
  and fails with this code. It covers unprovable aliases and routes through a
  third thread. Added to `02_error-codes.md`, which is build input for the
  `errorCode::` constants.

Correctness risk: the pointer-storage rules must keep "owned by the outermost
scope that touches it". A pool list is a pointer holder, and its elements are
closed by their owning scope. Reuse the §15.6 tests' shapes for threads.

## Phases

> Keep checkboxes current (see plan-156-A's note). **An unticked box means NOT
> DONE.**

### Phase 1 — failing tests

- [ ] `tests/rt-behavior/threads/thread-pool-list/` (8 workers in a
      `List OF RES Thread`), `thread-transfer-parent-view/` (a coordinator
      thread waits a transferred view), and `bug692-record-thread-copy/`
      (expects the compile error).
- [ ] `tests/syntax/threads/thread_transfer_self_invalid/` and
      `thread_worker_transfer_invalid/` for the two rule codes.
- [ ] `tests/rt-error/threads/thread-transfer-to-own-worker-alias-rt/`:
      expects `ErrThreadTransferToSelf`.

Acceptance: all fail for the documented reason (today's rejections or
successes).
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/threads/thread-pool-list' 'rt-behavior/threads/thread-transfer-parent-view' 'rt-behavior/threads/bug692-*' 'rt-error/threads/thread-transfer-to-own-*'`
  → 4 failures; the syntax fixtures are compared by hand (sync-goldens skips
  `syntax/**`) (est. 2 min).
Commit: —

### Phase 2 — storage and bug-692

- [ ] The verifier storage rules (§3); the verifier unit tests for field,
      element and value, and record-copy rejection.
- [ ] Re-baseline the 8 verifier tests. The list-element and map-value
      rejections change to "requires `RES`"; the unsendable ones keep their
      rule for `ThreadWorker`. Proof: §15.6, plus the design settled in
      plan-156-A.
- [ ] Update the two `func_thread_{start,send}_invalid` fixtures' expected
      diagnostics.
- [ ] Collection and record drop of thread elements through `thread.__drop`.

Acceptance: `thread-pool-list` and `bug692-*` pass.
  Check: `cargo test --bin mfb verify` → pass (est. 3 min), and the two
  fixtures via `test-accept.sh` → pass (est. 1 min).
Commit: —

### Phase 3 — transfer and the three errors

- [ ] Parent-view sendability; the transfer and accept lowering for
      `ThreadHandle`.
- [ ] The two rule codes: `src/rules/table.rs` and
      `src/docs/spec/diagnostics/01_rule-codes.md`.
- [ ] The runtime code in `02_error-codes.md`, and the check in
      `transferResource`.

Acceptance: all Phase 1 tests pass.
  Check: the Phase 1 `test-accept.sh` command → 0 failures (est. 2 min);
  `cargo test errorcode` → pass (the error table changed) (est. 2 min).
Commit: —

## Validation Plan

- Tests: the Phase 1 fixtures and the verifier unit tests.
- Runtime proof: the pool fixture under a `--debug` build shows parent-arena
  live bytes back at baseline after the pool's scope ends.
- Doc sync: `01_rule-codes.md` and `02_error-codes.md` (build input). The
  §15.6 and §16 prose is written in F.
- Final gate: at the end of F.

## Corrections

## Summary

Opens storage and transfer on top of C. The risk is the pointer-storage
ownership rules applied to a handle whose drop cancels a worker: a pool
element dropped by the wrong scope would cancel a live worker.
