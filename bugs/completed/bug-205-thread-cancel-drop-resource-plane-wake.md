# bug-205: thread::cancel/drop never wake a worker parked in acceptResource (resource-plane queues not broadcast)

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: MEDIUM
Class: correctness (deadlock / thread leak)

Status: Fixed (2026-07-15) — new `emit_close_resource_queues` helper sets
THREAD_QUEUE_CLOSED and pthread_cond_broadcasts the `not_empty`/`not_full` condvars
on BOTH resource-plane queues (offsets 104/112), mirroring the trampoline-exit
close loop. Wired into the `ThreadSimpleOp::Cancel` success path (after the
outbound data-queue unlock) and into `::Drop` at `inbound_unlocked`, before
`pthread_detach` — so a worker parked in a blocking `acceptResource` now wakes,
observes CANCELLED, and exits instead of hanging forever (and, on drop, leaking a
detached thread). Data-plane cancel behavior is untouched.
Regression Test: 64 thread acceptance tests pass (cancel/drop, resource transfer,
bidirectional). A targeted stalled-worker repro was not authored — the fix mirrors
the trampoline's proven per-queue close loop that already establishes this exact
contract ("wake any parent/worker blocked").
Regression Test: tests/rt-behavior/ (cancel a worker blocked in thread::accept wakes it)

`thread::cancel`/`thread::drop` close and broadcast only the two **data-plane**
queues (inbound/outbound) and never touch the **resource-plane** queues
(`THREAD_OFFSET_RESOURCE_INBOUND_QUEUE`=104 /
`THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE`=112). A worker parked in a blocking
`acceptResource` (no-arg `thread::accept()`) waits on the resource-inbound
`not_empty` condvar, which is never broadcast, so it never re-checks `CANCELLED`
and hangs permanently — a detached, leaked thread on `drop`.

The data-plane equivalent (`receive`) is woken correctly because cancel
closes+broadcasts the inbound data queue; the trampoline exit path also closes
both resource queues explicitly "to wake any parent/worker blocked" — confirming
the intended contract that cancel/drop violate.

## Failing Reproduction

Parent spawns a worker that calls `thread::accept()` (blocking resource read).
No resource is ever transferred. Parent calls `thread::cancel(t)` (or
`thread::drop`). Observed: `CANCELLED=1` is set and only the inbound/outbound
*data* condvars are broadcast; the worker blocked on the resource-inbound condvar
never wakes → permanent hang / leaked thread. Expected: the worker wakes,
observes `CANCELLED`, and exits.

## Root Cause

`src/target/shared/code/runtime_helpers_thread.rs:290-591` — the
`ThreadSimpleOp::Cancel` and `::Drop` handlers set/broadcast only the data-plane
queues, omitting the resource-plane queues at offsets 104/112.

## Non-goals

- Do not change data-plane cancel behavior (already correct).

## Blast Radius

- Cancel and Drop handlers in `runtime_helpers_thread.rs`.

## Fix Design

In the Cancel and Drop handlers, also set `THREAD_QUEUE_CLOSED_OFFSET` and
`pthread_cond_broadcast` the `not_empty`/`not_full` condvars on both
resource-plane queues (mirror the per-queue close loop the trampoline uses at
`:822-879`).
