# bug-629: every `thread::send` leaks the message in the sender's arena (the argument temp and the queued copy)

Last updated: 2026-09-19
Effort: large (3h–1d) — **actual: small**, once Part B turned out to be already fixed
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: tests/runtime/rt_debug_soak.rs
(`a_thread_send_of_a_computed_message_keeps_live_bytes_constant`,
`a_worker_send_of_a_computed_message_keeps_live_bytes_constant`) — placed beside
bug-646's `a_thread_string_send_loop_…`, the literal-message sibling that isolates
Part A from Part B, not in `rt_scope_drop_leaks.rs` as Phase 1 planned.

## STATUS: FIXED (cb6b8101f)

**Part B was already fixed before this work started** — by bug-646 (`e124b7eee`),
which implemented exactly this doc's Open-Decisions recommendation (sender-side
return through the queue's pending-free list). Re-measuring the reproduction table
below at `ac0f61964` showed the literal-message row already flat
(`live_bytes 0`, not 3,200/6,400), which is the row that isolates Part B. Only Part A
remained, and it is the same defect as bug-649, filed later from the other side of
bug-646's fix; both are closed by the one commit.

**Part A, fixed:** `claim_moved_thread_arg_temp` no longer claims the data argument
of `thread.send` / `thread.emit`. Those two targets deep-copy the message into the
sender's own arena and hand the COPY across (bug-498), so the original never crosses
a boundary and the claim — written when it did — was removing the block's only
owner. The statement-scope temp cleanup now frees it.

The doc's own reproduction table goes to zero: `tw_send_received` reports
`live_bytes 0`, `alloc_calls 1302 / free_calls 1302` at N=100 and `2602/2602` at
N=200, `double_free_skips 0` (was 3,200 / 6,400 B live). bug-649's own shape is flat
too (`1702/1702` at N=50, `3402/3402` at N=100).

**Deviation from the Fix Design:** none for Part A. Phase 3 and Phase 4's golden
regeneration were not needed — see the audit below for why the artifact gate cannot
see this change.

**Found by this bug's Blast-Radius audit, NOT fixed here:** `thread::start`'s data
argument leaks one block per start (16 B for a `String` seed), for a different
reason than Part A — start does not copy, so its claim is load-bearing, and the
handed-over block simply has no owner. Filed as bug-655.

A program that sends messages across a thread boundary in a loop grows the **sender's**
arena without bound, even when every message is received. Two blocks are left live per
send, and nothing frees either:

- **Part A — the argument temp.** `thread::send(t, "message-" & toString(i))` deep-copies
  its message argument before enqueueing it, but the original fresh temp (the `&` result)
  is claimed out of the statement's temp cleanup, so it is never freed.
- **Part B — the queued copy.** The copy `thread::send` makes in the sender's arena and
  hands through the queue is never freed after a successful receive: the receiver copies it
  again into its own arena at the call site, and the queued block has no owner.

**The single correct behavior a fix produces:** once a message has been received (or
dropped by the queue's owner), neither the sender's argument nor the queued copy stays
live, so a send/receive loop reports equal sender-arena `live_bytes` at N and 2N.

References:

- `src/docs/spec/threading/08_queue-semantics.md` (message copies; "If a queued value is
  never received, the destination queue/runtime drops or closes it exactly once"),
  `src/docs/spec/threading/02_isolation.md` (the sender-arena copy, bug-498).
- Found while fixing bug-622 (thread start/waitFor plumbing), on the `worktree-B-622`
  integration build, with the bug-622 plumbing leak already removed so this residue is
  isolated.
- bug-498 (copy in the sender's arena), bug-147.5b (pending-free list for a FAILED send's
  orphan — the only path that frees a queued copy today), plan-25 (the temp claim).

## Failing Reproduction

`--debug` build, macOS aarch64, `worktree-B-622` build (bug-622 Part B applied), main arena
(`arena.0.*`), `{N}` = 100 and 200:

```
IMPORT io
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  LET m AS String = thread::receive(w)
  RETURN len(m)
END FUNC

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET t AS Thread OF String TO Integer = thread::start(work, "abc", 4, 4)
    thread::send(t, "message-" & toString(i MOD 10))
    total = total + thread::waitFor(t)
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

| Message argument | N=100 `live_bytes` / allocs / frees | N=200 | Per send |
| --- | --- | --- | --- |
| `"message-" & toString(i MOD 10)` (`tw_send_received`) | 6,400 / 1,302 / 1,102 | 12,800 / 2,602 / 2,202 | 64 B, 2 blocks |
| literal `"message-1"` (`tw_send_literal`) | 3,200 / 1,102 / 1,002 | 6,400 / 2,202 / 2,002 | 32 B, 1 block |
| no send, scalar result (`tw_int`, contrast) | 0 / 1,002 / 1,002 | 0 / 2,002 / 2,002 | 0 |

- Observed: `live_bytes` doubles with N; the literal case isolates Part B (1 block), the
  concatenation adds Part A (1 more block). Output values are correct (`total=900`/`1800`).
- Expected: equal `live_bytes` at both N.

Probe harness: `/tmp/b622/run.sh <stage> <N>` (builds `stages/<stage>.mfb` with `--debug`
and prints the `arena.0` counters) — recreate from this table if `/tmp` is gone.

## Root Cause

**Part A.** `CodeBuilder::emit_thread_send_runtime_helper_call`
(`src/codegen/cleanup/thread/builder_thread_cleanup.rs`) calls
`claim_moved_thread_arg_temp(target, &arg_values)` before it
`copy_value_to_current_arena`s `args[1]` and enqueues the COPY. The claim
(`claim_moved_thread_arg_temp`, same file) was written for the pre-bug-498 behavior
("these cross-arena values were never freed by the sender"), when the original itself
crossed. Since bug-498 the original never leaves the sender, so claiming it only removes
its one owner.

**Part B.** `thread_queue_read_helper`'s `found` path
(`src/codegen/runtime/thread/runtime_helpers_thread.rs`) dequeues the value pointer and
returns it; the receiving call site copies it into the receiver's arena
(`src/codegen/engine/builder/builder_emit_helpers.rs`, the `thread.receive`/`thread.read`
copy). Nothing frees the dequeued block afterwards. The only path that frees a queued copy
is the pending-free list, and only for a send that FAILED (bug-147.5b). Freeing it from the
reader would adopt it into the reader's arena, which bug-498's reasoning already permits
(`arena_free` only touches the freeing thread's arena state) — but it is a free-list
transfer to the reader, so the sender's `live_bytes` still would not return unless the
design changes (see Open Decisions).

## Goal

- The loop above, for both message shapes, shows no per-send growth in the sender's arena.
- The same holds for the worker-to-parent direction (`thread::send(w, …)` / parent
  `thread::receive(t)`) and for `thread.emit`.

### Non-goals (must NOT change)

- Message values and queue semantics (bounded capacity, timeouts, cancel, close).
- bug-498: no allocation in a peer thread's live arena.
- bug-147.5b: a failed send's orphan still goes to the pending-free list.
- A resource message's move semantics (bug-425, bug-546).
- Not a fix: skipping the call-site copy and handing the queued block itself to the
  receiver's binding (it is carved in the sender's arena; the receiver's scope-drop would
  then free it into the receiver's free list — the same accounting as Part B's adoption,
  but it also changes what the bug-622 Part A ownership split assumes).

## Blast Radius

Audited 2026-09-19 at `ac0f61964`, one verdict per site:

- `thread.send`, `thread.emit` data argument claim — **Part A, fixed here.** Both ride
  `emit_thread_send_runtime_helper_call`, which copies; `thread::send` inside a worker IS
  `thread.emit` (`func_send.rs:77`, `Body::abi_function_aliased(lower_send, &["emit"])`),
  so one predicate covers both directions. Measured before: 32 B per parent send
  (`arena.0` 6,400 → 12,800 at 200/400), 16 B per worker send (`arena.1` 7,088 → 13,024
  at 400/800). After: flat, `free_calls == alloc_calls`.
- `thread.transferResource`, `thread.emitResource` — **claim kept, correct as-is.** They
  ride the same copying emitter, but their argument travels the resource plane
  (`copy_resource_to_current_arena` + the moved/closed flag), not the data-plane temp
  cleanup, and a `RES` binding is a `Local` that was never registered as a pending temp,
  so the claim is already a no-op for every spelling reachable today. Narrowing it here
  would risk the statement cleanup racing the resource-record ownership for an inline
  resource argument, for no measured gain.
- `thread.start` data argument — **NOT the same leak; separate bug, filed as bug-655.**
  `lower_thread_start_helper` stores the caller's pointer straight into
  `THREAD_OFFSET_DATA` (`runtime_helpers.rs:754`) and the worker trampoline loads it as
  the entry's `c_arg(1)` (`runtime_helpers.rs:1216`) — no copy anywhere. So the claim is
  load-bearing (dropping it is a use-after-free), and the leak is that the handed-over
  block has no owner at all. Measured 16 B per start with a computed `String` seed
  (`arena.0` 1,600 → 3,200 at 100/200).
- `thread.read`, `thread.receive`, `thread.acceptResource`, `thread.readResource` dequeue
  paths — **Part B, already fixed by bug-646** (`e124b7eee`), which parks each queued copy
  on the queue's pending-free list for the SENDER to reclaim. The literal-message row of
  the table below is the isolating measurement: `live_bytes 0` at both N.
- Unread messages left in a queue at `thread.drop` / `thread::waitFor` — **still open**,
  tracked as bug-650 case 1 (bug-646's reclaim cannot reach a message the reader never
  dequeues). Out of scope here.

## Fix Design

Part A: stop claiming a `thread.send`/`thread.emit` data-argument temp once the call copies
it; the original is then freed by the statement's normal temp cleanup (a failed send keeps
the orphan COPY on the pending-free list, so the original is still dead either way).

Part B: give the dequeued copy an owner. Recommended: the reader frees the dequeued block
after its call-site copy (or reads straight into its binding without the second copy),
using the size the sender already passes for the pending-free path. See Open Decisions for
the sender-arena accounting consequence.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Cases for both directions of a computed message, N vs 2N on the sending arena's
      `live_bytes`; confirmed RED. Landed in `rt_debug_soak.rs`, not
      `rt_scope_drop_leaks.rs`: `assert_block_flat` there already asserts exactly this
      bug's three signals (`live_bytes` growth, `double_free_skips`,
      `free_calls <= alloc_calls`) and bug-646's literal-message case — the control that
      isolates Part A from Part B — is its immediate neighbour.
      **Both counts had to be doubled from the plan:** at 100/200 the parent-send case
      grows 3,200 B, *under* `BLOCK_BOUND` (4,096), so a case written to the
      reproduction table's counts passes while leaking. 200/400 grows 6,400 B. Same trap
      bug-646's case documents; the worker case runs at 400/800 for the same reason.
- [x] Audit every site in Blast Radius with a verdict and a measurement — see above. It
      turned up one leak this doc did not own (`thread::start`'s data block, bug-655) and
      confirmed Part B was already closed by bug-646.

Acceptance: met — the cases fail for the documented reason; every site has a verdict.
Commit: 0887d0353

### Phase 2 — Part A, the argument temp

- [x] Narrow `claim_moved_thread_arg_temp` for copying sends
      (`builder_thread_cleanup.rs`).

Acceptance: met, and better than stated — the computed case does not drop *to* the
literal case's growth, it goes to zero, because the literal case is itself flat since
bug-646. No double free (`double_free_skips 0`, `free_calls == alloc_calls`).
Commit: cb6b8101f

### Phase 3 — Part B, the queued copy

- [x] Owner for the dequeued block — **done by bug-646 (`e124b7eee`), not here.** It took
      this doc's recommended option (sender-side return via the queue's pending-free
      list). Unread-queue disposal is bug-650 case 1 and is still open.

Acceptance: met for the dequeued block; Phase 1 cases flat, threading suites green
(66 thread tests, `rt_debug_soak` 40/40).
Commit: — (bug-646's `e124b7eee`)

### Phase 4 — expected outputs + full validation

- [x] Full suite; artifact gate. **No goldens shifted, and that is correct, not a miss:**
      `artifact-gate.sh target/release/mfb thread` reports 7 goldens checked, 0 diffs
      because `tests/byte-identity/thread/src/main.mfb` sends only string *literals*
      (lines 36–37), which are not pending temps — the claim was already a no-op there,
      so no covered instruction moved. The owning gate for this change is the runtime
      `live_bytes` measurement in `rt_debug_soak.rs`, per `.ai/testing-gates.md` §18.

Acceptance: met; full suite green, zero golden delta with a reason.
Commit: (see the merge commit)

## Validation Plan

- Regression tests: Phase 1 cases.
- Runtime proof: the reproduction table above goes to 0 growth.
- Doc sync: `src/docs/spec/threading/08_queue-semantics.md` (message ownership after
  receive), `02_isolation.md`.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Part B ownership — the reader frees the dequeued block into its own arena (adoption; the
  sender's arena never gets the bytes back, so a long-lived sender feeding short-lived
  workers still grows — only the reader's arena reuses them) vs. returning the block to the
  sender (a sender-side pending-free list drained on the sender's next send/receive, which
  keeps each arena's accounting balanced). Recommended: sender-side return, because the
  common shape is a long-lived parent feeding workers that exit.

## Summary

Part A is a one-predicate ownership fix. Part B is the real design work: a cross-arena
block needs an owner that gives the bytes back to the arena that carved them.
