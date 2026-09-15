# bug-629: every `thread::send` leaks the message in the sender's arena (the argument temp and the queued copy)

Last updated: 2026-09-14
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1)

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

To be verified by search and measurement in Phase 1; current reading:

- `thread.send`, `thread.emit` data argument claim — Part A, fixed here.
- `thread.transferResource`, `thread.emitResource` — the claim covers them too; a resource
  argument is not a freeable temp, so likely unaffected — verify.
- `thread.start` data argument — the same claim; whether `thread::start` copies its data
  argument or hands the block to the worker decides whether this is the same leak — audit.
- `thread.read`, `thread.receive`, `thread.acceptResource`, `thread.readResource` dequeue
  paths — Part B; the resource plane copies a resource record + STATE — audit.
- Unread messages left in a queue at `thread.drop` / `thread::waitFor` — the queue counts
  are zeroed and the pointers lost (`tw_unread_inbound2`: +64 B per unread send on the
  bug-622 build) — same ownership question, in scope for the design.

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

- [ ] `rt_scope_drop_leaks.rs`: literal and concatenated sends, both directions, N vs 2N on
      the sender arena's `live_bytes`; confirm they fail.
- [ ] Audit every site in Blast Radius with a verdict and a measurement.

Acceptance: the cases fail for the documented reason; every site has a verdict.
Commit: —

### Phase 2 — Part A, the argument temp

- [ ] Narrow `claim_moved_thread_arg_temp` for copying sends
      (`builder_thread_cleanup.rs`).

Acceptance: the concatenated case drops to the literal case's growth; no double free.
Commit: —

### Phase 3 — Part B, the queued copy

- [ ] Owner for the dequeued block (read helper and/or receive call site); unread-queue
      disposal at drop/close.

Acceptance: Phase 1 cases flat; threading suites green.
Commit: —

### Phase 4 — expected outputs + full validation

- [ ] Regenerate shifted goldens (thread helpers' native code sums); full suite;
      `scripts/test-accept.sh`.

Acceptance: full suite green; golden deltas limited to the thread send/read helpers.
Commit: —

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
