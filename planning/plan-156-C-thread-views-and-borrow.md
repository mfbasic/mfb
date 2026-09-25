# plan-156-C: two view records, the hidden drop op, borrow-on-pass, repeatable `waitFor`

Last updated: 2026-09-24
Effort: large (3h–1d)
Depends on: plan-156-B

This is the semantic flip:
- **Views.** `Thread` and `ThreadWorker` become resources in the verifier and
  the codegen. Each is a standard resource record pointing at the control
  block (plan-156-A).
- **Passing** either view to a function lends it: the caller keeps it.
- **Scope end.** The parent view's drop, at the end of its scope, cancels and
  detaches through a hidden drop op.
- **`thread::waitFor`** is repeatable and closes nothing.
- **Spelling.** `RES` becomes a real binding for a thread, which fixes
  bug-691. `LET`/`MUT` bindings are still **accepted** in C and get exactly
  the same resource semantics, so the corpus keeps building. F makes `RES`
  the only spelling and migrates the corpus.

**Checkable outcome:**
- `/tmp/thrrepro`'s shape runs: a helper `drain(t)` called in a loop, then the
  caller's `thread::isRunning(t)` and `waitFor`.
- bug-691's reproduction prints `TRUE` then `5`.
- Two `waitFor(t)` calls both return the result.

References: plan-156-A §The plan-156 feature (the settled semantics); spec
§15 (`15_resource-management.md`), §16; bug-691; bug-622 (the owner count
this deletes); `.ai/resources-packages.md`, `.ai/codegen-invariants.md`.

## Prerequisites

See plan-156-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-156-B complete | `ls planning/plan-156-B-*` → no match | NOT MET |

## 1. Goal

- A `Thread` value passed to a user function, stored in a `RES`/`LET`
  binding, or returned behaves exactly as §15 specifies for a resource.
- The parent view's scope drop cancels and detaches a running worker.
- `waitFor` never closes.

### Non-goals

- No storage in records, lists or maps, and no parent-view transfer (D). Those
  keep today's rejections: `TYPE_COLLECTION_OWNERSHIP_VIOLATION`,
  `TYPE_THREAD_NOT_SENDABLE`. The record-field hole (bug-692) is D's.
- No per-view STATE (E).
- `LET`/`MUT` stay accepted (F rejects them).
- Message and cancel semantics are unchanged.

## 2. Current State

- Type: `src/types.rs:ParameterType::ThreadHandle { worker, msg, res, out }`.
  Thread is deliberately **not** a registry resource
  (`src/codegen/builtins/thread/mod.rs:register`: `add_source_types` "without
  the RES resource-table machinery").
- Resources are recognised by nominal name
  (`src/codegen/resource/mod.rs:builtin_resource_close_function`,
  `is_builtin_resource_type` via `registry().resolve_type`). A generic
  `Thread OF …` has no single name, so it needs a structural `ThreadHandle`
  arm.
- The predicates that need that arm, all present today (2026-09-24 research):
  - `ir/verify/link.rs`: `close_op_for`, `is_resource_or_resource_union`,
    `provably_data_type`, `consumed_resource`, `contains_thread`,
    `contains_resource_or_thread`.
  - `ir/verify/resources.rs`: `is_copyable`, `is_resource_type`,
    `is_defaultable`, `res_axis_slot`.
  - `ir/verify/mod.rs:is_thread_type`.
  - `ir/shape.rs:is_resource_type`.
  - `codegen/builtins/mod.rs:is_resource_type`.
  - `codegen/resource/mod.rs:builtin_resource_close_function`.
  - `CodeBuilder::is_thread_type`.
  - `target/shared/plan/symbols.rs:is_thread_type`, `ops_have_thread_owner`.
- `ThreadHandle` appears 105 times in `src` (`grep -rn ThreadHandle src | wc -l`),
  in 28 non-test files.
- The hidden-drop-op precedent is `process::Process`:
  - `src/codegen/builtins/process/mod.rs:DROP = "process.__drop"`, registered
    as `RegistryResource.close_function`;
  - lowered by name in `codegen/engine/builder/mod.rs`
    (`lower_process_drop_helper`);
  - also named in the per-target lists `target/{linux_common,win_x86_64,macos_aarch64}`
    and `target/shared/runtime/mod.rs`;
  - `process::waitFor` returns a value and leaves the handle open.
- Thread cleanup today:
  - `ActiveCleanup::Thread(ThreadCleanup{name, symbol, close, release})`
    (`codegen/engine/builder/mod.rs`);
  - owner increment on alias and parameter
    (`builder_control.rs:lower_ops_inner`, `function_lowering.rs:lower_function`,
    `builder_thread_cleanup.rs:emit_thread_owner_increment`);
  - move-on-pass (`deactivate_moved_thread_arguments`,
    `maybe_deactivate_moved_thread_local`);
  - return (`builder_exits.rs:emit_return_exit_inner` →
    `deactivate_thread_cleanup`);
  - the drop call (`emit_thread_cleanup_call` → `thread.drop(h, CLOSE|RELEASE)`,
    an `os_alias` of cancel, `thread/lowering.rs:lower_cancel`).
- Not deletable: `builder_thread_cleanup.rs` also holds the send and seed
  lowering (`emit_thread_send_runtime_helper_call`,
  `claim_moved_thread_arg_temp`, `emit_thread_seed_size`,
  `emit_failed_thread_start_seed_free`).
- Resource records: `RESOURCE_OFFSET_TAG=0`, `HANDLE=8`, `CLOSED=16` (bit 0
  closed, bit 1 moved), `STATE=24`, `RESOURCE_RECORD_SIZE_BYTES=96`. Tags
  1–12 are taken, plus 255 for native (`error_constants.rs`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Tests pinning move-on-pass, aliasing and one-shot `waitFor` | `rt_scope_drop_leaks.rs`: `B622_MOVED_TO_CALLEE`, `B622_REASSIGNED`, `B622_ALIAS`, `B622_MOVED_REUSE`, `B622_INLINE_TRAP_START`, `B622_TRAP_ROUTED`, plus 3 plumbing tests; fixtures `syntax/threads/func_thread_waitFor_valid` (`doubleWaitCloses`), `rt-behavior/threads/func_thread_result_valid`, `rt-behavior/thread/thread-start-inline-trap-rt` (`closedAnswers`) | census 2026-09-24 (`rg -n 'B622_' tests/runtime/rt_scope_drop_leaks.rs`; `rg -ln 'doubleWaitCloses\|closedAnswers' tests`) |
| Drop-semantics fixture | `rt-behavior/threads/thread-drop-cleanup` (5 drop cases + `makeThread`'s return) | same census |

## 3. Design

- **Resource tag.** `RESOURCE_TAG_THREAD` (next free, 13) for the parent view,
  and `RESOURCE_TAG_THREAD_WORKER` (14).
- **The parent view** is a 96-byte resource record in the binding's arena:
  `HANDLE` = control-block pointer, `CLOSED` per §15, STATE slot reserved
  (E).
  - `thread::start` returns a fresh record.
  - The trampoline builds the worker's own record in the worker arena and
    passes it as the entry's first argument.
- **Every `thread::*` builtin** reads the control block through
  `record.HANDLE`. That's one extra load per call; all `THREAD_OFFSET_`/
  `CONTROL_OFFSET_` users go through a single `emit_control_block_of(view)`
  helper.
- **Hidden drop op.** `thread.__drop`, the registered close op for
  `ThreadHandle` (worker = false). It cancels and detaches a running worker,
  then releases the parent side of the block (A §4.4, whose `refs` replaces
  the owner count).
  - `ThreadHandle` with worker = true has **no** drop op: the runtime closes
    it at worker return.
  - The verifier knows the one op through `close_op_for`, so `RETURN` and
    scope drop are standard resource events.
- **Borrow-on-pass.** Delete the non-`thread.` arm of
  `deactivate_moved_thread_arguments`, the parameter owner increment, the
  alias owner increment, `ThreadCleanup`, and `ActiveCleanup::Thread`.
  Threads use `ActiveCleanup::Resource` with the `thread.__drop` symbol
  (`resource_cleanup_symbol`).
- **`waitFor`.** Waits on the block condvar until the worker is closed, then
  copies the result out of the block arena into the caller's arena on every
  call. It no longer sets any closed flag on the parent view.
- **`LET`/`MUT` in C.** A `LET` thread binding lowers exactly like a `RES`
  one, and the `Bind` arm treats it as a resource binding. It stays
  temporarily accepted; F turns it into `TYPE_RESOURCE_REQUIRES_RES`, the
  existing rule.

Correctness risk: the drop, which is now on the resource path, must still
cancel a running worker on every exit path (`RETURN`, `FAIL`, trap routing,
`EXIT PROGRAM`). `thread-drop-cleanup` covers all five paths and must keep
passing unchanged.

Byte-identity is not the gate. `.ir` and `.ncodesum` of every thread fixture
are expected to diff. `.ast` must not diff in C, since C changes no syntax.

## Phases

> Keep checkboxes current (see plan-156-A's note). **An unticked box means NOT
> DONE.**

### Phase 1 — failing tests for the new semantics

- [ ] `tests/rt-behavior/threads/thread-borrow-on-pass/`: `/tmp/thrrepro`'s
      helper-in-a-loop shape, expected to print its total and result, exit 0.
      Fails today with `7-703-0004`.
- [ ] `tests/rt-behavior/threads/thread-waitfor-repeatable/`: two
      `waitFor(t)` calls both print 5. Fails today.
- [ ] bug-691's reproduction as `tests/rt-behavior/threads/bug691-res-thread-param/`.

Acceptance: all three fail for the documented reason.
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/threads/thread-borrow-on-pass' 'rt-behavior/threads/thread-waitfor-repeatable' 'rt-behavior/threads/bug691-*'`
  → 3 failures (est. 1 min).
Commit: —

### Phase 2 — the verifier sees a resource

- [ ] Add the `ThreadHandle` arms to every predicate listed in §2 (each file
      named there). `close_op_for` answers `thread.__drop` for worker = false
      and `None` for worker = true.
- [ ] `ops.rs` Bind arm: a `ThreadHandle` is a resource, so `RES` is accepted
      and `LET`/`MUT` temporarily map to it (comment: "until plan-156-F").
- [ ] Verifier unit tests in `src/ir/verify/tests.rs`: `TYPE_USE_AFTER_MOVE`
      after `RETURN t`; a borrowed parameter is not a move.

Acceptance: the new unit tests pass; existing verifier tests pass.
  Check: `cargo test --bin mfb verify` → pass (est. 3 min).
Commit: —

### Phase 3 — view records, drop op, borrow, repeatable `waitFor`

- [ ] Register `thread.__drop` (a `RegistryResource`-equivalent structural
      entry), lower it, and add it to the per-target runtime lists beside
      `process.__drop`.
- [ ] `thread::start` and the trampoline build the view records (§3). Add
      `emit_control_block_of` and re-point every builtin.
- [ ] Delete the owner count, move-on-pass, `ThreadCleanup` and
      `ActiveCleanup::Thread` (§3). Threads go through the resource cleanup.
- [ ] `waitFor` per §3. Update `func_wait_for.rs` DESC ("closes the handle" →
      repeatable).
- [ ] Re-baseline only the disproved expectations. Proof for each is spec §15
      plus the settled design:
  - `B622_MOVED_REUSE`, `B622_MOVED_TO_CALLEE`: the caller now reads the
    handle after the call.
  - `doubleWaitCloses` and `closedAnswers`: a second `waitFor` now returns
    the result.
  - Commit each change with that proof (AGENTS.md "never edit a test to
    pass").

Acceptance: Phase 1's three fixtures pass; `thread-drop-cleanup` passes
unchanged; the re-baselined tests assert the new behavior.
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/thread*/**' 'syntax/threads/**'`
  → 0 failures (est. 5 min); `cargo test --test rt_scope_drop_leaks` → pass
  (est. 3 min).
Commit: —

## Validation Plan

- Tests: the Phase 1 fixtures, the verifier unit tests, `rt_scope_drop_leaks`
  re-baselined with proof.
- Runtime proof: `examples/wind` with its queue drain moved back into a
  helper `withNews(worker, p)` in a scratch copy under `/tmp`: it runs to
  autoplay.
- Doc sync: `func_wait_for.rs` DESC (`mfb man thread waitFor`) and
  `func_cancel.rs` prose about drop. The spec prose waits for F; C's
  behavior is documented in F in one pass.
- Final gate: at the end of F.

## Corrections

## Summary

The semantic core. The risk is the drop path's cancel on every exit, guarded
by `thread-drop-cleanup`, and the deletion of the bug-622 owner count, whose
job A's `refs` and the resource drop now do.
