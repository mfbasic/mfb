# bug-181 — `thread::receive`'s default `timeoutMs = 0` (non-blocking) makes a worker's natural "wait for a message" loop race the parent's `send`

**Status:** OPEN. Filed 2026-07-13 (observed during plan-15 stdin-broadcast verification).
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

## Fix sketch (options — needs a decision)

1. **Diagnostic (recommended, non-breaking):** emit a lint/warning when
   `thread::receive(self, …)` / a `ThreadWorker`-handle receive is called with no
   explicit timeout, suggesting `-1` (block) or an explicit `0` (poll). Keeps the
   documented default, removes the footgun.
2. **Change the worker-side default to `-1` (blocking):** matches the common intent,
   but is a semantic change to a documented default and could surprise a program that
   deliberately polls with the no-arg form; the parent-side default `0` (poll) should
   stay. Would need spec/man/goldens updates.
3. **Docs-only:** make every worker example in the spec/man use `thread::receive(self,
   -1)` and add an explicit "the no-arg form does not block" caution at each worker
   receive site.

## Notes

- Discovered while writing plan-15 stdin-broadcast fixtures; the fixtures avoid it by
  using `thread::receive(self, -1)`. plan-15 itself does **not** introduce or depend on
  this (the race reproduces with no `openStdIn`/broadcast usage).
- The same non-blocking-default reasoning applies to `thread::accept` (resource plane),
  which shares the `timeoutMs = 0` default; worth checking under the same fix.
