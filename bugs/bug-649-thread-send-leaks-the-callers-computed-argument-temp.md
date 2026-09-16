# bug-649: `thread::send` of a computed value leaks the caller's argument temp (16 B per send)

Last updated: 2026-09-15
Effort: small (< 1h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`thread::send(a, "msg-" & toString(k))` leaves the caller's own argument temp live, 16 B per
send. Distinct from bug-646: that was the QUEUE's copy of the message (now reclaimed by the
sender); this is the temp the caller built to pass in, which the send helper deep-copies and
then never releases. A message that is a plain literal is flat.

**The single correct behavior a fix produces:** a loop of `thread::send` with a computed
String argument reports equal `live_bytes` at N and 2N, with `double_free_skips 0`.

References:

- Found by bug-646's fix work (subagent report), measured on both sides of that fix.
- `builder_thread_cleanup.rs::claim_moved_thread_arg_temp`.

## Failing Reproduction

```
IMPORT io
IMPORT thread

ISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO 8
    LET m AS String = thread::receive(t, 20000)
    total = total + len(m)
  NEXT
  RETURN total
END FUNC

SUB main()
  MUT total AS Integer = 0
  FOR i = 1 TO {n}
    LET a AS Thread OF String TO Integer = thread::start(worker, 0)
    FOR k = 1 TO 8
      thread::send(a, "msg-" & toString(k))
    NEXT
    total = total + thread::waitFor(a)
  NEXT
  io::print("total=" & toString(total))
END SUB
```

`mfb build --debug`, 40 outer iterations × 8 sends:

- Observed at `1963472c6` (before bug-646): `alloc 1362`, `free 722`, `live_bytes 10240` —
  two blocks per send. After bug-646's fix: `1362`/`1042`, `live_bytes 5120` — one block per
  send, 16 B. bug-646 reclaimed the queue's copy; this is the other one.
- Expected: equal `live_bytes` at both N.

Contrast: a literal message (`thread::send(a, "hello-world")`) is flat at both counts — that
is bug-646's own regression test, which is green.

Pick N so the growth clears `rt_debug_soak.rs`'s `BLOCK_BOUND` (4096): at 16 B per send,
40×8 = 320 sends grows 5120 B, so 40/80 outer iterations suffices.

## Root Cause

To confirm in Phase 1. `claim_moved_thread_arg_temp` (`builder_thread_cleanup.rs`) pops
`thread.send`/`thread.emit`'s argument-1 off the pending-temp list, commented as
"conservatively preserving the pre-plan-25 behaviour" — i.e. it treats the argument as handed
over. But `emit_thread_send_runtime_helper_call` **unconditionally deep-copies** that argument
into the queue block, so the original temp is never referenced by the queue and nothing
releases it.

Candidate fix from the reporting agent: drop `"thread.send" | "thread.emit"` from that
`matches!`, keeping `thread.start` (whose data argument really IS handed over by pointer) and
the resource arms. Confirm against the resource arms before applying — a resource argument is
a different ownership story.

## Goal

- The reproduction flat at N and 2N; the literal-message and `thread::start` shapes stay flat.

### Non-goals (must NOT change)

- `thread::start`'s data argument, which is handed over by pointer.
- The resource arms of `claim_moved_thread_arg_temp`.
- bug-646's pending-free reclaim, and bug-498's no-cross-arena rule.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak test for the computed-argument shape; confirm RED; confirm the `matches!` arm is
      the cause and that the resource arms are unaffected.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

An argument treated as moved that the callee actually copies.
