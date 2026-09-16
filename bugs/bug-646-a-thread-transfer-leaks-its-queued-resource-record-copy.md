# bug-646: thread::transfer leaks the queued copy of the resource record (96 B per transfer)

Last updated: 2026-09-15
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

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

- [ ] Soak tests for a resource transfer and a String send; confirm RED.
- [ ] Localize each block (queued copy, receiver copy) and audit every queue hand-over.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

A thread-runtime ownership gap: the queued copy has no owner once the receiver re-copies it.
The risk is in keeping bug-498's no-cross-arena-allocation rule while giving that copy one.
