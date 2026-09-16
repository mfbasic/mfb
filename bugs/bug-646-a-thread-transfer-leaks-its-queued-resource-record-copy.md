# bug-646: thread::transfer leaks the queued copy of the resource record (96 B per transfer)

Last updated: 2026-09-15
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: `tests/runtime/rt_debug_soak.rs` —
`a_thread_resource_transfer_loop_keeps_live_bytes_constant`,
`a_thread_string_send_loop_keeps_live_bytes_constant`

> **STATUS: FIXED (e124b7eee)** — the queued copy is now parked on the queue's pending-free
> list and reclaimed by the SENDER, which is the only thread whose bins the memory can return
> to. A ring entry grew from a bare value to a `{value, size}` pair so a type-agnostic reader
> can hand the block back; each read parks the PREVIOUS read's block, which is the earliest
> safe point (the call-site copy of the current one has not run yet). Both shapes report
> `free_calls == alloc_calls`, `live_bytes 0`, `double_free_skips 0`. **Deviation:** the
> release-time drain is restricted to the two INBOUND queues — an intermediate version with
> the parent draining outbound queues was measured to double-free (`free_calls 884 >
> alloc_calls 725` on a worker→parent send loop; restricting it restored 725/725). bug-498's
> rule holds: no thread allocates in another's arena, and none frees into one. Stability: 3
> programs × 20 runs = 60/60 clean, no crash report. **Residuals filed separately:** bug-649
> (the caller's computed argument temp) and bug-650 (undelivered messages, a stateful
> resource's STATE block, and messages whose copy size is not computable).

Every `thread::transfer` of a resource leaves one 96 B block live in the SENDER's arena. A
server that hands each accepted connection to a worker grows without bound. A data message
leaks the same way: 64 B per `thread::send` of a String (subagent measurement, not yet
reproduced on the main thread).

**The single correct behavior a fix produces:** a loop of bind → `thread::start` →
`thread::transfer` → close-in-worker → `thread::waitFor` reports equal main-arena
`live_bytes` at N=30 and N=60.

References:

- bug-498 (`cleanup/thread/builder_thread_cleanup.rs`): the send path deep-copies the message
  into the SENDER's own arena and hands the copy through the queue.
- bug-623 D (`a9a14985e`): the sender's `moved|closed` tombstone record is now freed at the
  sender binding's drop, which halved this loop's leak (192 → 96 B).
- `.ai/canvas-threading.md` §2: a free pushes onto the freeing thread's bins; a worker's
  arena is never unmapped or reused.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT thread

ISOLATED FUNC worker(t AS ThreadWorker OF RES udp::Socket TO Integer, n AS Integer) AS Integer
  RES s AS udp::Socket = thread::accept(t, 20000)
  udp::close(s)
  RETURN 1
END FUNC

SUB main()
  MUT total AS Integer = 0
  FOR i = 1 TO {n}
    RES c AS udp::Socket = udp::bind("127.0.0.1", 0)
    LET a AS Thread OF RES udp::Socket TO Integer = thread::start(worker, 0)
    thread::transfer(a, c)
    total = total + thread::waitFor(a)
  NEXT
  io::print("total=" & toString(total))
END SUB
```

`mfb build --debug`, integration branch after `a9a14985e`, macOS:

- Observed: N=30 `total=30`, `alloc_calls 392`, `free_calls 362`, `live_bytes 2880`; N=60
  `total=60`, `782`/`722`, `live_bytes 5760` — one 96 B block per transfer,
  `double_free_skips 0`.
- Expected: equal `live_bytes` at both N.

Contrast (subagent-measured): udp bind/close with a thread started and waited but no
transfer is flat.

## Phase 1 measurements (main thread, `1963472c6`)

Both halves reproduce, and the `thread::send` figure in the header is corrected:

| loop | N | counters | per operation |
| --- | --- | --- | --- |
| resource `thread::transfer` | 30 / 60 | `392`/`362` `live_bytes 2880`; `782`/`722` `5760` | 96 B |
| resource `thread::transfer` | 100 / 200 | `live_bytes 9600`; `19200` | 96 B |
| String `thread::send` | 30 / 60 | `332`/`302` `live_bytes 960`; `662`/`602` `1920` | **32 B**, not the 64 B estimated |
| String `thread::send` | 200 / 400 | `live_bytes 6400`; `12800` | 32 B |

`double_free_skips 0` throughout.

Measurement trap for the regression tests: at 96 B and 32 B per operation, the N=30/60 counts
in this doc grow 2880 B and 960 B — both **under** `rt_debug_soak.rs`'s `BLOCK_BOUND` (4096),
so a test written to the numbers above passes while the leak is live. The tests run at
100/200 and 200/400.

## Root Cause

From the subagent's analysis, to be confirmed in Phase 1: the send side copies the record
into the sender's arena for the queue (bug-498). The receiver copies it again into its own
arena at `thread::accept`, and nothing frees the queued copy. Freeing it on the receiving
thread would be sound (a free touches only the freeing thread's bins) but would not return
memory to the sender, because a worker's bins die with the worker. Getting the sender's
memory back needs the queued copy parked on the queue's pending-free list, so the spawning
thread frees it at `thread.drop`, or the receiver must stop making a second copy.

## Goal

- The reproduction flat at N and 2N; the same for a String `thread::send` loop.

### Non-goals (must NOT change)

- bug-498's race fix: a sender must never allocate in the receiver's arena.
- A failed transfer still leaves the sender owning its handle.

## Blast Radius

- Every message and resource hand-over through the thread queue: `thread::transfer`,
  `thread::send`, `thread::emit`, thread results (bug-622's copy-back) — audit in Phase 1.

## Phases

### Phase 1 — failing test + audit

- [x] Soak tests for a resource transfer and a String send; confirm RED. (Measurements above;
      the send figure is 32 B, not the estimated 64 B.)
- [x] Localize each block and audit every queue hand-over. Confirmed: the send deep-copies
      into the SENDER's arena (bug-498), the reader copies again at the call site
      (`runtime_call_result_is_copied_at_call_site`), and the queued block then has no owner.

Commit: `60aac1623` (tests), `e124b7eee` (audit)

### Phase 2 — the fix

- [x] Pending-free list drained by the sender, not the receiver. Receiver-adopts and
      receiver-frees were both rejected on the same ground: `arena_free` pushes onto the
      FREEING thread's bins and a worker's bins die with the worker, so either merely moves
      the leak instead of returning the sender's memory.

Commit: `e124b7eee`

### Phase 3 — full validation

- [x] 88 `test-accept.sh` thread fixtures; `rt_thread_accept_res_drop_closes`,
      `rt_recursive_thread_transfer`, `rt_thread_send_cross_arena`,
      `rt_native_size_arith_overflow` — 8 passed, including both cross-arena race tests and
      both queue-limit boundary tests.
- [x] 20× stability loop over three programs (resource transfer, string send, and an 8-deep
      bidirectional ping-pong): 60/60 clean, `live_bytes 0` and `double_free_skips 0` every
      run, and no new macOS crash report.
- [x] Spec synced (`threading/07_control-block.md`: the queue record was already documented
      stale at 240 B with a `capacity * 8` array); full suite — see the integration commit.

Commit: `e124b7eee`

## Summary

A thread-runtime ownership gap: the queued copy has no owner once the receiver re-copies it.
The risk is in keeping bug-498's no-cross-arena-allocation rule while giving that copy one.
