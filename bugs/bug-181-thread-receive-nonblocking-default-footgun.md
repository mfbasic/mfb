# bug-181 — `thread::receive`'s default `timeoutMs = 0` (non-blocking) makes a worker's natural "wait for a message" loop race the parent's `send`

**Status:** FIX DECIDED (2026-07-13). Reshape `receive`/`accept` to two overloads:
the no-arg form **blocks** (new default) and the explicit-timeout form takes a
**non-negative** `timeoutMs`. Symmetric across parent `Thread` and worker
`ThreadWorker`. See **Decision** below.
**Severity:** LOW — documented behavior; no data corruption or leak. Intermittent
`ErrInterrupted`/`ErrNotFound` in programs that rely on `thread::receive(self)`
blocking. Fully avoidable by passing an explicit timeout.
**Class:** footgun / API ergonomics (NOT a correctness defect — the immediate
behavior matches the spec and man page).

## Finding

`thread::receive(t)` with no timeout argument defaults `timeoutMs` to `0`, which is
**non-blocking**: it returns a queued message if one is present and otherwise fails
**immediately** with `ErrNotFound`. This default is documented — `mfb spec threading
queue-semantics` (`src/docs/spec/threading/08_queue-semantics.md:47`: "`timeoutMs = 0`
means non-blocking") and `mfb man thread receive` ("`timeoutMs = 0` (the default) …
does not wait … fails at once with `ErrNotFound`") — and the padding is applied in
`src/target/shared/code/builder_values.rs` (`thread.receive && helper_args.len() == 1
=> push Integer "0"`).

The footgun is the worker-side overload. The natural way to write a worker that waits
for its first message is:

```mfbasic
EXPORT ISOLATED FUNC worker(self AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  LET go AS Integer = thread::receive(self) TRAP(e)   ' <-- non-blocking! returns ErrNotFound at once
    RECOVER 0
  END TRAP
  RETURN n * 2
END FUNC
```

Because the receive does not wait, if the worker reaches it before the parent's
`thread::send` lands, it gets `ErrNotFound`, recovers, and **completes early**. The
parent's subsequent `thread::send(w, …)` then observes the worker as ended and fails
with `ErrInterrupted` (`77050009`) — also documented-correct for `send` to a completed
worker. The two documented behaviors compose into a surprising, timing-dependent
failure of a program the author reasonably expected to work.

## Trigger / repro

A parent that starts a worker and then sends it a message, where the worker's first
act is `thread::receive(self)` with no timeout:

```
worker: LET go = thread::receive(self) TRAP(e) RECOVER 0 END TRAP ; RETURN n*2
parent: w = thread::start(worker, 21, 1, 1) ; thread::send(w, 0) ; thread::waitFor(w)
```

Observed ~2% (≈9/500 runs) `send` failing with `ErrInterrupted` on macOS-aarch64,
**with no stdin/broadcast-log involvement** (so it is unrelated to plan-15). The rate
rises with worker count / added latency between `start` and `send`.

## Root cause

The non-blocking (`timeoutMs = 0`) default is unusual for a message-queue receive —
most queue APIs block by default. For the **worker-side** receive in particular, the
common intent is "wait for work", so the safe form is `thread::receive(self, -1)`
(indefinite), but nothing steers the author there; the ergonomic default silently
does the opposite.

## Decision

Reshape both `thread::receive` and `thread::accept` into **two overloads**, and
apply the identical rule to the parent `Thread` and worker `ThreadWorker` handle so
there is no parent/worker asymmetry:

```
thread::receive(t)              → BLOCK: wait until a message arrives, the queue
                                  closes, or (worker side) the worker is cancelled.
thread::receive(t, timeoutMs)   → TIMED: timeoutMs has no default and must be >= 0.
                                    0 = poll once (ErrNotFound if empty),
                                    N = wait up to N ms (ErrTimeout).
                                    A negative timeoutMs → ErrInvalidArgument.
```

and likewise `thread::accept(t)` / `thread::accept(t, timeoutMs)`.

This removes **both** footguns at once: the accidental non-blocking default that
races `send`, and the parent-vs-worker asymmetry (previously the parent rejected any
negative `timeoutMs` while the worker accepted `-1`). The old `receive(self, -1)` /
`accept(t, -1)` "block forever" idiom is replaced by the no-arg form; an explicit
negative timeout is now an **error** (`ErrInvalidArgument`), consistent with "an
explicit timeout is a non-negative duration."

### Implementation

- **Lowering** (`builder_values.rs`): the no-arg (1-arg) form pads the missing
  `timeoutMs` with an unreachable **block sentinel** — `i64::MIN`, materialized as
  the `u64` bit pattern `9223372036854775808` (the immediate encoder parses `u64`,
  and every valid explicit timeout is `>= 0`, so no user value can collide). Padding
  is added for `thread.acceptResource` (which previously had none) alongside the
  existing `thread.receive` branch. The 2-arg form passes the user value through
  unchanged.
- **Runtime** (`runtime_helpers_thread.rs` / `runtime_helpers.rs`): the shared
  queue-read helper now treats **all** read modes as waitable — it blocks
  indefinitely on the block sentinel and rejects any other negative `timeoutMs` with
  `ErrInvalidArgument`. The former `ParentBounded` mode (parent data `receive`, which
  rejected `-1` and could not wait) collapses into a single `Parent` mode that wakes
  with `ErrNotFound` when the worker completes/closes its outbound queue (the worker
  trampoline already broadcasts that queue's `not_empty` condvar on exit, so a
  blocked parent never deadlocks).
- `thread::send` / `thread::poll` are unchanged (out of scope): `send` keeps its
  `timeoutMs = 0` non-blocking default, `poll` keeps rejecting negatives.

## Notes

- Discovered while writing plan-15 stdin-broadcast fixtures; the fixtures avoided it
  by using `thread::receive(self, -1)` (now spelled `thread::receive(self)`). plan-15
  itself does **not** introduce or depend on this (the race reproduces with no
  `openStdIn`/broadcast usage).
- `thread::accept` (resource plane) shared the same `timeoutMs = 0` default and is
  fixed under the same reshape; its parent overload already permitted the indefinite
  wait, so only the default (block) and the negative-rejection change for it.
