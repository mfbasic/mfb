# bug-622: every thread::start + thread::waitFor leaks in the parent arena (the result copy and the thread's plumbing)

Last updated: 2026-09-13
Effort: x-large (1d–3d)
Severity: HIGH
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1)

A program that starts and waits for threads in a loop grows its **parent** arena without bound.
Each `thread::start` + `thread::waitFor` leaves two things live there that nothing frees:

- **Part A — the result copy.** `thread::waitFor` deep-copies the worker's result into the
  parent arena, but the binding that receives it is never given a free. For the browser
  example's page load (a `LoadResult` holding the whole styled document) that is
  **5,268,592 B per load** — the main thread keeps every page it ever loaded.
- **Part B — the thread's plumbing.** `thread::start` allocates the control block, the worker
  arena's state-plus-globals block and four queue records with their ring buffers from the
  parent arena (10 blocks, ≈7.1 KB). No code path frees them.

(The worker arena itself is never reclaimed either — that is `planning/todo.md` Bucket List 1,
a separate item, not this bug.)

**The single correct behavior a fix produces:** once a thread has been waited for and its
handle and result are dropped, the parent arena holds nothing for it, so a start/waitFor loop
reports equal parent-arena `live_bytes` at N and 2N.

References:

- `src/docs/spec/threading/07_control-block.md` (result payload lifetime),
  `08_queue-semantics.md` ("If such a value is re-bound or returned it is deep-copied first,
  so the copy is owned and freed normally"; drop semantics), `09_os-integration.md` (start
  allocates the control block and worker arena state from the spawning thread's arena),
  `src/docs/spec/language/16_threads.md`.
- Found by plan-133-A Phase 2 (copy-back stage). `planning/todo.md` § 3 item 10 already notes
  the control block lives in the parent's arena.
- bug-566 / bug-576: the "runtime-managed result" exemption this bug narrows.

## Failing Reproduction

`/tmp/plan-133-a/stages/tw_string.mfb` (plan-133-A harness), `{N}` = 100 and 200,
`target/release/mfb build --debug`, macOS, main `14c9fc1ca`:

```
IMPORT io
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF String TO String, seed AS String) AS String
  RETURN seed & "-" & seed
END FUNC

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET t AS Thread OF String TO String = thread::start(work, "abc")
    LET r AS String = thread::waitFor(t)
    total = total + len(r)
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

- Observed (`arena.0.*`, the main arena): N=100 `live_bytes 727120`, `alloc_calls 1104`,
  `free_calls 2`; N=200 `live_bytes 1440720`, `alloc_calls 2204`, `free_calls 2`.
  7,136 B and 11 allocations per thread, none freed.
- Expected: equal `live_bytes` at both N.

Other result shapes (same loop, N=100 → 200, main arena):

| Worker result | `live_bytes` | Per thread |
|---|---|---|
| `String` (`tw_string`) | 727,120 → 1,440,720 | 7,136 B; 11 allocs, 0 frees |
| recursive union `TwNode` (`tw_union`) | 751,120 → 1,488,720 | 7,376 B; 17 allocs, 5 frees |
| record `{ok, document AS TwNode, title}` (`tw_record`) | 755,920 → 1,498,320 | 7,424 B; 14 allocs, 1 free |
| browser `LoadResult` of the `BASIC` page (`copyback`, N=1 → 2) | 5,282,112 → 10,550,704 | 5,268,592 B; 963 allocs, 3 frees |

## Root Cause

**Part A.** The `thread.waitFor` call site copies the result into the current (parent) arena —
`src/codegen/engine/builder/builder_emit_helpers.rs`: `copy_value_to_current_arena(result_type,
RESULT_VALUE_REGISTER)` for `thread.waitFor | thread.read | thread.receive | …` — but
`value_is_runtime_managed` / `runtime_call_result_is_foreign_arena`
(`src/codegen/engine/value/builder_values.rs`, `target.starts_with("thread.")`) classify every
`thread.*` result as foreign-arena. So `LET r` gets no `OwnedValue` cleanup
(`builder_control.rs`, `owns_freeable_value = … && !runtime_managed`), statement temps are not
freeable (`pending_temp_is_freeable`), and trapped results are `OwnedElsewhere`. The exemption
was right for the raw worker-arena pointer (bug-566/576) and wrong for the parent-arena copy,
which has no other owner.

**Part B.** `lower_thread_start_helper` (`src/codegen/runtime/thread/runtime_helpers.rs`)
allocates the control block (`THREAD_BLOCK_SIZE` 120 B), the worker arena state plus globals,
and four queue records + rings (`emit_thread_queue_alloc`, 760 B each) from the parent arena.
The `thread.drop` helper (`runtime_helpers_thread.rs`, `Drop` arm) only locks, marks
`CLOSED`/cancelled, broadcasts, closes resource queues and detaches; after `waitFor` the handle
is already `CLOSED`, so drop is a no-op. No thread helper calls `arena_free` on any of these
blocks, and the spec never assigns them an owner. `waitFor` uses `pthread_detach`, so the parent
has no point at which the worker is known to have stopped touching them.

## Goal

- Part A: the loop above with a `String`, a recursive union and a record result shows no
  per-thread growth from the result; the value is still correct after the worker finishes.
- Part B: the same loop shows no per-thread growth at all (parent `live_bytes` equal at N and
  2N), including when the handle is dropped without `waitFor` after the worker completed.

### Non-goals (must NOT change)

- Result values and thread semantics (cancel, detach-on-drop of a running worker).
- The raw worker-arena result pointer must still never be freed from the parent (the
  bug-566/576 cross-arena free).
- Worker-arena reclamation (Bucket List 1) — a separate change.

## Blast Radius

- `thread.waitFor`, `thread.read`, `thread.receive`, `thread.acceptResource`,
  `thread.readResource` results copied at the call site — Part A, fixed here (same exemption).
- A worker error's message and `ErrorLoc` copied by `emit_finalize_worker_error_source`
  (`src/codegen/error/emission/builder_error_emission.rs`), whose comment assumes
  `thread.drop` frees — latent, same hazard, audit in Phase 1.
- Dropping a still-running worker's handle — Part B cannot free from the parent without a join;
  out of scope unless the design gives the trampoline ownership (Open Decisions).
- `examples/browser/app` — the consumer that loses ≈5 MB per page load.

## Fix Design

Part A: split the classification — the raw `RESULT_VALUE` stays runtime-managed; the value
returned by `copy_value_to_current_arena` is an owned fresh value (bindings, temps and traps
free it normally). Part B: give the plumbing an owner and a synchronization point —
recommended: `waitFor` joins (`pthread_join`) instead of detaching, then frees the control block,
queues and worker arena state block; a completed-then-dropped handle does the same in
`thread.drop`. Rejected: freeing in `thread.drop` without a join — the trampoline still unlocks
the outbound queue mutex after `COMPLETED`, and the worker arena state lives inside the block
(use-after-free).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `rt_scope_drop_leaks.rs`: the three result shapes above, N vs 2N on the main arena's
      `live_bytes` (a `--debug` build); confirm they fail. Landed as
      `a_waited_for_thread_leaves_nothing_in_the_parent_arena`,
      `a_completed_thread_dropped_without_waitfor_leaves_nothing_in_the_parent_arena` (Part B)
      and `a_thread_result_copy_is_freed_by_its_owner` (Part A: bound/unbound/trapped String,
      recursive union, record, parent `thread::receive`, each against the scalar-result
      baseline's growth). RED as documented: 348,800 B between 50 and 100 threads for both
      Part B cases (6,976 B/thread); `b622_string_bound` 800 B over the baseline (16 B/thread).
- [x] Audit `thread.read`/`receive`/`acceptResource`/`readResource` and the worker-error copy.
      - `thread.receive`/`thread.read` — same call-site copy, fixed by Part A
        (`runtime_call_result_is_copied_at_call_site`; the `b622_receive` case).
      - `thread.acceptResource`/`thread.readResource` — same predicate; a resource is neither
        freeable-flat nor a graph, so a plain bind is unchanged. A trapped
        `thread::accept(…) TRAP` now gets the bug-593 wrapper drop of its own `Result` block
        (this frame's allocation) — covered by the full suite's transfer fixtures, not measured.
      - `emit_finalize_worker_error_source` — unaffected: the message and `ErrorLoc` are copied
        into the caller's arena before any drop, and `thread.drop` never touches the worker
        arena's chunks; its comment claimed the drop frees the worker arena and was corrected.
      - `t.result` (`MemberAccess "result"`, rejected at source by `TYPE_THREAD_RESULT_REMOVED`)
        — unchanged, stays runtime-managed.
      - Found, out of scope: `thread::send` leaks its argument temp and the queued copy in the
        sender's arena → bug-629. `thread::waitFor(thread::start(…))` does not compile
        (`native inlined field size not available for type 'Out'`, pre-existing on an older
        main build) → bug-630.

Acceptance: the cases fail for the documented reason; the audit list has a verdict per site.
Commit: 7d5d04cf0

### Phase 2 — Part A, the result copy

- [x] Owned classification for the call-site copy (`builder_values.rs`,
      `builder_emit_helpers.rs`): `runtime_call_result_is_copied_at_call_site` names the five
      copied reads; `value_is_runtime_managed` and `mark_runtime_helper_result_fresh` treat
      their value as owned; `runtime_call_result_is_foreign_arena` (the raw block) unchanged.

Acceptance: per-thread growth drops by the result's size; cross-arena guards still pass
(`cargo test --bin mfb registry` 93 passed; `rt_scope_drop_leaks trap` 24 passed;
`codegen_raw_helper_result_drop` 5 passed).
Commit: 4ee20d9e0

### Phase 3 — Part B, the plumbing

- [x] Join in `waitFor`; free control block, queues and worker arena state; the same on drop
      of a completed handle (`runtime_helpers.rs`, `runtime_helpers_thread.rs`); spec update
      in `07_control-block.md` / `08_queue-semantics.md` (and `09_os-integration.md`).
      Also: the thread cleanup call nulls the binding's slot after the drop; the `--debug`
      arena registry retires a freed worker's slot; `pthread_join` imported for `thread.*`
      on macOS/Linux, `WaitForSingleObject` + `CloseHandle` on Windows. Positive pin:
      `a_moved_or_reassigned_thread_handle_is_freed_exactly_once`.

Acceptance: Phase 1 cases flat; threading suites green (`rt_debug_arena` 8, 
`rt_thread_send_cross_arena` 2, `rt_recursive_thread_transfer` 1 passed). Runtime proof on
macOS aarch64, Linux aarch64 glibc (box 2223) and Windows x86_64 (box 2230): `tw_int` and
`tw_drop` at N=100/200 report `arena.0.live_bytes 0`, `alloc_calls == free_calls`,
`double_free_skips 0`.
Commit: eb467c304

### Phase 4 — expected outputs + full validation

- [ ] Regenerate shifted goldens; full suite; `scripts/test-accept.sh`; plan-133-A copy-back
      stage re-run.

Acceptance: full suite green; copy-back main-arena growth 0.
Commit: —

## Corrections

1. **Free-at-drop was unsound; the plumbing is owner-counted.** The Fix Design froze the
   plumbing at `waitFor`'s join and freed it in the handle's drop, assuming a drop is the
   handle's last reader. It is not. The language lets a `Thread` handle be read after it
   was dropped or passed to a function (the op answers `ErrResourceClosed`), bound under
   two names (`LET t2 = t`), and read by a function `TRAP` handler after the trap route
   dropped it (`trap_route_cleanups` relied on the drop being an idempotent no-op). The
   first landing (eb467c304) freed at the first drop and nulled only that binding's slot,
   which crashed (exit 139) four legal shapes: `tests/rt-behavior/thread/thread-start-inline-trap-rt`
   (the inline `TRAP` binds the handle as `$trap_valN` and again as `t`),
   `tests/rt-behavior/threads/thread-queue-timeout-cancel` (a handler reads a routed
   handle), an alias, and a caller reading a handle after passing it to a function —
   found by the acceptance run. The design that landed instead:
   - the control block grows to 128 B with `owners` at offset 120 (`THREAD_OFFSET_OWNERS`):
     `thread::start` sets 1, a bind or `MUT` assign from another handle binding adds 1 (an
     assign before it drops the old handle), and a function parameter adds 1 on entry;
   - `thread.drop` takes a mode: CLOSE (mark CLOSED, cancel, join a completed worker or
     detach a running one — the drop's old meaning) and RELEASE (give up one owner; the
     release that reaches 0 closes if needed and frees the plumbing once the worker is
     joined). A handle passed to a function is closed by the callee's parameter and
     released by both bindings; a `RETURN` hands the callee's count to the caller; a trap
     route only closes a handle the handler can name, and the handler's exit releases it.
   - the zeroed CLOSED handle an inline `TRAP` on `thread::start` binds as the trap's
     value (bug-479) is counted too (`owners` 1): the successful assign drops it at once,
     and its last release frees the 128 B block. Uncounted, it leaked one control block
     per trapped start (128 B; 120 B before this bug), which the inline-`TRAP` case of the
     pin below caught after the owner count landed.
   - Regression pin: `a_shared_thread_handle_stays_readable_and_is_freed_once` (alias,
     move-then-reuse, inline-`TRAP` start, trap-routed handler).
2. **Trapped non-copied thread calls leaked their `Result` wrapper.** Beyond the five
   call-site-copied reads, `value_is_runtime_managed` still classified every `thread.*`
   `CallResult` as runtime-managed, so the wrapper an inline `TRAP` builds in this frame
   around a `thread::isRunning` / `poll` / `cancel` / `send` result had no owner — 144 B
   per trapped `isRunning` (probe `tw_trap_isrunning`, pre-existing). A `CallResult` is
   now never runtime-managed; the raw helper block stays `RawSuccessBlock::OwnedElsewhere`.
   Regression case: `b622_trapped_query` in `a_thread_result_copy_is_freed_by_its_owner`.

## Validation Plan

- Regression tests: Phase 1 cases.
- Runtime proof: plan-133-A `copyback` stage; the browser's main-arena `live_bytes` after
  several page loads.
- Doc sync: `src/docs/spec/threading/07_control-block.md`, `08_queue-semantics.md`.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Part B ownership — join in `waitFor` (recommended) vs. the trampoline freeing its own
  plumbing into a parent-visible free list.

## Summary

Part A is a classification fix with a known shape; Part B is the risky one (thread lifetime and
a new synchronization point). The worker arena itself is left to Bucket List 1.
