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
| plan-156-C complete | `ls planning/plan-156-C-*` → no match | MET (2026-09-25: "no matches found"; archived) |

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

- [x] `tests/rt-behavior/threads/thread-pool-list/` (8 workers in a
      `List OF RES Thread`), `thread-transfer-parent-view/` (a coordinator
      thread waits a transferred view), and `bug692-record-thread-copy/`
      (expects the compile error).
      (Pool: expected `workers 8 total 204`; today
      `TYPE_COLLECTION_OWNERSHIP_VIOLATION` at the `List OF RES Thread`.
      Transfer: expected `coordinator answered 50`; today four
      `TYPE_THREAD_NOT_SENDABLE`. bug-692 split per Correction 1: the runtime
      fixture spells the field `RES` and expects the copy to alias one open
      thread — `5`, `5` — and today fails in codegen ("native runtime call
      'thread.waitFor' has no return type"); the bare-field reproduction is
      `tests/syntax/threads/bug692_record_thread_field_invalid/`, which today
      builds and runs, and must answer `TYPE_RESOURCE_REQUIRES_RES`. Goldens are
      the hand-written `build.log` and `.run`; `.ast`/`.ir` are recorded once
      each program compiles.)
- [x] `tests/syntax/threads/thread_transfer_self_invalid/` and
      `thread_worker_transfer_invalid/` for the two rule codes.
      (Worker: today three `TYPE_THREAD_NOT_SENDABLE` (the channel type at
      lines 11 and 16, the transfer at 17); expected
      `TYPE_THREAD_WORKER_NOT_TRANSFERABLE`. Self: Correction 2 — `transfer(t, t)`
      is `TYPE_CALL_ARGUMENT_MISMATCH` today and stays so; the fixture guards
      that.)
- [x] ~~`tests/rt-error/threads/thread-transfer-to-own-worker-alias-rt/`:
      expects `ErrThreadTransferToSelf`.~~ — moot: no program can transfer a
      thread's view to that thread (Correction 2), so there is nothing for the
      fixture to run.

Acceptance: all fail for the documented reason (today's rejections or
successes).
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/threads/thread-pool-list' 'rt-behavior/threads/thread-transfer-parent-view' 'rt-behavior/threads/bug692-*' 'rt-error/threads/thread-transfer-to-own-*'`
  → 4 failures; the syntax fixtures are compared by hand (sync-goldens skips
  `syntax/**`) (est. 2 min).
  Result: "acceptance tests failed: 5 mismatch(es) (3 test(s) ran)" — the three
  runtime fixtures (the rt-error one is moot), each for the reason above.
Commit: 008c91e1e

### Phase 2 — storage and bug-692

- [x] The verifier storage rules (§3); the verifier unit tests for field,
      element and value, and record-copy rejection.
      (`contains_thread` → `contains_thread_worker` and
      `check_collection_element_thread_free` → `…_worker_free`: only a
      `ThreadWorker` is still refused as an element or map value; a parent
      `Thread` now takes `res_axis_slot`'s marker rules — C's exemption there is
      removed — so a bare one is `TYPE_RESOURCE_REQUIRES_RES` and a `RES` one is
      legal, in a record field too. Record copy is NOT rejected (Correction 1).
      New tests: `accepts_res_thread_elements_values_and_fields`,
      `rejects_a_bare_thread_record_field`, `rejects_a_thread_worker_as_a_list_element`.
      `cargo test --release --bin mfb verify` → 490 passed.)
- [x] Re-baseline the 8 verifier tests. The list-element and map-value
      rejections change to "requires `RES`"; the unsendable ones keep their
      rule for `ThreadWorker`. Proof: §15.6, plus the design settled in
      plan-156-A.
      (Two changed — `rejects_a_thread_handle_as_a_list_element` and
      `…_as_a_map_value` now expect `TYPE_RESOURCE_REQUIRES_RES`. The other six
      still pass unchanged: a thread-carrying union as a map key and map-key
      ownership are still refused (a resource is no key), and the four
      unsendability tests are Phase 3's.)
- [x] Update the two `func_thread_{start,send}_invalid` fixtures' expected
      diagnostics.
      (Each gains one line: `TYPE_RESOURCE_REQUIRES_RES` on its bare `Thread`
      record field; nothing else in either log moved.)
- [x] Collection and record drop of thread elements through `thread.__drop`.
      (`is_resource_nominal` answers for a parent view, so a list lays it out as
      a pointer payload and the owned-list drain and record drop close it with
      `thread.drop`; `thread_runtime_return_type` strips the `RES` marker a view
      read out of a field or element carries. `--debug` measurements: 16 pooled
      workers dropped without a read → `thread.table_live 0`,
      `arena.0.live_bytes 0`; a record holding a `RES` thread bound first →
      `table_live 0`, `live_bytes 0`, `double_free_skips 0`. Two leaks found on
      the way, neither thread-specific — Corrections 3 and 4.)

Acceptance: `thread-pool-list` and `bug692-*` pass.
  Check: `cargo test --bin mfb verify` → pass (est. 3 min), and the two
  fixtures via `test-accept.sh` → pass (est. 1 min).
  Result: verify → 490 passed; `thread-pool-list` (`workers 8 total 204`) and
  `bug692-record-thread-copy` (`5`, `5`) pass, their `.ast`/`.ir` recorded;
  the thread set (`rt-behavior/thread*/**`, `syntax/threads/**`,
  `rt-error/threads/**`) fails only on Phase 3's three fixtures.
Commit: f1e2ba176

### Phase 3 — transfer and the three errors

- [x] Parent-view sendability; the transfer and accept lowering for
      `ThreadHandle`.
      (`is_builtin_sendable_resource_type` answers `true` for a parent view —
      the one structural point the verifier's `is_resource_sendable` and
      codegen's `is_sendable_resource_nominal` both reach — so the verifier's
      cause walk returns a sendable-resource answer for it and a `Worker` cause
      for a `ThreadWorker`. Codegen: a transferred view takes the ordinary
      resource record copy (`copy_resource_to_current_arena`: the 96 bytes into
      the receiver's arena, the source flagged `moved|closed`) and the sender's
      cleanup retires as for any sendable resource; `thread::start`'s own view is
      carried, not copied, when an inline `TRAP` wraps it
      (`runtime_result_is_owned_resource_record`). `thread-transfer-parent-view`
      → `coordinator answered 50`; its `--debug` report: main and worker arenas
      `live_bytes 0`, `double_free_skips 0`, `thread.table_live 0`. Also closed
      a data-plane hole — Correction 6.)
- [x] The two rule codes: `src/rules/table.rs` and
      `src/docs/spec/diagnostics/01_rule-codes.md`.
      (One rule, Correction 2: `TYPE_THREAD_WORKER_NOT_TRANSFERABLE`, numbered
      `2-203-0140` — Correction 5 — for a `ThreadWorker` on a resource plane or
      transferred; on a data plane a worker stays `TYPE_THREAD_NOT_SENDABLE`, as
      §3 asks of "the unsendable ones". `thread_worker_transfer_invalid` →
      three `2-203-0140` (lines 11, 16, 17); `thread_transfer_self_invalid` →
      one `TYPE_CALL_ARGUMENT_MISMATCH`. Unit tests
      `a_thread_may_carry_parent_threads_on_its_resource_plane`,
      `a_thread_worker_on_a_resource_plane_is_not_transferable`; the four
      unsendability tests re-baselined (Correction 7).)
- [x] ~~The runtime code in `02_error-codes.md`, and the check in
      `transferResource`.~~ — moot: no program can transfer a thread to itself
      (Correction 2); the runtime check would never fire.

Acceptance: all Phase 1 tests pass.
  Check: the Phase 1 `test-accept.sh` command → 0 failures (est. 2 min);
  `cargo test errorcode` → pass (the error table changed) (est. 2 min).
  Result: the thread set (`rt-behavior/thread*/**`, `syntax/threads/**`,
  `rt-error/threads/**`, `byte-identity/{thread,resource-xfer-slots}`) →
  "acceptance tests passed (103 test(s) ran)"; the other eleven fixtures that
  use threads plus the `acceptance` app → 12 passed; `examples/{network-server,
  wind,browser/app}` build. `cargo test errorcode` not run: the error table did
  not change (the runtime error is moot). `cargo test --bin mfb verify` → 492
  passed, `… diagnostics` → 6 passed.
Commit: f9895926e

## Validation Plan

- Tests: the Phase 1 fixtures and the verifier unit tests.
- Runtime proof: the pool fixture under a `--debug` build shows parent-arena
  live bytes back at baseline after the pool's scope ends.
  **Corrected (Correction 3)** — measured instead: a 16-worker pool dropped
  without a read-back → `arena.0.live_bytes 0`, `thread.table_live 0`; read
  back through `FOR EACH` → `thread.table_live 0`, `double_free_skips 0`, and
  `live_bytes 2352`, identical to the same pool of `fs::File` (bug-645's
  residual); the transferred view → main and worker arenas at 0.
- Doc sync: `01_rule-codes.md` and `02_error-codes.md` (build input). The
  §15.6 and §16 prose is written in F.
  **Done, and more** (as C's Correction 4): the rule row; the sentences D made
  false in `language/16` (sendability, the resource plane), `language/14`
  (what containers may hold), `language/08` (a thread in the trap route);
  `mfb man thread transfer` and the package page list a `Thread` among what
  may cross. `scripts/spec-census.sh --citations` → 0 misses;
  `scripts/man-census.sh --memory-scope` → 0 unclassified.
- Final gate: at the end of F.

## Corrections

1. **A record with a `RES` field is copyable; the copy aliases.** §3 calls
   non-copyability "the existing §15.6 rule", and the checkable outcome wants
   bug-692's copy to be a compile error. §15.6 says the reverse
   (`15_resource-management.md`: "copying the record copies the pointer and
   aliases the same resource, it does not duplicate it"), and a record with a
   `RES fs::File` field copies today (`LET copy AS H = h` builds). So a thread
   follows §15.6 like every other resource: a bare `Thread` field is
   `TYPE_RESOURCE_REQUIRES_RES` (the bug's reproduction, now a syntax fixture),
   and a `RES` field's record copies as an alias of one open thread, closed once
   by its owning scope. bug-692's hazard — two values each closing the thread,
   the second `waitFor` failing — is gone either way: the field is a pointer,
   and `waitFor` is repeatable (plan-156-C).

2. **A thread can never be transferred to itself; `2-203-0140` and
   `7-703-0011` are moot.** `thread::transfer(dest, v)` requires `v` to have
   `dest`'s resource-channel type, and `dest`'s type contains that type, so `v`
   being a view of `dest`'s own thread needs an infinite type. A union variant
   is a nominal, so no union closes the loop, and a generic instantiates to the
   same infinite type; routing through a third thread only moves the equation.
   Measured: `thread::transfer(t, t)` is `TYPE_CALL_ARGUMENT_MISMATCH` ("has
   argument type(s) (Thread OF RES Thread OF Integer TO Integer TO Integer, …),
   expected … Res"). A rule and a runtime check for a case no program can
   express would be dead code, so neither is added;
   `thread_transfer_self_invalid` pins the existing rejection.
   `TYPE_THREAD_WORKER_NOT_TRANSFERABLE` stays: a `ThreadWorker` channel type is
   spellable.

3. **A pool read back through `FOR EACH` or `get` keeps its elements' records
   until exit — for every resource, not just threads.** The Validation Plan's
   "parent-arena live bytes back at baseline after the pool's scope ends" does
   not hold for the pool fixture, and cannot for any resource: the same shape
   over `fs::File` reports the identical `arena.0.live_bytes 2352` at 16
   elements. `record_ownership::owning_collections` refuses to free the records
   of a collection named as a `FOR EACH` iterable or read through `get`
   (bug-645's recorded residual decline, fail-safe against a double free). The
   threads are still dropped exactly once — every block is released — so the
   validation is strengthened to what does hold and is checkable: a pool
   dropped without a read-back is flat (`live_bytes 0`), and every pool ends
   with `thread.table_live 0` and `double_free_skips 0`.

4. **A resource placed into a container without a `RES` binding is never
   closed — bug-695**, found here: `append(pool, thread::start(…))` and
   `H[t := thread::start(…)]` leave the worker running and its block live, and
   the same shapes over `fs::File` exhaust the descriptors. Pre-existing and not
   thread-specific; filed with its reproduction and fixed separately. The pool
   fixture binds each thread `RES` first, the spelling that floats correctly.

5. **The worker rule is `2-203-0140`, not `0141`.** With the self-transfer rule
   moot (Correction 2), `0140` would otherwise be skipped; codes are assigned in
   order.

6. **A resource named directly as `thread::start`'s input crossed a data
   plane unchecked.** Spec §16 applies `TYPE_THREAD_RESOURCE_PLANE_REQUIRED` to
   "the message, output and input slots … whether the resource is named
   directly or is reached through" a container, but only the declared message
   plane pre-checked a direct resource; `require_thread_sendable`'s cause walk
   answers `None` for a sendable one. Measured: `thread::start(takesFile, f)`
   with an `fs::File` compiled clean — and with a parent `Thread` sendable, so
   did `thread::start(takesHandle, a)` in `thread-start-input-not-sendable`.
   `require_thread_sendable` now reports a direct resource on a data plane.
   That fixture's line now reads `TYPE_THREAD_RESOURCE_PLANE_REQUIRED` instead
   of `TYPE_THREAD_NOT_SENDABLE`, its comment updated.

7. **Re-baselined with proof.** `func_thread_start_valid` passed an
   `fs::File` as a start input (`thread_file_workers::acceptFile`, whose
   comment describes sharing one handle across two threads) — disproved by §16
   line 60 above; its `t7` now starts `receiveFile` and `thread::transfer`s the
   file on the resource plane, keeping the fixture's coverage of that package.
   `func_thread_{send,start}_invalid`'s unsendable `BadMessage` records held a
   bare `Thread`, which is now a sendable resource, so they stopped reaching
   `TYPE_THREAD_NOT_SENDABLE` (the syntax corpus fell from 77 to 76 rules,
   `the_syntax_corpus_still_reaches_every_rule_it_used_to`); the field is now a
   `ThreadWorker`, and both logs are byte-identical to their pre-D goldens. The
   verifier's `unsendable_record()` helper made the same switch, and
   `cause_walk_reports_func_and_thread_handle_as_other_not_resource` now pins a
   `Worker` cause for a `ThreadWorker` and none for a parent `Thread`.

## Summary

Opens storage and transfer on top of C. The risk is the pointer-storage
ownership rules applied to a handle whose drop cancels a worker: a pool
element dropped by the wrong scope would cancel a live worker.
